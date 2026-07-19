use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::domain::{
    AddMemoryInput, Termbase, TranslationContext, ValidationFinding, WorkspaceStore,
};

use super::IngestConfig;

pub type Result<T> = std::result::Result<T, IngestError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngestError {
    #[error("{0}")]
    Validation(String),
    #[error("ingest.conf is not valid TOML: {0}")]
    InvalidToml(#[source] toml::de::Error),
    #[error("could not serialize ingest.conf: {0}")]
    Serialize(#[source] toml::ser::Error),
    #[error("unsafe ingest configuration path `{path}`: {reason}")]
    UnsafePath {
        path: std::path::PathBuf,
        reason: &'static str,
    },
    #[error("could not access ingest configuration `{path}`: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Workspace(#[from] crate::domain::WorkspaceError),
    #[error(transparent)]
    Termbase(#[from] crate::domain::TermbaseError),
    #[error("read ingestion requires an active session: {session_id}")]
    SessionInactive { session_id: i64 },
    #[error("session {session_id} has mismatched {field}")]
    SessionContext {
        session_id: i64,
        field: &'static str,
    },
    #[error("ingestion database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnInput {
    pub text: String,
    pub source_role: String,
    pub source_ref: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnResult {
    pub memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadInput {
    pub text: String,
    pub source_ref: Option<String>,
    pub store_observation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub candidate_terms: Vec<String>,
    pub findings: Vec<ValidationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningBlock {
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct IngestionService<'a> {
    pool: &'a SqlitePool,
    config: IngestConfig,
}

impl<'a> IngestionService<'a> {
    pub fn new(pool: &'a SqlitePool, config: IngestConfig) -> Result<Self> {
        let config = config.validate()?;
        WorkspaceStore::with_limits(pool, config.short_memory)?;
        Ok(Self { pool, config })
    }

    pub async fn learn(&self, session_id: i64, input: LearnInput) -> Result<LearnResult> {
        let blocks = split_blocks(&input.text, self.config.learn.max_block_chars)?;
        if blocks.is_empty() {
            return Ok(LearnResult {
                memory_ids: Vec::new(),
            });
        }
        let block_count = blocks.len();
        let memories = blocks
            .into_iter()
            .enumerate()
            .map(|(index, block)| AddMemoryInput {
                source_role: input.source_role.clone(),
                kind: input.kind.clone(),
                text: block.text,
                source_ref: input.source_ref.clone().unwrap_or_default(),
                metadata: serde_json::json!({
                    "ingestion_mode": "learn",
                    "block_index": index + 1,
                    "block_count": block_count,
                }),
                ..AddMemoryInput::default()
            })
            .collect::<Vec<_>>();
        let stored = WorkspaceStore::with_limits(self.pool, self.config.short_memory)?
            .add_short_term_batch(session_id, &memories)
            .await?;
        Ok(LearnResult {
            memory_ids: stored.into_iter().map(|result| result.memory.id).collect(),
        })
    }

    pub async fn read(&self, session_id: i64, input: ReadInput) -> Result<ReadResult> {
        let workspace = WorkspaceStore::with_limits(self.pool, self.config.short_memory)?;
        let session = workspace.get_session(session_id).await?;
        if session.status != "active" {
            return Err(IngestError::SessionInactive { session_id });
        }
        let context = self.session_context(&session).await?;
        let candidate_terms = extract_terms(&input.text);
        let findings = Termbase::new(self.pool, context.clone())
            .validate(&input.text, Some(&input.text), None)
            .await?;
        if input.store_observation {
            workspace
                .add_short_term(
                    session_id,
                    AddMemoryInput {
                        source_role: "observation".into(),
                        kind: "read_observation".into(),
                        text: input.text,
                        source_ref: input.source_ref.unwrap_or_default(),
                        metadata: serde_json::json!({
                            "ingestion_mode": "read",
                            "candidate_terms": candidate_terms,
                        }),
                        language_tags: context.language_tags,
                        story_scopes: context.story_scopes,
                        semantic_tags: context.semantic_tags,
                        ..AddMemoryInput::default()
                    },
                )
                .await?;
        }
        Ok(ReadResult {
            candidate_terms,
            findings,
        })
    }

    async fn session_context(
        &self,
        session: &crate::domain::TaskSession,
    ) -> Result<TranslationContext> {
        let series = sqlx::query(
            "SELECT default_source_language, default_target_language FROM series WHERE slug = ?",
        )
        .bind(&session.series_slug)
        .fetch_optional(self.pool)
        .await?
        .ok_or(IngestError::SessionContext {
            session_id: session.id,
            field: "series_slug",
        })?;
        let source: String = series.try_get("default_source_language")?;
        let target: String = series.try_get("default_target_language")?;
        for (field, matches) in [
            (
                "source_language",
                source.to_lowercase() == session.source_language,
            ),
            (
                "target_language",
                target.to_lowercase() == session.target_language,
            ),
            (
                "language_tags",
                session.language_tags.contains(&session.source_language)
                    && session.language_tags.contains(&session.target_language),
            ),
        ] {
            if !matches {
                return Err(IngestError::SessionContext {
                    session_id: session.id,
                    field,
                });
            }
        }
        Ok(TranslationContext::new(
            &session.series_slug,
            &session.source_language,
            &session.target_language,
        )
        .with_metadata(
            &session.language_tags,
            &session.story_scopes,
            &session.semantic_tags,
            &[],
        ))
    }
}

pub fn split_blocks(text: &str, max_chars: usize) -> Result<Vec<LearningBlock>> {
    if max_chars == 0 {
        return Err(IngestError::Validation("max_chars must be positive".into()));
    }
    let paragraph_boundary =
        regex::Regex::new(r"\n\s*\n").expect("static paragraph regex is valid");
    let paragraphs = paragraph_boundary
        .split(text)
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let boundary = Regex::new(r"([.!?。！？])\s+").expect("static sentence regex is valid");
    let mut block_texts = Vec::new();
    for paragraph in paragraphs {
        if paragraph.chars().count() <= max_chars {
            block_texts.push(paragraph.to_owned());
            continue;
        }
        let mut sentences = Vec::new();
        let mut start = 0;
        for capture in boundary.captures_iter(paragraph) {
            let matched = capture.get(0).expect("whole regex match exists");
            sentences.push(paragraph[start..matched.end()].trim().to_owned());
            start = matched.end();
        }
        if start < paragraph.len() {
            sentences.push(paragraph[start..].trim().to_owned());
        }
        let mut current = String::new();
        for sentence in sentences
            .into_iter()
            .filter(|sentence| !sentence.is_empty())
        {
            if sentence.chars().count() > max_chars {
                if !current.is_empty() {
                    block_texts.push(std::mem::take(&mut current));
                }
                block_texts.extend(split_oversized(&sentence, max_chars));
                continue;
            }
            let candidate_chars = current.chars().count()
                + usize::from(!current.is_empty())
                + sentence.chars().count();
            if candidate_chars > max_chars && !current.is_empty() {
                block_texts.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&sentence);
        }
        if !current.is_empty() {
            block_texts.push(current);
        }
    }
    Ok(block_texts
        .into_iter()
        .map(|text| LearningBlock { text })
        .collect())
}

fn split_oversized(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > max_chars {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let characters = word.chars().collect::<Vec<_>>();
            chunks.extend(
                characters
                    .chunks(max_chars)
                    .map(|chunk| chunk.iter().collect()),
            );
            continue;
        }
        let candidate_chars =
            current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
        if candidate_chars > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        let characters = text.chars().collect::<Vec<_>>();
        chunks.extend(
            characters
                .chunks(max_chars)
                .map(|chunk| chunk.iter().collect()),
        );
    }
    chunks
}

#[must_use]
pub fn extract_terms(text: &str) -> Vec<String> {
    let pattern =
        Regex::new(r"\b[A-Z][A-Za-z0-9'_-]{2,}\b").expect("static candidate-term regex is valid");
    let mut seen = std::collections::HashSet::new();
    pattern
        .find_iter(text)
        .map(|matched| matched.as_str().to_owned())
        .filter(|term| seen.insert(term.clone()))
        .collect()
}

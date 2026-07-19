mod candidates;
mod graph;
mod ranking;

use std::sync::{Arc, RwLock};

use chrono::Utc;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::domain::{
    FeedbackEvent, FeedbackStore, StoreError, TranslationContext, WorkspaceError, WorkspaceStore,
};
use crate::{
    rag::{RagStore, RetrievalMode, SearchOptions},
    semantic::{EmbeddingProvider, SemanticDiagnostic, SemanticIndex},
};

use self::{
    candidates::{collect_base_candidates, long_term_candidate_limit},
    graph::expand_one_hop,
    ranking::{rank_and_limit, score_base_candidates},
};

pub use crate::domain::{MemorySource, RecallResult};

pub const RULE_INTENT_BOOST: f64 = 0.20;
pub const SPREADING_ACTIVATION_THRESHOLD: f64 = 0.55;
pub const SPREADING_ATTENUATION: f64 = 0.5;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecallError {
    #[error("invalid recall {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("recall requires an active session: {session_id}")]
    SessionInactive { session_id: i64 },
    #[error("recall session {session_id} does not match supplied context field {field}")]
    SessionContext {
        session_id: i64,
        field: &'static str,
    },
    #[error(transparent)]
    Crystal(#[from] StoreError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("recall feedback logging failed: {source}")]
    Feedback {
        #[source]
        source: crate::domain::FeedbackError,
    },
    #[error("recall {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("RAG recall failed: {0}")]
    Rag(#[from] crate::rag::RagError),
}

pub type Result<T> = std::result::Result<T, RecallError>;

pub struct RecallService<'a> {
    pool: &'a SqlitePool,
    semantic: Option<(Arc<dyn EmbeddingProvider>, Arc<dyn SemanticIndex>)>,
    semantic_diagnostic: Arc<RwLock<SemanticDiagnostic>>,
}

impl<'a> RecallService<'a> {
    #[must_use]
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self {
            pool,
            semantic: None,
            semantic_diagnostic: Arc::new(RwLock::new(SemanticDiagnostic::degraded(
                "semantic retrieval is not configured",
            ))),
        }
    }

    #[must_use]
    pub fn with_semantic(
        pool: &'a SqlitePool,
        embedding: Arc<dyn EmbeddingProvider>,
        index: Arc<dyn SemanticIndex>,
    ) -> Self {
        Self {
            pool,
            semantic: Some((embedding, index)),
            semantic_diagnostic: Arc::new(RwLock::new(SemanticDiagnostic::healthy())),
        }
    }

    #[must_use]
    pub fn semantic_diagnostic(&self) -> SemanticDiagnostic {
        self.semantic_diagnostic.read().map_or_else(
            |_| SemanticDiagnostic::degraded("semantic diagnostic lock is poisoned"),
            |value| value.clone(),
        )
    }

    pub async fn recall(
        &self,
        session_id: i64,
        ctx: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallResult>> {
        validate_limit(limit)?;
        validate_session(&WorkspaceStore::new(self.pool), session_id, ctx).await?;
        let candidate_limit = long_term_candidate_limit(limit);
        let base =
            collect_base_candidates(self.pool, session_id, ctx, query, candidate_limit).await?;
        let rag_store = self.semantic.as_ref().map_or_else(
            || RagStore::new(self.pool),
            |(embedding, index)| {
                RagStore::with_semantic(self.pool, embedding.clone(), index.clone())
            },
        );
        let rag = rag_store
            .search(
                &ctx.series_slug,
                query,
                SearchOptions {
                    limit: candidate_limit,
                    language_tags: ctx.language_tags.clone(),
                    story_scopes: ctx.story_scopes.clone(),
                    semantic_tags: ctx.semantic_tags.clone(),
                    retrieval_mode: if self.semantic.is_some() {
                        RetrievalMode::Hybrid
                    } else {
                        RetrievalMode::Lexical
                    },
                },
            )
            .await?;
        if self.semantic.is_some()
            && let Ok(mut diagnostic) = self.semantic_diagnostic.write()
        {
            *diagnostic = rag_store.semantic_diagnostic();
        }
        let scored = score_base_candidates(
            ctx,
            base.into_iter()
                .chain(rag.into_iter().map(candidates::Candidate::rag))
                .collect(),
        );
        let expanded = expand_one_hop(self.pool, ctx, &scored, candidate_limit).await?;
        let ranked = rank_and_limit(scored.into_iter().chain(expanded), limit);

        persist_recall_side_effects(self.pool, session_id, ctx, query, &ranked).await?;
        Ok(ranked
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| candidate.into_result(index + 1))
            .collect())
    }
}

async fn validate_session(
    workspace: &WorkspaceStore<'_>,
    session_id: i64,
    ctx: &TranslationContext,
) -> Result<()> {
    let session = workspace.get_session(session_id).await?;
    if session.status != "active" {
        return Err(RecallError::SessionInactive { session_id });
    }
    for (field, matches) in [
        ("series_slug", session.series_slug == ctx.series_slug),
        (
            "scope_key",
            ctx.scope_key == format!("series:{}", ctx.series_slug),
        ),
        (
            "source_language",
            session.source_language == ctx.source_language,
        ),
        (
            "target_language",
            session.target_language == ctx.target_language,
        ),
        ("language_tags", session.language_tags == ctx.language_tags),
        ("story_scopes", session.story_scopes == ctx.story_scopes),
        ("semantic_tags", session.semantic_tags == ctx.semantic_tags),
    ] {
        if !matches {
            return Err(RecallError::SessionContext { session_id, field });
        }
    }
    Ok(())
}

async fn persist_recall_side_effects(
    pool: &SqlitePool,
    session_id: i64,
    ctx: &TranslationContext,
    query: &str,
    candidates: &[candidates::Candidate],
) -> Result<()> {
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|source| database("begin recall side effects", source))?;
    let result = async {
        let session = revalidate_session_in(&mut transaction, session_id, ctx).await?;
        let workspace = WorkspaceStore::new(pool);
        let feedback = FeedbackStore::new(pool);
        let now = Utc::now();
        for (index, candidate) in candidates.iter().enumerate() {
            let Some(crystal) = candidate.crystal.as_ref() else {
                continue;
            };
            let (_, created) = workspace
                .get_or_create_working_copy_in(&mut transaction, session_id, crystal)
                .await?;
            if !created {
                feedback
                    .record_in(
                        &mut transaction,
                        FeedbackEvent {
                            crystal_id: crystal.id,
                            event_type: "recalled_again".to_owned(),
                            source_role: "recall".to_owned(),
                            evidence: None,
                            session_id: Some(session_id),
                        },
                    )
                    .await
                    .map_err(|source| RecallError::Feedback { source })?;
            }
            sqlx::query("INSERT INTO crystal_activations(crystal_id, session_id, recall_query, rank, score, reason, cycle_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(crystal.id)
                .bind(session_id)
                .bind(query)
                .bind(i64::try_from(index + 1).expect("recall limit fits in i64"))
                .bind(candidate.score)
                .bind(&candidate.reason)
                .bind(session.cycle_id)
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("insert activation", source))?;
        }
        Ok(())
    }
    .await;
    commit_write(transaction, result).await
}

async fn revalidate_session_in(
    transaction: &mut Transaction<'static, Sqlite>,
    session_id: i64,
    ctx: &TranslationContext,
) -> Result<crate::db::TaskSessionRecord> {
    let session = sqlx::query_as::<_, crate::db::TaskSessionRecord>(
        "SELECT id, series_slug, source_language, target_language, task_type, volume, chapter, status, cycle_id, created_at, last_activity_at, completed_at FROM task_sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|source| database("revalidate recall session", source))?
    .ok_or(WorkspaceError::SessionNotFound { id: session_id })?;
    if session.status != "active" {
        return Err(RecallError::SessionInactive { session_id });
    }
    for (field, matches) in [
        ("series_slug", session.series_slug == ctx.series_slug),
        (
            "scope_key",
            ctx.scope_key == format!("series:{}", ctx.series_slug),
        ),
        (
            "source_language",
            session.source_language == ctx.source_language,
        ),
        (
            "target_language",
            session.target_language == ctx.target_language,
        ),
        (
            "language_tags",
            session_metadata_in(
                transaction,
                session_id,
                "task_session_language_tags",
                "language_tag",
            )
            .await?
                == ctx.language_tags,
        ),
        (
            "story_scopes",
            session_metadata_in(
                transaction,
                session_id,
                "task_session_story_scopes",
                "story_scope",
            )
            .await?
                == ctx.story_scopes,
        ),
        (
            "semantic_tags",
            session_metadata_in(
                transaction,
                session_id,
                "task_session_semantic_tags",
                "semantic_tag",
            )
            .await?
                == ctx.semantic_tags,
        ),
    ] {
        if !matches {
            return Err(RecallError::SessionContext { session_id, field });
        }
    }
    Ok(session)
}

async fn session_metadata_in(
    transaction: &mut Transaction<'static, Sqlite>,
    session_id: i64,
    table: &'static str,
    column: &'static str,
) -> Result<Vec<String>> {
    let mut query = sqlx::QueryBuilder::<Sqlite>::new(format!(
        "SELECT {column} AS value FROM {table} WHERE session_id = "
    ));
    query.push_bind(session_id).push(" ORDER BY rowid");
    query
        .build()
        .fetch_all(&mut **transaction)
        .await
        .map_err(|source| database("revalidate recall session metadata", source))?
        .into_iter()
        .map(|row| {
            sqlx::Row::try_get(&row, "value")
                .map_err(|source| database("decode recall session metadata", source))
        })
        .collect()
}

async fn commit_write<T>(
    transaction: Transaction<'static, Sqlite>,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => transaction
            .commit()
            .await
            .map(|()| value)
            .map_err(|source| database("commit recall side effects", source)),
        Err(error) => Err(error),
    }
}

fn validate_limit(limit: usize) -> Result<()> {
    if limit == 0 {
        Err(RecallError::Validation {
            field: "limit",
            reason: "must be at least 1".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn database(operation: &'static str, source: sqlx::Error) -> RecallError {
    RecallError::Database { operation, source }
}

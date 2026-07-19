use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, RwLock},
};

use chrono::Utc;
use icu_casemap::CaseMapper;
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use crate::{
    db::RagChunkRecord as RawRagChunkRecord,
    domain::search_expression,
    recall::MemorySource,
    semantic::{
        EmbeddingProvider, SearchFilter, SemanticDiagnostic, SemanticIndex, reciprocal_rank_fusion,
    },
};

use super::{
    ImportOptions, RagChunkRecord, RagError, RagImportResult, RagSearchResult, RagSourceRecord,
    RetrievalMode, SearchOptions, SourceType, conversion::prepare_rag_source,
    parsing::load_rag_bytes,
};

const MAX_SEARCH_LIMIT: usize = 50;

pub struct RagStore<'a> {
    pool: &'a SqlitePool,
    semantic: Option<SemanticRuntime>,
    semantic_diagnostic: Arc<RwLock<SemanticDiagnostic>>,
}

#[derive(Clone)]
struct SemanticRuntime {
    embedding: Arc<dyn EmbeddingProvider>,
    index: Arc<dyn SemanticIndex>,
}

impl<'a> RagStore<'a> {
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
            semantic: Some(SemanticRuntime { embedding, index }),
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

    pub async fn import_file(
        &self,
        series_slug: &str,
        path: &Path,
        options: ImportOptions,
    ) -> Result<RagImportResult, RagError> {
        let managed = options
            .managed_root
            .clone()
            .unwrap_or_else(super::default_managed_root);
        let requested = options.source_type.unwrap_or(SourceType::Auto);
        let parse_path = path.to_owned();
        let parsed = tokio::task::spawn_blocking(move || {
            let prepared = prepare_rag_source(&parse_path, &managed)?;
            let source_type = if prepared.normalized.path == prepared.normalized.original_path {
                requested
            } else {
                SourceType::Auto
            };
            let parsed = load_rag_bytes(&prepared.normalized.path, &prepared.bytes, source_type)?;
            Ok::<_, RagError>((prepared.normalized, parsed))
        })
        .await
        .map_err(RagError::Join)??;
        let (normalized, parsed) = parsed;
        let path = normalized.original_path.clone();
        let source_ref = options
            .source_ref
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let language_tags = clean(&options.language_tags);
        let story_scopes = clean(&options.story_scopes);
        let semantic_tags = clean(&options.semantic_tags);
        let metadata = BTreeMap::from([
            (
                "original_path".into(),
                serde_json::Value::String(normalized.original_path.to_string_lossy().into_owned()),
            ),
            (
                "normalized_path".into(),
                serde_json::Value::String(normalized.path.to_string_lossy().into_owned()),
            ),
            (
                "normalized_format".into(),
                serde_json::Value::String(normalized.format.clone()),
            ),
        ]);
        let mut connection = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = self
            .persist(
                &mut connection,
                series_slug,
                &source_ref,
                &normalized,
                &parsed,
                &metadata,
                &language_tags,
                &story_scopes,
                &semantic_tags,
            )
            .await;
        match result {
            Ok(result) => {
                connection.commit().await?;
                Ok(result)
            }
            // SQLx queues rollback on drop, including future cancellation.
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "transaction payload is explicit and cohesive"
    )]
    async fn persist(
        &self,
        connection: &mut Transaction<'_, Sqlite>,
        series_slug: &str,
        source_ref: &str,
        normalized: &super::NormalizedRagSource,
        parsed: &super::ParsedRagFile,
        metadata: &BTreeMap<String, serde_json::Value>,
        language_tags: &[String],
        story_scopes: &[String],
        semantic_tags: &[String],
    ) -> Result<RagImportResult, RagError> {
        let existing = sqlx::query("SELECT id, source_type, content_type, checksum, metadata_json FROM rag_sources WHERE series_slug = ? AND source_ref = ?")
            .bind(series_slug).bind(source_ref).fetch_optional(&mut **connection).await?;
        if let Some(row) = &existing {
            if row.get::<String, _>("checksum") == parsed.checksum
                && row.get::<String, _>("source_type") == parsed.source_type.persisted()
                && row.get::<String, _>("content_type") == parsed.content_type
            {
                let id: i64 = row.get("id");
                sqlx::query("UPDATE rag_sources SET metadata_json = ?, updated_at = ? WHERE id = ? AND series_slug = ?")
                    .bind(serde_json::to_string(metadata)?).bind(Utc::now()).bind(id).bind(series_slug).execute(&mut **connection).await?;
                let ids: Vec<i64> = sqlx::query_scalar(
                    "SELECT id FROM rag_chunks WHERE source_id = ? AND series_slug = ? ORDER BY id",
                )
                .bind(id)
                .bind(series_slug)
                .fetch_all(&mut **connection)
                .await?;
                replace_tags(connection, &ids, language_tags, story_scopes, semantic_tags).await?;
                return Ok(result_from(
                    id,
                    series_slug,
                    source_ref,
                    parsed,
                    metadata.clone(),
                    ids.len(),
                    true,
                    normalized,
                ));
            }
            let old_source_id = row.get::<i64, _>("id");
            sqlx::query("DELETE FROM semantic_chunk_state WHERE chunk_id IN (SELECT id FROM rag_chunks WHERE source_id = ? AND series_slug = ?)")
                .bind(old_source_id).bind(series_slug).execute(&mut **connection).await?;
            sqlx::query("DELETE FROM rag_sources WHERE id = ? AND series_slug = ?")
                .bind(old_source_id)
                .bind(series_slug)
                .execute(&mut **connection)
                .await?;
        }
        let now = Utc::now();
        let metadata_json = serde_json::to_string(metadata)?;
        let source_id = sqlx::query("INSERT INTO rag_sources(series_slug, source_ref, source_type, content_type, checksum, metadata_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id")
            .bind(series_slug).bind(source_ref).bind(parsed.source_type.persisted()).bind(&parsed.content_type).bind(&parsed.checksum).bind(metadata_json).bind(now).bind(now)
            .fetch_one(&mut **connection).await?.get("id");
        let mut ids = Vec::with_capacity(parsed.chunks.len());
        for chunk in &parsed.chunks {
            let id: i64 = sqlx::query("INSERT INTO rag_chunks(source_id, series_slug, chunk_kind, text, display_text, location, metadata_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id")
                .bind(source_id).bind(series_slug).bind(&chunk.chunk_kind).bind(&chunk.text).bind(&chunk.display_text).bind(&chunk.location).bind(serde_json::to_string(&chunk.metadata)?).bind(now)
                .fetch_one(&mut **connection).await?.get("id");
            ids.push(id);
        }
        replace_tags(connection, &ids, language_tags, story_scopes, semantic_tags).await?;
        sqlx::query("INSERT INTO semantic_index_jobs(status, created_at) VALUES ('pending', ?)")
            .bind(now)
            .execute(&mut **connection)
            .await?;
        Ok(result_from(
            source_id,
            series_slug,
            source_ref,
            parsed,
            metadata.clone(),
            ids.len(),
            false,
            normalized,
        ))
    }

    pub async fn search(
        &self,
        series_slug: &str,
        query: &str,
        options: SearchOptions,
    ) -> Result<Vec<RagSearchResult>, RagError> {
        if options.limit == 0 {
            return Err(RagError::InvalidLimit);
        }
        let mode = options.retrieval_mode;
        let lexical = self.lexical_search(series_slug, query, &options).await?;
        if mode == RetrievalMode::Lexical {
            return Ok(lexical);
        }
        let Some(semantic) = &self.semantic else {
            self.set_semantic_degraded("semantic retrieval is not configured");
            return Ok(lexical);
        };
        let vector_results = match semantic.embedding.embed_query(query).await {
            Ok(vector) => {
                semantic
                    .index
                    .search(
                        &vector,
                        &SearchFilter {
                            series_slug: series_slug.to_owned(),
                        },
                        options.limit.min(MAX_SEARCH_LIMIT),
                    )
                    .await
            }
            Err(error) => {
                self.set_semantic_degraded(error.to_string());
                return Ok(lexical);
            }
        };
        let vector_results = match vector_results {
            Ok(results) => results,
            Err(error) => {
                self.set_semantic_degraded(error.to_string());
                return Ok(lexical);
            }
        };
        self.set_semantic_healthy();
        let lexical_lane = lexical
            .iter()
            .map(|result| (result.chunk.id, result.score))
            .collect::<Vec<_>>();
        let vector_lane = vector_results
            .iter()
            .map(|result| (result.chunk_id, result.distance))
            .collect::<Vec<_>>();
        let fused = reciprocal_rank_fusion(&lexical_lane, &vector_lane, 60.0);
        let ids = fused.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let chunks = self.load_chunks(series_slug, &ids, &options).await?;
        let by_id = chunks
            .into_iter()
            .map(|chunk| (chunk.id, chunk))
            .collect::<HashMap<_, _>>();
        Ok(fused
            .into_iter()
            .filter_map(|(id, score)| {
                by_id.get(&id).cloned().map(|chunk| RagSearchResult {
                    chunk,
                    score,
                    source: MemorySource::Rag,
                    reason: "rag hybrid reciprocal-rank fusion".into(),
                })
            })
            .take(options.limit.min(MAX_SEARCH_LIMIT))
            .collect())
    }

    async fn lexical_search(
        &self,
        series_slug: &str,
        query: &str,
        options: &SearchOptions,
    ) -> Result<Vec<RagSearchResult>, RagError> {
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(vec![]);
        }
        let language = clean(&options.language_tags);
        let stories = clean(&options.story_scopes);
        let semantic = clean(&options.semantic_tags);
        let language_json = serde_json::to_string(&language)?;
        let story_json = serde_json::to_string(&stories)?;
        let semantic_json = serde_json::to_string(&semantic)?;
        let rows = sqlx::query_as::<_, SearchRow>(SEARCH_SQL)
            .bind(language_json)
            .bind(story_json)
            .bind(semantic_json)
            .bind(expression)
            .bind(series_slug)
            .bind(options.limit.min(MAX_SEARCH_LIMIT) as i64)
            .fetch_all(self.pool)
            .await?;
        let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        let tags = hydrate_tags(self.pool, &ids).await?;
        rows.into_iter()
            .map(|row| {
                let metadata = serde_json::from_str(&row.metadata_json)?;
                let kind = row.chunk_kind.clone();
                let values = tags.get(&row.id).cloned().unwrap_or_default();
                Ok(RagSearchResult {
                    chunk: RagChunkRecord {
                        record: row.clone().into_raw(),
                        source_ref: row.source_ref,
                        metadata,
                        language_tags: values.0,
                        story_scopes: values.1,
                        semantic_tags: values.2,
                    },
                    score: row.score,
                    source: MemorySource::Rag,
                    reason: if kind == "glossary_entry" {
                        "rag glossary match".into()
                    } else if kind == "markdown_section" {
                        "rag markdown section match".into()
                    } else {
                        "rag project text match".into()
                    },
                })
            })
            .collect()
    }

    async fn load_chunks(
        &self,
        series_slug: &str,
        ids: &[i64],
        options: &SearchOptions,
    ) -> Result<Vec<RagChunkRecord>, RagError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut builder = QueryBuilder::new(
            "SELECT c.*, s.source_ref FROM rag_chunks c JOIN rag_sources s ON s.id = c.source_id AND s.series_slug = c.series_slug WHERE c.series_slug = ",
        );
        builder.push_bind(series_slug).push(" AND c.id IN (");
        let mut separated = builder.separated(",");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let rows = builder
            .build_query_as::<SearchChunkRow>()
            .fetch_all(self.pool)
            .await?;
        let loaded_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
        let tags = hydrate_tags(self.pool, &loaded_ids).await?;
        let language = clean(&options.language_tags);
        let stories = clean(&options.story_scopes);
        let semantic = clean(&options.semantic_tags);
        rows.into_iter()
            .filter_map(|row| {
                let values = tags.get(&row.id).cloned().unwrap_or_default();
                if (!language.is_empty() && !values.0.iter().any(|value| language.contains(value)))
                    || (!stories.is_empty()
                        && !values.1.iter().any(|value| stories.contains(value)))
                    || (!semantic.is_empty()
                        && !values.2.iter().any(|value| semantic.contains(value)))
                {
                    return None;
                }
                let metadata = match serde_json::from_str(&row.metadata_json) {
                    Ok(value) => value,
                    Err(error) => return Some(Err(RagError::Json(error))),
                };
                Some(Ok(RagChunkRecord {
                    record: row.clone().into_raw(),
                    source_ref: row.source_ref,
                    metadata,
                    language_tags: values.0,
                    story_scopes: values.1,
                    semantic_tags: values.2,
                }))
            })
            .collect()
    }

    fn set_semantic_degraded(&self, message: impl Into<String>) {
        if let Ok(mut diagnostic) = self.semantic_diagnostic.write() {
            *diagnostic = SemanticDiagnostic::degraded(message);
        }
    }

    fn set_semantic_healthy(&self) {
        if let Ok(mut diagnostic) = self.semantic_diagnostic.write() {
            *diagnostic = SemanticDiagnostic::healthy();
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "flat public result is assembled from its explicit persisted inputs"
)]
fn result_from(
    id: i64,
    series_slug: &str,
    source_ref: &str,
    parsed: &super::ParsedRagFile,
    metadata: BTreeMap<String, serde_json::Value>,
    count: usize,
    skipped: bool,
    normalized: &super::NormalizedRagSource,
) -> RagImportResult {
    RagImportResult {
        source: RagSourceRecord {
            id,
            series_slug: series_slug.into(),
            source_ref: source_ref.into(),
            source_type: parsed.source_type.persisted().into(),
            content_type: parsed.content_type.clone(),
            checksum: parsed.checksum.clone(),
            metadata,
        },
        source_id: id,
        chunks_created: count,
        skipped,
        normalized_path: normalized.path.to_string_lossy().into_owned(),
        normalized_format: normalized.format.clone(),
    }
}

fn clean(values: &[String]) -> Vec<String> {
    let folded = values
        .iter()
        .map(|value| CaseMapper::new().fold_string(value.trim()).into_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    crate::values::normalize_tuple(&folded)
}

async fn replace_tags(
    connection: &mut Transaction<'_, Sqlite>,
    ids: &[i64],
    language: &[String],
    stories: &[String],
    semantic: &[String],
) -> Result<(), sqlx::Error> {
    for id in ids {
        sqlx::query("DELETE FROM rag_chunk_language_tags WHERE chunk_id = ?")
            .bind(id)
            .execute(&mut **connection)
            .await?;
        sqlx::query("DELETE FROM rag_chunk_story_scopes WHERE chunk_id = ?")
            .bind(id)
            .execute(&mut **connection)
            .await?;
        sqlx::query("DELETE FROM rag_chunk_semantic_tags WHERE chunk_id = ?")
            .bind(id)
            .execute(&mut **connection)
            .await?;
        insert_tags(connection, TagKind::Language, *id, language).await?;
        insert_tags(connection, TagKind::Story, *id, stories).await?;
        insert_tags(connection, TagKind::Semantic, *id, semantic).await?;
    }
    Ok(())
}

enum TagKind {
    Language,
    Story,
    Semantic,
}
async fn insert_tags(
    connection: &mut Transaction<'_, Sqlite>,
    kind: TagKind,
    id: i64,
    values: &[String],
) -> Result<(), sqlx::Error> {
    for value in values {
        match kind {
            TagKind::Language => {
                sqlx::query(
                    "INSERT INTO rag_chunk_language_tags(chunk_id, language_tag) VALUES (?, ?)",
                )
                .bind(id)
                .bind(value)
                .execute(&mut **connection)
                .await?
            }
            TagKind::Story => {
                sqlx::query(
                    "INSERT INTO rag_chunk_story_scopes(chunk_id, story_scope) VALUES (?, ?)",
                )
                .bind(id)
                .bind(value)
                .execute(&mut **connection)
                .await?
            }
            TagKind::Semantic => {
                sqlx::query(
                    "INSERT INTO rag_chunk_semantic_tags(chunk_id, semantic_tag) VALUES (?, ?)",
                )
                .bind(id)
                .bind(value)
                .execute(&mut **connection)
                .await?
            }
        };
    }
    Ok(())
}

type Tags = (Vec<String>, Vec<String>, Vec<String>);
async fn hydrate_tags(pool: &SqlitePool, ids: &[i64]) -> Result<HashMap<i64, Tags>, sqlx::Error> {
    let mut result: HashMap<i64, Tags> = ids.iter().map(|id| (*id, Default::default())).collect();
    if ids.is_empty() {
        return Ok(result);
    }
    for (table, column, slot) in [
        ("rag_chunk_language_tags", "language_tag", 0),
        ("rag_chunk_story_scopes", "story_scope", 1),
        ("rag_chunk_semantic_tags", "semantic_tag", 2),
    ] {
        let mut builder = QueryBuilder::new(format!(
            "SELECT chunk_id, {column} AS value FROM {table} WHERE chunk_id IN ("
        ));
        let mut separated = builder.separated(",");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") ORDER BY chunk_id, value");
        for row in builder.build().fetch_all(pool).await? {
            let entry = result.entry(row.get("chunk_id")).or_default();
            let value = row.get("value");
            match slot {
                0 => entry.0.push(value),
                1 => entry.1.push(value),
                _ => entry.2.push(value),
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, FromRow)]
struct SearchRow {
    id: i64,
    source_id: i64,
    series_slug: String,
    chunk_kind: String,
    text: String,
    display_text: String,
    location: String,
    metadata_json: String,
    created_at: chrono::DateTime<Utc>,
    source_ref: String,
    score: f64,
}

#[derive(Debug, Clone, FromRow)]
struct SearchChunkRow {
    id: i64,
    source_id: i64,
    series_slug: String,
    chunk_kind: String,
    text: String,
    display_text: String,
    location: String,
    metadata_json: String,
    created_at: chrono::DateTime<Utc>,
    source_ref: String,
}

impl SearchChunkRow {
    fn into_raw(self) -> RawRagChunkRecord {
        RawRagChunkRecord {
            id: self.id,
            source_id: self.source_id,
            series_slug: self.series_slug,
            chunk_kind: self.chunk_kind,
            text: self.text,
            display_text: self.display_text,
            location: self.location,
            metadata_json: self.metadata_json,
            created_at: self.created_at,
        }
    }
}
impl SearchRow {
    fn into_raw(self) -> RawRagChunkRecord {
        RawRagChunkRecord {
            id: self.id,
            source_id: self.source_id,
            series_slug: self.series_slug,
            chunk_kind: self.chunk_kind,
            text: self.text,
            display_text: self.display_text,
            location: self.location,
            metadata_json: self.metadata_json,
            created_at: self.created_at,
        }
    }
}

const SEARCH_SQL: &str = r#"
SELECT c.*, s.source_ref,
 max(-bm25(rag_chunks_fts), 0.000001)
 + CASE WHEN c.chunk_kind = 'glossary_entry' THEN 0.25 ELSE 0 END
 + CASE WHEN EXISTS (SELECT 1 FROM rag_chunk_language_tags t JOIN json_each(?) q ON t.language_tag = q.value WHERE t.chunk_id = c.id) THEN 0.05 ELSE 0 END
 + CASE WHEN EXISTS (SELECT 1 FROM rag_chunk_story_scopes t JOIN json_each(?) q ON t.story_scope = q.value WHERE t.chunk_id = c.id) THEN 0.10 ELSE 0 END
 + CASE WHEN EXISTS (SELECT 1 FROM rag_chunk_semantic_tags t JOIN json_each(?) q ON t.semantic_tag = q.value WHERE t.chunk_id = c.id) THEN 0.10 ELSE 0 END AS score
FROM rag_chunks_fts JOIN rag_chunks c ON c.id = rag_chunks_fts.rowid JOIN rag_sources s ON s.id = c.source_id AND s.series_slug = c.series_slug
WHERE rag_chunks_fts MATCH ? AND c.series_slug = ? ORDER BY score DESC, c.id LIMIT ?
"#;

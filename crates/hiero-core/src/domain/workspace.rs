use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool, Transaction};

use crate::{
    db::{ShortTermMemoryRecord, TaskSessionRecord},
    values::SOURCE_CREDIBILITY_CONFIDENCE,
};

use super::{
    AddMemoryInput, AddMemoryResult, Crystal, ShortMemoryLimits, ShortTermMemory, TaskSession,
    TranslationContext, models::normalize_texts, search_expression,
};

const MAX_BATCH_SIZE: usize = 500;
const MAX_SEARCH_LIMIT: usize = 50;
const METADATA_CHUNK_SIZE: usize = 500;
const SELECT_SESSION: &str = "SELECT id, series_slug, source_language, target_language, task_type, volume, chapter, status, cycle_id, created_at, last_activity_at, completed_at FROM task_sessions WHERE id = ?";
const SELECT_MEMORY: &str = "SELECT id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, source_crystal_id, created_at, archived_at FROM short_term_memories WHERE id = ?";
const MEMORY_COLUMNS: &str = "id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, source_crystal_id, created_at, archived_at";
const LIST_MEMORIES: &str = "SELECT id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, source_crystal_id, created_at, archived_at FROM short_term_memories WHERE session_id = ? AND archived_at IS NULL ORDER BY id";
const SEARCH_MEMORIES: &str = "SELECT short_term_memories.id, short_term_memories.session_id, short_term_memories.source_role, short_term_memories.kind, short_term_memories.text, short_term_memories.source_ref, short_term_memories.metadata_json, short_term_memories.source_credibility, short_term_memories.rule_intent, short_term_memories.soft_origin, short_term_memories.source_crystal_id, short_term_memories.created_at, short_term_memories.archived_at FROM short_term_memories_fts JOIN short_term_memories ON short_term_memories.id = short_term_memories_fts.rowid WHERE short_term_memories_fts MATCH ? AND short_term_memories.session_id = ? AND short_term_memories.archived_at IS NULL ORDER BY bm25(short_term_memories_fts), short_term_memories.id LIMIT ?";
#[allow(
    dead_code,
    reason = "Phase 003 Task 5 consumes the crate-private working-copy API"
)]
const SELECT_WORKING_COPY: &str = "SELECT id, session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, source_crystal_id, created_at, archived_at FROM short_term_memories WHERE session_id = ? AND source_crystal_id = ?";

pub type Result<T> = std::result::Result<T, WorkspaceError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkspaceError {
    #[error("invalid workspace {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("unknown session: {id}")]
    SessionNotFound { id: i64 },
    #[error("short-term memories require an active session: {id}")]
    SessionInactive { id: i64 },
    #[error("unknown short-term memory: {id}")]
    MemoryNotFound { id: i64 },
    #[error(
        "working copy for session {session_id} and crystal {crystal_id} conflicts with the source crystal"
    )]
    WorkingCopyConflict { session_id: i64, crystal_id: i64 },
    #[error("workspace {operation} violated a database constraint: {message}")]
    Constraint {
        operation: &'static str,
        message: String,
        #[source]
        source: sqlx::Error,
    },
    #[error("workspace {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("workspace metadata must be a JSON object")]
    MetadataObject,
    #[error("workspace metadata is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct WorkspaceStore<'a> {
    pool: &'a SqlitePool,
    limits: ShortMemoryLimits,
}

impl<'a> WorkspaceStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self {
            pool,
            limits: ShortMemoryLimits::DEFAULT,
        }
    }

    pub fn with_limits(pool: &'a SqlitePool, limits: ShortMemoryLimits) -> Result<Self> {
        validate_limits(limits)?;
        Ok(Self { pool, limits })
    }

    pub async fn start_session(
        &self,
        ctx: &TranslationContext,
        task_type: &str,
        volume: &str,
        chapter: &str,
    ) -> Result<TaskSession> {
        let task_type = non_empty("task_type", task_type)?;
        let volume = volume.trim();
        let chapter = chapter.trim();
        let context = ValidatedContext::try_from(ctx)?;
        let mut transaction = begin_immediate(self.pool, "start session").await?;
        let result = async {
            validate_series(&mut transaction, &context).await?;
            let now = Utc::now();
            let inserted = sqlx::query(
                "INSERT INTO task_sessions(series_slug, source_language, target_language, task_type, volume, chapter, status, created_at, last_activity_at) VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?)",
            )
            .bind(&context.series_slug)
            .bind(&context.source_language)
            .bind(&context.target_language)
            .bind(task_type)
            .bind(volume)
            .bind(chapter)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(|source| database("start session", source))?;
            let id = inserted.last_insert_rowid();
            insert_text_values(
                &mut transaction,
                "task_session_language_tags",
                "session_id",
                id,
                "language_tag",
                &context.language_tags,
                "start session",
            )
            .await?;
            insert_text_values(
                &mut transaction,
                "task_session_story_scopes",
                "session_id",
                id,
                "story_scope",
                &context.story_scopes,
                "start session",
            )
            .await?;
            insert_text_values(
                &mut transaction,
                "task_session_semantic_tags",
                "session_id",
                id,
                "semantic_tag",
                &context.semantic_tags,
                "start session",
            )
            .await?;
            Ok(id)
        }
        .await;
        let id = commit_write(transaction, "start session", result).await?;
        self.get_session(id).await
    }

    pub async fn get_session(&self, id: i64) -> Result<TaskSession> {
        let record = sqlx::query_as::<_, TaskSessionRecord>(SELECT_SESSION)
            .bind(id)
            .fetch_optional(self.pool)
            .await
            .map_err(|source| database("get session", source))?
            .ok_or(WorkspaceError::SessionNotFound { id })?;
        hydrate_sessions(self.pool, vec![record])
            .await?
            .pop()
            .ok_or(WorkspaceError::SessionNotFound { id })
    }

    pub async fn complete_session(&self, id: i64) -> Result<bool> {
        let mut transaction = begin_immediate(self.pool, "complete session").await?;
        let result = async {
            let session = get_session_on(&mut transaction, id, "complete session").await?;
            if session.status != "active" {
                return Ok(false);
            }
            let updated = sqlx::query(
                "UPDATE task_sessions SET status = 'completed', completed_at = ? WHERE id = ? AND status = 'active'",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(|source| database("complete session", source))?;
            Ok(updated.rows_affected() == 1)
        }
        .await;
        commit_write(transaction, "complete session", result).await
    }

    pub async fn complete_inactive(&self, cutoff: DateTime<Utc>) -> Result<Vec<i64>> {
        let mut transaction = begin_immediate(self.pool, "complete inactive sessions").await?;
        let result = async {
            let mut ids = sqlx::query_scalar::<_, i64>(
                "UPDATE task_sessions SET status = 'completed', completed_at = ? WHERE status = 'active' AND last_activity_at < ? RETURNING id",
            )
            .bind(Utc::now())
            .bind(cutoff)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|source| database("complete inactive sessions", source))?;
            ids.sort_unstable();
            Ok(ids)
        }
        .await;
        commit_write(transaction, "complete inactive sessions", result).await
    }

    pub async fn add_short_term(
        &self,
        session_id: i64,
        input: AddMemoryInput,
    ) -> Result<AddMemoryResult> {
        let results = self.add_short_term_batch(session_id, &[input]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| invalid("items", "single insertion returned no id"))
    }

    pub async fn add_short_term_batch(
        &self,
        session_id: i64,
        items: &[AddMemoryInput],
    ) -> Result<Vec<AddMemoryResult>> {
        if items.is_empty() {
            return Err(invalid("items", "must not be empty"));
        }
        if items.len() > MAX_BATCH_SIZE {
            return Err(invalid(
                "items",
                format!("a batch may contain at most {MAX_BATCH_SIZE} memories"),
            ));
        }
        let prepared = items
            .iter()
            .cloned()
            .map(|input| PreparedMemory::new(input, self.limits))
            .collect::<Result<Vec<_>>>()?;
        let mut transaction = begin_immediate(self.pool, "add short-term memory batch").await?;
        let result = async {
            require_active_session(&mut transaction, session_id, "add short-term memory batch")
                .await?;
            let now = Utc::now();
            let mut ids = Vec::with_capacity(prepared.len());
            for memory in &prepared {
                ids.push(insert_memory(&mut transaction, session_id, memory, None, now).await?);
            }
            sqlx::query("UPDATE task_sessions SET last_activity_at = ? WHERE id = ?")
                .bind(now)
                .bind(session_id)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("touch session", source))?;
            Ok(ids)
        }
        .await;
        let ids = commit_write(transaction, "add short-term memory batch", result).await?;
        let memories = load_memories_by_ids(self.pool, &ids).await?;
        Ok(memories
            .into_iter()
            .zip(prepared)
            .map(|(memory, prepared)| AddMemoryResult {
                memory,
                warnings: prepared.warnings,
            })
            .collect())
    }

    pub async fn list_short_term(&self, session_id: i64) -> Result<Vec<ShortTermMemory>> {
        ensure_session(self.pool, session_id).await?;
        let records = sqlx::query_as::<_, ShortTermMemoryRecord>(LIST_MEMORIES)
            .bind(session_id)
            .fetch_all(self.pool)
            .await
            .map_err(|source| database("list short-term memories", source))?;
        hydrate_memories(self.pool, records).await
    }

    pub async fn search_short_term(
        &self,
        session_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ShortTermMemory>> {
        ensure_session(self.pool, session_id).await?;
        let limit = bounded_limit(limit)?;
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let records = sqlx::query_as::<_, ShortTermMemoryRecord>(SEARCH_MEMORIES)
            .bind(expression)
            .bind(session_id)
            .bind(limit as i64)
            .fetch_all(self.pool)
            .await
            .map_err(|source| database("search short-term memories", source))?;
        hydrate_memories(self.pool, records).await
    }

    #[allow(dead_code, reason = "Phase 003 Task 5 wires this into RecallService")]
    pub(crate) async fn get_or_create_working_copy(
        &self,
        session_id: i64,
        crystal: &Crystal,
    ) -> Result<(ShortTermMemory, bool)> {
        let mut transaction = begin_immediate(self.pool, "get or create working copy").await?;
        let result = self
            .get_or_create_working_copy_in(&mut transaction, session_id, crystal)
            .await;
        let (id, created) = commit_write(transaction, "get or create working copy", result).await?;
        Ok((self.get_memory(id).await?, created))
    }

    pub(crate) async fn get_or_create_working_copy_in(
        &self,
        transaction: &mut Transaction<'static, Sqlite>,
        session_id: i64,
        crystal: &Crystal,
    ) -> Result<(i64, bool)> {
        let expected = PreparedMemory::from_crystal(crystal)?;
        let session =
            require_active_session(transaction, session_id, "get or create working copy").await?;
        validate_crystal_for_session(&session, crystal)?;
        if let Some(existing) = get_working_copy_on(transaction, session_id, crystal.id).await? {
            if !working_copy_equivalent(&existing, &expected, crystal.id)
                || !working_copy_metadata_equivalent(transaction, existing.id, &expected).await?
            {
                return Err(WorkspaceError::WorkingCopyConflict {
                    session_id,
                    crystal_id: crystal.id,
                });
            }
            return Ok((existing.id, false));
        }
        let now = Utc::now();
        let id = insert_memory(transaction, session_id, &expected, Some(crystal.id), now).await?;
        sqlx::query("UPDATE task_sessions SET last_activity_at = ? WHERE id = ?")
            .bind(now)
            .bind(session_id)
            .execute(&mut **transaction)
            .await
            .map_err(|source| database("touch session", source))?;
        Ok((id, true))
    }

    pub async fn archive(&self, id: i64) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, "archive short-term memory").await?;
        let result = async {
            get_memory_on(&mut transaction, id, "archive short-term memory").await?;
            sqlx::query(
                "UPDATE short_term_memories SET archived_at = COALESCE(archived_at, ?) WHERE id = ?",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(|source| database("archive short-term memory", source))?;
            Ok(())
        }
        .await;
        commit_write(transaction, "archive short-term memory", result).await
    }

    #[allow(
        dead_code,
        reason = "used by the pending crate-private working-copy consumer"
    )]
    pub(crate) async fn get_memory(&self, id: i64) -> Result<ShortTermMemory> {
        let record = sqlx::query_as::<_, ShortTermMemoryRecord>(SELECT_MEMORY)
            .bind(id)
            .fetch_optional(self.pool)
            .await
            .map_err(|source| database("get short-term memory", source))?
            .ok_or(WorkspaceError::MemoryNotFound { id })?;
        hydrate_memories(self.pool, vec![record])
            .await?
            .pop()
            .ok_or(WorkspaceError::MemoryNotFound { id })
    }
}

pub async fn complete_stale_sessions(pool: &SqlitePool, cutoff: DateTime<Utc>) -> Result<Vec<i64>> {
    WorkspaceStore::new(pool).complete_inactive(cutoff).await
}

#[derive(Debug)]
struct ValidatedContext {
    series_slug: String,
    source_language: String,
    target_language: String,
    language_tags: Vec<String>,
    story_scopes: Vec<String>,
    semantic_tags: Vec<String>,
}

impl TryFrom<&TranslationContext> for ValidatedContext {
    type Error = WorkspaceError;

    fn try_from(context: &TranslationContext) -> Result<Self> {
        let series_slug = context.series_slug.trim().to_owned();
        if !valid_series_slug(&series_slug) {
            return Err(invalid("series_slug", "must be a canonical series slug"));
        }
        let scope_key = context.scope_key.trim().to_owned();
        if scope_key != format!("series:{series_slug}") {
            return Err(invalid(
                "scope_key",
                "must be exactly `series:{series_slug}`",
            ));
        }
        let source_language =
            non_empty("source_language", &context.source_language)?.to_lowercase();
        let target_language =
            non_empty("target_language", &context.target_language)?.to_lowercase();
        let language_tags = normalize_texts(&context.language_tags, true);
        if !language_tags.is_empty()
            && (!language_tags.contains(&source_language)
                || !language_tags.contains(&target_language))
        {
            return Err(invalid(
                "language_tags",
                "must include both session languages when provided",
            ));
        }
        Ok(Self {
            series_slug,
            source_language,
            target_language,
            language_tags,
            story_scopes: normalize_texts(&context.story_scopes, false),
            semantic_tags: normalize_texts(&context.semantic_tags, false),
        })
    }
}

#[derive(Debug)]
struct PreparedMemory {
    source_role: String,
    kind: String,
    text: String,
    source_ref: String,
    metadata_json: String,
    language_tags: Vec<String>,
    story_scopes: Vec<String>,
    semantic_tags: Vec<String>,
    source_credibility: String,
    rule_intent: String,
    soft_origin: Option<String>,
    warnings: Vec<String>,
}

impl PreparedMemory {
    fn new(input: AddMemoryInput, limits: ShortMemoryLimits) -> Result<Self> {
        Self::prepare(input, Some(limits))
    }

    fn prepare(input: AddMemoryInput, limits: Option<ShortMemoryLimits>) -> Result<Self> {
        let source_role = non_empty("source_role", &input.source_role)?;
        let kind = non_empty("kind", &input.kind)?;
        let text = non_empty("text", &input.text)?;
        let sentence_count = sentence_count(&text);
        let symbol_count = text.chars().count();
        if let Some(limits) = limits {
            if sentence_count > limits.rejection_sentence_count {
                return Err(invalid("text", "short-term memory is too large"));
            }
            if limits.rejection_symbol_count != 0 && symbol_count > limits.rejection_symbol_count {
                return Err(invalid(
                    "text",
                    format!(
                        "short-term memory exceeds {} symbols",
                        limits.rejection_symbol_count
                    ),
                ));
            }
        }
        let source_credibility = input.source_credibility.trim();
        if !SOURCE_CREDIBILITY_CONFIDENCE.contains_key(source_credibility) {
            return Err(invalid("source_credibility", "unknown label"));
        }
        let mut metadata = input
            .metadata
            .as_object()
            .cloned()
            .ok_or(WorkspaceError::MetadataObject)?;
        for key in ["sentence_count", "symbol_count", "validation_warning"] {
            metadata.remove(key);
        }
        let language_tags = normalize_texts(&input.language_tags, true);
        let story_scopes = normalize_texts(&input.story_scopes, false);
        let semantic_tags = normalize_texts(&input.semantic_tags, false);
        if !language_tags.is_empty() {
            metadata.insert("language_tags".into(), serde_json::json!(language_tags));
        }
        if !story_scopes.is_empty() {
            metadata.insert("story_scopes".into(), serde_json::json!(story_scopes));
        }
        if !semantic_tags.is_empty() {
            metadata.insert("semantic_tags".into(), serde_json::json!(semantic_tags));
        }
        if source_credibility != "observation" {
            metadata.insert(
                "source_credibility".into(),
                Value::String(source_credibility.to_owned()),
            );
        }
        let rule_intent = input.rule_intent.trim().to_owned();
        if !rule_intent.is_empty() {
            metadata.insert("rule_intent".into(), Value::String(rule_intent.clone()));
        }
        let soft_origin = input.soft_origin.and_then(|value| {
            let value = value.trim().to_owned();
            (!value.is_empty()).then_some(value)
        });
        if let Some(value) = &soft_origin {
            metadata.insert("soft_origin".into(), Value::String(value.clone()));
        }
        metadata.insert("sentence_count".into(), serde_json::json!(sentence_count));
        metadata.insert("symbol_count".into(), serde_json::json!(symbol_count));
        let mut warnings = Vec::new();
        if let Some(limits) = limits {
            if sentence_count > limits.warning_sentence_count {
                warnings.push("short-term memory is large; prefer 1-6 sentences".to_owned());
            }
            if limits.warning_symbol_count != 0 && symbol_count > limits.warning_symbol_count {
                warnings.push(format!(
                    "short-term memory is large; prefer <= {} symbols",
                    limits.warning_symbol_count
                ));
            }
        }
        if !warnings.is_empty() {
            metadata.insert(
                "validation_warning".into(),
                Value::String(warnings.join("; ")),
            );
        }
        let metadata_json =
            serde_json::to_string(&metadata).map_err(|source| WorkspaceError::Json { source })?;
        Ok(Self {
            source_role,
            kind,
            text,
            source_ref: input.source_ref.trim().to_owned(),
            metadata_json,
            language_tags,
            story_scopes,
            semantic_tags,
            source_credibility: source_credibility.to_owned(),
            rule_intent,
            soft_origin,
            warnings,
        })
    }
    #[allow(
        dead_code,
        reason = "used by the pending crate-private working-copy consumer"
    )]
    fn from_crystal(crystal: &Crystal) -> Result<Self> {
        // Recall copies trusted persisted crystals. External size limits protect new user/agent
        // input and must not make an already-valid source crystal impossible to recall.
        Self::prepare(
            AddMemoryInput {
                source_role: "recall".into(),
                kind: "working_copy".into(),
                text: crystal.text.clone(),
                source_ref: format!("crystal:{}", crystal.id),
                metadata: serde_json::json!({"source_crystal_id": crystal.id}),
                language_tags: crystal.language_tags.clone(),
                story_scopes: crystal.story_scopes.clone(),
                semantic_tags: crystal.semantic_tags.clone(),
                source_credibility: crystal.source_credibility.clone(),
                rule_intent: crystal.rule_intent.clone(),
                soft_origin: crystal.soft_origin.clone(),
            },
            None,
        )
    }
}

async fn validate_series(
    connection: &mut SqliteConnection,
    context: &ValidatedContext,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT default_source_language, default_target_language FROM series WHERE slug = ?",
    )
    .bind(&context.series_slug)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|source| database("validate session series", source))?
    .ok_or_else(|| invalid("series_slug", "does not identify an existing series"))?;
    let source: String = row.get("default_source_language");
    let target: String = row.get("default_target_language");
    if source.to_lowercase() != context.source_language
        || target.to_lowercase() != context.target_language
    {
        return Err(invalid(
            "languages",
            "must match the series default source and target languages",
        ));
    }
    Ok(())
}

async fn insert_memory(
    connection: &mut SqliteConnection,
    session_id: i64,
    memory: &PreparedMemory,
    source_crystal_id: Option<i64>,
    now: DateTime<Utc>,
) -> Result<i64> {
    let inserted = sqlx::query(
        "INSERT INTO short_term_memories(session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, soft_origin, source_crystal_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(&memory.source_role)
    .bind(&memory.kind)
    .bind(&memory.text)
    .bind(&memory.source_ref)
    .bind(&memory.metadata_json)
    .bind(&memory.source_credibility)
    .bind(&memory.rule_intent)
    .bind(&memory.soft_origin)
    .bind(source_crystal_id)
    .bind(now)
    .execute(&mut *connection)
    .await
    .map_err(|source| database("insert short-term memory", source))?;
    let id = inserted.last_insert_rowid();
    insert_text_values(
        connection,
        "short_term_memory_language_tags",
        "memory_id",
        id,
        "language_tag",
        &memory.language_tags,
        "insert short-term memory language tags",
    )
    .await?;
    insert_text_values(
        connection,
        "short_term_memory_story_scopes",
        "memory_id",
        id,
        "story_scope",
        &memory.story_scopes,
        "insert short-term memory story scopes",
    )
    .await?;
    insert_text_values(
        connection,
        "short_term_memory_semantic_tags",
        "memory_id",
        id,
        "semantic_tag",
        &memory.semantic_tags,
        "insert short-term memory semantic tags",
    )
    .await?;
    Ok(id)
}

async fn insert_text_values(
    connection: &mut SqliteConnection,
    table: &'static str,
    owner_column: &'static str,
    owner_id: i64,
    value_column: &'static str,
    values: &[String],
    operation: &'static str,
) -> Result<()> {
    for value in values {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "INSERT INTO {table}({owner_column}, {value_column}) VALUES ("
        ));
        query
            .push_bind(owner_id)
            .push(", ")
            .push_bind(value)
            .push(")");
        query
            .build()
            .execute(&mut *connection)
            .await
            .map_err(|source| database(operation, source))?;
    }
    Ok(())
}

async fn hydrate_sessions(
    pool: &SqlitePool,
    records: Vec<TaskSessionRecord>,
) -> Result<Vec<TaskSession>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = records.iter().map(|record| record.id).collect();
    let language_tags = load_text_metadata(pool, &ids, MetadataKind::SessionLanguage).await?;
    let story_scopes = load_text_metadata(pool, &ids, MetadataKind::SessionStory).await?;
    let semantic_tags = load_text_metadata(pool, &ids, MetadataKind::SessionSemantic).await?;
    Ok(records
        .into_iter()
        .map(|record| {
            let id = record.id;
            TaskSession {
                record,
                language_tags: language_tags.get(&id).cloned().unwrap_or_default(),
                story_scopes: story_scopes.get(&id).cloned().unwrap_or_default(),
                semantic_tags: semantic_tags.get(&id).cloned().unwrap_or_default(),
            }
        })
        .collect())
}

async fn hydrate_memories(
    pool: &SqlitePool,
    records: Vec<ShortTermMemoryRecord>,
) -> Result<Vec<ShortTermMemory>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = records.iter().map(|record| record.id).collect();
    let language_tags = load_text_metadata(pool, &ids, MetadataKind::MemoryLanguage).await?;
    let story_scopes = load_text_metadata(pool, &ids, MetadataKind::MemoryStory).await?;
    let semantic_tags = load_text_metadata(pool, &ids, MetadataKind::MemorySemantic).await?;
    records
        .into_iter()
        .map(|record| {
            let metadata = serde_json::from_str::<Value>(&record.metadata_json)
                .map_err(|source| WorkspaceError::Json { source })?
                .as_object()
                .cloned()
                .ok_or(WorkspaceError::MetadataObject)?;
            let id = record.id;
            Ok(ShortTermMemory {
                record,
                metadata,
                language_tags: language_tags.get(&id).cloned().unwrap_or_default(),
                story_scopes: story_scopes.get(&id).cloned().unwrap_or_default(),
                semantic_tags: semantic_tags.get(&id).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

async fn load_memories_by_ids(pool: &SqlitePool, ids: &[i64]) -> Result<Vec<ShortTermMemory>> {
    let mut records = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(METADATA_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {MEMORY_COLUMNS} FROM short_term_memories WHERE id IN ("
        ));
        push_ids(&mut query, chunk);
        query.push(")");
        records.extend(
            query
                .build_query_as::<ShortTermMemoryRecord>()
                .fetch_all(pool)
                .await
                .map_err(|source| database("load added short-term memories", source))?,
        );
    }
    let mut by_id = hydrate_memories(pool, records)
        .await?
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<BTreeMap<_, _>>();
    ids.iter()
        .map(|id| {
            by_id
                .remove(id)
                .ok_or(WorkspaceError::MemoryNotFound { id: *id })
        })
        .collect()
}

#[derive(Clone, Copy)]
enum MetadataKind {
    SessionLanguage,
    SessionStory,
    SessionSemantic,
    MemoryLanguage,
    MemoryStory,
    MemorySemantic,
}

async fn load_text_metadata(
    pool: &SqlitePool,
    ids: &[i64],
    kind: MetadataKind,
) -> Result<BTreeMap<i64, Vec<String>>> {
    let (table, owner, value) = match kind {
        MetadataKind::SessionLanguage => {
            ("task_session_language_tags", "session_id", "language_tag")
        }
        MetadataKind::SessionStory => ("task_session_story_scopes", "session_id", "story_scope"),
        MetadataKind::SessionSemantic => {
            ("task_session_semantic_tags", "session_id", "semantic_tag")
        }
        MetadataKind::MemoryLanguage => (
            "short_term_memory_language_tags",
            "memory_id",
            "language_tag",
        ),
        MetadataKind::MemoryStory => ("short_term_memory_story_scopes", "memory_id", "story_scope"),
        MetadataKind::MemorySemantic => (
            "short_term_memory_semantic_tags",
            "memory_id",
            "semantic_tag",
        ),
    };
    let mut values = BTreeMap::<i64, Vec<String>>::new();
    for chunk in ids.chunks(METADATA_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {owner} AS owner_id, {value} AS value FROM {table} WHERE {owner} IN ("
        ));
        push_ids(&mut query, chunk);
        query.push(") ORDER BY rowid");
        for row in query
            .build()
            .fetch_all(pool)
            .await
            .map_err(|source| database("hydrate workspace metadata", source))?
        {
            values
                .entry(row.get("owner_id"))
                .or_default()
                .push(row.get("value"));
        }
    }
    Ok(values)
}

async fn ensure_session(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM task_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|source| database("find session", source))?
        .map(|_| ())
        .ok_or(WorkspaceError::SessionNotFound { id })
}

async fn require_active_session(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<TaskSessionRecord> {
    let session = get_session_on(connection, id, operation).await?;
    if session.status != "active" {
        return Err(WorkspaceError::SessionInactive { id });
    }
    Ok(session)
}

async fn get_session_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<TaskSessionRecord> {
    sqlx::query_as::<_, TaskSessionRecord>(SELECT_SESSION)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|source| database(operation, source))?
        .ok_or(WorkspaceError::SessionNotFound { id })
}

async fn get_memory_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<ShortTermMemoryRecord> {
    sqlx::query_as::<_, ShortTermMemoryRecord>(SELECT_MEMORY)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|source| database(operation, source))?
        .ok_or(WorkspaceError::MemoryNotFound { id })
}

#[allow(
    dead_code,
    reason = "used by the pending crate-private working-copy consumer"
)]
async fn get_working_copy_on(
    connection: &mut SqliteConnection,
    session_id: i64,
    crystal_id: i64,
) -> Result<Option<ShortTermMemoryRecord>> {
    sqlx::query_as::<_, ShortTermMemoryRecord>(SELECT_WORKING_COPY)
        .bind(session_id)
        .bind(crystal_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|source| database("get working copy", source))
}

#[allow(
    dead_code,
    reason = "used by the pending crate-private working-copy consumer"
)]
fn working_copy_equivalent(
    existing: &ShortTermMemoryRecord,
    expected: &PreparedMemory,
    crystal_id: i64,
) -> bool {
    existing.source_role == expected.source_role
        && existing.kind == expected.kind
        && existing.text == expected.text
        && existing.source_ref == expected.source_ref
        && serde_json::from_str::<Value>(&existing.metadata_json).ok()
            == serde_json::from_str::<Value>(&expected.metadata_json).ok()
        && existing.source_credibility.as_deref() == Some(expected.source_credibility.as_str())
        && existing.rule_intent.as_deref() == Some(expected.rule_intent.as_str())
        && existing.soft_origin == expected.soft_origin
        && existing.source_crystal_id == Some(crystal_id)
        && existing.archived_at.is_none()
}

#[allow(
    dead_code,
    reason = "used by the pending crate-private working-copy consumer"
)]
async fn working_copy_metadata_equivalent(
    connection: &mut SqliteConnection,
    memory_id: i64,
    expected: &PreparedMemory,
) -> Result<bool> {
    for (table, column, values) in [
        (
            "short_term_memory_language_tags",
            "language_tag",
            expected.language_tags.as_slice(),
        ),
        (
            "short_term_memory_story_scopes",
            "story_scope",
            expected.story_scopes.as_slice(),
        ),
        (
            "short_term_memory_semantic_tags",
            "semantic_tag",
            expected.semantic_tags.as_slice(),
        ),
    ] {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {column} AS value FROM {table} WHERE memory_id = "
        ));
        query.push_bind(memory_id).push(" ORDER BY rowid");
        let actual = query
            .build_query_scalar::<String>()
            .fetch_all(&mut *connection)
            .await
            .map_err(|source| database("compare working-copy metadata", source))?;
        if actual != values {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(
    dead_code,
    reason = "used by the pending crate-private working-copy consumer"
)]
fn validate_crystal_for_session(session: &TaskSessionRecord, crystal: &Crystal) -> Result<()> {
    if !matches!(crystal.status.as_str(), "active" | "candidate") {
        return Err(invalid("crystal", "must be active or candidate"));
    }
    if crystal.scope_type == "series" && crystal.series_slug != session.series_slug {
        return Err(invalid("crystal", "belongs to a different series"));
    }
    if !crystal.source_language.is_empty() && crystal.source_language != session.source_language {
        return Err(invalid("crystal", "has a different source language"));
    }
    if !crystal.target_language.is_empty() && crystal.target_language != session.target_language {
        return Err(invalid("crystal", "has a different target language"));
    }
    Ok(())
}

async fn begin_immediate(
    pool: &SqlitePool,
    operation: &'static str,
) -> Result<Transaction<'static, Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|source| database(operation, source))
}

async fn commit_write<T>(
    transaction: Transaction<'static, Sqlite>,
    operation: &'static str,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => transaction
            .commit()
            .await
            .map(|()| value)
            .map_err(|source| database(operation, source)),
        Err(error) => Err(error),
    }
}

fn push_ids(query: &mut QueryBuilder<Sqlite>, ids: &[i64]) {
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
}

fn bounded_limit(limit: usize) -> Result<usize> {
    if limit == 0 {
        Err(invalid("limit", "must be at least 1"))
    } else {
        Ok(limit.min(MAX_SEARCH_LIMIT))
    }
}

fn validate_limits(limits: ShortMemoryLimits) -> Result<()> {
    if limits.warning_sentence_count == 0 {
        return Err(invalid("warning_sentence_count", "must be at least 1"));
    }
    if limits.rejection_sentence_count == 0 {
        return Err(invalid("rejection_sentence_count", "must be at least 1"));
    }
    if limits.rejection_sentence_count < limits.warning_sentence_count {
        return Err(invalid(
            "rejection_sentence_count",
            "must be greater than or equal to warning_sentence_count",
        ));
    }
    if limits.warning_symbol_count != 0
        && limits.rejection_symbol_count != 0
        && limits.rejection_symbol_count < limits.warning_symbol_count
    {
        return Err(invalid(
            "rejection_symbol_count",
            "must be greater than or equal to warning_symbol_count",
        ));
    }
    Ok(())
}

fn non_empty(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid(field, "must not be empty"))
    } else {
        Ok(value.to_owned())
    }
}

fn sentence_count(text: &str) -> usize {
    let mut count = 0;
    let mut in_terminator = false;
    for character in text.chars() {
        let terminator = matches!(character, '.' | '!' | '?' | '。' | '！' | '？');
        if terminator && !in_terminator {
            count += 1;
        }
        in_terminator = terminator;
    }
    if count == 0 || !text.ends_with(['.', '!', '?', '。', '！', '？']) {
        count += 1;
    }
    count
}

fn valid_series_slug(slug: &str) -> bool {
    let mut bytes = slug.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn invalid(field: &'static str, reason: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Validation {
        field,
        reason: reason.into(),
    }
}

fn database(operation: &'static str, source: sqlx::Error) -> WorkspaceError {
    let constraint = source.as_database_error().is_some_and(|error| {
        error.is_check_violation()
            || error.is_foreign_key_violation()
            || error.is_unique_violation()
    });
    if constraint {
        WorkspaceError::Constraint {
            operation,
            message: source.to_string(),
            source,
        }
    } else {
        WorkspaceError::Database { operation, source }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::{
        db::connect_url,
        domain::{
            AddCrystalInput, AddMemoryInput, CrystalStore, ShortMemoryLimits, TranslationContext,
        },
        registry::SeriesRegistry,
    };

    use super::{WorkspaceError, WorkspaceStore, sentence_count};

    #[test]
    fn sentence_count_supports_latin_and_japanese_punctuation() {
        assert_eq!(sentence_count("One. Two! Three?"), 3);
        assert_eq!(sentence_count("一つ。二つ！三つ？"), 3);
        assert_eq!(sentence_count("No terminator"), 1);
        assert_eq!(sentence_count("One... Two?! trailing fragment"), 3);
    }

    async fn working_copy_fixture() -> (sqlx::SqlitePool, i64, crate::domain::Crystal) {
        let pool = connect_url(&format!(
            "sqlite:file:working-copy-{}?mode=memory&cache=shared",
            Uuid::new_v4()
        ))
        .await
        .expect("test database should migrate");
        let (session_id, crystal) = seed_working_copy(&pool).await;
        (pool, session_id, crystal)
    }

    async fn seed_working_copy(pool: &sqlx::SqlitePool) -> (i64, crate::domain::Crystal) {
        SeriesRegistry::new(pool)
            .create("oso", "Only Sense Online", "ja", "ru")
            .await
            .unwrap();
        let context = TranslationContext::new("oso", "ja", "ru");
        let session = WorkspaceStore::new(pool)
            .start_session(&context, "translate", "1", "2")
            .await
            .unwrap();
        let crystal_id = CrystalStore::new(pool)
            .add(AddCrystalInput {
                text: "Remember canonical inventory wording. Preserve every source detail. Keep the exact trusted text.".into(),
                title: "Inventory".into(),
                scope_type: "series".into(),
                scope_key: "series:oso".into(),
                series_slug: "oso".into(),
                source_language: "ja".into(),
                target_language: "ru".into(),
                language_tags: vec!["ja".into(), "ru".into()],
                story_scopes: vec!["chapter:2".into()],
                semantic_tags: vec!["ui".into()],
                source_credibility: "expert".into(),
                rule_intent: "terminology_override".into(),
                soft_origin: Some("dream_cycle:7".into()),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        let crystal = CrystalStore::new(pool).get(crystal_id).await.unwrap();
        (session.id, crystal)
    }

    fn restrictive_limits() -> ShortMemoryLimits {
        ShortMemoryLimits {
            warning_sentence_count: 1,
            rejection_sentence_count: 2,
            warning_symbol_count: 8,
            rejection_symbol_count: 16,
        }
    }

    #[tokio::test]
    async fn trusted_crystal_working_copy_bypasses_external_size_limits_without_warnings() {
        let (pool, session_id, crystal) = working_copy_fixture().await;
        let store = WorkspaceStore::with_limits(&pool, restrictive_limits()).unwrap();

        let external_error = store
            .add_short_term(
                session_id,
                AddMemoryInput {
                    text: crystal.text.clone(),
                    ..AddMemoryInput::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            external_error,
            WorkspaceError::Validation { field: "text", .. }
        ));

        let (copy, created) = store
            .get_or_create_working_copy(session_id, &crystal)
            .await
            .unwrap();

        assert!(created);
        assert_eq!(copy.source_role, "recall");
        assert_eq!(copy.kind, "working_copy");
        assert_eq!(copy.text, crystal.text);
        assert_eq!(copy.source_ref, format!("crystal:{}", crystal.id));
        assert_eq!(copy.source_crystal_id, Some(crystal.id));
        assert_eq!(copy.metadata["source_crystal_id"], crystal.id);
        assert_eq!(copy.metadata["sentence_count"], 3);
        assert_eq!(copy.metadata["symbol_count"], crystal.text.chars().count());
        assert!(!copy.metadata.contains_key("validation_warning"));
        assert_eq!(copy.language_tags, crystal.language_tags);
        assert_eq!(copy.story_scopes, crystal.story_scopes);
        assert_eq!(copy.semantic_tags, crystal.semantic_tags);
        assert_eq!(
            copy.source_credibility.as_deref(),
            Some(crystal.source_credibility.as_str())
        );
        assert_eq!(
            copy.rule_intent.as_deref(),
            Some(crystal.rule_intent.as_str())
        );
        assert_eq!(copy.soft_origin, crystal.soft_origin);
    }

    #[tokio::test]
    async fn independent_file_pools_create_one_equivalent_working_copy() {
        let directory = tempfile::TempDir::new().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("working-copy.sqlite").display()
        );
        let first_pool = connect_url(&url).await.unwrap();
        let second_pool = connect_url(&url).await.unwrap();
        let observer_pool = connect_url(&url).await.unwrap();
        let (session_id, crystal) = seed_working_copy(&first_pool).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let first_barrier = Arc::clone(&barrier);
        let second_barrier = Arc::clone(&barrier);
        let first_crystal = crystal.clone();
        let second_crystal = crystal.clone();
        let first = tokio::spawn(async move {
            first_barrier.wait().await;
            WorkspaceStore::with_limits(&first_pool, restrictive_limits())
                .unwrap()
                .get_or_create_working_copy(session_id, &first_crystal)
                .await
        });
        let second = tokio::spawn(async move {
            second_barrier.wait().await;
            WorkspaceStore::with_limits(&second_pool, restrictive_limits())
                .unwrap()
                .get_or_create_working_copy(session_id, &second_crystal)
                .await
        });
        barrier.wait().await;
        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();

        assert_eq!(first.0, second.0);
        assert_ne!(first.1, second.1);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM short_term_memories WHERE session_id = ? AND source_crystal_id = ?",
        )
        .bind(session_id)
        .bind(crystal.id)
        .fetch_one(&observer_pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn mismatched_existing_working_copy_is_a_typed_conflict() {
        let (pool, session_id, crystal) = working_copy_fixture().await;
        sqlx::query("INSERT INTO short_term_memories(session_id, source_role, kind, text, source_ref, metadata_json, source_credibility, rule_intent, source_crystal_id, created_at) VALUES (?, 'recall', 'working_copy', 'tampered', ?, '{}', 'observation', '', ?, ?)")
            .bind(session_id)
            .bind(format!("crystal:{}", crystal.id))
            .bind(crystal.id)
            .bind(chrono::Utc::now())
            .execute(&pool)
            .await
            .unwrap();

        let error = WorkspaceStore::new(&pool)
            .get_or_create_working_copy(session_id, &crystal)
            .await
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::WorkingCopyConflict { .. }));
    }

    #[tokio::test]
    async fn mismatched_working_copy_side_metadata_is_a_typed_conflict() {
        let (pool, session_id, crystal) = working_copy_fixture().await;
        let (copy, _) = WorkspaceStore::new(&pool)
            .get_or_create_working_copy(session_id, &crystal)
            .await
            .unwrap();
        sqlx::query("DELETE FROM short_term_memory_semantic_tags WHERE memory_id = ?")
            .bind(copy.id)
            .execute(&pool)
            .await
            .unwrap();

        let error = WorkspaceStore::new(&pool)
            .get_or_create_working_copy(session_id, &crystal)
            .await
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::WorkingCopyConflict { .. }));
    }

    #[tokio::test]
    async fn deleting_source_crystal_preserves_working_copy_and_nulls_marker() {
        let (pool, session_id, crystal) = working_copy_fixture().await;
        let (copy, _) = WorkspaceStore::new(&pool)
            .get_or_create_working_copy(session_id, &crystal)
            .await
            .unwrap();
        sqlx::query("DELETE FROM crystals WHERE id = ?")
            .bind(crystal.id)
            .execute(&pool)
            .await
            .unwrap();

        let marker: Option<i64> =
            sqlx::query_scalar("SELECT source_crystal_id FROM short_term_memories WHERE id = ?")
                .bind(copy.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        assert_eq!(
            WorkspaceStore::new(&pool)
                .list_short_term(session_id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

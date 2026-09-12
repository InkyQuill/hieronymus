use chrono::Utc;
use rusqlite::Connection;
use serde_json::json;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::ingest_config::load_ingest_config;
use crate::memory_models::{
    MetadataMap, ShortTermMemoryRecord, TaskSessionRecord, TranslationContext,
    normalize_string_tuple,
};
use crate::registry::Registry;
use crate::short_memory::{search_expression, validate_short_memory_text};

const MAX_SHORT_TERM_MEMORIES_PER_BATCH: usize = 500;
const MAX_SEARCH_LIMIT: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Claim(#[from] crate::authority_models::DecisionErrorV1),
    #[error("unknown session: {0}")]
    UnknownSession(i64),
    #[error("research mode is request-local and cannot be stored in a session")]
    ResearchSession,
    #[error(transparent)]
    StoryContext(#[from] crate::story_applicability::ApplicabilityError),
    #[error("unknown series: {0}")]
    UnknownSeries(String),
    #[error("short-term memories require an active session")]
    InactiveSession,
    #[error("kind must not be empty")]
    EmptyKind,
    #[error("content must not be empty")]
    EmptyContent,
    #[error("items must not be empty")]
    EmptyBatch,
    #[error("a batch may contain at most {MAX_SHORT_TERM_MEMORIES_PER_BATCH} short-term memories")]
    BatchTooLarge,
    #[error("limit must be at least 1")]
    LimitTooSmall,
    #[error("{0}")]
    ShortMemory(#[from] crate::short_memory::ShortMemoryError),
    #[error("{0}")]
    Json(String),
    #[error(transparent)]
    Ingest(#[from] crate::ingest_config::IngestConfigError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
    #[error(transparent)]
    Registry(#[from] crate::registry::RegistryError),
}

/// Task-session and short-term-memory storage over the data-root database.
pub struct WorkspaceStore {
    config: HieronymusConfig,
}

/// One batch item for [`WorkspaceStore::add_short_term_memory`].
#[derive(Debug, Clone)]
pub struct ShortTermMemoryInput {
    pub claims: Vec<crate::claim_capture::ClaimInput>,
    pub source_role: String,
    pub kind: String,
    pub text: String,
    pub source_ref: String,
    pub metadata: Option<MetadataMap>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub source_credibility: String,
    pub rule_intent: String,
    pub soft_origin: String,
}

/// Identity and conservative capture diagnostics for a claim bound to one
/// short-term memory. Diagnostics describe missing metadata only; they do not
/// widen applicability or decide whether a resolved claim matches a query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CapturedShortTermClaim {
    pub claim_id: i64,
    pub revision: i64,
    pub warnings: Vec<&'static str>,
}

/// A memory and its claim diagnostics, collected in the original write transaction.
#[derive(Debug, Clone)]
pub struct ShortTermCapture {
    pub memory: ShortTermMemoryRecord,
    pub claims: Vec<CapturedShortTermClaim>,
}

impl Default for ShortTermMemoryInput {
    fn default() -> Self {
        Self {
            claims: Vec::new(),
            source_role: "agent".to_string(),
            kind: String::new(),
            text: String::new(),
            source_ref: String::new(),
            metadata: None,
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
            source_credibility: "observation".to_string(),
            rule_intent: String::new(),
            soft_origin: String::new(),
        }
    }
}

impl ShortTermMemoryInput {
    pub fn new(kind: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            text: text.into(),
            ..Self::default()
        }
    }
}

impl WorkspaceStore {
    pub(crate) fn for_read(config: &HieronymusConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn open(config: &HieronymusConfig) -> Result<Self, WorkspaceError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    pub fn start_session(
        &self,
        context: &TranslationContext,
    ) -> Result<TaskSessionRecord, WorkspaceError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        validate_session_context(&transaction, context)?;
        let viewpoint = serde_json::to_string(&context.story_viewpoint)
            .map_err(|e| WorkspaceError::Json(e.to_string()))?;
        transaction.execute(
            "insert into task_sessions(
               series_slug,
               source_language,
               target_language,
               task_type,
               volume,
               chapter,
               status,
               created_at,
               last_activity_at, story_timeline_id, story_scene_key, story_viewpoint_json
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                context.series_slug,
                context.source_language,
                context.target_language,
                context.task_type,
                context.volume,
                context.chapter,
                now,
                now,
                context.story_timeline_id,
                context.story_scene_key,
                viewpoint,
            ],
        )?;
        let session_id = transaction.last_insert_rowid();
        insert_metadata_values(
            &transaction,
            "task_session_language_tags",
            "session_id",
            session_id,
            "language_tag",
            &context.language_tags,
        )?;
        insert_metadata_values(
            &transaction,
            "task_session_story_scopes",
            "session_id",
            session_id,
            "story_scope",
            &context.story_scopes,
        )?;
        insert_metadata_values(
            &transaction,
            "task_session_semantic_tags",
            "session_id",
            session_id,
            "semantic_tag",
            &context.semantic_tags,
        )?;
        transaction.commit()?;
        Ok(TaskSessionRecord {
            id: session_id,
            context: context.clone(),
            status: "active".to_string(),
            cycle_id: None,
        })
    }

    /// The newest active session whose full context matches exactly (the
    /// Python `_active_default_session`/`_ensure_default_session` lookup used
    /// by the legacy memory wrappers). `None` when no such session exists.
    pub fn active_default_session(
        &self,
        context: &TranslationContext,
    ) -> Result<Option<TaskSessionRecord>, WorkspaceError> {
        let connection = self.connection()?;
        let session_id: Option<i64> = connection
            .query_row(
                "select id from task_sessions
                 where series_slug = ?1
                   and source_language = ?2
                   and target_language = ?3
                   and task_type = ?4
                   and volume = ?5
                   and chapter = ?6
                   and story_timeline_id is ?7
                   and story_scene_key is ?8
                   and story_viewpoint_json = ?9
                   and status = 'active'
                   and not exists (
                     select 1 from task_session_story_scopes scopes
                     where scopes.session_id = task_sessions.id
                       and scopes.story_scope not in (select value from json_each(?10))
                   )
                   and not exists (
                     select 1 from json_each(?10) requested
                     where requested.value not in (
                       select story_scope from task_session_story_scopes
                       where session_id = task_sessions.id
                     )
                   )
                 order by id desc
                 limit 1",
                rusqlite::params![
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                    context.story_timeline_id,
                    context.story_scene_key,
                    serde_json::to_string(&context.story_viewpoint)
                        .map_err(|e| WorkspaceError::Json(e.to_string()))?,
                    serde_json::to_string(&context.story_scopes)
                        .map_err(|e| WorkspaceError::Json(e.to_string()))?,
                ],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match session_id {
            Some(id) => Ok(Some(self.get_session(id)?)),
            None => Ok(None),
        }
    }

    /// Legacy fallback search (Python `MemoryStore._fallback_search`, the
    /// short-term half): FTS over non-archived short-term memories whose
    /// owning session matches the full context, bm25 order. Used when no
    /// active default session exists to recall through.
    pub fn search_short_term_memories_for_context(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        let connection = self.connection()?;
        self.search_short_term_memories_for_context_with_connection(
            &connection,
            None,
            context,
            query,
            limit,
        )
    }

    pub(crate) fn search_short_term_memories_for_context_with_connection(
        &self,
        connection: &Connection,
        current: Option<&crate::story_applicability::StoryQueryV1>,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        if limit < 1 {
            return Err(WorkspaceError::LimitTooSmall);
        }
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select short_term_memories.id
                 from short_term_memories_fts
                 join short_term_memories
                   on short_term_memories.id = short_term_memories_fts.rowid
                 join task_sessions
                   on task_sessions.id = short_term_memories.session_id
                 where short_term_memories_fts match ?1
                   and short_term_memories.archived_at is null
                   and task_sessions.series_slug = ?2
                   and task_sessions.source_language = ?3
                   and task_sessions.target_language = ?4
                   and task_sessions.task_type = ?5
                   and task_sessions.volume = ?6
                   and task_sessions.chapter = ?7
                  order by bm25(short_term_memories_fts), short_term_memories.id
                 limit ?8",
            )?;
            let rows = statement.query_map(
                rusqlite::params![
                    expression,
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                    if current.is_some() {
                        crate::claim_reads::CANDIDATE_BUDGET as i64
                    } else {
                        limit as i64
                    },
                ],
                |row| row.get(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let ids = crate::claim_reads::select_candidates(
            connection,
            ids,
            |id| crate::claim_reads::ClaimTarget::ShortTerm(*id),
            current,
            limit,
        )?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_memory(connection, id)?);
        }
        Ok(records)
    }

    pub fn get_session(&self, session_id: i64) -> Result<TaskSessionRecord, WorkspaceError> {
        let connection = self.connection()?;
        self.get_session_with_connection(&connection, session_id)
    }

    pub(crate) fn get_session_with_connection(
        &self,
        connection: &Connection,
        session_id: i64,
    ) -> Result<TaskSessionRecord, WorkspaceError> {
        let row = connection
            .query_row(
                "select series_slug, source_language, target_language, task_type,
                        volume, chapter, status, cycle_id, story_timeline_id, story_scene_key, story_viewpoint_json
                 from task_sessions where id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => WorkspaceError::UnknownSession(session_id),
                other => other.into(),
            })?;
        let language_tags = load_metadata_values(
            connection,
            "task_session_language_tags",
            "session_id",
            session_id,
            "language_tag",
        )?;
        let story_scopes = load_metadata_values(
            connection,
            "task_session_story_scopes",
            "session_id",
            session_id,
            "story_scope",
        )?;
        let semantic_tags = load_metadata_values(
            connection,
            "task_session_semantic_tags",
            "session_id",
            session_id,
            "semantic_tag",
        )?;
        let mut context = TranslationContext::new(row.0, row.1, row.2, row.3)
            .volume(row.4)
            .chapter(row.5)
            .with_metadata(Some(language_tags), Some(story_scopes), Some(semantic_tags));
        context.story_timeline_id = row.8;
        context.story_scene_key = row.9;
        context.story_viewpoint =
            serde_json::from_str(&row.10).map_err(|e| WorkspaceError::Json(e.to_string()))?;
        validate_session_context(connection, &context)?;
        Ok(TaskSessionRecord {
            id: session_id,
            context,
            status: row.6,
            cycle_id: row.7,
        })
    }

    /// Mark a session completed. Returns `false` when the session exists but
    /// is not active; unknown sessions are errors.
    pub fn complete_session(&self, session_id: i64) -> Result<bool, WorkspaceError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "update task_sessions
             set status = 'completed', completed_at = ?1
             where id = ?2 and status = 'active'",
            rusqlite::params![now_iso8601(), session_id],
        )?;
        if changed == 0 {
            let exists: Option<i64> = connection
                .query_row(
                    "select id from task_sessions where id = ?1",
                    [session_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            if exists.is_none() {
                return Err(WorkspaceError::UnknownSession(session_id));
            }
            return Ok(false);
        }
        Ok(true)
    }

    pub fn add_short_term_memory(
        &self,
        session_id: i64,
        input: &ShortTermMemoryInput,
    ) -> Result<ShortTermMemoryRecord, WorkspaceError> {
        Ok(self
            .add_short_term_memories_batch(session_id, std::iter::once(input))?
            .into_iter()
            .next()
            .expect("one item yields one record"))
    }

    /// Validate and insert a batch of short-term memories in one transaction:
    /// text thresholds from `ingest.conf`, normalized typed metadata side
    /// tables, an FTS row, and a session activity bump.
    pub fn add_short_term_memories_batch<'a>(
        &self,
        session_id: i64,
        items: impl IntoIterator<Item = &'a ShortTermMemoryInput>,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        Ok(self
            .capture_short_term_memories_batch(session_id, items)?
            .into_iter()
            .map(|capture| capture.memory)
            .collect())
    }

    /// Insert memories and collect all diagnostics before committing the batch.
    /// A diagnostic failure rolls back every write; success needs no second read.
    pub fn capture_short_term_memories_batch<'a>(
        &self,
        session_id: i64,
        items: impl IntoIterator<Item = &'a ShortTermMemoryInput>,
    ) -> Result<Vec<ShortTermCapture>, WorkspaceError> {
        let items: Vec<&ShortTermMemoryInput> = items.into_iter().collect();
        if items.is_empty() {
            return Err(WorkspaceError::EmptyBatch);
        }
        if items.len() > MAX_SHORT_TERM_MEMORIES_PER_BATCH {
            return Err(WorkspaceError::BatchTooLarge);
        }

        let short_memory_limits = load_ingest_config(&self.config)?.short_memory;
        let prepared: Vec<PreparedMemory> = items
            .iter()
            .map(|item| prepare_short_term_memory(item, &short_memory_limits))
            .collect::<Result<_, WorkspaceError>>()?;

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_active_session(&transaction, session_id)?;

        let mut records = Vec::with_capacity(prepared.len());
        for (memory, input) in prepared.iter().zip(&items) {
            let now = now_iso8601();
            transaction.execute(
                "insert into short_term_memories(
                   session_id,
                   source_role,
                   kind,
                   text,
                   source_ref,
                   metadata_json,
                   source_credibility,
                   rule_intent,
                   soft_origin,
                   created_at
                 )
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    session_id,
                    memory.source_role,
                    memory.kind,
                    memory.text,
                    memory.source_ref,
                    memory.metadata_json,
                    memory.source_credibility,
                    memory.rule_intent,
                    memory.soft_origin,
                    now,
                ],
            )?;
            let memory_id = transaction.last_insert_rowid();
            insert_metadata_values(
                &transaction,
                "short_term_memory_language_tags",
                "memory_id",
                memory_id,
                "language_tag",
                &memory.language_tags,
            )?;
            insert_metadata_values(
                &transaction,
                "short_term_memory_story_scopes",
                "memory_id",
                memory_id,
                "story_scope",
                &memory.story_scopes,
            )?;
            insert_metadata_values(
                &transaction,
                "short_term_memory_semantic_tags",
                "memory_id",
                memory_id,
                "semantic_tag",
                &memory.semantic_tags,
            )?;
            transaction.execute(
                "insert into short_term_memories_fts(rowid, text) values (?1, ?2)",
                rusqlite::params![memory_id, memory.text],
            )?;
            if input.claims.is_empty() {
                let context = self
                    .get_session_with_connection(&transaction, session_id)?
                    .context;
                crate::claim_capture::capture_context_tx(
                    &transaction,
                    crate::claim_reads::ClaimTarget::ShortTerm(memory_id),
                    &memory.text,
                    &context,
                    None,
                )?;
            } else {
                for claim in &input.claims {
                    crate::claim_capture::capture_claim_tx(
                        &transaction,
                        crate::claim_reads::ClaimTarget::ShortTerm(memory_id),
                        claim,
                    )?;
                }
            }
            records.push(hydrate_memory(&transaction, memory_id)?);
        }
        transaction.execute(
            "update task_sessions set last_activity_at = ?1 where id = ?2",
            rusqlite::params![now_iso8601(), session_id],
        )?;
        let captures = records
            .into_iter()
            .map(|memory| {
                let claims = Self::captured_short_term_claims(&transaction, memory.id)?;
                Ok(ShortTermCapture { memory, claims })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        transaction.commit()?;
        Ok(captures)
    }

    pub fn list_short_term_memories(
        &self,
        session_id: i64,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        let connection = self.connection()?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select id from short_term_memories
                 where session_id = ?1 and archived_at is null order by id",
            )?;
            let rows = statement.query_map([session_id], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_memory(&connection, id)?);
        }
        Ok(records)
    }

    fn captured_short_term_claims(
        connection: &Connection,
        memory_id: i64,
    ) -> Result<Vec<CapturedShortTermClaim>, WorkspaceError> {
        let mut statement = connection.prepare(
            "select c.id,c.revision,a.metadata_state,
                    exists(select 1 from knowledge_gates g where g.applicability_id=a.id)
             from memory_claims c
             join claim_bindings b on b.claim_id=c.id
             join applicabilities a on a.id=c.applicability_id
             where b.short_term_id=?1 order by c.id",
        )?;
        let rows = statement.query_map([memory_id], |row| {
            let metadata_state: String = row.get(2)?;
            let has_knowledge_gate: bool = row.get(3)?;
            let mut warnings = Vec::new();
            if metadata_state != "resolved" {
                warnings.push("unresolved_story_context");
            }
            if !has_knowledge_gate {
                warnings.push("missing_knowledge_gate");
            }
            Ok(CapturedShortTermClaim {
                claim_id: row.get(0)?,
                revision: row.get(1)?,
                warnings,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn search_short_term_memories(
        &self,
        session_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        let connection = self.connection()?;
        self.search_short_term_memories_with_connection(&connection, None, session_id, query, limit)
    }

    pub(crate) fn search_short_term_memories_with_connection(
        &self,
        connection: &Connection,
        current: Option<&crate::story_applicability::StoryQueryV1>,
        session_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ShortTermMemoryRecord>, WorkspaceError> {
        if limit < 1 {
            return Err(WorkspaceError::LimitTooSmall);
        }
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        if limit > MAX_SEARCH_LIMIT {
            return Err(WorkspaceError::LimitTooSmall);
        }
        let mut statement = connection.prepare(
            "select short_term_memories.id
             from short_term_memories_fts
             join short_term_memories
               on short_term_memories.id = short_term_memories_fts.rowid
             where short_term_memories_fts match ?1
               and short_term_memories.session_id = ?2
               and short_term_memories.archived_at is null
              order by bm25(short_term_memories_fts), short_term_memories.id
             limit ?3",
        )?;
        let ids = statement
            .query_map(
                rusqlite::params![
                    expression,
                    session_id,
                    if current.is_some() {
                        crate::claim_reads::CANDIDATE_BUDGET as i64
                    } else {
                        limit as i64
                    }
                ],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let ids = crate::claim_reads::select_candidates(
            connection,
            ids,
            |id| crate::claim_reads::ClaimTarget::ShortTerm(*id),
            current,
            limit,
        )?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            records.push(hydrate_memory(connection, id)?);
        }
        Ok(records)
    }

    fn connection(&self) -> Result<Connection, WorkspaceError> {
        Ok(open_migrated(&self.config.database_path())?)
    }
}

pub fn ensure_series_exists(
    config: &HieronymusConfig,
    series_slug: &str,
) -> Result<(), WorkspaceError> {
    let registry = Registry::open(config)?;
    match registry.get_series(series_slug) {
        Ok(_) => Ok(()),
        Err(crate::registry::RegistryError::UnknownSeries(_)) => {
            Err(WorkspaceError::UnknownSeries(series_slug.to_string()))
        }
        Err(error) => Err(error.into()),
    }
}

fn require_active_session(connection: &Connection, session_id: i64) -> Result<(), WorkspaceError> {
    let status: Option<String> = connection
        .query_row(
            "select status from task_sessions where id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match status.as_deref() {
        None => Err(WorkspaceError::UnknownSession(session_id)),
        Some("active") => Ok(()),
        Some(_) => Err(WorkspaceError::InactiveSession),
    }
}

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
    soft_origin: String,
}

fn prepare_short_term_memory(
    item: &ShortTermMemoryInput,
    short_memory_limits: &crate::ingest_config::ShortMemoryLimits,
) -> Result<PreparedMemory, WorkspaceError> {
    let source_role = item.source_role.trim();
    if source_role.is_empty() {
        return Err(WorkspaceError::Json("source_role must not be empty".into()));
    }
    let kind = item.kind.trim();
    if kind.is_empty() {
        return Err(WorkspaceError::Json("kind must not be empty".into()));
    }
    let text = item.text.trim().to_string();
    let validation = validate_short_memory_text(&text, short_memory_limits)?;

    let normalized_language_tags =
        normalize_string_tuple(item.language_tags.iter().map(String::as_str), true);
    let normalized_story_scopes =
        normalize_string_tuple(item.story_scopes.iter().map(String::as_str), false);
    let normalized_semantic_tags =
        normalize_string_tuple(item.semantic_tags.iter().map(String::as_str), false);

    let source_credibility = match item.source_credibility.trim() {
        "" => "observation".to_string(),
        trimmed => trimmed.to_string(),
    };
    let rule_intent = item.rule_intent.trim().to_string();
    let soft_origin = item.soft_origin.trim().to_string();

    let mut metadata: MetadataMap = item.metadata.clone().unwrap_or_default();
    for reserved in ["sentence_count", "symbol_count", "validation_warning"] {
        metadata.remove(reserved);
    }
    if !normalized_language_tags.is_empty() {
        metadata.insert("language_tags".into(), json!(normalized_language_tags));
    }
    if !normalized_story_scopes.is_empty() {
        metadata.insert("story_scopes".into(), json!(normalized_story_scopes));
    }
    if !normalized_semantic_tags.is_empty() {
        metadata.insert("semantic_tags".into(), json!(normalized_semantic_tags));
    }
    if source_credibility != "observation" {
        metadata.insert("source_credibility".into(), json!(source_credibility));
    }
    if !rule_intent.is_empty() {
        metadata.insert("rule_intent".into(), json!(rule_intent));
    }
    if !soft_origin.is_empty() {
        metadata.insert("soft_origin".into(), json!(soft_origin));
    }
    metadata.insert(
        "sentence_count".into(),
        json!(validation.sentence_count as i64),
    );
    metadata.insert("symbol_count".into(), json!(validation.symbol_count as i64));
    if !validation.warning.is_empty() {
        metadata.insert("validation_warning".into(), json!(validation.warning));
    }
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|error| WorkspaceError::Json(error.to_string()))?;

    Ok(PreparedMemory {
        source_role: source_role.to_string(),
        kind: kind.to_string(),
        text,
        source_ref: item.source_ref.clone(),
        metadata_json,
        language_tags: normalized_language_tags,
        story_scopes: normalized_story_scopes,
        semantic_tags: normalized_semantic_tags,
        source_credibility,
        rule_intent,
        soft_origin,
    })
}

pub(crate) fn hydrate_memory(
    connection: &Connection,
    memory_id: i64,
) -> Result<ShortTermMemoryRecord, WorkspaceError> {
    let row = connection
        .query_row(
            "select id, session_id, source_role, kind, text, source_ref, metadata_json,
                    source_credibility, rule_intent, soft_origin
             from short_term_memories where id = ?1",
            [memory_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                WorkspaceError::Json(format!("unknown short-term memory: {memory_id}"))
            }
            other => other.into(),
        })?;
    let metadata: MetadataMap =
        serde_json::from_str(&row.6).map_err(|error| WorkspaceError::Json(error.to_string()))?;
    let language_tags = match load_metadata_values(
        connection,
        "short_term_memory_language_tags",
        "memory_id",
        memory_id,
        "language_tag",
    )? {
        tags if !tags.is_empty() => tags,
        _ => metadata_strings(metadata.get("language_tags"), true),
    };
    let story_scopes = match load_metadata_values(
        connection,
        "short_term_memory_story_scopes",
        "memory_id",
        memory_id,
        "story_scope",
    )? {
        scopes if !scopes.is_empty() => scopes,
        _ => metadata_strings(metadata.get("story_scopes"), false),
    };
    let semantic_tags = match load_metadata_values(
        connection,
        "short_term_memory_semantic_tags",
        "memory_id",
        memory_id,
        "semantic_tag",
    )? {
        tags if !tags.is_empty() => tags,
        _ => metadata_strings(metadata.get("semantic_tags"), false),
    };
    Ok(ShortTermMemoryRecord {
        claim_annotation: crate::claim_reads::source_annotation(
            connection,
            crate::claim_reads::ClaimTarget::ShortTerm(memory_id),
        )?,
        id: row.0,
        session_id: row.1,
        source_role: row.2,
        kind: row.3,
        text: row.4,
        source_ref: row.5,
        metadata: metadata.clone(),
        language_tags,
        story_scopes,
        semantic_tags,
        source_credibility: row
            .7
            .filter(|value| !value.is_empty())
            .or_else(|| metadata_string(&metadata, "source_credibility"))
            .unwrap_or_else(|| "observation".to_string()),
        rule_intent: row
            .8
            .filter(|value| !value.is_empty())
            .or_else(|| metadata_string(&metadata, "rule_intent"))
            .unwrap_or_default(),
        soft_origin: row
            .9
            .filter(|value| !value.is_empty())
            .or_else(|| metadata_string(&metadata, "soft_origin"))
            .unwrap_or_default(),
    })
}

fn metadata_string(metadata: &MetadataMap, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(String::from)
}

fn metadata_strings(value: Option<&serde_json::Value>, lowercase: bool) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|item| {
                let text = item.trim().to_string();
                if lowercase { text.to_lowercase() } else { text }
            })
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn insert_metadata_values(
    connection: &Connection,
    table: &str,
    owner_column: &str,
    owner_id: i64,
    value_column: &str,
    values: &[String],
) -> Result<(), WorkspaceError> {
    for value in values {
        // Table/column names come from call sites inside this module, never
        // from user input; values are bound parameters.
        connection.execute(
            &format!(
                "insert or ignore into {table}({owner_column}, {value_column})
                 values (?1, ?2)"
            ),
            rusqlite::params![owner_id, value],
        )?;
    }
    Ok(())
}

fn load_metadata_values(
    connection: &Connection,
    table: &str,
    owner_column: &str,
    owner_id: i64,
    value_column: &str,
) -> Result<Vec<String>, WorkspaceError> {
    let mut statement = connection.prepare(&format!(
        "select {value_column} from {table}
         where {owner_column} = ?1 order by {value_column}"
    ))?;
    let rows = statement.query_map([owner_id], |row| row.get::<_, String>(0))?;
    let mut values = Vec::new();
    for value in rows {
        values.push(value?);
    }
    Ok(values)
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

fn validate_session_context(
    db: &Connection,
    context: &TranslationContext,
) -> Result<(), WorkspaceError> {
    use crate::story_applicability::{
        ApplicabilityError, QueryMode, StoryApplicability, Viewpoint,
    };
    if context.story_query_mode != QueryMode::Current {
        return Err(WorkspaceError::ResearchSession);
    }
    // Legacy sessions may predate registration. Supplied story context must resolve
    // ownership, while absent position/order remains unknown.
    if context.story_timeline_id.is_some()
        || context.story_scene_key.is_some()
        || context.story_viewpoint != Viewpoint::Unspecified
    {
        StoryApplicability::resolve_context(db, context)?;
        if let Viewpoint::Character(id) = context.story_viewpoint {
            let valid: bool = db.query_row("select exists(select 1 from concepts where id=?1 and (scope_type='global' or scope_type='series' and scope_key='series:'||?2))", rusqlite::params![id,context.series_slug], |r| r.get(0))?;
            if !valid {
                return Err(ApplicabilityError::WrongSeries.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_path_writes_typed_metadata_and_activity() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("hieronymus"));
        let registry = Registry::open(&config).unwrap();
        registry
            .create_series("demo", "Demo", "ja", "en", None)
            .unwrap();
        let store = WorkspaceStore::open(&config).unwrap();
        let context = TranslationContext::new("demo", "ja", "en", "translation")
            .volume("1")
            .chapter("2");
        let session = store.start_session(&context).unwrap();

        assert_eq!(session.status, "active");
        assert_eq!(session.context.story_scopes, vec!["volume:1", "chapter:2"]);
        assert_eq!(session.context.language_tags, vec!["ja", "en"]);
        let loaded = store.get_session(session.id).unwrap();
        // Round-trip through side tables rehydrates in their stored order;
        // assert on observable fields like the Python suite does.
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.status, "active");
        assert_eq!(loaded.context.series_slug, "demo");
        assert_eq!(loaded.context.volume, "1");
        assert_eq!(loaded.context.chapter, "2");
        assert_eq!(loaded.context.story_scopes, vec!["chapter:2", "volume:1"]);
        assert!(store.complete_session(session.id).unwrap());
        assert!(!store.complete_session(session.id).unwrap());
    }
}

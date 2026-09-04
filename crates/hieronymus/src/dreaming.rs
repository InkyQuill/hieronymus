//! Dreaming core (slice 5 of the Rust port, per
//! `docs/superpowers/specs/2026-08-31-rust-dreaming-design.md`): bounded,
//! auditable dream cycles with explicit mutation ownership.
//!
//! One cycle: OS lock acquisition (one nonblocking `try_lock`, held for the
//! whole cycle), a durable run row, a bounded selection of completed-session
//! short-term memories, the seven evidence passes behind an injected
//! [`DreamProvider`], one validated mutation batch applied transactionally,
//! and audit entries covering inputs, outputs, parse decisions, and
//! mutations with redacted payloads. `open` is the fail-closed workflow
//! gate: every enabled `dream.conf` workflow assignment must resolve
//! against the `provider.conf` catalog before any cycle runs (spec
//! §Provider Policy).
//!
//! Real configured LLM clients live in [`crate::dream_providers`] behind the
//! same [`DreamProvider`] seam. Still out of scope here: concept/facet
//! application, supersede/reinforce provider actions, passive feedback
//! events, decay/maintenance, and scheduler timers. Provider output
//! sections that would feed those slices fail the run closed instead of
//! being silently dropped.

use std::collections::{BTreeMap, HashSet};

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::dream_audit::DreamAuditStore;
use crate::dream_config::{DREAM_WORKFLOW_NAMES, DreamConfig, load_dream_config};
use crate::dream_locks::{DreamCycleState, DreamLockError, dream_cycle_lock};
use crate::memory_models::{ShortTermMemoryRecord, TranslationContext};
use crate::provider_config::{ProviderCatalog, load_provider_catalog};

pub const ALLOWED_CRYSTAL_TYPES: [&str; 7] = [
    "lesson",
    "rule",
    "thought",
    "observation",
    "concept_note",
    "concept",
    "erudition",
];

pub const MALFORMED_CONFIDENCE_PENALTY: f64 = 0.2;

const MIN_NORMALIZED_CONFIDENCE: f64 = 0.05;

/// Python `SOURCE_CREDIBILITY_CONFIDENCE` with the `observation` fallback.
fn source_credibility_confidence(source_credibility: &str) -> f64 {
    match source_credibility {
        "rumor" => 0.15,
        "source_text" => 0.7,
        "expert" => 0.85,
        "user_suggestion" => 0.8,
        "user_rule" => 0.95,
        "thought" => 0.2,
        _ => 0.35,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DreamError {
    #[error("dream cycle already running{}", Self::already_running_detail(.0))]
    AlreadyRunning(Option<DreamCycleState>),
    #[error(transparent)]
    Lock(#[from] DreamLockError),
    #[error(transparent)]
    Config(#[from] crate::dream_config::DreamConfigError),
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    InvalidOutput(String),
    /// Fail-closed workflow gate rejection at `open` time: names the workflow
    /// and the problem (no run context exists yet, so the text is the audit).
    #[error("{0}")]
    InvalidWorkflow(String),
    #[error(transparent)]
    Catalog(#[from] crate::provider_config::ProviderCatalogError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Audit(#[from] crate::dream_audit::DreamAuditError),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
    #[error(transparent)]
    Workspace(#[from] crate::workspace::WorkspaceError),
    #[error("{0}")]
    Json(String),
}

impl DreamError {
    fn already_running_detail(state: &Option<DreamCycleState>) -> String {
        match state {
            Some(state) => format!(" by {} pid {}", state.owner, state.pid),
            None => String::new(),
        }
    }

    /// Build a provider failure from any message; the text is stored
    /// (redacted) on the failed run exactly as the provider supplied it.
    pub fn provider(message: impl Into<String>) -> Self {
        Self::Provider(message.into())
    }
}

/// The durable outcome of one dream run (port of `DreamRunRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct DreamRunRecord {
    pub id: i64,
    pub cycle_id: i64,
    pub status: String,
    pub provider: String,
    pub input_count: i64,
    pub created_crystal_count: i64,
    pub proposal_count: i64,
    pub error: String,
}

/// Provider access seam. The dreaming core never touches the network; real
/// configured LLM clients live in [`crate::dream_providers`] behind this
/// trait. Phase orchestration stays typed: the seven passes are fixed calls,
/// not erased trait objects.
pub trait DreamProvider {
    fn name(&self) -> &str;

    /// Provider profile id recorded on phase runs and audit entries.
    fn profile_name(&self) -> &str {
        self.name()
    }

    /// Model id recorded on phase runs and audit entries.
    fn model(&self) -> &str {
        self.name()
    }

    /// True only for the deterministic provider. The open-time gate uses this
    /// to refuse deterministic substitution for LLM-declared workflows (spec
    /// §Provider Policy: it never silently replaces a configured workflow).
    fn is_deterministic(&self) -> bool {
        false
    }

    /// Run one evidence pass over the bounded selection. The returned JSON
    /// must be an object (the phase payload schema).
    fn run_pass(
        &self,
        pass_name: &str,
        context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError>;
}

/// The deterministic provider: tests, diagnostics, and workflows explicitly
/// declared deterministic. It derives rule/concept crystals from memory
/// credibility and rule intent — never a silent fallback for a failed LLM.
pub struct DeterministicDreamProvider;

impl DreamProvider for DeterministicDreamProvider {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn is_deterministic(&self) -> bool {
        true
    }

    fn run_pass(
        &self,
        pass_name: &str,
        _context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        if pass_name == "coverage_audit" {
            let ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
            return Ok(json!({ "covered_memory_ids": ids }));
        }
        if pass_name != "knowledge_crystals" {
            return Ok(json!({}));
        }
        let crystals: Vec<Value> = memories
            .iter()
            .map(|memory| {
                let crystal_type = if memory.source_credibility == "user_rule"
                    || !memory.rule_intent.trim().is_empty()
                {
                    "rule"
                } else {
                    "concept"
                };
                json!({
                    "crystal_type": crystal_type,
                    "title": title_from_kind(&memory.kind),
                    "text": normalize_candidate_text(&memory.text),
                    "strength": 0.6,
                    "confidence": source_credibility_confidence(&memory.source_credibility),
                    "source_memory_ids": [memory.id],
                    "source_credibility": memory.source_credibility,
                    "rule_intent": memory.rule_intent,
                })
            })
            .collect();
        Ok(json!({ "crystals": crystals }))
    }
}

/// A recoverable parse decision (port of `DreamParseWarning`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParseWarning {
    pub entry_path: String,
    pub code: String,
    pub message: String,
    pub confidence_penalty: f64,
}

/// A validated, normalized crystal candidate (port of `_NormalizedDreamCrystal`).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedCrystal {
    pub crystal_type: String,
    pub title: String,
    pub text: String,
    pub strength: f64,
    pub confidence: f64,
    pub source_memory_ids: Vec<i64>,
    pub source_credibility: String,
    pub rule_intent: String,
    pub malformed_penalty: f64,
    pub is_inferred: bool,
    pub supersedes_crystal_id: Option<i64>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub concept_ids: Vec<i64>,
}

/// One pass's normalized output (port of `_NormalizedDreamOutput`, reduced to
/// the sections the dreaming core applies in this slice).
#[derive(Debug, Clone, Default)]
pub struct NormalizedOutput {
    pub crystals: Vec<NormalizedCrystal>,
    pub warnings: Vec<ParseWarning>,
    pub skipped_candidates: Vec<Value>,
}

/// Provider sections the dreaming core does not apply in this slice.
const UNSUPPORTED_OUTPUT_SECTIONS: [&str; 4] = ["concepts", "facets", "supersede", "reinforce"];

/// Provider output sections that resolve concepts by name; concept
/// application is a later slice, so they are ignored with an audited warning.
const UNSUPPORTED_CONCEPT_NAME_KEYS: [&str; 3] = ["concept_names", "concepts", "concept_name"];

struct SelectionGroup {
    session_id: i64,
    context: TranslationContext,
    memories: Vec<ShortTermMemoryRecord>,
}

struct ApplySummary {
    created_crystal_ids: Vec<i64>,
    archived_memory_ids: Vec<i64>,
    dreamed_session_ids: Vec<i64>,
    rejected_entries: Vec<Value>,
    skipped_candidates: Vec<Value>,
    related_candidates: Value,
    affected_memory_set: Value,
}

/// The dreaming service: typed phase orchestration over the data root with an
/// injected provider. One instance may run any number of cycles; each cycle
/// takes the OS dream-cycle lock.
pub struct DreamService<P: DreamProvider> {
    config: HieronymusConfig,
    dream_config: DreamConfig,
    provider: P,
    audit: DreamAuditStore,
}

/// `Debug` names the provider instead of dumping it, so `unwrap_err` in
/// tests and any diagnostic path stay redacted by construction.
impl<P: DreamProvider> std::fmt::Debug for DreamService<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DreamService")
            .field("provider", &self.provider.name())
            .finish_non_exhaustive()
    }
}

impl<P: DreamProvider> DreamService<P> {
    pub fn open(config: &HieronymusConfig, provider: P) -> Result<Self, DreamError> {
        let dream_config = load_dream_config(config)?;
        // The fail-closed workflow gate (spec §Provider Policy): every
        // enabled workflow assignment must resolve against provider.conf and
        // match the injected provider before the service accepts any run.
        // There is no run context here, so the precise redacted error text
        // is the audit record.
        let catalog = load_provider_catalog(config)?;
        validate_workflow_wiring(
            &dream_config,
            &catalog,
            provider.is_deterministic(),
            provider.profile_name(),
            provider.model(),
        )?;
        let audit = DreamAuditStore::open(config)?;
        Ok(Self {
            config: config.clone(),
            dream_config,
            provider,
            audit,
        })
    }

    /// One dream cycle over one bounded selection (`run_cycle`).
    pub fn run_cycle(
        &self,
        owner: &str,
        skip_when_locked: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        self.run_locked(owner, true, skip_when_locked)
    }

    /// Drain every pending completed-session memory in one run (`run_all`).
    pub fn run_all(
        &self,
        owner: &str,
        ignore_minimum: bool,
        skip_when_locked: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        self.run_locked(owner, ignore_minimum, skip_when_locked)
    }

    fn run_locked(
        &self,
        owner: &str,
        ignore_minimum: bool,
        skip_when_locked: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        let trigger_type = trigger_type_from_owner(owner);
        // The guard is held for the whole cycle and released on drop; there
        // is deliberately no wait/retry path around it. `_lock` is a real
        // binding: it keeps the file handle alive until this function returns.
        let lock = match dream_cycle_lock(&self.config, owner) {
            Ok(lock) => lock,
            Err(DreamLockError::AlreadyRunning { state }) => {
                if skip_when_locked {
                    return self.record_skipped_run("dream cycle already running");
                }
                return Err(DreamError::AlreadyRunning(state));
            }
            Err(error) => return Err(error.into()),
        };
        let _lock = lock;
        self.run_evidence_pass_cycle(&trigger_type, ignore_minimum)
    }

    fn run_evidence_pass_cycle(
        &self,
        trigger_type: &str,
        ignore_minimum: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let cycle_id = next_cycle_id(&connection)?;
        connection.execute(
            "insert into dream_runs(cycle_id, status, provider, created_at)
             values (?1, 'running', ?2, ?3)",
            rusqlite::params![cycle_id, self.provider.name(), now()],
        )?;
        let run_id = connection.last_insert_rowid();
        drop(connection);

        match self.execute_passes(run_id, cycle_id, trigger_type, ignore_minimum) {
            Ok(record) => Ok(record),
            Err(error) => {
                self.fail_run(run_id, &error)?;
                Err(error)
            }
        }
    }

    fn execute_passes(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        ignore_minimum: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        let mut phase_run_ids: Vec<i64> = Vec::new();
        match self.execute_passes_inner(
            run_id,
            cycle_id,
            trigger_type,
            ignore_minimum,
            &mut phase_run_ids,
        ) {
            Ok(record) => Ok(record),
            Err(error) => {
                // Any failure marks every phase record of the run failed, so
                // no phase can be mistaken for applied work.
                for phase_run_id in &phase_run_ids {
                    let _ = self.fail_phase_run(*phase_run_id, &error);
                }
                Err(error)
            }
        }
    }

    fn execute_passes_inner(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        ignore_minimum: bool,
        phase_run_ids: &mut Vec<i64>,
    ) -> Result<DreamRunRecord, DreamError> {
        let pending_count = self.pending_short_term_memory_count()?;
        let threshold_state = self.threshold_state(pending_count, ignore_minimum, trigger_type);
        if !ignore_minimum && pending_count < self.dream_config.min_pending_short_term_memories {
            return self.complete_run(run_id, cycle_id, 0, 0, 0);
        }

        // Selection: the bounded affected-memory set. One snapshot feeds
        // every pass, capped by max_short_term_memories_per_run.
        let groups = self.select_pending_completed_groups(
            self.dream_config.max_short_term_memories_per_run as usize,
        )?;
        if groups.is_empty() {
            return self.complete_run(run_id, cycle_id, 0, 0, 0);
        }
        let selected_memory_ids: Vec<i64> = groups
            .iter()
            .flat_map(|group| group.memories.iter().map(|memory| memory.id))
            .collect();
        let allowed_memory_ids: HashSet<i64> = selected_memory_ids.iter().copied().collect();
        let selection_context = groups[0].context.clone();
        let selected_memories: Vec<ShortTermMemoryRecord> = groups
            .iter()
            .flat_map(|group| group.memories.iter().cloned())
            .collect();
        let valid_concept_ids = self.valid_concept_ids()?;

        let mut covered_memory_ids: HashSet<i64> = HashSet::new();
        let mut staged: Vec<NormalizedOutput> = Vec::new();

        for pass_name in DREAM_WORKFLOW_NAMES {
            let phase_run_id =
                self.start_phase_run(run_id, pass_name, selected_memory_ids.len() as i64)?;
            phase_run_ids.push(phase_run_id);
            self.audit_provider_request(
                run_id,
                Some(phase_run_id),
                trigger_type,
                &threshold_state,
                &selected_memory_ids,
                pass_name,
                &groups,
            )?;

            let raw = self
                .provider
                .run_pass(pass_name, &selection_context, &selected_memories)?;
            if !raw.is_object() {
                return Err(DreamError::InvalidOutput(format!(
                    "{pass_name} output must be an object"
                )));
            }

            if pass_name == "coverage_audit" {
                let covered = coverage_ids(&raw, &allowed_memory_ids)?;
                let covered_count = covered.len() as i64;
                covered_memory_ids.extend(covered);
                self.complete_phase_run(phase_run_id, covered_count)?;
                self.audit_provider_response(
                    run_id,
                    Some(phase_run_id),
                    trigger_type,
                    &threshold_state,
                    &selected_memory_ids,
                    pass_name,
                    &self.response_summary(&[]),
                )?;
                continue;
            }

            let output = normalize_dict_output(&raw, &allowed_memory_ids, &valid_concept_ids)?;
            if !output.warnings.is_empty() {
                self.audit_parse_warnings(
                    run_id,
                    Some(phase_run_id),
                    trigger_type,
                    &threshold_state,
                    &selected_memory_ids,
                    pass_name,
                    &output.warnings,
                )?;
            }
            validate_normalized_output(&output, &selection_context, &allowed_memory_ids)?;
            self.validate_pass_output(pass_name, &output)?;
            let output_count = output.crystals.len() as i64;
            let response_summary = self.response_summary(std::slice::from_ref(&output));
            staged.push(output);
            self.complete_phase_run(phase_run_id, output_count)?;
            self.audit_provider_response(
                run_id,
                Some(phase_run_id),
                trigger_type,
                &threshold_state,
                &selected_memory_ids,
                pass_name,
                &response_summary,
            )?;
        }

        // The coverage pass must account for every selected memory id before
        // anything is applied.
        let mut missing: Vec<i64> = allowed_memory_ids
            .difference(&covered_memory_ids)
            .copied()
            .collect();
        missing.sort_unstable();
        if !missing.is_empty() {
            let rendered = missing
                .iter()
                .map(|memory_id| memory_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(DreamError::InvalidOutput(format!(
                "coverage_incomplete: {rendered}"
            )));
        }

        let staged_record_count: usize = staged.iter().map(|output| output.crystals.len()).sum();
        if staged_record_count as i64 > self.dream_config.max_long_term_records_affected_per_run {
            return Err(DreamError::InvalidOutput(
                "dream run exceeds max_long_term_records_affected_per_run".to_string(),
            ));
        }

        // Persistence: one validated mutation batch in one transaction.
        let persistence_phase_run_id =
            self.start_phase_run(run_id, "persistence", selected_memory_ids.len() as i64)?;
        phase_run_ids.push(persistence_phase_run_id);
        let summary = self.apply_outputs(run_id, cycle_id, &groups, &staged, &selection_context)?;
        self.complete_phase_run(
            persistence_phase_run_id,
            summary.created_crystal_ids.len() as i64,
        )?;
        self.audit_phase_completed(
            run_id,
            Some(persistence_phase_run_id),
            trigger_type,
            &threshold_state,
            &selected_memory_ids,
            &groups,
            &staged,
            &summary,
        )?;

        self.complete_run(
            run_id,
            cycle_id,
            selected_memory_ids.len() as i64,
            summary.created_crystal_ids.len() as i64,
            0,
        )
    }

    // ------------------------------------------------------------------
    // Selection
    // ------------------------------------------------------------------

    fn pending_short_term_memory_count(&self) -> Result<i64, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let count = connection.query_row(
            "select count(*)
             from short_term_memories
             join task_sessions on task_sessions.id = short_term_memories.session_id
             where task_sessions.status = 'completed'
               and short_term_memories.archived_at is null",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count)
    }

    /// Port of `_load_pending_completed_groups`: at most `limit` pending
    /// memories from completed sessions, grouped by session, ordered by
    /// session then memory id.
    fn select_pending_completed_groups(
        &self,
        limit: usize,
    ) -> Result<Vec<SelectionGroup>, DreamError> {
        if limit < 1 {
            return Ok(Vec::new());
        }
        let connection = open_migrated(&self.config.database_path())?;
        let mut statement = connection.prepare(
            "select task_sessions.id, short_term_memories.id
             from short_term_memories
             join task_sessions on task_sessions.id = short_term_memories.session_id
             where task_sessions.status = 'completed'
               and short_term_memories.archived_at is null
             order by task_sessions.id, short_term_memories.id
             limit ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut session_order: Vec<i64> = Vec::new();
        let mut by_session: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for row in rows {
            let (session_id, memory_id) = row?;
            if !by_session.contains_key(&session_id) {
                session_order.push(session_id);
            }
            by_session.entry(session_id).or_default().push(memory_id);
        }
        drop(statement);
        drop(connection);

        let workspace = crate::workspace::WorkspaceStore::open(&self.config)?;
        let mut groups = Vec::with_capacity(session_order.len());
        for session_id in session_order {
            let session = workspace.get_session(session_id)?;
            let wanted: HashSet<i64> = by_session[&session_id].iter().copied().collect();
            let memories: Vec<ShortTermMemoryRecord> = workspace
                .list_short_term_memories(session_id)?
                .into_iter()
                .filter(|memory| wanted.contains(&memory.id))
                .collect();
            groups.push(SelectionGroup {
                session_id,
                context: session.context,
                memories,
            });
        }
        Ok(groups)
    }

    fn valid_concept_ids(&self) -> Result<HashSet<i64>, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let mut statement = connection
            .prepare("select id from concepts where status not in ('archived', 'merged')")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        let mut ids = HashSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    }

    // ------------------------------------------------------------------
    // Persistence
    // ------------------------------------------------------------------

    /// Apply every staged crystal as one validated mutation batch:
    /// crystals plus their source links, then memory archiving, then session
    /// marking — all in one transaction, rolled back entirely on failure.
    fn apply_outputs(
        &self,
        _run_id: i64,
        cycle_id: i64,
        groups: &[SelectionGroup],
        staged: &[NormalizedOutput],
        context: &TranslationContext,
    ) -> Result<ApplySummary, DreamError> {
        let outputs = deduplicate_staged_outputs(staged);
        let mut created_crystal_ids: Vec<i64> = Vec::new();
        let mut skipped_candidates: Vec<Value> = Vec::new();

        let mut connection = open_migrated(&self.config.database_path())?;
        let transaction = connection.transaction()?;
        for output in &outputs {
            skipped_candidates.extend(output.skipped_candidates.iter().cloned());
            for candidate in &output.crystals {
                let crystal_id = insert_dream_crystal(&transaction, context, candidate, cycle_id)?;
                created_crystal_ids.push(crystal_id);
            }
        }
        let archived_memory_ids: Vec<i64> = groups
            .iter()
            .flat_map(|group| group.memories.iter().map(|memory| memory.id))
            .collect();
        for memory_id in &archived_memory_ids {
            transaction.execute(
                "update short_term_memories set archived_at = ?1 where id = ?2",
                rusqlite::params![now(), memory_id],
            )?;
        }
        let dreamed_session_ids: Vec<i64> = groups.iter().map(|group| group.session_id).collect();
        for session_id in &dreamed_session_ids {
            transaction.execute(
                "update task_sessions
                 set status = 'dreamed', cycle_id = ?1
                 where status = 'completed'
                   and id = ?2
                   and not exists (
                     select 1 from short_term_memories
                     where short_term_memories.session_id = task_sessions.id
                       and archived_at is null
                   )",
                rusqlite::params![cycle_id, session_id],
            )?;
        }
        transaction.commit()?;
        drop(connection);

        // Bounded affected-memory set for the audit record. Related
        // candidates activate when the concept-application slice starts
        // creating concepts; the caps are part of this contract already.
        let related_candidates = self.searched_related_candidates(&[])?;
        let affected_memory_set =
            self.affected_memory_set(&created_crystal_ids, &related_candidates);
        Ok(ApplySummary {
            created_crystal_ids,
            archived_memory_ids,
            dreamed_session_ids,
            rejected_entries: Vec::new(),
            skipped_candidates,
            related_candidates,
            affected_memory_set,
        })
    }

    /// Port of `_searched_related_candidates` (caps live here so the concept
    /// slice cannot grow past them).
    fn searched_related_candidates(
        &self,
        created_concept_ids: &[i64],
    ) -> Result<Value, DreamError> {
        let concept_ids = unique_ints(created_concept_ids);
        let capped: Vec<i64> = concept_ids
            .into_iter()
            .take(self.dream_config.max_related_concepts_per_cycle.max(0) as usize)
            .collect();
        let connection = open_migrated(&self.config.database_path())?;
        let mut statement = connection.prepare(
            "select crystal_id from crystal_concepts
             where concept_id = ?1
             order by crystal_id
             limit ?2",
        )?;
        let mut crystals_by_concept = Vec::with_capacity(capped.len());
        for concept_id in &capped {
            let rows = statement.query_map(
                rusqlite::params![
                    concept_id,
                    self.dream_config.max_related_crystals_per_concept
                ],
                |row| row.get::<_, i64>(0),
            )?;
            let mut crystal_ids = Vec::new();
            for row in rows {
                crystal_ids.push(row?);
            }
            crystals_by_concept.push(json!({
                "concept_id": concept_id,
                "crystal_ids": crystal_ids,
            }));
        }
        Ok(json!({
            "concept_ids": capped,
            "crystals_by_concept": crystals_by_concept,
            "caps": {
                "max_related_concepts_per_cycle": self.dream_config.max_related_concepts_per_cycle,
                "max_related_crystals_per_concept": self
                    .dream_config
                    .max_related_crystals_per_concept,
            },
        }))
    }

    /// Port of `_affected_memory_set`: changed crystals first, then related
    /// candidates, all under the recorded caps.
    fn affected_memory_set(
        &self,
        changed_crystal_ids: &[i64],
        related_candidates: &Value,
    ) -> Value {
        let max_changed = self.dream_config.max_changed_crystals_per_cycle.max(0) as usize;
        let max_total = self.dream_config.max_total_affected_crystals.max(0) as usize;
        let mut changed_ids = unique_ints(changed_crystal_ids);
        changed_ids.truncate(max_changed);

        let mut related_ids: Vec<i64> = Vec::new();
        if let Some(items) = related_candidates
            .get("crystals_by_concept")
            .and_then(Value::as_array)
        {
            for item in items {
                if let Some(crystal_ids) = item.get("crystal_ids").and_then(Value::as_array) {
                    related_ids.extend(crystal_ids.iter().filter_map(Value::as_i64));
                }
            }
        }
        let changed_set: HashSet<i64> = changed_ids.iter().copied().collect();
        let related_ids: Vec<i64> = unique_ints(&related_ids)
            .into_iter()
            .filter(|crystal_id| !changed_set.contains(crystal_id))
            .collect();

        let mut all_crystal_ids: Vec<i64> = Vec::new();
        for crystal_id in changed_ids.iter().chain(related_ids.iter()) {
            if all_crystal_ids.len() >= max_total {
                break;
            }
            all_crystal_ids.push(*crystal_id);
        }
        let affected: HashSet<i64> = all_crystal_ids.iter().copied().collect();
        let capped_changed: Vec<i64> = changed_ids
            .into_iter()
            .filter(|crystal_id| affected.contains(crystal_id))
            .collect();
        let capped_related: Vec<i64> = related_ids
            .into_iter()
            .filter(|crystal_id| affected.contains(crystal_id))
            .collect();
        json!({
            "changed_crystal_ids": capped_changed,
            "related_crystal_ids": capped_related,
            "all_crystal_ids": all_crystal_ids,
            "total_crystal_count": all_crystal_ids.len(),
            "caps": {
                "max_changed_crystals_per_cycle": self.dream_config.max_changed_crystals_per_cycle,
                "max_total_affected_crystals": self.dream_config.max_total_affected_crystals,
            },
        })
    }

    // ------------------------------------------------------------------
    // Run registry
    // ------------------------------------------------------------------

    fn complete_run(
        &self,
        run_id: i64,
        cycle_id: i64,
        input_count: i64,
        created_crystal_count: i64,
        proposal_count: i64,
    ) -> Result<DreamRunRecord, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "update dream_runs
             set status = 'completed', input_count = ?1, created_crystal_count = ?2,
                 proposal_count = ?3, completed_at = ?4
             where id = ?5",
            rusqlite::params![
                input_count,
                created_crystal_count,
                proposal_count,
                now(),
                run_id
            ],
        )?;
        Ok(DreamRunRecord {
            id: run_id,
            cycle_id,
            status: "completed".to_string(),
            provider: self.provider.name().to_string(),
            input_count,
            created_crystal_count,
            proposal_count,
            error: String::new(),
        })
    }

    fn fail_run(&self, run_id: i64, error: &DreamError) -> Result<(), DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "update dream_runs
             set status = 'failed', input_count = 0, created_crystal_count = 0,
                 proposal_count = 0, error = ?1, completed_at = ?2
             where id = ?3",
            rusqlite::params![self.redacted_error_message(error), now(), run_id],
        )?;
        Ok(())
    }

    fn record_skipped_run(&self, reason: &str) -> Result<DreamRunRecord, DreamError> {
        let mut connection = open_migrated(&self.config.database_path())?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let cycle_id = next_skipped_cycle_id(&transaction)?;
        let timestamp = now();
        transaction.execute(
            "insert into dream_runs(cycle_id, status, provider, error, created_at, completed_at)
             values (?1, 'skipped', ?2, ?3, ?4, ?5)",
            rusqlite::params![cycle_id, self.provider.name(), reason, timestamp, timestamp],
        )?;
        let run_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(DreamRunRecord {
            id: run_id,
            cycle_id,
            status: "skipped".to_string(),
            provider: self.provider.name().to_string(),
            input_count: 0,
            created_crystal_count: 0,
            proposal_count: 0,
            error: reason.to_string(),
        })
    }

    fn start_phase_run(
        &self,
        run_id: i64,
        phase: &str,
        input_count: i64,
    ) -> Result<i64, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "insert into dream_phase_runs(
               dream_run_id, phase, provider_profile, provider_type, model, status,
               input_count, created_at
             )
             values (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7)",
            rusqlite::params![
                run_id,
                phase,
                self.provider.profile_name(),
                self.provider.name(),
                self.provider.model(),
                input_count,
                now(),
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    fn complete_phase_run(&self, phase_run_id: i64, output_count: i64) -> Result<(), DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "update dream_phase_runs
             set status = 'completed', output_count = ?1, completed_at = ?2
             where id = ?3",
            rusqlite::params![output_count, now(), phase_run_id],
        )?;
        Ok(())
    }

    fn fail_phase_run(&self, phase_run_id: i64, error: &DreamError) -> Result<(), DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "update dream_phase_runs
             set status = 'failed', error = ?1, completed_at = ?2
             where id = ?3",
            rusqlite::params![self.redacted_error_message(error), now(), phase_run_id],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Audit
    // ------------------------------------------------------------------

    fn audit_base(
        &self,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        phase_name: &str,
    ) -> serde_json::Map<String, Value> {
        let mut payload = serde_json::Map::new();
        payload.insert("trigger_type".into(), json!(trigger_type));
        payload.insert("threshold_state".into(), threshold_state.clone());
        payload.insert(
            "selected_short_term_memory_ids".into(),
            json!(selected_memory_ids),
        );
        payload.insert("phase_name".into(), json!(phase_name));
        payload.insert("prompt_version".into(), json!(format!("{phase_name}:v1")));
        payload.insert(
            "provider_profile".into(),
            json!(self.provider.profile_name()),
        );
        payload.insert("model".into(), json!(self.provider.model()));
        payload
    }

    fn request_summary(&self, groups: &[SelectionGroup]) -> Value {
        json!({
            "memory_count": groups
                .iter()
                .map(|group| group.memories.len())
                .sum::<usize>(),
            "session_ids": groups.iter().map(|group| group.session_id).collect::<Vec<_>>(),
            "context_count": groups.len(),
            "batch_cap": self.dream_config.max_short_term_memories_per_cycle,
        })
    }

    fn response_summary(&self, staged: &[NormalizedOutput]) -> Value {
        json!({
            "crystal_count": staged.iter().map(|output| output.crystals.len()).sum::<usize>(),
            "concept_count": 0,
            "facet_count": 0,
            "concept_proposal_count": 0,
            "supersede_action_count": 0,
            "parse_warning_count": staged
                .iter()
                .map(|output| output.warnings.len())
                .sum::<usize>(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_provider_request(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        pass_name: &str,
        groups: &[SelectionGroup],
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
        );
        payload.insert("request_summary".into(), self.request_summary(groups));
        self.audit.append(
            run_id,
            phase_run_id,
            "provider_request",
            "info",
            &format!("sent {pass_name} request"),
            &Value::Object(payload),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_provider_response(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        pass_name: &str,
        response_summary: &Value,
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
        );
        payload.insert("response_summary".into(), response_summary.clone());
        payload.insert("parse_warnings".into(), json!([]));
        self.audit.append(
            run_id,
            phase_run_id,
            "provider_response",
            "info",
            &format!("received {pass_name} response"),
            &Value::Object(payload),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_parse_warnings(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        pass_name: &str,
        warnings: &[ParseWarning],
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
        );
        payload.insert(
            "warnings".into(),
            json!(
                warnings
                    .iter()
                    .map(|warning| json!({
                        "entry_path": warning.entry_path,
                        "code": warning.code,
                        "message": warning.message,
                        "confidence_penalty": warning.confidence_penalty,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
        self.audit.append(
            run_id,
            phase_run_id,
            "parse_warnings",
            "warning",
            "dream response parsed with recoverable warnings",
            &Value::Object(payload),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_phase_completed(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        groups: &[SelectionGroup],
        staged: &[NormalizedOutput],
        summary: &ApplySummary,
    ) -> Result<(), DreamError> {
        let phase_name = "persistence";
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            phase_name,
        );
        payload.insert("request_summary".into(), self.request_summary(groups));
        payload.insert("response_summary".into(), self.response_summary(staged));
        let warnings: Vec<&ParseWarning> =
            staged.iter().flat_map(|output| &output.warnings).collect();
        payload.insert(
            "parse_warnings".into(),
            json!(
                warnings
                    .iter()
                    .map(|warning| json!({
                        "entry_path": warning.entry_path,
                        "code": warning.code,
                        "message": warning.message,
                        "confidence_penalty": warning.confidence_penalty,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
        payload.insert(
            "accepted_entries".into(),
            json!({
                "crystals": staged.iter().map(|output| output.crystals.len()).sum::<usize>(),
                "concepts": 0,
                "facets": 0,
                "concept_proposals": 0,
                "supersede_actions": 0,
            }),
        );
        payload.insert("rejected_entries".into(), json!(summary.rejected_entries));
        payload.insert(
            "confidence_penalties".into(),
            json!(
                warnings
                    .iter()
                    .filter(|warning| warning.confidence_penalty > 0.0)
                    .map(|warning| json!({
                        "entry_path": warning.entry_path,
                        "code": warning.code,
                        "confidence_penalty": warning.confidence_penalty,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
        payload.insert(
            "created_crystals".into(),
            json!(summary.created_crystal_ids),
        );
        payload.insert("created_concepts".into(), json!([]));
        payload.insert("created_facets".into(), json!([]));
        payload.insert("created_links".into(), json!([]));
        payload.insert("superseded_crystals".into(), json!([]));
        payload.insert("reinforced_crystals".into(), json!([]));
        payload.insert("decayed_crystals".into(), json!([]));
        payload.insert(
            "archived_short_term_memory_ids".into(),
            json!(summary.archived_memory_ids),
        );
        payload.insert(
            "dreamed_session_ids".into(),
            json!(summary.dreamed_session_ids),
        );
        payload.insert(
            "searched_related_candidates".into(),
            summary.related_candidates.clone(),
        );
        payload.insert(
            "affected_memory_set".into(),
            summary.affected_memory_set.clone(),
        );
        payload.insert(
            "skipped_candidates".into(),
            json!(summary.skipped_candidates),
        );
        self.audit.append(
            run_id,
            phase_run_id,
            "phase_completed",
            "info",
            &format!("completed {phase_name} phase"),
            &Value::Object(payload),
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Caps, thresholds, and redaction
    // ------------------------------------------------------------------

    fn validate_pass_output(
        &self,
        pass_name: &str,
        output: &NormalizedOutput,
    ) -> Result<(), DreamError> {
        let workflow = &self.dream_config.workflows[pass_name];
        let mut limit = workflow.max_records_per_pass;
        if pass_name == "relations" {
            limit = limit.min(self.dream_config.max_relation_records_per_pass);
        }
        if output.crystals.len() as i64 > limit {
            return Err(DreamError::InvalidOutput(format!(
                "{pass_name} output exceeds max_records_per_pass"
            )));
        }
        Ok(())
    }

    fn threshold_state(
        &self,
        pending_count: i64,
        ignore_minimum: bool,
        trigger_type: &str,
    ) -> Value {
        json!({
            "pending_short_term_memories": pending_count,
            "min_pending_short_term_memories": self.dream_config.min_pending_short_term_memories,
            "max_pending_short_term_memories": self.dream_config.max_pending_short_term_memories,
            "max_short_term_memories_per_cycle": self.dream_config.max_short_term_memories_per_cycle,
            "minimum_met": pending_count >= self.dream_config.min_pending_short_term_memories,
            "urgent_threshold_met": pending_count >= self.dream_config.max_pending_short_term_memories,
            "ignore_minimum": ignore_minimum,
            "stale_cycle_override": trigger_type == "backlog_escape",
            "not_enough_memories_cycle_threshold": self
                .dream_config
                .not_enough_memories_cycle_threshold,
        })
    }

    fn redacted_error_message(&self, error: &DreamError) -> String {
        let message = error.to_string();
        match load_provider_catalog(&self.config) {
            Ok(catalog) => {
                let keys: Vec<&str> = catalog
                    .providers
                    .values()
                    .map(|profile| profile.key().expose_secret().as_str())
                    .collect();
                crate::secret::redact_values(&message, &keys)
            }
            Err(_) => message,
        }
    }
}

/// Resolve every enabled `dream.conf` workflow assignment against the
/// `provider.conf` catalog (ADR 0007 resolution: explicit assignment, then
/// catalog defaults, then fail closed) and require the injected provider to
/// be the one the wiring declares. Disabled workflows are skipped, never
/// errors. The error text names the workflow and the problem and carries no
/// secret material, so it is safe for run records and audit surfaces.
fn validate_workflow_wiring(
    dream_config: &DreamConfig,
    catalog: &ProviderCatalog,
    injected_deterministic: bool,
    injected_profile: &str,
    injected_model: &str,
) -> Result<(), DreamError> {
    for (name, workflow) in &dream_config.workflows {
        if !workflow.enabled {
            continue;
        }
        let provider_id = if workflow.provider.trim().is_empty() {
            catalog.defaults.provider.trim()
        } else {
            workflow.provider.trim()
        };
        let model = if workflow.model.trim().is_empty() {
            catalog.defaults.model.trim()
        } else {
            workflow.model.trim()
        };
        if provider_id.is_empty() {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name}: enabled workflow must have a provider"
            )));
        }
        if model.is_empty() {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name}: enabled workflow must have a model"
            )));
        }
        if provider_id == "deterministic" {
            if !injected_deterministic {
                return Err(DreamError::InvalidWorkflow(format!(
                    "workflow {name} is declared deterministic; only the deterministic \
                     provider may run it"
                )));
            }
            continue;
        }
        if injected_deterministic {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name} requires configured provider {provider_id}; the \
                 deterministic provider never substitutes for a configured LLM workflow"
            )));
        }
        let Some(profile) = catalog.providers.get(provider_id) else {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name}: provider profile missing: {provider_id}"
            )));
        };
        // Python `_provider_from_profile`: only ollama runs without a key.
        if profile.provider_type() != "ollama" && profile.key().is_blank() {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name}: API key missing for provider profile: {provider_id}"
            )));
        }
        // One provider runs the whole cycle, so the injected provider must be
        // exactly the wiring the enabled workflows resolve to.
        if injected_profile != provider_id || injected_model != model {
            return Err(DreamError::InvalidWorkflow(format!(
                "workflow {name} is assigned to provider {provider_id} model {model}; \
                 the injected provider serves {injected_profile} model {injected_model}"
            )));
        }
    }
    Ok(())
}

fn next_cycle_id(connection: &Connection) -> Result<i64, DreamError> {
    let cycle_id = connection.query_row(
        "select coalesce(max(cycle_id), 0) + 1 from dream_runs where cycle_id > 0",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(cycle_id)
}

fn next_skipped_cycle_id(connection: &Connection) -> Result<i64, DreamError> {
    let cycle_id = connection.query_row(
        "select coalesce(min(cycle_id), 0) - 1 from dream_runs where cycle_id < 0",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(cycle_id)
}

/// Python `_trigger_type_from_owner`: manual/admin map to "manual",
/// scheduler maps to "scheduled", anything else passes through.
fn trigger_type_from_owner(owner: &str) -> String {
    match owner {
        "manual" | "admin" => "manual".to_string(),
        "scheduler" => "scheduled".to_string(),
        other => other.to_string(),
    }
}

fn unique_ints(values: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::with_capacity(values.len());
    let mut unique = Vec::with_capacity(values.len());
    for value in values {
        if seen.insert(*value) {
            unique.push(*value);
        }
    }
    unique
}

fn clamp_score(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

// ----------------------------------------------------------------------
// Deterministic-provider helpers
// ----------------------------------------------------------------------

/// Python `_normalize_candidate_text`: keep at most the first three sentences
/// (unicode terminators included), whitespace-normalized.
pub fn normalize_candidate_text(text: &str) -> String {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if matches!(character, '.' | '!' | '?' | '。' | '！' | '？') {
            current.push(character);
            let chunk = current.trim().to_string();
            current.clear();
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
        } else {
            current.push(character);
        }
    }
    let rest = current.trim().to_string();
    if !rest.is_empty() {
        chunks.push(rest);
    }
    if chunks.is_empty() {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    chunks.into_iter().take(3).collect::<Vec<_>>().join(" ")
}

fn is_title_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, 'А'..='Я' | 'а'..='я' | 'Ё' | 'ё')
}

/// Python `_title_from_kind`: up to four word characters runs, first letter
/// capitalized, joined with spaces.
pub fn title_from_kind(kind: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in kind.replace('-', " ").chars() {
        if is_title_word_character(character) {
            current.push(character);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    if words.is_empty() {
        return String::new();
    }
    words
        .into_iter()
        .take(4)
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ----------------------------------------------------------------------
// Provider output parsing (dict schema)
// ----------------------------------------------------------------------

const CRYSTAL_BUCKETS: [(&str, &str, &str, bool); 4] = [
    ("crystals", "observation", "observation", false),
    ("rule_crystals", "rule", "user_rule", false),
    ("thoughts", "thought", "thought", true),
    ("inferred_additions", "thought", "thought", true),
];

/// Port of `_normalize_dict_output` (reduced to the sections this slice
/// applies). Unsupported-but-non-empty sections fail the run closed so no
/// provider data is ever dropped silently.
pub fn normalize_dict_output(
    payload: &Value,
    allowed_memory_ids: &HashSet<i64>,
    valid_concept_ids: &HashSet<i64>,
) -> Result<NormalizedOutput, DreamError> {
    for section in UNSUPPORTED_OUTPUT_SECTIONS {
        if !list_from_payload(payload.get(section)).is_empty() {
            return Err(DreamError::InvalidOutput(format!(
                "provider output section is not applied by the dreaming core yet: {section}"
            )));
        }
    }

    let mut output = NormalizedOutput::default();
    for (bucket, default_crystal_type, default_source_credibility, force_thought) in CRYSTAL_BUCKETS
    {
        for (index, item) in list_from_payload(payload.get(bucket))
            .into_iter()
            .enumerate()
        {
            if let Some(crystal) = normalize_dict_crystal(
                &item,
                &format!("{bucket}[{index}]"),
                &mut output.warnings,
                &mut output.skipped_candidates,
                default_crystal_type,
                default_source_credibility,
                force_thought,
                allowed_memory_ids,
                valid_concept_ids,
            )? {
                output.crystals.push(crystal);
            }
        }
    }
    Ok(output)
}

fn list_from_payload(value: Option<&Value>) -> Vec<Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "float"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Port of `_normalize_dict_crystal`. Returns `Ok(None)` when the candidate
/// is skipped (the skip reason is appended to `skipped_candidates`); hard
/// errors (missing content) fail the run.
#[allow(clippy::too_many_arguments)]
fn normalize_dict_crystal(
    item: &Value,
    entry_path: &str,
    warnings: &mut Vec<ParseWarning>,
    skipped_candidates: &mut Vec<Value>,
    default_crystal_type: &str,
    default_source_credibility: &str,
    force_thought: bool,
    allowed_memory_ids: &HashSet<i64>,
    valid_concept_ids: &HashSet<i64>,
) -> Result<Option<NormalizedCrystal>, DreamError> {
    let payload: serde_json::Map<String, Value> = match item {
        Value::Object(map) => map.clone(),
        Value::String(text) => serde_json::Map::from_iter([("text".to_string(), json!(text))]),
        other => {
            skipped_candidates.push(json!({
                "entry_path": entry_path,
                "reason": "malformed_candidate",
                "candidate_type": value_type_name(other),
            }));
            return Ok(None);
        }
    };

    let (text, mut penalty) = recover_crystal_text(&payload)?;
    let (mut crystal_type, kind_penalty) = recover_crystal_type(&payload, default_crystal_type);
    if force_thought {
        crystal_type = "thought".to_string();
    }
    if kind_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_type",
            "used fallback crystal type",
            kind_penalty,
        );
    }
    penalty += kind_penalty;

    let (mut source_credibility, credibility_penalty) =
        recover_source_credibility(&payload, default_source_credibility);
    if force_thought {
        source_credibility = "thought".to_string();
    }
    if credibility_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_source_credibility",
            "used fallback source credibility",
            credibility_penalty,
        );
    }
    penalty += credibility_penalty;

    penalty += numeric_field(payload.get("malformed_penalty"), 0.0).max(0.0);

    let is_inferred = force_thought || bool_field(payload.get("is_inferred"), false);
    if is_inferred && !force_thought {
        crystal_type = "thought".to_string();
        source_credibility = "thought".to_string();
    }

    let (story_scopes, story_scope_penalty) =
        recover_string_tuple(&payload, &["story_scopes", "story_scope"]);
    if story_scope_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_story_scopes",
            "ignored malformed crystal story scope metadata",
            story_scope_penalty,
        );
    }
    let (semantic_tags, semantic_tag_penalty) =
        recover_string_tuple(&payload, &["semantic_tags", "tags"]);
    if semantic_tag_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_semantic_tags",
            "ignored malformed crystal semantic tag metadata",
            semantic_tag_penalty,
        );
    }
    let (concept_ids, concept_id_penalty) = recover_int_tuple(
        &payload,
        &["concept_ids", "concept_id"],
        Some(valid_concept_ids),
    );
    if concept_id_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "invalid_crystal_concept_ids",
            "ignored unknown or inactive crystal concept ids",
            concept_id_penalty,
        );
    }
    let concept_name_penalty = unsupported_concept_name_penalty(&payload);
    if concept_name_penalty {
        append_parse_warning(
            warnings,
            entry_path,
            "unsupported_crystal_concept_metadata",
            "ignored crystal concept names (concept application is a later slice)",
            0.0,
        );
    }
    penalty += story_scope_penalty + semantic_tag_penalty + concept_id_penalty;

    let confidence = normalized_confidence(&payload, &source_credibility, penalty);
    let title = string_field(payload.get("title")).trim().to_string();
    let title = if title.is_empty() {
        title_from_kind(&crystal_type)
    } else {
        title
    };
    let source_memory_ids = match source_memory_ids(&payload, allowed_memory_ids) {
        Some(ids) => ids,
        None => {
            skipped_candidates.push(json!({
                "entry_path": entry_path,
                "reason": "invalid_source_memory_ids",
                "source_memory_ids": clean_int_tuple(&[
                    payload.get("source_memory_ids"),
                    payload.get("source_memory_id"),
                ]),
            }));
            return Ok(None);
        }
    };

    // Reserved for the supersede slice; carried so the schema stays stable.
    let supersedes_crystal_id = optional_int(payload.get("supersedes_crystal_id"));
    Ok(Some(NormalizedCrystal {
        crystal_type,
        title,
        text,
        strength: clamp_score(numeric_field(payload.get("strength"), 0.5)),
        confidence,
        source_memory_ids,
        source_credibility,
        rule_intent: string_field(payload.get("rule_intent")),
        malformed_penalty: penalty,
        is_inferred,
        supersedes_crystal_id,
        story_scopes,
        semantic_tags,
        concept_ids,
    }))
}

fn append_parse_warning(
    warnings: &mut Vec<ParseWarning>,
    entry_path: &str,
    code: &str,
    message: &str,
    confidence_penalty: f64,
) {
    warnings.push(ParseWarning {
        entry_path: entry_path.to_string(),
        code: code.to_string(),
        message: message.to_string(),
        confidence_penalty,
    });
}

/// True when a crystal entry carries concept-name metadata this slice cannot
/// resolve (well-formed or not, it must be surfaced, never dropped silently).
fn unsupported_concept_name_penalty(payload: &serde_json::Map<String, Value>) -> bool {
    UNSUPPORTED_CONCEPT_NAME_KEYS.iter().any(|key| {
        payload
            .get(*key)
            .is_some_and(|value| !list_from_payload(Some(value)).is_empty())
    })
}

/// Python `_recover_crystal_text`: `text`/`content` are clean; `body` is a
/// malformed fallback costing a confidence penalty.
fn recover_crystal_text(
    payload: &serde_json::Map<String, Value>,
) -> Result<(String, f64), DreamError> {
    for key in ["content", "text"] {
        if let Some(Value::String(value)) = payload.get(key)
            && !value.trim().is_empty()
        {
            return Ok((collapse_whitespace(value), 0.0));
        }
    }
    if let Some(Value::String(value)) = payload.get("body")
        && !value.trim().is_empty()
    {
        return Ok((collapse_whitespace(value), MALFORMED_CONFIDENCE_PENALTY));
    }
    Err(DreamError::InvalidOutput(
        "dream candidate content is required".to_string(),
    ))
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Port of `_recover_crystal_type`.
fn recover_crystal_type(
    payload: &serde_json::Map<String, Value>,
    default_crystal_type: &str,
) -> (String, f64) {
    let value = payload
        .get("crystal_type")
        .or_else(|| payload.get("type"))
        .or_else(|| payload.get("kind"));
    let Some(value) = value else {
        return (default_crystal_type.to_string(), 0.0);
    };
    let Some(text) = value.as_str() else {
        return ("observation".to_string(), MALFORMED_CONFIDENCE_PENALTY);
    };
    let candidate = text.trim().to_lowercase().replace('-', "_");
    if candidate == "rule_crystal" {
        return ("rule".to_string(), MALFORMED_CONFIDENCE_PENALTY);
    }
    if ALLOWED_CRYSTAL_TYPES.contains(&candidate.as_str()) {
        return (candidate, 0.0);
    }
    ("observation".to_string(), MALFORMED_CONFIDENCE_PENALTY)
}

/// Port of `_recover_source_credibility`: a missing key takes the default
/// cleanly; only a present-but-malformed value costs a confidence penalty.
fn recover_source_credibility(
    payload: &serde_json::Map<String, Value>,
    default_source_credibility: &str,
) -> (String, f64) {
    match payload.get("source_credibility") {
        None => (default_source_credibility.to_string(), 0.0),
        Some(Value::String(value)) if !value.trim().is_empty() => (value.trim().to_string(), 0.0),
        Some(_) => (
            default_source_credibility.to_string(),
            MALFORMED_CONFIDENCE_PENALTY,
        ),
    }
}

/// Port of `_normalized_confidence`.
fn normalized_confidence(
    payload: &serde_json::Map<String, Value>,
    source_credibility: &str,
    penalty: f64,
) -> f64 {
    let base = match source_credibility {
        "rumor" | "source_text" | "expert" | "user_suggestion" | "user_rule" | "thought"
        | "observation" => source_credibility_confidence(source_credibility),
        _ => numeric_field(payload.get("confidence"), 0.35),
    };
    (base - penalty).clamp(MIN_NORMALIZED_CONFIDENCE, 1.0)
}

/// Port of `_recover_facet_string_tuple` (string or list of strings).
fn recover_string_tuple(
    payload: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> (Vec<String>, f64) {
    let mut values: Vec<String> = Vec::new();
    let mut penalty = 0.0;
    for key in keys {
        let Some(value) = payload.get(*key) else {
            continue;
        };
        match value {
            Value::String(text) => {
                if text.trim().is_empty() {
                    penalty += MALFORMED_CONFIDENCE_PENALTY;
                } else {
                    values.push(text.clone());
                }
            }
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::String(text) if !text.trim().is_empty() => values.push(text.clone()),
                        _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
                    }
                }
            }
            _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
        }
    }
    (clean_text_tuple(values), penalty)
}

/// Port of `_recover_crystal_int_tuple` with optional valid-id filtering.
fn recover_int_tuple(
    payload: &serde_json::Map<String, Value>,
    keys: &[&str],
    valid_ids: Option<&HashSet<i64>>,
) -> (Vec<i64>, f64) {
    let mut integers: Vec<i64> = Vec::new();
    let mut penalty = 0.0;
    for key in keys {
        let Some(value) = payload.get(*key) else {
            continue;
        };
        match value {
            Value::Bool(_) => penalty += MALFORMED_CONFIDENCE_PENALTY,
            Value::Number(number) => match number.as_i64() {
                Some(parsed) => integers.push(parsed),
                None => penalty += MALFORMED_CONFIDENCE_PENALTY,
            },
            Value::Array(items) => {
                for item in items {
                    match item.as_i64() {
                        Some(parsed) if !item.is_boolean() => integers.push(parsed),
                        _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
                    }
                }
            }
            _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
        }
    }
    integers.sort_unstable();
    integers.dedup();
    let Some(valid_ids) = valid_ids else {
        return (integers, penalty);
    };
    let valid: Vec<i64> = integers
        .iter()
        .copied()
        .filter(|id| valid_ids.contains(id))
        .collect();
    let invalid_count = integers.len() - valid.len();
    if invalid_count > 0 {
        penalty += MALFORMED_CONFIDENCE_PENALTY * invalid_count as f64;
    }
    (valid, penalty)
}

fn clean_text_tuple(values: Vec<String>) -> Vec<String> {
    let mut unique = std::collections::BTreeSet::new();
    for value in values {
        let trimmed = value.trim().to_string();
        if !trimmed.is_empty() {
            unique.insert(trimmed);
        }
    }
    unique.into_iter().collect()
}

fn clean_int_tuple(values: &[Option<&Value>]) -> Vec<i64> {
    let mut integers: Vec<i64> = Vec::new();
    for value in values.iter().flatten() {
        match value {
            Value::Bool(_) => {}
            Value::Number(number) => {
                if let Some(parsed) = number.as_i64() {
                    integers.push(parsed);
                }
            }
            Value::Array(items) => {
                for item in items {
                    if !item.is_boolean()
                        && let Some(parsed) = item.as_i64()
                    {
                        integers.push(parsed);
                    }
                }
            }
            _ => {}
        }
    }
    integers.sort_unstable();
    integers.dedup();
    integers
}

/// Port of `_source_memory_ids`: explicit ids must intersect the allowed set
/// (otherwise the candidate is skipped); without a source field every allowed
/// id is a source.
fn source_memory_ids(
    payload: &serde_json::Map<String, Value>,
    allowed_memory_ids: &HashSet<i64>,
) -> Option<Vec<i64>> {
    let has_source_field =
        payload.contains_key("source_memory_ids") || payload.contains_key("source_memory_id");
    let clean_ids: Vec<i64> = clean_int_tuple(&[
        payload.get("source_memory_ids"),
        payload.get("source_memory_id"),
    ])
    .into_iter()
    .filter(|memory_id| allowed_memory_ids.contains(memory_id))
    .collect();
    if !clean_ids.is_empty() {
        return Some(clean_ids);
    }
    if has_source_field {
        return None;
    }
    let mut all: Vec<i64> = allowed_memory_ids.iter().copied().collect();
    all.sort_unstable();
    Some(all)
}

fn string_field(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn numeric_field(value: Option<&Value>, default: f64) -> f64 {
    match value {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(default),
        _ => default,
    }
}

fn bool_field(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(parsed)) => *parsed,
        _ => default,
    }
}

fn optional_int(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Port of `_validate_normalized_output` (reduced to crystals).
fn validate_normalized_output(
    output: &NormalizedOutput,
    _context: &TranslationContext,
    allowed_memory_ids: &HashSet<i64>,
) -> Result<(), DreamError> {
    for candidate in &output.crystals {
        if !ALLOWED_CRYSTAL_TYPES.contains(&candidate.crystal_type.as_str()) {
            return Err(DreamError::InvalidOutput(format!(
                "unknown crystal_type: {}",
                candidate.crystal_type
            )));
        }
        if candidate.text.trim().is_empty() {
            return Err(DreamError::InvalidOutput(
                "candidate text must not be empty".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&candidate.strength) {
            return Err(DreamError::InvalidOutput(
                "candidate strength must be between 0 and 1".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&candidate.confidence) {
            return Err(DreamError::InvalidOutput(
                "candidate confidence must be between 0 and 1".to_string(),
            ));
        }
        let unknown: Vec<i64> = candidate
            .source_memory_ids
            .iter()
            .copied()
            .filter(|memory_id| !allowed_memory_ids.contains(memory_id))
            .collect();
        if !unknown.is_empty() {
            return Err(DreamError::InvalidOutput(format!(
                "unknown source_memory_ids: {unknown:?}"
            )));
        }
    }
    Ok(())
}

/// Port of `_coverage_ids`.
fn coverage_ids(
    payload: &Value,
    allowed_memory_ids: &HashSet<i64>,
) -> Result<HashSet<i64>, DreamError> {
    let raw = payload
        .get("covered_memory_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            DreamError::InvalidOutput(
                "coverage_audit output requires covered_memory_ids".to_string(),
            )
        })?;
    let mut covered = HashSet::new();
    for item in raw {
        let memory_id = item.as_i64().ok_or_else(|| {
            DreamError::InvalidOutput(
                "coverage_audit output requires covered_memory_ids".to_string(),
            )
        })?;
        covered.insert(memory_id);
    }
    let unknown: Vec<i64> = covered.difference(allowed_memory_ids).copied().collect();
    if !unknown.is_empty() {
        return Err(DreamError::InvalidOutput(
            "coverage_audit references unknown source_memory_ids".to_string(),
        ));
    }
    Ok(covered)
}

/// Port of `_deduplicate_staged_outputs`: passes run over the same selection,
/// so identical crystals (type + case-insensitive title/text) apply once.
fn deduplicate_staged_outputs(staged: &[NormalizedOutput]) -> Vec<NormalizedOutput> {
    let mut seen_crystals: HashSet<(String, String, String)> = HashSet::new();
    let mut result = Vec::with_capacity(staged.len());
    for output in staged {
        let mut crystals = Vec::with_capacity(output.crystals.len());
        for crystal in &output.crystals {
            let key = (
                crystal.crystal_type.to_lowercase(),
                crystal.title.to_lowercase(),
                crystal.text.to_lowercase(),
            );
            if seen_crystals.insert(key) {
                crystals.push(crystal.clone());
            }
        }
        result.push(NormalizedOutput {
            crystals,
            warnings: output.warnings.clone(),
            skipped_candidates: output.skipped_candidates.clone(),
        });
    }
    result
}

/// Insert one dream crystal with FTS row, source links, typed side tables,
/// concept links, and the run's `created_cycle` attribution.
fn insert_dream_crystal(
    transaction: &rusqlite::Transaction<'_>,
    context: &TranslationContext,
    candidate: &NormalizedCrystal,
    cycle_id: i64,
) -> Result<i64, DreamError> {
    let timestamp = now();
    let legacy_tags: Vec<String> = if candidate.semantic_tags.is_empty() {
        context.tags.clone()
    } else {
        candidate.semantic_tags.clone()
    };
    let tags_json =
        serde_json::to_string(&legacy_tags).map_err(|error| DreamError::Json(error.to_string()))?;
    transaction.execute(
        "insert into crystals(
           crystal_type, text, title, scope_type, scope_key, series_slug,
           source_language, target_language, tags_json, strength, confidence,
           source_credibility, rule_intent, is_inferred, malformed_penalty,
           supersedes_crystal_id, status, created_cycle, created_at, updated_at
         )
         values (?1, ?2, ?3, 'series', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, 'active', ?16, ?17, ?18)",
        rusqlite::params![
            candidate.crystal_type,
            candidate.text,
            candidate.title,
            context.scope_key(),
            context.series_slug,
            context.source_language,
            context.target_language,
            tags_json,
            candidate.strength,
            candidate.confidence,
            candidate.source_credibility,
            candidate.rule_intent,
            candidate.is_inferred,
            candidate.malformed_penalty,
            candidate.supersedes_crystal_id,
            cycle_id,
            timestamp,
            timestamp,
        ],
    )?;
    let crystal_id = transaction.last_insert_rowid();
    transaction.execute(
        "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
        rusqlite::params![crystal_id, candidate.title, candidate.text],
    )?;
    for memory_id in &candidate.source_memory_ids {
        transaction.execute(
            "insert into crystal_sources(crystal_id, short_term_memory_id) values (?1, ?2)",
            rusqlite::params![crystal_id, memory_id],
        )?;
    }
    for story_scope in &candidate.story_scopes {
        transaction.execute(
            "insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
             values (?1, ?2, ?3, ?4)
             on conflict(crystal_id, scope) do update set
               confidence = max(crystal_story_scopes.confidence, excluded.confidence)",
            rusqlite::params![crystal_id, story_scope, candidate.confidence, timestamp],
        )?;
    }
    for semantic_tag in &candidate.semantic_tags {
        transaction.execute(
            "insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
             values (?1, ?2, ?3, ?4)
             on conflict(crystal_id, tag) do update set
               confidence = max(crystal_semantic_tags.confidence, excluded.confidence)",
            rusqlite::params![crystal_id, semantic_tag, candidate.confidence, timestamp],
        )?;
    }
    for concept_id in &candidate.concept_ids {
        transaction.execute(
            "insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
             values (?1, ?2, 'mentions', ?3, ?4)
             on conflict(crystal_id, concept_id, link_type) do update set
               confidence = max(crystal_concepts.confidence, excluded.confidence)",
            rusqlite::params![crystal_id, concept_id, candidate.confidence, timestamp],
        )?;
    }
    Ok(crystal_id)
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

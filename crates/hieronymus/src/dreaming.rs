//! Dreaming core (slice 5 of the Rust port, per
//! `docs/superpowers/specs/2026-08-31-rust-dreaming-design.md`): bounded,
//! auditable dream cycles with explicit mutation ownership.
//!
//! One cycle: OS lock acquisition (one nonblocking `try_lock`, held for the
//! whole cycle), a durable run row, a bounded selection of completed-session
//! short-term memories, the enabled evidence passes each behind its own
//! freshly resolved [`crate::dream_workflows::WorkflowResolver`] provider
//! (ADR 0007), one validated mutation batch applied transactionally, and
//! audit entries covering inputs, outputs, parse decisions, and mutations
//! with redacted payloads. `open` is the fail-closed workflow gate: every
//! enabled `dream.conf` workflow assignment must resolve against the
//! `provider.conf` catalog before any cycle runs (spec §Provider Policy);
//! a run whose required coverage audit is disabled is rejected before any
//! input is processed.
//!
//! Real configured LLM clients live in [`crate::dream_providers`] behind the
//! same [`DreamProvider`] seam. Provider output is normalized through
//! [`crate::dream_output`]: every section of the Python
//! `_NormalizedDreamOutput` contract (crystals with concept names, concept
//! proposals, concepts, facets, supersede actions, reinforce actions) is
//! parsed with per-entry rejection into durable audit channels, validated
//! against the selected context, and applied inside the persistence
//! transaction. Supersede and reinforce targets are authorized against the
//! selected context and active-rule protection (ADR 0011) before any store
//! call; rule-related model output can only ever produce candidates and
//! proposals. Passive feedback events, decay, and the scheduler's interval
//! clock belong to the daemon's dream controller (task D5): this module
//! owns the bounded single cycle, the drain over successive capped batches
//! ([`DreamService::run_all`], [`drain_batches`]), and the scheduling
//! threshold decision ([`scheduled_decision`], ADR 0005).
//! [`DeterministicDreamProvider`] remains an explicit test and diagnostic
//! injection via [`crate::dream_workflows::WorkflowResolver::deterministic`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::dream_audit::{DreamAuditStore, commit_audited};
use crate::dream_config::{DreamConfig, load_dream_config};
use crate::dream_link_progress::LinkProgress;
use crate::dream_locks::{DreamCycleState, DreamLockError, dream_cycle_lock};
use crate::dream_output::{
    ConceptProposal, NormalizedConcept, NormalizedFacet, ReinforceAction, SupersedeAction,
    validate_action_targets, validate_reinforce_targets, validate_supersede_targets,
};
use crate::dream_workflows::{WorkflowChoice, WorkflowResolver, enabled_choices};
use crate::feedback::apply_score_delta;
use crate::memory_models::{ShortTermMemoryRecord, TranslationContext};
use crate::provider_config::load_provider_catalog;

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

pub(crate) const MIN_NORMALIZED_CONFIDENCE: f64 = 0.05;

/// Token-set Jaccard similarity at or above which two co-activated `useful`
/// crystals are near-duplicates and combine pairwise (July design
/// §Dream-Time Integration, LinkReinforcer).
pub const COMBINATION_SIMILARITY_THRESHOLD: f64 = 0.7;

/// Longest provider free-text title echoed verbatim into a rejection
/// record; longer titles are cut to this prefix with an explicit marker
/// (see [`bounded_rejection_title`]).
const REJECTION_TITLE_MAX_CHARS: usize = 80;

/// In-place reinforcement the reconsolidator applies when a working copy
/// stays below the diff threshold: strength-only, one notch.
const RECONSOLIDATION_REINFORCE_DELTAS: (f64, f64) = (0.02, 0.0);

/// Python `SOURCE_CREDIBILITY_CONFIDENCE` with the `observation` fallback
/// (`0.35` = `observation`'s weight as the schema default). Shared by recall
/// scoring and dream-time survivor selection.
pub fn source_credibility_confidence(source_credibility: &str) -> f64 {
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
    #[error(transparent)]
    Concept(#[from] crate::concepts::ConceptError),
    #[error(transparent)]
    Crystal(#[from] crate::crystals::CrystalError),
    #[error("{0}")]
    Json(String),
    /// The drain stopped without a clean completion record: cancellation
    /// before any batch ran, or zero progress while eligible work remained
    /// (recorded as a durable failed run row before this error is raised).
    #[error("dream drain interrupted: {0}")]
    DrainInterrupted(String),
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

/// The durable outcome of one drain ([`DreamService::run_all`]): the final
/// batch's own run record plus the totals across every completed batch of
/// the same drain. Each batch is one capped selection with one durable run
/// row, so the aggregate is a summary over rows, never a replacement for
/// them.
#[derive(Debug, Clone, PartialEq)]
pub struct DrainRecord {
    /// The last batch's own durable run record.
    pub record: DreamRunRecord,
    /// How many capped batches the drain ran (>= 1).
    pub batches: usize,
    /// Totals across every completed batch of the drain.
    pub input_count: i64,
    pub created_crystal_count: i64,
    pub proposal_count: i64,
}

/// The bounded drain loop: call `next` — one bounded batch, already durably
/// recorded by the caller — until it completes nothing or `cancelled` is
/// observed. Never lifts a per-batch cap: re-selection is `next`'s job.
pub fn drain_batches(
    mut next: impl FnMut() -> Result<usize, DreamError>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<usize, DreamError> {
    let mut total = 0;
    while !cancelled.load(std::sync::atomic::Ordering::Acquire) {
        let completed = next()?;
        if completed == 0 {
            break;
        }
        total += completed;
    }
    Ok(total)
}

/// The scheduled-tick decision (ADR 0005 §Scheduling And Drain Behavior).
/// The urgent maximum backlog is a trigger in its own right: the host
/// evaluates it at every scheduler gate, independently of the interval, so a
/// backlog at `max_pending_short_term_memories` never waits for the next
/// scheduled firing. With the urgent trigger absent, a backlog at the
/// minimum runs on the elapsed interval, a smaller backlog is skipped
/// honestly (`not_enough_memories`) until
/// `not_enough_memories_cycle_threshold` consecutive skips arm the backlog
/// escape that processes the small leftover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledDecision {
    /// Nothing eligible: no run and no skip row.
    NothingPending,
    /// Below the minimum: record an honest `not_enough_memories` skip.
    NotEnoughMemories,
    /// The interval fired with the minimum met.
    Scheduled,
    /// The urgent maximum backlog: run now (evaluated at every scheduler
    /// gate, independent of the interval), ignoring the minimum.
    Urgent,
    /// Enough consecutive skips: process the small leftover batch.
    BacklogEscape,
}

/// Decide one scheduled tick from actual config state and the consecutive
/// `not_enough_memories` skip count (ADR 0005's default shape). The interval
/// clock is the caller's concern; the thresholds are the config's.
pub fn scheduled_decision(
    dream_config: &DreamConfig,
    pending: i64,
    consecutive_skips: i64,
) -> ScheduledDecision {
    if pending <= 0 {
        return ScheduledDecision::NothingPending;
    }
    if pending >= dream_config.max_pending_short_term_memories {
        return ScheduledDecision::Urgent;
    }
    if pending >= dream_config.min_pending_short_term_memories {
        return ScheduledDecision::Scheduled;
    }
    if consecutive_skips >= dream_config.not_enough_memories_cycle_threshold {
        return ScheduledDecision::BacklogEscape;
    }
    ScheduledDecision::NotEnoughMemories
}

/// Completed-session short-term memories that have not been archived and are
/// crystallization-eligible (the dreaming input count).
pub fn pending_short_term_memory_count(config: &HieronymusConfig) -> Result<i64, DreamError> {
    let connection = open_migrated(&config.database_path())?;
    let count = connection.query_row(
        "select count(*)
         from short_term_memories
         join task_sessions on task_sessions.id = short_term_memories.session_id
         where task_sessions.status = 'completed'
           and short_term_memories.archived_at is null
           and short_term_memories.source_crystal_id is null",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count)
}

/// Trailing consecutive scheduled `not_enough_memories` skips (ADR 0005's
/// backlog-escape counter), read from the durable skip rows. A
/// `dream cycle already running` skip preserves the count (Python's
/// cycle-active case); any other run row breaks the streak.
pub fn consecutive_not_enough_memories_skips(config: &HieronymusConfig) -> Result<i64, DreamError> {
    const SKIP_REASON: &str = "not_enough_memories";
    const LOCKED_REASON: &str = "dream cycle already running";
    let connection = open_migrated(&config.database_path())?;
    let mut statement = connection
        .prepare("select status, coalesce(error, '') from dream_runs order by id desc limit 200")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut count = 0;
    for row in rows {
        let (status, error) = row?;
        if status == "skipped" && error.starts_with(LOCKED_REASON) {
            continue;
        }
        if status == "skipped" && error.starts_with(SKIP_REASON) {
            count += 1;
            continue;
        }
        break;
    }
    Ok(count)
}

/// The provider label a production run would record (`dream_runs.provider`):
/// the first enabled workflow's wire provider type. Empty when no workflow
/// is enabled — such a run fails the coverage gate before any pass.
pub fn resolved_provider_label(config: &HieronymusConfig) -> Result<String, DreamError> {
    let dream_config = load_dream_config(config)?;
    let resolver =
        WorkflowResolver::from_catalog(load_provider_catalog(config).unwrap_or_default());
    let choices = resolver.translate_choices(&dream_config)?;
    Ok(resolver.run_provider_name(&choices))
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

    /// True only for the deterministic provider. The
    /// [`crate::dream_workflows::WorkflowResolver`] test seam uses this to
    /// accept a config-enabled workflow assignment that names the
    /// `deterministic` profile id — while an assignment naming a configured
    /// provider still refuses deterministic substitution (spec §Provider
    /// Policy: it never silently replaces a configured workflow).
    fn is_deterministic(&self) -> bool {
        false
    }

    /// The provider endpoint recorded (redacted) on audit entries (spec
    /// §Audit: redacted endpoint). Empty for providers without an HTTP
    /// endpoint; the dreaming core strips query strings and credentials
    /// before storing it.
    fn endpoint(&self) -> &str {
        ""
    }

    /// The rendered prompt for one pass. The dreaming core stores its
    /// SHA-256 on the pass's request and response audit entries (spec
    /// §Audit: prompt hash); an LLM provider must return exactly the text
    /// it sends on the wire.
    fn render_pass_prompt(
        &self,
        pass_name: &str,
        context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<String, DreamError> {
        Ok(canonical_pass_projection(pass_name, context, memories))
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

/// The provider identity one pass writes into phase rows and audit payloads:
/// the actual resolved profile id, wire provider name, model, and endpoint
/// (redacted before storage) — never one injected provider's identity for
/// every phase.
#[derive(Debug, Clone)]
pub(crate) struct ProviderIdentity {
    pub(crate) profile: String,
    pub(crate) name: String,
    pub(crate) model: String,
    pub(crate) endpoint: String,
}

/// The identity of the algorithmic, provider-free phases.
fn deterministic_identity() -> ProviderIdentity {
    ProviderIdentity {
        profile: "deterministic".to_string(),
        name: "deterministic".to_string(),
        model: "deterministic".to_string(),
        endpoint: String::new(),
    }
}

/// The default prompt projection for providers without a custom prompt
/// rendering (the deterministic provider, diagnostics): a stable JSON
/// object binding the pass name and the exact evidence the pass consumes,
/// so the audited prompt hash stays content-addressed without an LLM.
fn canonical_pass_projection(
    pass_name: &str,
    context: &TranslationContext,
    memories: &[ShortTermMemoryRecord],
) -> String {
    json!({
        "pass": pass_name,
        "context": {
            "series_slug": context.series_slug,
            "source_language": context.source_language,
            "target_language": context.target_language,
            "task_type": context.task_type,
            "volume": context.volume,
            "chapter": context.chapter,
        },
        "memories": memories.iter().map(|memory| json!({
            "id": memory.id,
            "text": memory.text,
            "source_credibility": memory.source_credibility,
            "rule_intent": memory.rule_intent,
        })).collect::<Vec<_>>(),
    })
    .to_string()
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
    /// Concept names the provider attached to this crystal; resolved to
    /// ids within the applying group's own context (never through another
    /// pass's resolution map).
    pub concept_names: Vec<String>,
}

/// One pass's normalized output (port of `_NormalizedDreamOutput`).
#[derive(Debug, Clone, Default)]
pub struct NormalizedOutput {
    pub crystals: Vec<NormalizedCrystal>,
    pub concept_proposals: Vec<ConceptProposal>,
    pub concepts: Vec<NormalizedConcept>,
    pub facets: Vec<NormalizedFacet>,
    pub supersede_actions: Vec<SupersedeAction>,
    pub reinforce_actions: Vec<ReinforceAction>,
    pub warnings: Vec<ParseWarning>,
    /// Durable per-entry rejections (ruling: malformed entries are never
    /// silently dropped, and one bad entry never drops its whole section).
    pub rejected_entries: Vec<Value>,
    pub skipped_candidates: Vec<Value>,
}

/// The `_normalized_output_count` port: every applied section counts against
/// the per-pass and per-run record budgets.
fn normalized_output_count(output: &NormalizedOutput) -> usize {
    output.crystals.len()
        + output.concept_proposals.len()
        + output.concepts.len()
        + output.facets.len()
        + output.supersede_actions.len()
        + output.reinforce_actions.len()
}

struct SelectionGroup {
    session_id: i64,
    context: TranslationContext,
    memories: Vec<ShortTermMemoryRecord>,
}

/// Aggregated output of the deterministic phases: ids for the audit payload
/// and the run record's crystal count.
#[derive(Debug, Default)]
struct DeterministicSummary {
    created_crystal_ids: Vec<i64>,
    changed_crystal_ids: Vec<i64>,
    archived_memory_ids: Vec<i64>,
    dreamed_session_ids: Vec<i64>,
    actions: Vec<Value>,
}

impl DeterministicSummary {
    fn merge(&mut self, other: DeterministicSummary) {
        self.created_crystal_ids.extend(other.created_crystal_ids);
        self.changed_crystal_ids.extend(other.changed_crystal_ids);
        self.archived_memory_ids.extend(other.archived_memory_ids);
        self.dreamed_session_ids.extend(other.dreamed_session_ids);
        self.actions.extend(other.actions);
    }
}

struct ApplySummary {
    created_crystal_ids: Vec<i64>,
    created_concept_ids: Vec<i64>,
    created_facet_ids: Vec<i64>,
    created_links: Vec<Value>,
    superseded_crystal_ids: Vec<i64>,
    reinforced_crystal_ids: Vec<i64>,
    archived_memory_ids: Vec<i64>,
    dreamed_session_ids: Vec<i64>,
    rejected_entries: Vec<Value>,
    skipped_candidates: Vec<Value>,
    related_candidates: Value,
    affected_memory_set: Value,
}

/// The dreaming service: typed phase orchestration over the data root with a
/// [`WorkflowResolver`] serving one fresh provider per selected workflow.
/// One instance may run any number of cycles; each cycle takes the OS
/// dream-cycle lock.
pub struct DreamService {
    config: HieronymusConfig,
    dream_config: DreamConfig,
    resolver: WorkflowResolver,
    /// The configured ordered workflow assignments as resolved at `open`
    /// time: the fail-closed gate has already run over the enabled ones.
    choices: Vec<WorkflowChoice>,
    audit: DreamAuditStore,
    /// Optional progress seam for long-lived hosts (task D5): called on the
    /// run's own thread with `(dream_run_id, cycle_id, phase)` each time a
    /// phase row starts running, so the daemon can stream phase progress
    /// without polling the registry.
    phase_observer: Option<PhaseObserver>,
}

/// A phase-progress observer: `(dream_run_id, cycle_id, phase_name)`.
pub type PhaseObserver = Arc<dyn Fn(i64, i64, &str) + Send + Sync>;

/// `Debug` names the resolver lane instead of dumping it, so `unwrap_err` in
/// tests and any diagnostic path stay redacted by construction.
impl std::fmt::Debug for DreamService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DreamService")
            .field("resolver", &self.resolver)
            .finish_non_exhaustive()
    }
}

impl DreamService {
    pub fn open(config: &HieronymusConfig, resolver: WorkflowResolver) -> Result<Self, DreamError> {
        let dream_config = load_dream_config(config)?;
        // The fail-closed workflow gate (ADR 0007): every enabled workflow
        // assignment must resolve against the provider catalog snapshot
        // before the service accepts any run. There is no run context here,
        // so the precise redacted error text is the audit record.
        let choices = resolver.translate_choices(&dream_config)?;
        let audit = DreamAuditStore::open(config)?;
        Ok(Self {
            config: config.clone(),
            dream_config,
            resolver,
            choices,
            audit,
            phase_observer: None,
        })
    }

    /// Install the phase-progress observer (see [`PhaseObserver`]). Call
    /// before the first run; the observer fires on the run's own thread.
    pub fn set_phase_observer(&mut self, observer: PhaseObserver) {
        self.phase_observer = Some(observer);
    }

    /// One dream cycle over one bounded selection (`run_cycle`).
    pub fn run_cycle(
        &self,
        owner: &str,
        skip_when_locked: bool,
    ) -> Result<DreamRunRecord, DreamError> {
        self.run_locked(owner, true, skip_when_locked)
    }

    /// Drain every pending completed-session memory (`run_all`): successive
    /// capped batches, each one bounded selection with its own durable run
    /// row, until a batch completes nothing (the backlog is drained, the
    /// minimum is not met, or another cycle holds the lock). Per-cycle caps
    /// are never lifted: the drain re-selects after each bounded batch, so
    /// newly eligible work is considered batch by batch.
    pub fn run_all(
        &self,
        owner: &str,
        ignore_minimum: bool,
        skip_when_locked: bool,
    ) -> Result<DrainRecord, DreamError> {
        self.run_draining_with_lock_mode(
            owner,
            ignore_minimum,
            skip_when_locked,
            &std::sync::atomic::AtomicBool::new(false),
        )
    }

    /// The controller's drain entry: like [`Self::run_all`], checking
    /// `cancelled` between batches so a shutdown stops the drain at the next
    /// batch boundary with the durable honest outcome of the batches that
    /// did run.
    pub fn run_draining(
        &self,
        owner: &str,
        ignore_minimum: bool,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<DrainRecord, DreamError> {
        self.run_draining_with_lock_mode(owner, ignore_minimum, false, cancelled)
    }

    fn run_draining_with_lock_mode(
        &self,
        owner: &str,
        ignore_minimum: bool,
        skip_when_locked: bool,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<DrainRecord, DreamError> {
        let mut batches = 0_usize;
        let mut input_total = 0_i64;
        let mut created_total = 0_i64;
        let mut proposal_total = 0_i64;
        let mut last: Option<DreamRunRecord> = None;
        drain_batches(
            || {
                // Re-select between batches: stop without an empty trailing
                // cycle once no eligible work remains. Crystallization
                // eligibility is the backlog (or, for threshold-respecting
                // drains, the minimum over it); the deterministic phases
                // keep justifying a cycle on their own material —
                // reconsolidation working copies, unconsumed feedback,
                // queued link pairs. The first batch is unconditional: a
                // drain with nothing to do still records one honest empty
                // cycle, exactly like a single cycle always has.
                if batches > 0 && !cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    let remaining = self.pending_short_term_memory_count()?;
                    let eligible = if remaining > 0 {
                        ignore_minimum
                            || remaining >= self.dream_config.min_pending_short_term_memories
                    } else {
                        self.cycle_has_deterministic_work()?
                    };
                    if !eligible {
                        return Ok(0_usize);
                    }
                }
                let record = self.run_locked(owner, ignore_minimum, skip_when_locked)?;
                batches += 1;
                // Progress counts completed/archived inputs — the memories a
                // batch actually archived, never merely selected rows (a
                // failed batch returns Err above and archives nothing). A
                // skipped row (another cycle holds the OS lock) is already
                // durable; its zero progress stops the drain without a spin.
                let progress = match record.status.as_str() {
                    "completed" => record.input_count.max(0) as usize,
                    "skipped" => 0_usize,
                    other => {
                        return Err(DreamError::DrainInterrupted(format!(
                            "batch recorded an unexpected status {other}"
                        )));
                    }
                };
                input_total += record.input_count;
                created_total += record.created_crystal_count;
                proposal_total += record.proposal_count;
                last = Some(record);
                Ok(progress)
            },
            cancelled,
        )?;
        let Some(record) = last else {
            return Err(DreamError::DrainInterrupted(
                "cancelled before the first batch".to_string(),
            ));
        };
        // The blocked guard: the loop stopped while eligible work remained
        // without a batch failure to name and without an honest skip row.
        // Nothing may spin here, and a backlog behind a completed run must
        // never read as success: record a durable failed run row naming the
        // stall, then fail the drain.
        if !cancelled.load(std::sync::atomic::Ordering::Acquire) && record.status != "skipped" {
            let remaining = self.pending_short_term_memory_count()?;
            let eligible = if ignore_minimum {
                remaining > 0
            } else {
                remaining >= self.dream_config.min_pending_short_term_memories
            };
            if eligible {
                let reason = format!(
                    "drain stopped with {remaining} eligible input(s) remaining \
                     after {batches} batch(es) of zero progress"
                );
                self.record_failed_run(&reason)?;
                return Err(DreamError::DrainInterrupted(reason));
            }
        }
        Ok(DrainRecord {
            record,
            batches,
            input_count: input_total,
            created_crystal_count: created_total,
            proposal_count: proposal_total,
        })
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
            rusqlite::params![cycle_id, self.run_provider_label(), now()],
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
        // The coverage audit is required before anything runs: a run whose
        // required coverage_audit workflow is disabled is rejected here,
        // before any input is processed. (The deterministic test seam claims
        // every workflow, so it never trips this gate.)
        let selected =
            enabled_choices(self.choices.clone()).map_err(DreamError::InvalidWorkflow)?;
        // `enabled_choices` guarantees at least the required coverage_audit
        // pass; the first enabled workflow is the run's primary lane and
        // lends its identity to the run-level records.
        let Some(primary_choice) = selected.first() else {
            return Err(DreamError::InvalidWorkflow(
                "coverage_audit must be enabled before processing memories".to_string(),
            ));
        };
        let primary_provider = self.resolver.identity(primary_choice)?;

        let pending_count = self.pending_short_term_memory_count()?;
        let threshold_state = self.threshold_state(pending_count, ignore_minimum, trigger_type);
        if !ignore_minimum && pending_count < self.dream_config.min_pending_short_term_memories {
            // Deterministic phases run on every cycle with pending work, even
            // below the provider crystallization threshold.
            let deterministic =
                self.run_deterministic_phases(run_id, cycle_id, trigger_type, &threshold_state, 0)?;
            return self.complete_run(
                run_id,
                cycle_id,
                0,
                deterministic.created_crystal_ids.len() as i64,
                0,
            );
        }

        // Selection: the bounded affected-memory set. One snapshot feeds
        // every pass, capped by max_short_term_memories_per_run.
        let groups = self.select_pending_completed_groups(
            self.dream_config.max_short_term_memories_per_run as usize,
        )?;
        if groups.is_empty() {
            let deterministic =
                self.run_deterministic_phases(run_id, cycle_id, trigger_type, &threshold_state, 0)?;
            return self.complete_run(
                run_id,
                cycle_id,
                0,
                deterministic.created_crystal_ids.len() as i64,
                0,
            );
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
        // Same-context authorization sets (ruling: derived from the selected
        // affected-memory context BEFORE any store call): the crystals scoped
        // to the selection's series contexts, and the active rules no dream
        // action may touch (ADR 0011 — dream has no approval authority).
        let allowed_crystal_ids = self.context_crystal_ids(&groups)?;
        let active_rule_ids = self.active_rule_crystal_ids()?;

        let mut covered_memory_ids: HashSet<i64> = HashSet::new();
        let mut staged: Vec<NormalizedOutput> = Vec::new();

        for choice in &selected {
            // A fresh provider per selected workflow, resolved inside the run
            // and dropped when the pass ends (worker-local: instances are
            // never stored on the service across threads). Disabled
            // assignments never reach this line, so they never require a
            // provider.
            let provider = self.resolver.provider(choice)?;
            let identity = self.resolver.identity(choice)?;
            let phase_run_id = self.start_phase_run(
                run_id,
                cycle_id,
                &choice.name,
                selected_memory_ids.len() as i64,
                &identity,
            )?;
            phase_run_ids.push(phase_run_id);
            // The prompt is rendered before the request audit so the stored
            // hash binds the exact text the pass runs against (spec §Audit).
            let prompt = provider.render_pass_prompt(
                &choice.name,
                &selection_context,
                &selected_memories,
            )?;
            let prompt_hash = prompt_sha256(&prompt);
            self.audit_provider_request(
                run_id,
                Some(phase_run_id),
                trigger_type,
                &threshold_state,
                &selected_memory_ids,
                &choice.name,
                &groups,
                &prompt_hash,
                &identity,
            )?;

            let raw = provider.run_pass(&choice.name, &selection_context, &selected_memories)?;
            if !raw.is_object() {
                return Err(DreamError::InvalidOutput(format!(
                    "{} output must be an object",
                    choice.name
                )));
            }

            if choice.name == "coverage_audit" {
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
                    &choice.name,
                    &self.response_summary(&[]),
                    &prompt_hash,
                    &identity,
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
                    &choice.name,
                    &output.warnings,
                    &identity,
                )?;
            }
            validate_normalized_output(&output, &selection_context, &allowed_memory_ids)?;
            self.validate_pass_output(&choice.name, &output)?;
            // No normalized action may touch an id outside the selected
            // context or an active rule: fail closed before anything is
            // staged (the raw guard sees the contract key; the typed guards
            // also cover the Python wire keys).
            validate_action_targets(&raw, &allowed_crystal_ids, &active_rule_ids)
                .map_err(DreamError::InvalidOutput)?;
            validate_supersede_targets(
                &output.supersede_actions,
                &allowed_crystal_ids,
                &active_rule_ids,
            )
            .map_err(DreamError::InvalidOutput)?;
            validate_reinforce_targets(&output.reinforce_actions, &allowed_crystal_ids)
                .map_err(DreamError::InvalidOutput)?;
            let output_count = normalized_output_count(&output) as i64;
            let response_summary = self.response_summary(std::slice::from_ref(&output));
            staged.push(output);
            self.complete_phase_run(phase_run_id, output_count)?;
            self.audit_provider_response(
                run_id,
                Some(phase_run_id),
                trigger_type,
                &threshold_state,
                &selected_memory_ids,
                &choice.name,
                &response_summary,
                &prompt_hash,
                &identity,
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

        let staged_record_count: usize = staged.iter().map(normalized_output_count).sum();
        if staged_record_count as i64 > self.dream_config.max_long_term_records_affected_per_run {
            return Err(DreamError::InvalidOutput(
                "dream run exceeds max_long_term_records_affected_per_run".to_string(),
            ));
        }

        // Persistence: one validated mutation batch in one transaction. The
        // persistence phase is not a provider pass; it records the run's
        // primary lane. Parsing and validation ran outside transactions; the
        // batch revalidates current state inside the transaction (supersede
        // and reinforce targets) before applying accepted output, and the
        // phase-completed status plus the redacted audit commit in the same
        // immediate transaction — the domain mutations are durable exactly
        // when their completion and audit are (task D3).
        let persistence_phase_run_id = self.start_phase_run(
            run_id,
            cycle_id,
            "persistence",
            selected_memory_ids.len() as i64,
            &primary_provider,
        )?;
        phase_run_ids.push(persistence_phase_run_id);
        let mut connection = open_migrated(&self.config.database_path())?;
        let committed = commit_audited(&mut connection, |transaction| {
            let summary = self
                .apply_outputs_in_transaction(
                    transaction,
                    run_id,
                    cycle_id,
                    &groups,
                    &staged,
                    &allowed_crystal_ids,
                    &active_rule_ids,
                )
                .map_err(tx_error)?;
            complete_phase_run_in_transaction(
                transaction,
                persistence_phase_run_id,
                summary.created_crystal_ids.len() as i64,
            )
            .map_err(tx_error)?;
            self.audit_phase_completed_in_transaction(
                transaction,
                run_id,
                Some(persistence_phase_run_id),
                trigger_type,
                &threshold_state,
                &selected_memory_ids,
                &groups,
                &staged,
                &summary,
                &primary_provider,
            )
            .map_err(tx_error)?;
            Ok(summary)
        });
        let summary = match committed {
            Ok(summary) => summary,
            Err(error) => {
                return Err(self.phase_commit_failure(
                    run_id,
                    Some(persistence_phase_run_id),
                    "persistence",
                    trigger_type,
                    &primary_provider,
                    error,
                ));
            }
        };

        // Deterministic phases (spec §Phase Boundaries steps 5-7): they run
        // after the provider batch so a failed run never leaves domain
        // mutations, and their own budget starts from what the provider
        // batch already spent.
        let deterministic = self.run_deterministic_phases(
            run_id,
            cycle_id,
            trigger_type,
            &threshold_state,
            summary.created_crystal_ids.len(),
        )?;

        let proposal_count = staged
            .iter()
            .map(|output| output.concept_proposals.len())
            .sum::<usize>() as i64;
        self.complete_run(
            run_id,
            cycle_id,
            selected_memory_ids.len() as i64,
            (summary.created_crystal_ids.len() + deterministic.created_crystal_ids.len()) as i64,
            proposal_count,
        )
    }

    // ------------------------------------------------------------------
    // Selection
    // ------------------------------------------------------------------

    fn pending_short_term_memory_count(&self) -> Result<i64, DreamError> {
        pending_short_term_memory_count(&self.config)
    }

    /// Whether the deterministic phases have material of their own: a
    /// session-scoped working copy, an unconsumed `recalled_again` event, or
    /// queued link work. These justify a cycle even with no crystallization
    /// inputs.
    fn cycle_has_deterministic_work(&self) -> Result<bool, DreamError> {
        Ok(self.reconsolidation_pending()?
            || self.reinforcement_pending()?
            || self.links_pending()?)
    }

    /// Port of `_load_pending_completed_groups`: at most `limit` pending
    /// memories from completed sessions, grouped by session, ordered by
    /// session then memory id. Session-scoped working copies
    /// (`source_crystal_id` set) are the reconsolidator's input, never
    /// crystallization input.
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
               and short_term_memories.source_crystal_id is null
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

    /// The crystals inside the run's selected context: series-scoped rows
    /// matching any selected group's series/language scope. The same-context
    /// `allowed` set for dream mutation targets, derived before store calls.
    fn context_crystal_ids(&self, groups: &[SelectionGroup]) -> Result<BTreeSet<i64>, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let mut statement = connection.prepare(
            "select id from crystals
             where scope_type = 'series' and scope_key = ?1
               and series_slug = ?2 and source_language = ?3 and target_language = ?4",
        )?;
        let mut ids = BTreeSet::new();
        for group in groups {
            let rows = statement.query_map(
                rusqlite::params![
                    group.context.scope_key(),
                    group.context.series_slug,
                    group.context.source_language,
                    group.context.target_language,
                ],
                |row| row.get::<_, i64>(0),
            )?;
            for row in rows {
                ids.insert(row?);
            }
        }
        Ok(ids)
    }

    /// The active-rule protection set (ADR 0011): active rule crystals plus
    /// the advisory projections of active `term_rules` authority. No dream
    /// action may touch these ids.
    fn active_rule_crystal_ids(&self) -> Result<BTreeSet<i64>, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let mut ids = BTreeSet::new();
        {
            let mut statement = connection.prepare(
                "select id from crystals where crystal_type = 'rule' and status = 'active'",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
            for row in rows {
                ids.insert(row?);
            }
        }
        {
            let mut statement = connection.prepare(
                "select rule_crystal_id from term_rules
                 where status = 'active' and rule_crystal_id is not null",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
            for row in rows {
                ids.insert(row?);
            }
        }
        Ok(ids)
    }

    // ------------------------------------------------------------------
    // Persistence
    // ------------------------------------------------------------------

    /// Apply every staged output as one validated mutation batch inside the
    /// caller's transaction: concepts, facets, crystals (with concept-name
    /// resolution inside each output's own map), concept proposals, reinforce
    /// actions, and supersede actions — then memory archiving and session
    /// marking. State is revalidated inside the transaction before applying
    /// (supersede targets must still exist, be active/candidate, and match
    /// shape; reinforce targets must still exist), and any failure rolls the
    /// whole batch back together with the phase's completion and audit.
    #[allow(clippy::too_many_arguments)]
    fn apply_outputs_in_transaction(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        run_id: i64,
        cycle_id: i64,
        groups: &[SelectionGroup],
        staged: &[NormalizedOutput],
        allowed_crystal_ids: &BTreeSet<i64>,
        active_rule_ids: &BTreeSet<i64>,
    ) -> Result<ApplySummary, DreamError> {
        let outputs = deduplicate_staged_outputs(staged);
        let mut created_crystal_ids: Vec<i64> = Vec::new();
        let mut created_concept_ids: Vec<i64> = Vec::new();
        let mut created_facet_ids: Vec<i64> = Vec::new();
        let mut created_links: Vec<Value> = Vec::new();
        let mut superseded_crystal_ids: Vec<i64> = Vec::new();
        let mut reinforced_crystal_ids: Vec<i64> = Vec::new();
        let mut rejected_entries: Vec<Value> = Vec::new();
        let mut skipped_candidates: Vec<Value> = Vec::new();

        // Memory id -> owning group: each crystal is attributed to the
        // context of the memories it cites (context isolation: never the
        // first group's context for another series).
        let mut group_of_memory: HashMap<i64, usize> = HashMap::new();
        for (index, group) in groups.iter().enumerate() {
            for memory in &group.memories {
                group_of_memory.insert(memory.id, index);
            }
        }

        let timestamp = now();
        for output in &outputs {
            skipped_candidates.extend(output.skipped_candidates.iter().cloned());
            rejected_entries.extend(output.rejected_entries.iter().cloned());
            // Concept-name resolution lives and dies with THIS output's
            // application: names resolve within the group's own context and
            // never through another pass's map.
            let mut concept_ids_by_name: BTreeMap<String, i64> = BTreeMap::new();
            for concept in &output.concepts {
                let concept_id = crate::concepts::create_or_reinforce_concept_in_transaction(
                    transaction,
                    &concept.canonical_name,
                    &concept.description,
                    &concept.tags,
                    concept.confidence_delta,
                    "global",
                    "",
                    &timestamp,
                )?;
                concept_ids_by_name.insert(concept.canonical_name.to_lowercase(), concept_id);
                created_concept_ids.push(concept_id);
            }
            for facet in &output.facets {
                let key = facet.concept_name.to_lowercase();
                let concept_id = match concept_ids_by_name.get(&key) {
                    Some(concept_id) => *concept_id,
                    None => {
                        let concept_id =
                            crate::concepts::create_or_reinforce_concept_in_transaction(
                                transaction,
                                &facet.concept_name,
                                "",
                                &[],
                                0.2,
                                "global",
                                "",
                                &timestamp,
                            )?;
                        concept_ids_by_name.insert(key, concept_id);
                        concept_id
                    }
                };
                let fields = crate::concepts::FacetFields {
                    kind: Some(facet.kind.clone()),
                    language_tags: facet.language_tags.clone(),
                    story_scopes: facet.story_scopes.clone(),
                    semantic_tags: facet.semantic_tags.clone(),
                    ..Default::default()
                };
                let facet_id = crate::concepts::add_facet_with_connection(
                    transaction,
                    concept_id,
                    facet.value.trim(),
                    &fields,
                    facet.confidence,
                    facet.is_canonical,
                    &timestamp,
                )?;
                created_facet_ids.push(facet_id);
            }
            for candidate in &output.crystals {
                let Some(group_index) = owning_crystal_group(groups, &group_of_memory, candidate)
                else {
                    // A crystal citing memories of several series (or none
                    // of the selection's contexts) has no honest context;
                    // it is rejected durably instead of landing in the
                    // first group's context.
                    rejected_entries.push(json!({
                        "stage": "apply",
                        "reason": "ambiguous_crystal_context",
                        "title": bounded_rejection_title(&candidate.title),
                        "source_memory_ids": candidate.source_memory_ids,
                    }));
                    continue;
                };
                let candidate = resolve_candidate_concepts(
                    transaction,
                    candidate,
                    &mut concept_ids_by_name,
                    &timestamp,
                )?;
                let crystal_id = insert_dream_crystal(
                    transaction,
                    &groups[group_index].context,
                    &candidate,
                    cycle_id,
                )?;
                created_crystal_ids.push(crystal_id);
                created_links.extend(candidate.concept_ids.iter().map(|concept_id| {
                    json!({
                        "crystal_id": crystal_id,
                        "concept_id": concept_id,
                        "link_type": "mentions",
                    })
                }));
            }
            for proposal in &output.concept_proposals {
                crate::concepts::create_concept_proposal_in_transaction(
                    transaction,
                    run_id,
                    proposal,
                    &timestamp,
                )?;
            }
        }

        // Reinforce actions: at most one per crystal per run, applied
        // through the event-sourced scoring primitive with the actual
        // (clamped) deltas recorded on the `dream_reinforce` event.
        for output in &outputs {
            validate_reinforce_targets(&output.reinforce_actions, allowed_crystal_ids)
                .map_err(DreamError::InvalidOutput)?;
        }
        let mut reinforced: HashSet<i64> = HashSet::new();
        for output in &outputs {
            for action in &output.reinforce_actions {
                if !reinforced.insert(action.crystal_id) {
                    continue;
                }
                let before: Option<(f64, f64)> = transaction
                    .query_row(
                        "select strength, confidence from crystals where id = ?1",
                        [action.crystal_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?;
                let Some((before_strength, before_confidence)) = before else {
                    continue;
                };
                apply_score_delta(
                    transaction,
                    action.crystal_id,
                    action.strength_delta,
                    action.confidence_delta,
                    &timestamp,
                )?;
                let (after_strength, after_confidence): (f64, f64) = transaction.query_row(
                    "select strength, confidence from crystals where id = ?1",
                    [action.crystal_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                transaction.execute(
                    "insert into memory_events(
                       crystal_id, session_id, event_type, source_role, evidence,
                       strength_delta, confidence_delta, applied, cycle_id, created_at
                     )
                     values (?1, null, 'dream_reinforce', 'system', ?2, ?3, ?4, 1, ?5, ?6)",
                    rusqlite::params![
                        action.crystal_id,
                        "dream reinforcement",
                        after_strength - before_strength,
                        after_confidence - before_confidence,
                        cycle_id,
                        timestamp,
                    ],
                )?;
                reinforced_crystal_ids.push(action.crystal_id);
            }
        }

        // Supersede actions: authorized against the selected context and the
        // active-rule protection immediately before the store calls, then
        // applied through the transaction-aware supersede primitive.
        for output in &outputs {
            validate_supersede_targets(
                &output.supersede_actions,
                allowed_crystal_ids,
                active_rule_ids,
            )
            .map_err(DreamError::InvalidOutput)?;
            for action in &output.supersede_actions {
                crate::crystals::supersede_in_transaction(
                    transaction,
                    action.old_crystal_id,
                    action.new_crystal_id,
                    &action.reason,
                    cycle_id,
                    &timestamp,
                )?;
                superseded_crystal_ids.push(action.old_crystal_id);
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
        // Bounded affected-memory set for the audit record: related
        // candidates search from the concepts this run created or reinforced,
        // read inside the transaction so the audited payload describes
        // exactly the state this transaction commits.
        let related_candidates =
            self.searched_related_candidates(transaction, &created_concept_ids)?;
        let affected_memory_set = self.affected_memory_set(
            &[created_crystal_ids.clone(), superseded_crystal_ids.clone()].concat(),
            &related_candidates,
        );
        Ok(ApplySummary {
            created_crystal_ids,
            created_concept_ids,
            created_facet_ids,
            created_links,
            superseded_crystal_ids,
            reinforced_crystal_ids,
            archived_memory_ids,
            dreamed_session_ids,
            rejected_entries,
            skipped_candidates,
            related_candidates,
            affected_memory_set,
        })
    }

    /// Port of `_searched_related_candidates` (caps live here so the concept
    /// slice cannot grow past them). Runs on the caller's connection — inside
    /// the persistence transaction for the atomic phase commit (task D3).
    fn searched_related_candidates(
        &self,
        connection: &Connection,
        created_concept_ids: &[i64],
    ) -> Result<Value, DreamError> {
        let concept_ids = unique_ints(created_concept_ids);
        let capped: Vec<i64> = concept_ids
            .into_iter()
            .take(self.dream_config.max_related_concepts_per_cycle.max(0) as usize)
            .collect();
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
    // Deterministic phases (spec §Phase Boundaries steps 5-7)
    // ------------------------------------------------------------------

    /// Run the algorithmic, provider-free phases over their own bounded
    /// selections, each as one audited phase record and one transaction:
    /// reconsolidation evaluates advisory working copies, reinforcement
    /// consumes unprocessed `recalled_again` memory events, and the link
    /// reinforcer consumes `useful` activation rows (hebbian links plus
    /// pairwise combination). Bounded mutation: every phase reads with an
    /// absolute limit from the validated caps and mutates only its recorded
    /// selection plus ids created in the same run.
    fn run_deterministic_phases(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        threshold_state: &Value,
        provider_created_crystals: usize,
    ) -> Result<DeterministicSummary, DreamError> {
        let mut total = DeterministicSummary::default();

        let crystal_budget = (self
            .dream_config
            .max_long_term_records_affected_per_run
            .max(0) as usize)
            .saturating_sub(provider_created_crystals);
        let reconsolidation = self.run_reconsolidation(
            run_id,
            cycle_id,
            trigger_type,
            threshold_state,
            crystal_budget,
        )?;
        total.merge(reconsolidation);

        let reinforcement =
            self.run_feedback_reinforcement(run_id, cycle_id, trigger_type, threshold_state)?;
        total.merge(reinforcement);

        let links = self.run_link_reinforcement(run_id, cycle_id, trigger_type, threshold_state)?;
        total.merge(links);
        Ok(total)
    }

    /// True when any non-archived session-scoped working copy exists.
    fn reconsolidation_pending(&self) -> Result<bool, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let pending: i64 = connection.query_row(
            "select exists (
                 select 1 from short_term_memories
                 where archived_at is null and source_crystal_id is not null
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(pending != 0)
    }

    /// The reconsolidator (July design §Dream-Time Integration): for every
    /// non-archived working copy, a token-level diff ratio against the source
    /// crystal's current text decides reinforce-in-place versus supersede.
    /// Either way the processed working copy is archived. Active rule crystals
    /// are never superseded or reinforced here (ADR 0011: dreaming cannot
    /// transition deterministic authority), and a source that is no longer
    /// active at all (combined away or superseded) only retires the copy —
    /// dreaming never mutates or succeeds a non-active row.
    fn run_reconsolidation(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        threshold_state: &Value,
        crystal_budget: usize,
    ) -> Result<DeterministicSummary, DreamError> {
        if crystal_budget == 0 || !self.reconsolidation_pending()? {
            return Ok(DeterministicSummary::default());
        }

        // Bounded plan: absolute limit from max_short_term_memories_per_run,
        // evaluated before persistence.
        let connection = open_migrated(&self.config.database_path())?;
        let copies: Vec<(i64, i64, i64, String)> = {
            let mut statement = connection.prepare(
                "select id, session_id, source_crystal_id, text
                 from short_term_memories
                 where archived_at is null and source_crystal_id is not null
                 order by id
                 limit ?1",
            )?;
            let rows = statement.query_map(
                [self.dream_config.max_short_term_memories_per_run],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        drop(connection);
        if copies.is_empty() {
            return Ok(DeterministicSummary::default());
        }
        let phase_run_id = self.start_deterministic_phase_run(
            run_id,
            cycle_id,
            "reconsolidation",
            copies.len() as i64,
        )?;

        let threshold = self.dream_config.reconsolidation_diff_threshold;
        // One immediate write transaction: the phase's crystal and memory
        // mutations (each source revalidated inside it), the
        // phase-completed status, and the redacted audit entry commit
        // together or not at all (task D3).
        let mut connection = open_migrated(&self.config.database_path())?;
        let committed = commit_audited(&mut connection, |transaction| {
            let mut summary = DeterministicSummary::default();
            let mut budget = crystal_budget;
            for (memory_id, session_id, crystal_id, working_text) in copies {
                // The crystal-mutation cost is charged against the remaining
                // run budget before any work happens (caps are enforced before
                // persistence): superseding creates one crystal and changes one,
                // reinforcing changes one; protected and retired copies are free.
                let Some(original) =
                    load_reconsolidation_source(transaction, crystal_id).map_err(tx_error)?
                else {
                    // The source crystal is gone; the working copy has nothing to
                    // consolidate against, so it is retired.
                    archive_working_copy(transaction, memory_id).map_err(tx_error)?;
                    summary.actions.push(json!({
                        "memory_id": memory_id,
                        "crystal_id": crystal_id,
                        "action": "source_missing",
                    }));
                    summary.archived_memory_ids.push(memory_id);
                    continue;
                };
                if original.status != "active" {
                    // The source was combined away or superseded after the
                    // working copy was created: like a missing source, it offers
                    // nothing live to consolidate against. Reinforcing would
                    // mutate a retired row and superseding would crystallize a
                    // fresh active successor of an absorbed crystal, resurfacing
                    // combined-away knowledge — so the copy just retires.
                    archive_working_copy(transaction, memory_id).map_err(tx_error)?;
                    summary.actions.push(json!({
                        "memory_id": memory_id,
                        "crystal_id": crystal_id,
                        "source_status": original.status,
                        "action": "source_inactive",
                    }));
                    summary.archived_memory_ids.push(memory_id);
                    continue;
                }
                let ratio = token_diff_ratio(&original.text, &working_text);
                let (action, cost) =
                    if crate::crystals::is_active_rule(&original.crystal_type, &original.status) {
                        // ADR 0011: dreaming never transitions active deterministic
                        // rule authority, however far the working copy diverged.
                        ("rule_protected", 0)
                    } else if ratio < threshold {
                        ("reinforced", 1)
                    } else {
                        ("superseded", 2)
                    };
                if cost > budget {
                    break;
                }
                match action {
                    "reinforced" => {
                        apply_score_delta(
                            transaction,
                            crystal_id,
                            RECONSOLIDATION_REINFORCE_DELTAS.0,
                            RECONSOLIDATION_REINFORCE_DELTAS.1,
                            &now(),
                        )?;
                        transaction.execute(
                            "update crystals set last_reinforced_cycle = ?1, updated_at = ?2
                         where id = ?3",
                            rusqlite::params![cycle_id, now(), crystal_id],
                        )?;
                        archive_working_copy(transaction, memory_id).map_err(tx_error)?;
                        summary.changed_crystal_ids.push(crystal_id);
                    }
                    "superseded" => {
                        let successor_id = insert_reconsolidated_crystal(
                            transaction,
                            crystal_id,
                            &original,
                            &working_text,
                            cycle_id,
                        )
                        .map_err(tx_error)?;
                        transaction.execute(
                        "update crystals set status = 'superseded', updated_at = ?1 where id = ?2",
                        rusqlite::params![now(), crystal_id],
                    )?;
                        archive_working_copy(transaction, memory_id).map_err(tx_error)?;
                        summary.created_crystal_ids.push(successor_id);
                        summary.changed_crystal_ids.push(crystal_id);
                        summary.dreamed_session_ids.push(session_id);
                    }
                    _ => {
                        archive_working_copy(transaction, memory_id).map_err(tx_error)?;
                    }
                }
                summary.archived_memory_ids.push(memory_id);
                summary.actions.push(json!({
                    "memory_id": memory_id,
                    "crystal_id": crystal_id,
                    "diff_ratio": ratio,
                    "action": action,
                }));
                budget -= cost;
            }
            // Sessions whose last pending memory was archived by this phase can
            // complete their dream lifecycle under the same guard as the
            // provider persistence batch.
            for session_id in unique_ints(&summary.dreamed_session_ids) {
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
            self.complete_deterministic_phase_in_transaction(
                transaction,
                run_id,
                phase_run_id,
                "reconsolidation",
                trigger_type,
                threshold_state,
                &summary,
            )
            .map_err(tx_error)?;
            Ok(summary)
        });
        match committed {
            Ok(summary) => Ok(summary),
            Err(error) => Err(self.phase_commit_failure(
                run_id,
                Some(phase_run_id),
                "reconsolidation",
                trigger_type,
                &deterministic_identity(),
                error,
            )),
        }
    }

    /// True when any `recalled_again` memory event was not yet consumed.
    fn reinforcement_pending(&self) -> Result<bool, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let pending: i64 = connection.query_row(
            "select exists (
                 select 1 from memory_events
                 where event_type = 'recalled_again' and applied = 0
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(pending != 0)
    }

    /// Passive reinforcement: `recalled_again` events accumulated since the
    /// last cycle contribute their recorded strength-only deltas once, then
    /// are stamped `applied = 1` with the run's cycle. Immediate feedback
    /// events (`recalled_useful`/`recalled_miss`) are inserted `applied = 1`
    /// by the feedback store and are never read here.
    fn run_feedback_reinforcement(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        threshold_state: &Value,
    ) -> Result<DeterministicSummary, DreamError> {
        if !self.reinforcement_pending()? {
            return Ok(DeterministicSummary::default());
        }
        let phase_run_id =
            self.start_deterministic_phase_run(run_id, cycle_id, "reinforcement", 0)?;
        let limit = self.dream_config.max_total_affected_crystals;

        // One immediate write transaction: the event consumption and score
        // mutations, the phase-completed status, and the redacted audit entry
        // commit together or not at all (task D3).
        let mut connection = open_migrated(&self.config.database_path())?;
        let committed = commit_audited(&mut connection, |transaction| {
            let mut summary = DeterministicSummary::default();
            let events: Vec<(i64, i64, f64, f64)> = {
                let mut statement = transaction.prepare(
                    "select id, crystal_id, strength_delta, confidence_delta
                 from memory_events
                 where event_type = 'recalled_again' and applied = 0
                 order by id
                 limit ?1",
                )?;
                let rows = statement.query_map([limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, f64>(3)?,
                    ))
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            for (event_id, crystal_id, strength_delta, confidence_delta) in events {
                apply_score_delta(
                    transaction,
                    crystal_id,
                    strength_delta,
                    confidence_delta,
                    &now(),
                )?;
                transaction.execute(
                    "update memory_events set applied = 1, cycle_id = ?1 where id = ?2",
                    rusqlite::params![cycle_id, event_id],
                )?;
                summary.changed_crystal_ids.push(crystal_id);
                summary.actions.push(json!({
                    "event_id": event_id,
                    "crystal_id": crystal_id,
                    "strength_delta": strength_delta,
                    "confidence_delta": confidence_delta,
                }));
            }
            self.complete_deterministic_phase_in_transaction(
                transaction,
                run_id,
                phase_run_id,
                "reinforcement",
                trigger_type,
                threshold_state,
                &summary,
            )
            .map_err(tx_error)?;
            Ok(summary)
        });
        match committed {
            Ok(summary) => Ok(summary),
            Err(error) => Err(self.phase_commit_failure(
                run_id,
                Some(phase_run_id),
                "reinforcement",
                trigger_type,
                &deterministic_identity(),
                error,
            )),
        }
    }

    /// True when link work remains: either an unconsumed `useful`
    /// activation (not yet snapshotted, or waiting in an open batch) or an
    /// open batch whose queued pairs still need processing — the latter
    /// matters when snapshot members were cascade-deleted after the batch
    /// was created, leaving tombstone skips to record.
    fn links_pending(&self) -> Result<bool, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        let pending: i64 = connection.query_row(
            "select exists (
                 select 1 from crystal_activations
                 where outcome = 'useful' and cycle_id is null
             )
             or exists (
                 select 1 from dream_link_batches
                 where completed_cycle is null
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(pending != 0)
    }

    /// The link reinforcer (July design §Dream-Time Integration): useful
    /// co-activated crystals strengthen (or create) their `crystal_links` row
    /// (hebbian rule); near-duplicate pairs combine pairwise. Progress is
    /// durable per pair (task D4): eligible activations are snapshotted into
    /// one durable batch per session, each budgeted pair commits its
    /// reinforcement, its pair-row terminalization, and its audit entry
    /// atomically, and unprocessed pairs stay queued — so an exhausted
    /// budget or a mid-phase crash resumes on the next cycle instead of
    /// dropping pairs. A batch's activations are stamped consumed only when
    /// its last pair is terminal; the phase's completion status and audit
    /// are the final atomic commit of the phase, and a failure there leaves
    /// the committed pairs durable while the failure is audited
    /// post-rollback (D3's `record_phase_failure` pattern).
    fn run_link_reinforcement(
        &self,
        run_id: i64,
        cycle_id: i64,
        trigger_type: &str,
        threshold_state: &Value,
    ) -> Result<DeterministicSummary, DreamError> {
        if !self.links_pending()? {
            return Ok(DeterministicSummary::default());
        }
        let phase_run_id =
            self.start_deterministic_phase_run(run_id, cycle_id, "link_reinforcement", 0)?;

        // The link budget (ruling: the existing max_relation_records_per_pass
        // config field) bounds the pairs terminalized in this cycle.
        let pair_budget = self.dream_config.max_relation_records_per_pass.max(0) as usize;
        let mut progress = LinkProgress::open(&self.config)?;
        progress.set_run_context(run_id, Some(phase_run_id));
        let summary = match progress.process(cycle_id, pair_budget) {
            Ok(_) => progress.take_summary(),
            Err(error) => {
                // Committed pairs of this call stay committed (each pair is
                // its own atomic commit); the failure is audited after the
                // rollback of the in-flight pair.
                return Err(self.record_link_phase_failure(
                    run_id,
                    Some(phase_run_id),
                    trigger_type,
                    progress.terminalized_in_call(),
                    error,
                ));
            }
        };
        let deterministic = DeterministicSummary {
            changed_crystal_ids: summary.changed_crystal_ids,
            actions: summary.actions,
            ..DeterministicSummary::default()
        };

        // The phase's final atomic commit: the phase-completed status and
        // its redacted audit entry. Committed pairs stay durable when this
        // fails; the next cycle resumes the queued remainder.
        let mut connection = open_migrated(&self.config.database_path())?;
        let committed = commit_audited(&mut connection, |transaction| {
            self.complete_deterministic_phase_in_transaction(
                transaction,
                run_id,
                phase_run_id,
                "link_reinforcement",
                trigger_type,
                threshold_state,
                &deterministic,
            )
            .map_err(tx_error)?;
            Ok(())
        });
        match committed {
            Ok(()) => Ok(deterministic),
            Err(error) => Err(self.phase_commit_failure(
                run_id,
                Some(phase_run_id),
                "link_reinforcement",
                trigger_type,
                &deterministic_identity(),
                error,
            )),
        }
    }

    fn start_deterministic_phase_run(
        &self,
        run_id: i64,
        cycle_id: i64,
        phase: &str,
        input_count: i64,
    ) -> Result<i64, DreamError> {
        let connection = open_migrated(&self.config.database_path())?;
        connection.execute(
            "insert into dream_phase_runs(
               dream_run_id, phase, provider_profile, provider_type, model, status,
               input_count, created_at
             )
             values (?1, ?2, 'deterministic', 'deterministic', 'deterministic', 'running', ?3, ?4)",
            rusqlite::params![run_id, phase, input_count, now()],
        )?;
        self.notify_phase_observer(run_id, cycle_id, phase);
        Ok(connection.last_insert_rowid())
    }

    /// Complete a deterministic phase inside the caller's transaction: the
    /// phase record's completed status plus one redacted `phase_completed`
    /// entry carrying the affected-id sets (spec §Audit), committed
    /// atomically with the phase's domain mutations (task D3).
    #[allow(clippy::too_many_arguments)]
    fn complete_deterministic_phase_in_transaction(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        run_id: i64,
        phase_run_id: i64,
        phase: &str,
        trigger_type: &str,
        threshold_state: &Value,
        summary: &DeterministicSummary,
    ) -> Result<(), DreamError> {
        transaction.execute(
            "update dream_phase_runs
             set status = 'completed', output_count = ?1, completed_at = ?2
             where id = ?3",
            rusqlite::params![
                summary.changed_crystal_ids.len() as i64,
                now(),
                phase_run_id
            ],
        )?;

        let provider = deterministic_identity();
        let mut payload = self.audit_base(trigger_type, threshold_state, &[], phase, &provider);
        payload.insert("provider_type".into(), json!("deterministic"));
        payload.insert(
            "created_crystals".into(),
            json!(summary.created_crystal_ids),
        );
        payload.insert(
            "changed_crystals".into(),
            json!(unique_ints(&summary.changed_crystal_ids)),
        );
        payload.insert(
            "archived_short_term_memory_ids".into(),
            json!(summary.archived_memory_ids),
        );
        payload.insert(
            "dreamed_session_ids".into(),
            json!(unique_ints(&summary.dreamed_session_ids)),
        );
        payload.insert("actions".into(), json!(summary.actions));
        DreamAuditStore::append_in_transaction(
            transaction,
            run_id,
            Some(phase_run_id),
            "phase_completed",
            "info",
            &format!("completed {phase} phase"),
            &Value::Object(payload),
        )?;
        Ok(())
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
            provider: self.run_provider_label(),
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

    /// Record an honest skip row (`record_skipped_run`): a run that did not
    /// happen, with the reason on the row — the scheduled tick's
    /// `not_enough_memories` skips and cross-process lock contention are
    /// recorded, never silent.
    pub fn record_skipped_run(&self, reason: &str) -> Result<DreamRunRecord, DreamError> {
        self.record_negative_run("skipped", reason)
    }

    /// The blocked-drain record: a durable failed run row naming why the
    /// drain stopped while eligible work remained (never a success claim).
    fn record_failed_run(&self, reason: &str) -> Result<DreamRunRecord, DreamError> {
        self.record_negative_run("failed", reason)
    }

    fn record_negative_run(
        &self,
        status: &str,
        reason: &str,
    ) -> Result<DreamRunRecord, DreamError> {
        let mut connection = open_migrated(&self.config.database_path())?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let cycle_id = next_skipped_cycle_id(&transaction)?;
        let timestamp = now();
        transaction.execute(
            "insert into dream_runs(cycle_id, status, provider, error, created_at, completed_at)
             values (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                cycle_id,
                status,
                self.run_provider_label(),
                reason,
                timestamp,
                timestamp
            ],
        )?;
        let run_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(DreamRunRecord {
            id: run_id,
            cycle_id,
            status: status.to_string(),
            provider: self.run_provider_label(),
            input_count: 0,
            created_crystal_count: 0,
            proposal_count: 0,
            error: reason.to_string(),
        })
    }

    /// The run-level provider summary (the `dream_runs.provider` label): the
    /// first enabled workflow's wire provider name, or the injected test
    /// provider under the deterministic seam. Empty when no workflow is
    /// enabled — such a run fails the coverage gate before any pass.
    fn run_provider_label(&self) -> String {
        self.resolver.run_provider_name(&self.choices)
    }

    fn start_phase_run(
        &self,
        run_id: i64,
        cycle_id: i64,
        phase: &str,
        input_count: i64,
        provider: &ProviderIdentity,
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
                provider.profile,
                provider.name,
                provider.model,
                input_count,
                now(),
            ],
        )?;
        self.notify_phase_observer(run_id, cycle_id, phase);
        Ok(connection.last_insert_rowid())
    }

    /// Fire the phase-progress observer, if one is installed. Best-effort by
    /// design: progress streaming never fails a run.
    fn notify_phase_observer(&self, run_id: i64, cycle_id: i64, phase: &str) {
        if let Some(observer) = &self.phase_observer {
            observer(run_id, cycle_id, phase);
        }
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
        provider: &ProviderIdentity,
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
        payload.insert("provider_profile".into(), json!(provider.profile));
        payload.insert("model".into(), json!(provider.model));
        payload.insert(
            "endpoint".into(),
            if provider.endpoint.is_empty() {
                Value::Null
            } else {
                json!(redact_endpoint(&provider.endpoint))
            },
        );
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
            "concept_count": staged.iter().map(|output| output.concepts.len()).sum::<usize>(),
            "facet_count": staged.iter().map(|output| output.facets.len()).sum::<usize>(),
            "concept_proposal_count": staged
                .iter()
                .map(|output| output.concept_proposals.len())
                .sum::<usize>(),
            "supersede_action_count": staged
                .iter()
                .map(|output| output.supersede_actions.len())
                .sum::<usize>(),
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
        prompt_sha256: &str,
        provider: &ProviderIdentity,
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
            provider,
        );
        payload.insert("request_summary".into(), self.request_summary(groups));
        payload.insert("prompt_sha256".into(), json!(prompt_sha256));
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
        prompt_sha256: &str,
        provider: &ProviderIdentity,
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
            provider,
        );
        payload.insert("response_summary".into(), response_summary.clone());
        payload.insert("parse_warnings".into(), json!([]));
        payload.insert("prompt_sha256".into(), json!(prompt_sha256));
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
        provider: &ProviderIdentity,
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            pass_name,
            provider,
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
    fn audit_phase_completed_in_transaction(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        threshold_state: &Value,
        selected_memory_ids: &[i64],
        groups: &[SelectionGroup],
        staged: &[NormalizedOutput],
        summary: &ApplySummary,
        provider: &ProviderIdentity,
    ) -> Result<(), DreamError> {
        let phase_name = "persistence";
        let mut payload = self.audit_base(
            trigger_type,
            threshold_state,
            selected_memory_ids,
            phase_name,
            provider,
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
                "concepts": staged.iter().map(|output| output.concepts.len()).sum::<usize>(),
                "facets": staged.iter().map(|output| output.facets.len()).sum::<usize>(),
                "concept_proposals": staged
                    .iter()
                    .map(|output| output.concept_proposals.len())
                    .sum::<usize>(),
                "supersede_actions": staged
                    .iter()
                    .map(|output| output.supersede_actions.len())
                    .sum::<usize>(),
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
        payload.insert(
            "created_concepts".into(),
            json!(summary.created_concept_ids),
        );
        payload.insert("created_facets".into(), json!(summary.created_facet_ids));
        payload.insert("created_links".into(), json!(summary.created_links));
        payload.insert(
            "superseded_crystals".into(),
            json!(unique_ints(&summary.superseded_crystal_ids)),
        );
        payload.insert(
            "reinforced_crystals".into(),
            json!(unique_ints(&summary.reinforced_crystal_ids)),
        );
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
        DreamAuditStore::append_in_transaction(
            transaction,
            run_id,
            phase_run_id,
            "phase_completed",
            "info",
            &format!("completed {phase_name} phase"),
            &Value::Object(payload),
        )?;
        Ok(())
    }

    /// The post-rollback failure record (controller ruling 3): when the
    /// atomic domain+phase+audit commit fails, the rollback also removed the
    /// audit entry, so a `phase_failed` record is appended through a separate
    /// connection AFTER the rollback. It names the failure and carries no
    /// affected-id sets or counts: nothing committed, so no domain effect may
    /// be claimed. Deliberately outside the rolled-back transaction.
    fn record_phase_failure(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        phase: &str,
        trigger_type: &str,
        provider: &ProviderIdentity,
        error: &DreamError,
    ) -> Result<(), DreamError> {
        let mut payload = self.audit_base(trigger_type, &json!({}), &[], phase, provider);
        payload.insert("error".into(), json!(self.redacted_error_message(error)));
        payload.insert(
            "committed_domain_effects".into(),
            json!("none: the phase transaction rolled back"),
        );
        self.audit.append(
            run_id,
            phase_run_id,
            "phase_failed",
            "error",
            &format!("failed {phase} phase"),
            &Value::Object(payload),
        )?;
        Ok(())
    }

    /// Map a failed [`commit_audited`] to the dreaming error and preserve the
    /// post-rollback failure audit. Best-effort record: if the failure audit
    /// itself cannot be written, the original error still wins.
    fn phase_commit_failure(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        phase: &str,
        trigger_type: &str,
        provider: &ProviderIdentity,
        error: rusqlite::Error,
    ) -> DreamError {
        let error = DreamError::from(error);
        let _ =
            self.record_phase_failure(run_id, phase_run_id, phase, trigger_type, provider, &error);
        error
    }

    /// The post-rollback failure record for the link phase (task D4): every
    /// budgeted pair commits on its own, so a mid-phase failure leaves the
    /// pairs committed earlier in the call durable — unlike the whole-phase
    /// failures, the record must name them instead of claiming no domain
    /// effects. The in-flight pair rolled back and stays queued.
    fn record_link_phase_failure(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        trigger_type: &str,
        committed_pairs: usize,
        error: DreamError,
    ) -> DreamError {
        let provider = deterministic_identity();
        let mut payload = self.audit_base(
            trigger_type,
            &json!({}),
            &[],
            "link_reinforcement",
            &provider,
        );
        payload.insert("error".into(), json!(self.redacted_error_message(&error)));
        payload.insert("committed_link_pairs".into(), json!(committed_pairs));
        payload.insert(
            "committed_domain_effects".into(),
            if committed_pairs == 0 {
                json!("none: only uncommitted pair work rolled back")
            } else {
                json!(format!(
                    "committed {committed_pairs} link pair(s) earlier in this phase; each pair's commit is durable"
                ))
            },
        );
        let _ = self.audit.append(
            run_id,
            phase_run_id,
            "phase_failed",
            "error",
            "failed link_reinforcement phase",
            &Value::Object(payload),
        );
        error
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
        if normalized_output_count(output) as i64 > limit {
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

fn next_cycle_id(connection: &Connection) -> Result<i64, DreamError> {
    let cycle_id = connection.query_row(
        "select coalesce(max(cycle_id), 0) + 1 from dream_runs where cycle_id > 0",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(cycle_id)
}

/// Lift a typed dreaming error through the [`commit_audited`] closure
/// boundary (which must return `rusqlite::Result`): the original message is
/// preserved verbatim as the wrapped failure source, so failed-run records
/// and audit entries keep naming the real cause.
pub(crate) fn tx_error(error: impl Into<DreamError>) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error.into()))
}

/// The transaction-aware phase completion used by the atomic phase commits
/// (task D3): the same statement as the standalone [`DreamService::complete_phase_run`],
/// but it rides in the caller's immediate transaction so the phase is
/// durably completed exactly when its domain effects and audit entry are.
fn complete_phase_run_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    phase_run_id: i64,
    output_count: i64,
) -> Result<(), DreamError> {
    transaction.execute(
        "update dream_phase_runs
         set status = 'completed', output_count = ?1, completed_at = ?2
         where id = ?3",
        rusqlite::params![output_count, now(), phase_run_id],
    )?;
    Ok(())
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

/// Port of `_normalize_dict_output` over the complete normalized contract:
/// concept proposals, concepts, facets, the four crystal buckets (with
/// concept names), supersede actions, and reinforce actions. Malformed
/// entries are rejected individually into `rejected_entries` /
/// `skipped_candidates` with durable details; a bad entry never drops its
/// whole section.
pub fn normalize_dict_output(
    payload: &Value,
    allowed_memory_ids: &HashSet<i64>,
    valid_concept_ids: &HashSet<i64>,
) -> Result<NormalizedOutput, DreamError> {
    let mut output = NormalizedOutput::default();

    for (index, item) in list_from_payload(payload.get("concept_proposals"))
        .into_iter()
        .enumerate()
    {
        if let Some(proposal) = crate::dream_output::normalize_concept_proposal_entry(
            &item,
            &format!("concept_proposals[{index}]"),
            &mut output.rejected_entries,
        ) {
            output.concept_proposals.push(proposal);
        }
    }
    for (index, item) in list_from_payload(payload.get("concepts"))
        .into_iter()
        .enumerate()
    {
        if let Some(concept) = crate::dream_output::normalize_concept_entry(
            &item,
            &format!("concepts[{index}]"),
            &mut output.warnings,
            &mut output.rejected_entries,
        ) {
            output.concepts.push(concept);
        }
    }
    for (index, item) in list_from_payload(payload.get("facets"))
        .into_iter()
        .enumerate()
    {
        if let Some(facet) = crate::dream_output::normalize_facet_entry(
            &item,
            &format!("facets[{index}]"),
            &mut output.warnings,
            &mut output.rejected_entries,
        ) {
            output.facets.push(facet);
        }
    }

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

    // Both the Python wire keys and the contract's long forms are accepted.
    for key in ["supersede", "supersede_actions"] {
        for (index, item) in list_from_payload(payload.get(key)).into_iter().enumerate() {
            match crate::dream_output::normalize_supersede_entry(&item) {
                Some(action) => output.supersede_actions.push(action),
                None => output.skipped_candidates.push(json!({
                    "entry_path": format!("{key}[{index}]"),
                    "reason": "malformed_supersede_action",
                    "candidate_type": value_type_name(&item),
                })),
            }
        }
    }
    for key in ["reinforce", "reinforce_actions"] {
        for (index, item) in list_from_payload(payload.get(key)).into_iter().enumerate() {
            if let Some(action) = crate::dream_output::normalize_reinforce_entry(
                &item,
                &format!("{key}[{index}]"),
                allowed_memory_ids,
                &mut output.rejected_entries,
            ) {
                output.reinforce_actions.push(action);
            }
        }
    }
    Ok(output)
}

pub(crate) fn list_from_payload(value: Option<&Value>) -> Vec<Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    }
}

pub(crate) fn value_type_name(value: &Value) -> &'static str {
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
    let (concept_names, concept_name_penalty) =
        crate::dream_output::concept_names_from_payload(&payload, entry_path, warnings);
    penalty +=
        story_scope_penalty + semantic_tag_penalty + concept_id_penalty + concept_name_penalty;

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

    // Provider-supersede metadata (an advisory provenance pointer; the
    // audited supersede section, not this field, performs mutations).
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
        concept_names,
    }))
}

pub(crate) fn append_parse_warning(
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

pub(crate) fn collapse_whitespace(text: &str) -> String {
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

pub(crate) fn clean_text_tuple(values: Vec<String>) -> Vec<String> {
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

pub(crate) fn string_field(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

pub(crate) fn numeric_field(value: Option<&Value>, default: f64) -> f64 {
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

pub(crate) fn optional_int(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Port of `_validate_normalized_output` (crystals plus the graph sections).
/// Normalization already rejects malformed entries individually; these are
/// the fail-closed invariants over the accepted records.
fn validate_normalized_output(
    output: &NormalizedOutput,
    context: &TranslationContext,
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
    crate::dream_output::validate_concepts(&output.concepts).map_err(DreamError::InvalidOutput)?;
    crate::dream_output::validate_facets(&output.facets).map_err(DreamError::InvalidOutput)?;
    crate::dream_output::validate_proposal_contexts(
        &output.concept_proposals,
        &context.series_slug,
        &context.source_language,
        &context.target_language,
    )
    .map_err(DreamError::InvalidOutput)?;
    crate::dream_output::validate_reinforce_actions(&output.reinforce_actions)
        .map_err(DreamError::InvalidOutput)?;
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
/// so identical crystals (type + case-insensitive title/text) and identical
/// concepts (case-insensitive canonical name) apply once.
fn deduplicate_staged_outputs(staged: &[NormalizedOutput]) -> Vec<NormalizedOutput> {
    let mut seen_crystals: HashSet<(String, String, String)> = HashSet::new();
    let mut seen_concepts: HashSet<String> = HashSet::new();
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
        let mut concepts = Vec::with_capacity(output.concepts.len());
        for concept in &output.concepts {
            if seen_concepts.insert(concept.canonical_name.to_lowercase()) {
                concepts.push(concept.clone());
            }
        }
        result.push(NormalizedOutput {
            crystals,
            concept_proposals: output.concept_proposals.clone(),
            concepts,
            facets: output.facets.clone(),
            supersede_actions: output.supersede_actions.clone(),
            reinforce_actions: output.reinforce_actions.clone(),
            warnings: output.warnings.clone(),
            rejected_entries: output.rejected_entries.clone(),
            skipped_candidates: output.skipped_candidates.clone(),
        });
    }
    result
}

/// Python `_resolve_candidate_concepts`: resolve the candidate's concept
/// names within this output's own map, creating or reinforcing missing
/// concepts as global candidates, and merge the sorted id set.
fn resolve_candidate_concepts(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &NormalizedCrystal,
    concept_ids_by_name: &mut BTreeMap<String, i64>,
    timestamp: &str,
) -> Result<NormalizedCrystal, DreamError> {
    let mut concept_ids = candidate.concept_ids.clone();
    for concept_name in &candidate.concept_names {
        let key = concept_name.to_lowercase();
        let concept_id = match concept_ids_by_name.get(&key) {
            Some(concept_id) => *concept_id,
            None => {
                let concept_id = crate::concepts::create_or_reinforce_concept_in_transaction(
                    transaction,
                    concept_name,
                    "",
                    &[],
                    0.2,
                    "global",
                    "",
                    timestamp,
                )?;
                concept_ids_by_name.insert(key, concept_id);
                concept_id
            }
        };
        concept_ids.push(concept_id);
    }
    concept_ids.sort_unstable();
    concept_ids.dedup();
    if concept_ids == candidate.concept_ids {
        return Ok(candidate.clone());
    }
    Ok(NormalizedCrystal {
        concept_ids,
        ..candidate.clone()
    })
}

/// Provider free text echoed into a rejection record stays bounded: at most
/// [`REJECTION_TITLE_MAX_CHARS`] characters plus an explicit ellipsis marker,
/// so the audit stays actionable without persisting the whole value.
pub(crate) fn bounded_rejection_title(title: &str) -> String {
    if title.chars().count() <= REJECTION_TITLE_MAX_CHARS {
        return title.to_string();
    }
    let prefix: String = title.chars().take(REJECTION_TITLE_MAX_CHARS).collect();
    format!("{prefix}[...]")
}

/// The group whose memories a crystal cites. `None` marks an ambiguous
/// crystal: its source memories span distinct series contexts (or none), so
/// it must not silently land in the first group's context. Sessions sharing
/// one context are the same series — their memories attribute cleanly.
fn owning_crystal_group(
    groups: &[SelectionGroup],
    group_of_memory: &HashMap<i64, usize>,
    candidate: &NormalizedCrystal,
) -> Option<usize> {
    fn same_series_context(left: &TranslationContext, right: &TranslationContext) -> bool {
        left.scope_key() == right.scope_key()
            && left.series_slug == right.series_slug
            && left.source_language == right.source_language
            && left.target_language == right.target_language
    }
    let mut owned: Option<usize> = None;
    for memory_id in &candidate.source_memory_ids {
        let group_index = *group_of_memory.get(memory_id)?;
        match owned {
            None => owned = Some(group_index),
            Some(existing) => {
                if !same_series_context(&groups[existing].context, &groups[group_index].context) {
                    return None;
                }
            }
        }
    }
    owned
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

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Lowercase hex SHA-256: the audited prompt hash (spec §Audit).
fn prompt_sha256(prompt: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(prompt.as_bytes()))
}

/// Redact an endpoint URL for the audit record (spec §Audit "redacted
/// endpoint"): scheme, host, port, and path are kept; query strings and
/// userinfo credentials are stripped.
fn redact_endpoint(url: &str) -> String {
    let without_suffix = url.split(['?', '#']).next().unwrap_or(url);
    let Some(colon) = without_suffix.find("://") else {
        return without_suffix.to_string();
    };
    let scheme = &without_suffix[..colon + 1];
    let rest = &without_suffix[colon + 3..];
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, ""),
    };
    let host = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    format!("{scheme}//{host}{path}")
}

// ----------------------------------------------------------------------
// Reconsolidation mechanics (deterministic, no provider)
// ----------------------------------------------------------------------

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .map(String::from)
        .collect()
}

/// Token-level Levenshtein distance normalized by the longer token count:
/// the reconsolidator's diff ratio (July design: "edit distance under 20% of
/// token count"). Identical texts are 0.0, disjoint texts are 1.0.
pub fn token_diff_ratio(source: &str, working: &str) -> f64 {
    let source = tokenize(source);
    let working = tokenize(working);
    if source.is_empty() && working.is_empty() {
        return 0.0;
    }
    let denominator = source.len().max(working.len());
    if denominator == 0 {
        return 1.0;
    }
    levenshtein_distance(&source, &working) as f64 / denominator as f64
}

/// Token-set Jaccard similarity, case-insensitive: the named combination
/// similarity mechanism for co-activated near-duplicate crystals.
pub fn token_similarity(left: &str, right: &str) -> f64 {
    let left_text = left.to_lowercase();
    let right_text = right.to_lowercase();
    let left: std::collections::HashSet<String> = tokenize(&left_text).into_iter().collect();
    let right: std::collections::HashSet<String> = tokenize(&right_text).into_iter().collect();
    if left.is_empty() && right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

fn levenshtein_distance(left: &[String], right: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_token) in left.iter().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_token) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_token != right_token);
            let insertion = current[column] + 1;
            let deletion = previous[column + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}

/// The `crystals` columns the reconsolidator needs from the source crystal.
struct ReconsolidationSource {
    text: String,
    status: String,
    crystal_type: String,
    title: String,
    scope_type: String,
    scope_key: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    tags_json: String,
    strength: f64,
    confidence: f64,
    source_credibility: String,
    rule_intent: String,
    soft_origin: String,
    is_inferred: bool,
    malformed_penalty: f64,
}

fn load_reconsolidation_source(
    transaction: &rusqlite::Transaction<'_>,
    crystal_id: i64,
) -> Result<Option<ReconsolidationSource>, DreamError> {
    let row = transaction
        .query_row(
            "select text, status, crystal_type, title, scope_type, scope_key,
                    series_slug, source_language, target_language, tags_json,
                    strength, confidence, source_credibility, rule_intent,
                    soft_origin, is_inferred, malformed_penalty
             from crystals where id = ?1",
            [crystal_id],
            |row| {
                Ok(ReconsolidationSource {
                    text: row.get(0)?,
                    status: row.get(1)?,
                    crystal_type: row.get(2)?,
                    title: row.get(3)?,
                    scope_type: row.get(4)?,
                    scope_key: row.get(5)?,
                    series_slug: row.get(6)?,
                    source_language: row.get(7)?,
                    target_language: row.get(8)?,
                    tags_json: row.get(9)?,
                    strength: row.get(10)?,
                    confidence: row.get(11)?,
                    source_credibility: row.get(12)?,
                    rule_intent: row.get(13)?,
                    soft_origin: row.get(14)?,
                    is_inferred: row.get::<_, i64>(15)? != 0,
                    malformed_penalty: row.get(16)?,
                })
            },
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(row)
}

/// Crystallize the successor crystal from a diverged working copy: the
/// original's identity and metadata, the working copy's text, the original's
/// concept rows copied (not moved), `supersedes_crystal_id` pointing back.
fn insert_reconsolidated_crystal(
    transaction: &rusqlite::Transaction<'_>,
    original_crystal_id: i64,
    original: &ReconsolidationSource,
    working_text: &str,
    cycle_id: i64,
) -> Result<i64, DreamError> {
    let timestamp = now();
    transaction.execute(
        "insert into crystals(
           crystal_type, text, title, scope_type, scope_key, series_slug,
           source_language, target_language, tags_json, strength, confidence,
           source_credibility, rule_intent, soft_origin, is_inferred,
           malformed_penalty, supersedes_crystal_id, status, created_cycle,
           created_at, updated_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, 'active', ?18, ?19, ?19)",
        rusqlite::params![
            original.crystal_type,
            working_text,
            original.title,
            original.scope_type,
            original.scope_key,
            original.series_slug,
            original.source_language,
            original.target_language,
            original.tags_json,
            original.strength,
            original.confidence,
            original.source_credibility,
            original.rule_intent,
            original.soft_origin,
            original.is_inferred as i64,
            original.malformed_penalty,
            original_crystal_id,
            cycle_id,
            timestamp,
        ],
    )?;
    let successor_id = transaction.last_insert_rowid();
    transaction.execute(
        "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
        rusqlite::params![successor_id, original.title, working_text],
    )?;
    // Typed side tables and concept rows are copied, not moved: the
    // superseded original keeps its rows as inert audit history.
    transaction.execute(
        "insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
         select ?1, scope, confidence, created_at
         from crystal_story_scopes where crystal_id = ?2",
        rusqlite::params![successor_id, original_crystal_id],
    )?;
    transaction.execute(
        "insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
         select ?1, tag, confidence, created_at
         from crystal_semantic_tags where crystal_id = ?2",
        rusqlite::params![successor_id, original_crystal_id],
    )?;
    transaction.execute(
        "insert into crystal_language_tags(crystal_id, language_tag)
         select ?1, language_tag
         from crystal_language_tags where crystal_id = ?2",
        rusqlite::params![successor_id, original_crystal_id],
    )?;
    transaction.execute(
        "insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
         select ?1, concept_id, link_type, confidence, created_at
         from crystal_concepts where crystal_id = ?2",
        rusqlite::params![successor_id, original_crystal_id],
    )?;
    Ok(successor_id)
}

fn archive_working_copy(
    transaction: &rusqlite::Transaction<'_>,
    memory_id: i64,
) -> Result<(), DreamError> {
    transaction.execute(
        "update short_term_memories set archived_at = ?1 where id = ?2",
        rusqlite::params![now(), memory_id],
    )?;
    Ok(())
}

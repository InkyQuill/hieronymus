use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::concepts::ConceptStore;
use crate::crystals::CrystalStore;
use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::dreaming::source_credibility_confidence;
use crate::feedback::RECALLED_AGAIN_DELTAS;
use crate::memory_models::{CrystalRecord, ShortTermMemoryRecord, TranslationContext};
use crate::rag::RagStore;
use crate::rag_models::{RagChunkRecord, RagSearchHit};
use crate::semantic_recall::{
    NO_SEMANTIC_LANE_REASON, SemanticLane, conflicting_rule_ids, fuse_chunk_lanes,
    incomplete_semantic_reason,
};
use crate::terminology::{ContractTerm, Termbase, TermbaseError};
use crate::workspace::WorkspaceStore;

const SHORT_TERM_BASE_SCORE: f64 = 0.30;
const SHORT_TERM_RANK_STEP: f64 = 0.01;
const STORY_SCOPE_BOOST: f64 = 0.25;
const SEMANTIC_TAG_BOOST: f64 = 0.18;
const ACTIVE_RULE_BOOST: f64 = 0.20;
const LOW_CONFIDENCE_THOUGHT_PENALTY: f64 = 0.12;
const RECALL_REASON: &str = "fts";
const LONG_TERM_METADATA_REASON: &str = "metadata";
const SPREADING_ACTIVATION_REASON: &str = "spreading_activation";

/// Post-boost score above which a recalled crystal pulls its 1-hop
/// `crystal_links` neighbors into the same response (July design
/// §Recall-Time Behavior; a hardcoded module constant, not user-tunable).
pub const SPREADING_ACTIVATION_THRESHOLD: f64 = 0.55;

/// Neighbor score factor: `source_score * link_weight * SPREADING_ATTENUATION`.
pub const SPREADING_ATTENUATION: f64 = 0.5;

/// Flat additive boost for crystals with a non-empty advisory `rule_intent`
/// (the Python `_ACTIVE_RULE_BOOST` magnitude, applied by presence check
/// instead of a `crystal_type` gate). The slice-4 boost for active rule
/// crystals above is a separate, untouched constant.
pub const RULE_INTENT_BOOST: f64 = 0.20;

/// Single bounded neighbor query per triggering crystal.
const SPREADING_NEIGHBOR_LIMIT: i64 = 50;

/// Session-scoped working-copy provenance marker.
const WORKING_COPY_SOURCE_ROLE: &str = "recall";

/// Fresh unique id per recall invocation (ADR 0011): wall-clock nanos plus a
/// process-local sequence; activation rows of one invocation share it.
static RECALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn next_recall_id() -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let sequence = RECALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("rc-{nanos}-{sequence}")
}

#[derive(Debug, thiserror::Error)]
pub enum RecallError {
    #[error(transparent)]
    Coherent(#[from] crate::coherent_reads::CoherentReadError),
    #[error(transparent)]
    Claim(#[from] crate::authority_models::DecisionErrorV1),
    #[error("limit must be at least 1")]
    LimitTooSmall,
    #[error("unknown session: {0}")]
    UnknownSession(i64),
    #[error("short-term memories require an active session")]
    InactiveSession,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
    #[error(transparent)]
    Workspace(#[from] crate::workspace::WorkspaceError),
    #[error(transparent)]
    Crystal(#[from] crate::crystals::CrystalError),
    #[error(transparent)]
    Concept(#[from] crate::concepts::ConceptError),
    #[error(transparent)]
    Rag(#[from] crate::rag::RagError),
    #[error(transparent)]
    Terminology(#[from] TermbaseError),
    /// A strict hybrid search could not run its required semantic half over a
    /// series that HAS indexed text (task C5). Never returned by
    /// [`RecallService::recall`], which degrades with a warning instead: the
    /// difference is that mixed recall still carries memory and deterministic
    /// terminology, while a session-less chunk search has nothing left to
    /// answer with but the lexical half it must not pass off as complete.
    #[error("required semantic retrieval is unavailable: {0}")]
    SemanticUnavailable(String),
}

/// One ranked recall hit (ADR 0011: flat graded list; the deterministic
/// contract is returned separately and never mixed into this ordering).
/// Long-term hits expose the activation id written for that hit, so callers
/// can address feedback (ADR 0011 §Recall Feedback). Advisory RAG hits carry
/// `conflicts_with_rule_ids`: the active term-contract rules whose forbidden
/// variants occur in the chunk text — markers only, never contract authority.
#[derive(Debug, Clone, PartialEq)]
pub enum RecallHit {
    LongTerm {
        crystal: CrystalRecord,
        score: f64,
        reason: String,
        activation_id: i64,
    },
    ShortTerm {
        memory: ShortTermMemoryRecord,
        score: f64,
    },
    Rag {
        chunk: RagChunkRecord,
        score: f64,
        reason: String,
        conflicts_with_rule_ids: Vec<i64>,
    },
}

/// One structured, machine-readable warning riding on a recall response
/// (additive response metadata: consumers that ignore it see unchanged hit
/// behavior). `kind` is one of the stable `WARNING_*` constants below;
/// `reason` is a human-readable explanation. The serde projection is the
/// transport DTO for the recall response's `warnings` list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecallWarning {
    pub kind: String,
    pub reason: String,
}

/// Required semantics did not run over the current corpus, so the response is
/// INCOMPLETE: the semantic lane never searched (unarmed, or armed and unable
/// to reach its generation), or the shared semantic service reports that it
/// cannot serve the current corpus yet. Whatever ranked rows ride along are
/// the lexical half only and must never be read as a complete answer (task
/// C5, review finding A5).
pub const WARNING_SEMANTIC_UNAVAILABLE: &str = "semantic_lane_unavailable";
/// Corrupt semantic hits were excluded and a rebuild was scheduled (or one is
/// already in progress).
pub const WARNING_REPAIR_SCHEDULED: &str = "semantic_repair_scheduled";
/// Corrupt semantic hits were excluded but scheduling the rebuild failed.
pub const WARNING_REPAIR_FAILED: &str = "semantic_repair_failed";

/// What the shared semantic service can do for a query right now, as its
/// owner sees it.
///
/// Deliberately two states rather than a copy of the daemon's controller
/// vocabulary (`Acquiring`/`Rebuilding`/`Ready`/`Failed`): this crate must not
/// learn the daemon's job states, and retrieval treats every non-ready one
/// identically — required semantics cannot be counted on, and the caller has
/// to be told. The `Unavailable` payload is the owner's actionable reason,
/// passed through verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticAvailability {
    Ready,
    Unavailable(String),
}

/// The installed availability probe: cheap, read once per query. The daemon
/// installs a closure over its semantic controller's state; a caller with no
/// semantic service installs nothing, and `None` is itself a fact (see
/// [`crate::semantic_recall::incomplete_semantic_reason`]).
pub type SemanticStatusHook = Arc<dyn Fn() -> SemanticAvailability + Send + Sync>;

/// One recall invocation: its durable `recall_id`, the deterministic term
/// contract computed from the query/source context BEFORE any lane fusion
/// (ADR 0011: returned separately, never recomputed from selected hits and
/// never mixed into the ordering), the ranked hits whose long-term activation
/// ids feed the feedback contract, and the structured degraded-mode/repair
/// `warnings` (an empty list means every lane ran clean).
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResponse {
    pub resulting_revision: u64,
    pub recall_id: String,
    pub deterministic_contract: Vec<ContractTerm>,
    pub hits: Vec<RecallHit>,
    pub non_current: Vec<RecallHit>,
    pub candidate_exhausted: bool,
    pub warnings: Vec<RecallWarning>,
}

/// Bounded standalone search. `candidate_exhausted` means the available
/// bounded candidates cannot fill the requested Current/Qualified limit;
/// it also includes natural exhaustion, not a claim of exhaustive coverage.
#[derive(Debug, Clone, PartialEq)]
pub struct RagSearchResponse {
    pub hits: Vec<RagSearchHit>,
    pub candidate_exhausted: bool,
}

impl RecallHit {
    pub fn claim_annotation(&self) -> &crate::claim_reads::ClaimReadAnnotation {
        match self {
            Self::LongTerm { crystal, .. } => &crystal.claim_annotation,
            Self::ShortTerm { memory, .. } => &memory.claim_annotation,
            Self::Rag { chunk, .. } => &chunk.claim_annotation,
        }
    }

    pub fn source(&self) -> &'static str {
        match self {
            RecallHit::LongTerm { .. } => "long_term",
            RecallHit::ShortTerm { .. } => "short_term",
            RecallHit::Rag { .. } => "rag",
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            RecallHit::LongTerm { score, .. }
            | RecallHit::ShortTerm { score, .. }
            | RecallHit::Rag { score, .. } => *score,
        }
    }

    pub fn item_id(&self) -> i64 {
        match self {
            RecallHit::LongTerm { crystal, .. } => crystal.id,
            RecallHit::ShortTerm { memory, .. } => memory.id,
            RecallHit::Rag { chunk, .. } => chunk.id,
        }
    }

    fn is_protected_active_rule(&self) -> bool {
        matches!(
            self,
            RecallHit::LongTerm {
                crystal,
                ..
            } if crystal.crystal_type == "rule" && crystal.status == "active"
        )
    }
}

fn sort_key(hit: &RecallHit) -> SortKey {
    // Higher score first; long-term before short-term before rag on ties; then id.
    let source_preference = match hit.source() {
        "long_term" => 0,
        "short_term" => 1,
        _ => 2,
    };
    SortKey(-hit.score(), source_preference, hit.item_id())
}

#[derive(PartialEq, PartialOrd)]
struct SortKey(f64, usize, i64);

impl SortKey {
    fn cmp(&self, other: &SortKey) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Recall service: bounded multi-lane retrieval combining the long-term FTS
/// lane, the session short-term lane, and the RAG lane behind the Python
/// `_merge_ranked_items` semantics.
///
/// Semantic discipline (task C5, review finding A5). Both working memory and
/// semantic RAG are mandatory, so an absent semantic half is never a silent
/// "supported degraded mode" — it is a reported limitation of the response:
///
/// - [`Self::recall`] always serves what it has (memory rows and the
///   deterministic term contract keep working) and adds
///   [`WARNING_SEMANTIC_UNAVAILABLE`] whenever required semantics did not run
///   over the current corpus: an unarmed lane, an armed lane that degraded
///   (no active generation, identity mismatch, index loss), or an attached
///   semantic service reporting anything but ready;
/// - [`Self::search_series`] — the session-less chunk search behind the public
///   `hieronymus_rag_search` tool — has no other lane to fall back on, so the
///   same condition is an error rather than a lexical-only answer. Its one
///   honest empty case is a series with no indexed chunks at all.
pub struct RecallService {
    config: HieronymusConfig,
    semantic_lane: Option<SemanticLane>,
    /// The shared availability probe, when the owner of the semantic service
    /// installed one. Independent of `semantic_lane`: the lane is what this
    /// process can execute, this is what the service says it can serve.
    semantic_status: Option<SemanticStatusHook>,
}

impl RecallService {
    pub fn open(config: &HieronymusConfig) -> Result<Self, RecallError> {
        open_migrated(Path::new(&config.database_path()))?;
        Ok(Self {
            config: config.clone(),
            semantic_lane: None,
            semantic_status: None,
        })
    }

    /// Arms the query-time semantic lane (design §Search And Fusion): the
    /// provided embedding provider must match the active generation's model
    /// identity or the lane degrades on every recall.
    pub fn with_semantic_lane(mut self, lane: SemanticLane) -> Self {
        self.semantic_lane = Some(lane);
        self
    }

    /// Installs the shared semantic-availability probe (task C5). Builder
    /// form of [`Self::set_semantic_status`].
    pub fn with_semantic_status(mut self, hook: SemanticStatusHook) -> Self {
        self.set_semantic_status(hook);
        self
    }

    /// Installs (or replaces) the shared semantic-availability probe. Taken
    /// by `&mut self` because the owner keeps one long-lived service behind a
    /// lock and re-arms its lane in place.
    pub fn set_semantic_status(&mut self, hook: SemanticStatusHook) {
        self.semantic_status = Some(hook);
    }

    /// The service owner's current verdict, or `None` when no semantic
    /// service is attached to this recall service.
    pub fn semantic_availability(&self) -> Option<SemanticAvailability> {
        self.semantic_status.as_ref().map(|hook| hook())
    }

    /// Standalone hybrid chunk search over one series: the FTS chunk lane
    /// fused with the semantic chunk lane by reciprocal rank, with no
    /// session, no short-term or long-term lane, no term contract, and no
    /// recall ledger. This is what the public `hieronymus_rag_search` tool
    /// serves (task C5); before it existed that tool queried
    /// `RagStore::search` directly and answered lexical-only rows with no way
    /// for a caller to tell that semantics never ran.
    ///
    /// Strictness is the point. The required semantic half either runs, or
    /// this fails:
    ///
    /// - the lane ran: the fused hybrid rows come back, semantic-only rows
    ///   marked by [`crate::semantic_recall::SEMANTIC_MATCH_REASON`];
    /// - the lane did not run and the series owns NO chunks: an empty result
    ///   is complete and true (the ready-for-ingest case — nothing indexed is
    ///   being withheld);
    /// - the lane did not run and the series owns chunks:
    ///   [`RecallError::SemanticUnavailable`], carrying the lane's own reason.
    ///
    /// Whether the *service* is allowed to be queried at all is a separate,
    /// earlier gate the caller applies over [`Self::semantic_availability`]
    /// (`hiero`'s `Application::search_rag`): this method is about execution.
    pub fn search_series(
        &self,
        series_slug: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RagSearchHit>, RecallError> {
        let context = TranslationContext::new(series_slug, "", "", "translation");
        Ok(self
            .search_series_context(&context, query, limit)?
            .value
            .hits)
    }
    pub fn search_series_context(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<crate::coherent_reads::Observed<RagSearchResponse>, RecallError> {
        self.search_series_context_required(context, query, limit, None)
    }

    pub fn search_series_context_required(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
        required_decision_id: Option<&str>,
    ) -> Result<crate::coherent_reads::Observed<RagSearchResponse>, RecallError> {
        let observed = crate::coherent_reads::stable_read(
            &self.config,
            &context.series_slug,
            required_decision_id,
            |db| self.search_series_with_connection(db, context, query, limit),
        )?;
        let (hits, mut warnings) = observed.value;
        if let Some(lane) = &self.semantic_lane {
            lane.publish_repairs(&self.config, &mut warnings);
        }
        if let Some(warning) = warnings.first() {
            return Err(RecallError::SemanticUnavailable(warning.reason.clone()));
        }
        Ok(crate::coherent_reads::Observed {
            resulting_revision: observed.resulting_revision,
            value: RagSearchResponse {
                candidate_exhausted: hits
                    .iter()
                    .filter(|hit| {
                        matches!(
                            hit.chunk.claim_annotation.disposition,
                            crate::claim_reads::ClaimDisposition::Current
                                | crate::claim_reads::ClaimDisposition::Qualified(_)
                        )
                    })
                    .count()
                    < limit.min(crate::rag::MAX_RAG_SEARCH_LIMIT),
                hits,
            },
        })
    }
    fn search_series_with_connection(
        &self,
        connection: &Connection,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<(Vec<RagSearchHit>, Vec<RecallWarning>), RecallError> {
        if limit == 0 {
            return Err(RecallError::LimitTooSmall);
        }
        let store = RagStore::for_read(&self.config);
        let series_slug = &context.series_slug;
        let story_query =
            crate::story_applicability::StoryApplicability::resolve_context(connection, context)
                .map_err(|_| crate::authority_models::DecisionErrorV1::ApplicabilityConflict)?;
        // The sessionless lexical lane uses the same resolved story
        // applicability and bounded candidate filtering as mixed recall.
        let fts_hits = store.search_with_connection(
            connection,
            Some(&story_query),
            series_slug,
            query,
            limit,
            &[],
            &[],
            &[],
        )?;
        let Some(lane) = &self.semantic_lane else {
            return empty_corpus_or_refuse(
                &store,
                connection,
                series_slug,
                NO_SEMANTIC_LANE_REASON,
            )
            .map(|hits| (hits, vec![]));
        };
        // Carry the supplied story context into semantic hydration too;
        // the ANN query itself is always bounded to this series.
        let run = lane.run_with_connection(connection, &self.config, context, query, limit);
        if run.degraded {
            let reason = run
                .warnings
                .iter()
                .find(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
                .map(|warning| warning.reason.clone())
                .unwrap_or_else(|| "the semantic lane could not run".to_string());
            return empty_corpus_or_refuse(&store, connection, series_slug, &reason)
                .map(|hits| (hits, vec![]));
        }
        // The strict API rejects semantic repair warnings rather than
        // presenting an incomplete semantic execution as success.
        let mut hits = fuse_chunk_lanes(fts_hits, run.records);
        let (mut records, mut non_current): (Vec<_>, Vec<_>) =
            rehydrate_hits(connection, advisory_hits(hits, &[]), &story_query)?
                .into_iter()
                .partition(is_current_hit);
        records.truncate(limit.min(crate::rag::MAX_RAG_SEARCH_LIMIT));
        non_current.truncate(limit.min(crate::rag::MAX_RAG_SEARCH_LIMIT));
        records.extend(non_current);
        hits = records
            .into_iter()
            .filter_map(|h| match h {
                RecallHit::Rag {
                    chunk,
                    score,
                    reason,
                    ..
                } => Some(RagSearchHit {
                    chunk,
                    score,
                    reason,
                }),
                _ => None,
            })
            .collect();
        Ok((hits, run.warnings))
    }

    pub fn recall(
        &self,
        session_id: i64,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<RecallResponse, RecallError> {
        self.recall_required(session_id, context, query, limit, None)
    }

    pub fn recall_required(
        &self,
        session_id: i64,
        context: &TranslationContext,
        query: &str,
        limit: usize,
        required_decision_id: Option<&str>,
    ) -> Result<RecallResponse, RecallError> {
        let observed = crate::coherent_reads::stable_read_with_publish(
            &self.config,
            &context.series_slug,
            required_decision_id,
            |db| self.recall_with_connection(db, Some(session_id), context, query, limit),
            |observed| {
                let response = &mut observed.value;
                response.resulting_revision = observed.resulting_revision;
                match record_recall_ledger(
                    &self.config,
                    session_id,
                    context,
                    query,
                    &response.recall_id,
                    observed.resulting_revision,
                    &mut response.hits,
                ) {
                    Ok(()) => Ok(true),
                    Err(RecallError::Coherent(
                        crate::coherent_reads::CoherentReadError::StaleContext,
                    )) => Ok(false),
                    Err(error) => Err(error),
                }
            },
        )?;
        let mut response = observed.value;
        if let Some(lane) = &self.semantic_lane {
            lane.publish_repairs(&self.config, &mut response.warnings);
        }
        Ok(response)
    }

    /// Coherent retrieval for a context with no active session. No recall ledger
    /// is written, and the same claim filters apply to every candidate source.
    pub fn recall_context(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<RecallResponse, RecallError> {
        self.recall_context_required(context, query, limit, None)
    }

    pub fn recall_context_required(
        &self,
        context: &TranslationContext,
        query: &str,
        limit: usize,
        required_decision_id: Option<&str>,
    ) -> Result<RecallResponse, RecallError> {
        let observed = crate::coherent_reads::stable_read(
            &self.config,
            &context.series_slug,
            required_decision_id,
            |db| self.recall_with_connection(db, None, context, query, limit),
        )?;
        let mut response = observed.value;
        response.resulting_revision = observed.resulting_revision;
        if let Some(lane) = &self.semantic_lane {
            lane.publish_repairs(&self.config, &mut response.warnings);
        }
        Ok(response)
    }

    fn recall_with_connection(
        &self,
        connection: &Connection,
        session_id: Option<i64>,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<RecallResponse, RecallError> {
        if limit == 0 {
            return Err(RecallError::LimitTooSmall);
        }
        let workspace = WorkspaceStore::for_read(&self.config);
        if let Some(id) = session_id {
            require_active_session(&workspace, connection, id)?;
        }
        let story_query =
            crate::story_applicability::StoryApplicability::resolve_context(connection, context)
                .map_err(|_| crate::authority_models::DecisionErrorV1::ApplicabilityConflict)?;

        // Short-term lane: base score decays by one rank step per position.
        let short_term = if let Some(id) = session_id {
            workspace.search_short_term_memories_with_connection(
                connection,
                Some(&story_query),
                id,
                query,
                limit,
            )?
        } else {
            workspace.search_short_term_memories_for_context_with_connection(
                connection,
                Some(&story_query),
                context,
                query,
                limit,
            )?
        };
        let short_term_hits: Vec<RecallHit> = short_term
            .into_iter()
            .enumerate()
            .map(|(position, memory)| RecallHit::ShortTerm {
                memory,
                score: SHORT_TERM_BASE_SCORE - (position as f64 * SHORT_TERM_RANK_STEP),
            })
            .collect();

        // Long-term lane: weighted FTS score plus context boosts.
        let crystals = CrystalStore::for_read(&self.config);
        let candidate_limit = limit * 2;
        let context_story_scopes: std::collections::HashSet<&str> = context
            .story_scopes
            .iter()
            .chain(context.tags.iter())
            .map(String::as_str)
            .collect();
        let context_semantic_tags: std::collections::HashSet<&str> = context
            .semantic_tags
            .iter()
            .chain(context.tags.iter())
            .map(String::as_str)
            .collect();

        let scored: Vec<(CrystalRecord, f64)> = crystals.search_scored_with_connection(
            connection,
            Some(&story_query),
            context,
            query,
            candidate_limit,
        )?;
        // Metadata-only candidates: crystals with no FTS hit but matching
        // story scopes/semantic tags still enter the pool at the metadata
        // base score. Resolved before ranking so the concept boosts below
        // cover the merged candidate pool, as in the Python call site.
        let metadata_candidates = crystals_matching_metadata(
            &self.config,
            connection,
            &story_query,
            context,
            &context_story_scopes,
            &context_semantic_tags,
        )?;

        // Concept recall boosts (Python `recall_boosts_for_crystals`):
        // query matches against the concepts linked to the candidates.
        let mut candidate_ids: Vec<i64> = scored.iter().map(|(crystal, _)| crystal.id).collect();
        candidate_ids.extend(metadata_candidates.iter().map(|crystal| crystal.id));
        let context_story_scope_values: Vec<String> = context
            .story_scopes
            .iter()
            .chain(context.tags.iter())
            .cloned()
            .collect();
        let concept_boosts = ConceptStore::for_read(&self.config)
            .recall_boosts_for_crystals_with_connection(
                connection,
                Some(&story_query),
                &candidate_ids,
                query,
                &context_story_scope_values,
            )?;

        let mut long_term: Vec<RecallHit> = scored
            .into_iter()
            .map(|(crystal, score)| {
                let mut ranked_score = score;
                if crystal
                    .story_scopes
                    .iter()
                    .any(|scope| context_story_scopes.contains(scope.as_str()))
                {
                    ranked_score += STORY_SCOPE_BOOST;
                }
                if crystal
                    .semantic_tags
                    .iter()
                    .any(|tag| context_semantic_tags.contains(tag.as_str()))
                {
                    ranked_score += SEMANTIC_TAG_BOOST;
                }
                ranked_score += concept_boosts.get(&crystal.id).copied().unwrap_or(0.0);
                if crystal.crystal_type == "rule" && crystal.status == "active" {
                    ranked_score += ACTIVE_RULE_BOOST;
                }
                // Graded-memory boosts (July design §Recall-Time Behavior):
                // source-credibility confidence plus the flat advisory
                // rule-intent boost. Prioritized, never a mandatory lane.
                ranked_score += source_credibility_confidence(&crystal.source_credibility);
                if !crystal.rule_intent.trim().is_empty() {
                    ranked_score += RULE_INTENT_BOOST;
                }
                if (crystal.crystal_type == "thought" || crystal.is_inferred)
                    && crystal.confidence < 0.5
                {
                    ranked_score = (ranked_score - LOW_CONFIDENCE_THOUGHT_PENALTY).max(0.0);
                }
                RecallHit::LongTerm {
                    crystal,
                    score: ranked_score,
                    reason: RECALL_REASON.to_string(),
                    activation_id: 0,
                }
            })
            .collect();

        let long_term_ids: std::collections::HashSet<i64> = long_term
            .iter()
            .filter_map(|hit| match hit {
                RecallHit::LongTerm { crystal, .. } => Some(crystal.id),
                _ => None,
            })
            .collect();
        for crystal in metadata_candidates {
            if !long_term_ids.contains(&crystal.id) {
                let concept_boost = concept_boosts.get(&crystal.id).copied().unwrap_or(0.0);
                long_term.push(RecallHit::LongTerm {
                    crystal,
                    score: 0.10 + concept_boost,
                    reason: LONG_TERM_METADATA_REASON.to_string(),
                    activation_id: 0,
                });
            }
        }

        long_term.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));

        // Live spreading activation: crystals above the threshold pull their
        // 1-hop neighbors into the pool at the attenuated score.
        apply_spreading_activation(&crystals, connection, &story_query, &mut long_term)?;

        // One ranked memory pool (long-term + short-term), as in the Python
        // recall which sorts all memory items together before the merge.
        let mut memory: Vec<RecallHit> = long_term;
        memory.extend(short_term_hits);
        memory.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));

        // Deterministic term contract (ADR 0011): computed from the same
        // query/source context BEFORE any lane fusion. Advisory chunks that
        // contradict an active rule's canonical rendering are marked with
        // `conflicts_with_rule_ids`; the contract itself is never removed,
        // ranked, or satisfied by retrieval — validation stays a separate,
        // post-retrieval step over this computation.
        let contract = Termbase::for_read(&self.config, context)
            .contract_with_connection(connection, query)?;

        // RAG lanes: the FTS chunk lane over the context's series with typed
        // metadata boosts, and — when armed — the semantic chunk lane, fused
        // by reciprocal rank over ranks only.
        let rag_store = RagStore::for_read(&self.config);
        let rag_story_scopes = merged_context_values(&context.story_scopes, &context.tags);
        let rag_semantic_tags = merged_context_values(&context.semantic_tags, &context.tags);
        let fts_hits: Vec<RagSearchHit> = rag_store.search_with_connection(
            connection,
            Some(&story_query),
            &context.series_slug,
            query,
            limit,
            &context.language_tags,
            &rag_story_scopes,
            &rag_semantic_tags,
        )?;

        let (rag_hits, mut warnings, lane_executed) = match &self.semantic_lane {
            None => (advisory_hits(fts_hits, &contract), Vec::new(), false),
            Some(lane) => {
                let run = lane.run_with_connection(connection, &self.config, context, query, limit);
                if run.degraded {
                    // Missing semantic state: the FTS results ride along
                    // untouched, exactly as in the unarmed service, with the
                    // structured degraded-mode warning on the response.
                    (advisory_hits(fts_hits, &contract), run.warnings, false)
                } else {
                    let fused = fuse_chunk_lanes(fts_hits, run.records)
                        .into_iter()
                        .map(|hit| RecallHit::Rag {
                            conflicts_with_rule_ids: conflicting_rule_ids(
                                &hit.chunk.text,
                                &contract,
                            ),
                            chunk: hit.chunk,
                            score: hit.score,
                            reason: hit.reason,
                        })
                        .collect();
                    (fused, run.warnings, true)
                }
            }
        };

        // Task C5 (review finding A5): required semantics that did not run is
        // reported, never inferred from the absence of a warning. A degraded
        // lane already pushed its own `semantic_lane_unavailable` above, so
        // the kind is added at most once — the warning list is a set of
        // conditions, not a log.
        if let Some(reason) =
            incomplete_semantic_reason(lane_executed, self.semantic_availability().as_ref())
            && !warnings
                .iter()
                .any(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        {
            warnings.push(RecallWarning {
                kind: WARNING_SEMANTIC_UNAVAILABLE.to_string(),
                reason,
            });
        }

        let (memory, mut non_current): (Vec<_>, Vec<_>) =
            rehydrate_hits(connection, memory, &story_query)?
                .into_iter()
                .partition(is_current_hit);
        let (rag_hits, outside): (Vec<_>, Vec<_>) =
            rehydrate_hits(connection, rag_hits, &story_query)?
                .into_iter()
                .partition(is_current_hit);
        non_current.extend(outside);
        if non_current.iter().any(|hit| {
            matches!(
                hit.claim_annotation().disposition,
                crate::claim_reads::ClaimDisposition::Unknown
            )
        }) {
            warnings.push(RecallWarning { kind: "unknown_story_order".into(), reason: "Some matching source material has unresolved story applicability; its assertion text is absent from current results.".into() });
        }
        non_current.truncate(limit);
        let selected = merge_ranked_items(memory, rag_hits, limit);
        let recall_id = next_recall_id();
        Ok(RecallResponse {
            resulting_revision: 0,
            recall_id,
            deterministic_contract: contract,
            candidate_exhausted: selected.len() < limit,
            hits: selected,
            non_current,
            warnings,
        })
    }
}

/// The one honest empty answer for a strict hybrid search whose semantic half
/// did not run: a series with no authoritative chunks has nothing to
/// retrieve, so "no results" withholds nothing. Any other series would be
/// answered with the lexical lane alone, which is precisely the outcome task
/// C5 exists to make impossible.
fn empty_corpus_or_refuse(
    store: &RagStore,
    connection: &Connection,
    series_slug: &str,
    reason: &str,
) -> Result<Vec<RagSearchHit>, RecallError> {
    if store.series_chunk_count_with_connection(connection, series_slug)? == 0 {
        return Ok(Vec::new());
    }
    Err(RecallError::SemanticUnavailable(reason.to_string()))
}

/// Fused-hit metadata without fusion: the FTS lane's hits with their
/// conflict markers, in lane rank order with lane scores.
fn advisory_hits(hits: Vec<RagSearchHit>, contract: &[ContractTerm]) -> Vec<RecallHit> {
    hits.into_iter()
        .map(|hit| RecallHit::Rag {
            conflicts_with_rule_ids: conflicting_rule_ids(&hit.chunk.text, contract),
            chunk: hit.chunk,
            score: hit.score,
            reason: hit.reason,
        })
        .collect()
}

fn merged_context_values(primary: &[String], extra: &[String]) -> Vec<String> {
    let mut merged = primary.to_vec();
    for value in extra {
        if !merged.contains(value) {
            merged.push(value.clone());
        }
    }
    merged
}

/// Python `_merge_ranked_items`: with no RAG hits the ranked memory list is
/// just truncated; otherwise protected active rules keep their slots, the
/// remaining budget splits evenly between the memory pool (larger half) and
/// the RAG hits, the primaries interleave, and overflow fills by rank.
fn merge_ranked_items(
    mut memory: Vec<RecallHit>,
    rag: Vec<RecallHit>,
    limit: usize,
) -> Vec<RecallHit> {
    if rag.is_empty() {
        memory.truncate(limit);
        return memory;
    }

    let mut protected: Vec<RecallHit> = Vec::new();
    let mut memory_pool: Vec<RecallHit> = Vec::new();
    for item in memory {
        if item.is_protected_active_rule() {
            protected.push(item);
        } else {
            memory_pool.push(item);
        }
    }

    let mut selected: Vec<RecallHit> = protected.into_iter().take(limit).collect();
    if selected.len() >= limit {
        return selected;
    }
    let remaining = limit - selected.len();
    let unsplit_memory_budget = remaining.div_ceil(2);
    let memory_budget = unsplit_memory_budget.min(memory_pool.len());
    let rag_budget = (remaining - unsplit_memory_budget).min(rag.len());

    let mut memory_primary = std::mem::take(&mut memory_pool);
    let memory_overflow = memory_primary.split_off(memory_budget);
    let mut rag_primary = rag;
    let rag_overflow = rag_primary.split_off(rag_budget);
    selected.extend(interleave_ranked_items(memory_primary, rag_primary));

    if selected.len() >= limit {
        return selected;
    }
    let remaining = limit - selected.len();
    let mut overflow = memory_overflow;
    overflow.extend(rag_overflow);
    overflow.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));
    selected.extend(overflow.into_iter().take(remaining));
    selected
}

/// Python `_interleave_ranked_items`: round-robin memory slot, rag slot.
fn interleave_ranked_items(memory: Vec<RecallHit>, rag: Vec<RecallHit>) -> Vec<RecallHit> {
    let mut memory = memory.into_iter();
    let mut rag = rag.into_iter();
    let mut interleaved: Vec<RecallHit> = Vec::new();
    loop {
        let mut pushed = false;
        if let Some(item) = memory.next() {
            interleaved.push(item);
            pushed = true;
        }
        if let Some(item) = rag.next() {
            interleaved.push(item);
            pushed = true;
        }
        if !pushed {
            break;
        }
    }
    interleaved
}

fn require_active_session(
    workspace: &WorkspaceStore,
    connection: &Connection,
    session_id: i64,
) -> Result<(), RecallError> {
    match workspace.get_session_with_connection(connection, session_id) {
        Ok(session) if session.status == "active" => Ok(()),
        Ok(_) => Err(RecallError::InactiveSession),
        Err(crate::workspace::WorkspaceError::UnknownSession(id)) => {
            Err(RecallError::UnknownSession(id))
        }
        Err(error) => Err(error.into()),
    }
}

fn crystals_matching_metadata(
    config: &HieronymusConfig,
    connection: &Connection,
    story_query: &crate::story_applicability::StoryQueryV1,
    _context: &TranslationContext,
    context_story_scopes: &std::collections::HashSet<&str>,
    context_semantic_tags: &std::collections::HashSet<&str>,
) -> Result<Vec<CrystalRecord>, RecallError> {
    if context_story_scopes.is_empty() && context_semantic_tags.is_empty() {
        return Ok(Vec::new());
    }
    let store = CrystalStore::for_read(config);
    let mut matches: std::collections::HashMap<i64, CrystalRecord> =
        std::collections::HashMap::new();
    for scope in context_story_scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("chapter:"))
    {
        // Chapter scopes map onto crystal story scopes verbatim.
        let _ = scope;
    }
    let scopes: Vec<String> = context_story_scopes
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let tags: Vec<String> = context_semantic_tags
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    for crystal in store.list_all_candidates_with_connection(
        connection,
        Some(story_query),
        200,
        Some((&scopes, &tags)),
    )? {
        let scope_hit = crystal
            .story_scopes
            .iter()
            .any(|scope| context_story_scopes.contains(scope.as_str()));
        let tag_hit = crystal
            .semantic_tags
            .iter()
            .any(|tag| context_semantic_tags.contains(tag.as_str()));
        if scope_hit || tag_hit {
            matches.insert(crystal.id, crystal);
        }
    }
    Ok(matches.into_values().collect())
}

/// Fold 1-hop `crystal_links` neighbors of above-threshold crystals into the
/// pool at `source_score * link_weight * SPREADING_ATTENUATION` (July design
/// §Recall-Time Behavior step 4). One bounded query per triggering crystal,
/// one hop only: neighbors added here never trigger further spreading.
fn apply_spreading_activation(
    crystals: &CrystalStore,
    connection: &Connection,
    story_query: &crate::story_applicability::StoryQueryV1,
    long_term: &mut Vec<RecallHit>,
) -> Result<(), RecallError> {
    let triggers: Vec<(i64, f64)> = long_term
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::LongTerm { crystal, score, .. }
                if *score > SPREADING_ACTIVATION_THRESHOLD =>
            {
                Some((crystal.id, *score))
            }
            _ => None,
        })
        .collect();
    let mut eligible_triggers = Vec::new();
    for (id, score) in triggers {
        if matches!(
            crate::claim_reads::rehydrate_claims(
                connection,
                crate::claim_reads::ClaimTarget::Crystal(id),
                story_query
            )?,
            crate::claim_reads::ClaimDisposition::Current
                | crate::claim_reads::ClaimDisposition::Qualified(_)
        ) {
            eligible_triggers.push((id, score));
        }
    }
    let triggers = eligible_triggers;
    if triggers.is_empty() {
        return Ok(());
    }
    let mut in_pool: std::collections::HashSet<i64> = long_term
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::LongTerm { crystal, .. } => Some(crystal.id),
            _ => None,
        })
        .collect();

    let mut statement = connection.prepare(
        "select case when source_crystal_id = ?1 then target_crystal_id
                        else source_crystal_id end as neighbor_id, weight
         from crystal_links
         where (source_crystal_id = ?1 or target_crystal_id = ?1)
         order by weight desc, neighbor_id limit ?2",
    )?;
    for (trigger_id, trigger_score) in triggers {
        let rows = statement.query_map(
            rusqlite::params![trigger_id, crate::claim_reads::CANDIDATE_BUDGET as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )?;
        let rows = rows.collect::<Result<Vec<_>, _>>()?;
        let rows = crate::claim_reads::select_candidates(
            connection,
            rows,
            |(id, _)| crate::claim_reads::ClaimTarget::Crystal(*id),
            Some(story_query),
            SPREADING_NEIGHBOR_LIMIT as usize,
        )?;
        for (neighbor_id, weight) in rows {
            if in_pool.contains(&neighbor_id) {
                continue;
            }
            in_pool.insert(neighbor_id);
            let crystal = crystals.get_with_connection(connection, neighbor_id)?;
            long_term.push(RecallHit::LongTerm {
                crystal,
                score: trigger_score * weight * SPREADING_ATTENUATION,
                reason: SPREADING_ACTIVATION_REASON.to_string(),
                activation_id: 0,
            });
        }
    }
    Ok(())
}

/// The recall ledger (July design §Recall-Time Behavior steps 2-3, under the
/// ADR 0011 feedback contract), applied in one transaction over the returned
/// long-term hits:
///
/// 1. session-scoped working-copy dedup: a returned crystal without a
///    non-archived `short_term_memories` row for this session gets one
///    (text seeded from the crystal); a repeat hit records one
///    `recalled_again` memory event instead of duplicating the copy;
/// 2. one `crystal_activations` row per returned hit, stamped with the
///    invocation's `recall_id`, exposing the activation id on the hit.
fn record_recall_ledger(
    config: &HieronymusConfig,
    session_id: i64,
    context: &TranslationContext,
    query: &str,
    recall_id: &str,
    expected_revision: u64,
    hits: &mut [RecallHit],
) -> Result<(), RecallError> {
    let mut connection = open_migrated(Path::new(&config.database_path()))?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if crate::coherent_reads::revision(&transaction, &context.series_slug)? != expected_revision {
        return Err(crate::coherent_reads::CoherentReadError::StaleContext.into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    for (position, hit) in hits.iter_mut().enumerate() {
        let RecallHit::LongTerm {
            crystal,
            score,
            reason,
            activation_id,
        } = hit
        else {
            continue;
        };
        let (crystal_id, hit_score, hit_reason) = (crystal.id, *score, reason.clone());
        let stored =
            CrystalStore::for_read(config).get_with_connection(&transaction, crystal_id)?;
        let existing_working_copy: Option<i64> = transaction
            .query_row(
                "select id from short_term_memories
                 where session_id = ?1 and source_crystal_id = ?2
                   and archived_at is null
                 limit 1",
                rusqlite::params![session_id, crystal_id],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match existing_working_copy {
            None => {
                transaction.execute(
                    "insert into short_term_memories(
                       session_id, source_role, kind, text, source_credibility,
                       rule_intent, source_crystal_id, created_at
                     )
                     values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        session_id,
                        WORKING_COPY_SOURCE_ROLE,
                        stored.crystal_type,
                        stored.text,
                        stored.source_credibility,
                        stored.rule_intent,
                        crystal_id,
                        now
                    ],
                )?;
                let memory_id = transaction.last_insert_rowid();
                crate::claim_capture::copy_bindings_tx(
                    &transaction,
                    crate::claim_reads::ClaimTarget::Crystal(crystal_id),
                    crate::claim_reads::ClaimTarget::ShortTerm(memory_id),
                )?;
                transaction.execute(
                    "insert into short_term_memories_fts(rowid, text) values (?1, ?2)",
                    rusqlite::params![memory_id, stored.text],
                )?;
            }
            Some(memory_id) => {
                transaction.execute(
                    "insert into memory_events(
                       crystal_id, session_id, event_type, source_role, evidence,
                       strength_delta, confidence_delta, applied, created_at
                     )
                     values (?1, ?2, 'recalled_again', 'system', ?3, ?4, ?5, 0, ?6)",
                    rusqlite::params![
                        crystal_id,
                        session_id,
                        memory_id.to_string(),
                        RECALLED_AGAIN_DELTAS.0,
                        RECALLED_AGAIN_DELTAS.1,
                        now
                    ],
                )?;
            }
        }
        transaction.execute(
            "insert into crystal_activations(
               crystal_id, session_id, recall_query, rank, score, reason,
               recall_id, outcome, cycle_id, created_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, null, null, ?8)",
            rusqlite::params![
                crystal_id,
                session_id,
                query,
                (position + 1) as i64,
                hit_score,
                hit_reason,
                recall_id,
                now
            ],
        )?;
        *activation_id = transaction.last_insert_rowid();
    }
    transaction.commit()?;
    Ok(())
}

fn is_current_hit(hit: &RecallHit) -> bool {
    matches!(
        hit.claim_annotation().disposition,
        crate::claim_reads::ClaimDisposition::Current
            | crate::claim_reads::ClaimDisposition::Qualified(_)
    )
}

fn rehydrate_hits(
    db: &Connection,
    hits: Vec<RecallHit>,
    q: &crate::story_applicability::StoryQueryV1,
) -> Result<Vec<RecallHit>, RecallError> {
    use crate::claim_reads::{ClaimDisposition, ClaimTarget, read_annotation};
    let mut out = vec![];
    for mut hit in hits {
        let target = match &hit {
            RecallHit::LongTerm { crystal, .. } => ClaimTarget::Crystal(crystal.id),
            RecallHit::ShortTerm { memory, .. } => ClaimTarget::ShortTerm(memory.id),
            RecallHit::Rag { chunk, .. } => ClaimTarget::RagChunk(chunk.id),
        };
        let annotation = read_annotation(db, target, q)?;
        let current = matches!(
            annotation.disposition,
            ClaimDisposition::Current | ClaimDisposition::Qualified(_)
        );
        let redacted = !current && !annotation.source_inspection;
        let prefix = match &annotation.disposition {
            ClaimDisposition::Current => None,
            ClaimDisposition::Qualified(values) => {
                Some(format!("Qualified memory ({}): ", values.join("; ")))
            }
            disposition => Some(format!(
                "Source evidence ({disposition:?}; not current truth): "
            )),
        };
        let annotate = |text: &mut String| {
            if redacted {
                text.clear();
            } else if let Some(prefix) = &prefix
                && !text.is_empty()
            {
                *text = format!("{prefix}{text}");
            }
        };
        match &mut hit {
            RecallHit::LongTerm { crystal, .. } => {
                annotate(&mut crystal.text);
                annotate(&mut crystal.title);
                annotate(&mut crystal.rule_intent);
                annotate(&mut crystal.soft_origin);
                for tag in &mut crystal.semantic_tags {
                    annotate(tag);
                }
                if redacted {
                    crystal.rule_intent.clear();
                    crystal.soft_origin.clear();
                    crystal.semantic_tags.clear();
                }
                crystal.claim_annotation = annotation;
            }
            RecallHit::ShortTerm { memory, .. } => {
                annotate(&mut memory.text);
                annotate(&mut memory.rule_intent);
                annotate(&mut memory.soft_origin);
                for value in memory.metadata.values_mut() {
                    annotate_json_strings(value, &annotate);
                }
                for tag in &mut memory.semantic_tags {
                    annotate(tag);
                }
                if redacted {
                    memory.metadata.clear();
                    memory.source_ref.clear();
                    memory.rule_intent.clear();
                    memory.soft_origin.clear();
                    memory.semantic_tags.clear();
                }
                memory.claim_annotation = annotation;
            }
            RecallHit::Rag { chunk, .. } => {
                annotate(&mut chunk.text);
                annotate(&mut chunk.display_text);
                for value in chunk.metadata.values_mut() {
                    annotate_json_strings(value, &annotate);
                }
                for tag in &mut chunk.semantic_tags {
                    annotate(tag);
                }
                if redacted {
                    chunk.metadata.clear();
                    chunk.source_ref.clear();
                    chunk.location.clear();
                    chunk.semantic_tags.clear();
                }
                chunk.claim_annotation = annotation;
            }
        }
        out.push(hit);
    }
    Ok(out)
}

fn annotate_json_strings(value: &mut serde_json::Value, annotate: &impl Fn(&mut String)) {
    match value {
        serde_json::Value::String(text) => annotate(text),
        serde_json::Value::Array(values) => {
            for value in values {
                annotate_json_strings(value, annotate);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                annotate_json_strings(value, annotate);
            }
        }
        _ => {}
    }
}

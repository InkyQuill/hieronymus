use std::path::Path;
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
use crate::semantic_recall::{SEMANTIC_MATCH_REASON, SemanticLane, conflicting_rule_ids, rrf_fuse};
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

/// The semantic lane could not run; the response carries FTS results only.
pub const WARNING_SEMANTIC_UNAVAILABLE: &str = "semantic_lane_unavailable";
/// Corrupt semantic hits were excluded and a rebuild was scheduled (or one is
/// already in progress).
pub const WARNING_REPAIR_SCHEDULED: &str = "semantic_repair_scheduled";
/// Corrupt semantic hits were excluded but scheduling the rebuild failed.
pub const WARNING_REPAIR_FAILED: &str = "semantic_repair_failed";

/// One recall invocation: its durable `recall_id`, the deterministic term
/// contract computed from the query/source context BEFORE any lane fusion
/// (ADR 0011: returned separately, never recomputed from selected hits and
/// never mixed into the ordering), the ranked hits whose long-term activation
/// ids feed the feedback contract, and the structured degraded-mode/repair
/// `warnings` (an empty list means every lane ran clean).
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResponse {
    pub recall_id: String,
    pub deterministic_contract: Vec<ContractTerm>,
    pub hits: Vec<RecallHit>,
    pub warnings: Vec<RecallWarning>,
}

impl RecallHit {
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
/// `_merge_ranked_items` semantics. An unarmed semantic lane is the supported
/// degraded mode (no warning: the lane is simply absent); an armed lane that
/// cannot run (no active generation, identity mismatch, index loss) degrades
/// with a structured warning and FTS-only results.
pub struct RecallService {
    config: HieronymusConfig,
    semantic_lane: Option<SemanticLane>,
}

impl RecallService {
    pub fn open(config: &HieronymusConfig) -> Result<Self, RecallError> {
        open_migrated(Path::new(&config.database_path()))?;
        Ok(Self {
            config: config.clone(),
            semantic_lane: None,
        })
    }

    /// Arms the query-time semantic lane (design §Search And Fusion): the
    /// provided embedding provider must match the active generation's model
    /// identity or the lane degrades on every recall.
    pub fn with_semantic_lane(mut self, lane: SemanticLane) -> Self {
        self.semantic_lane = Some(lane);
        self
    }

    pub fn recall(
        &self,
        session_id: i64,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<RecallResponse, RecallError> {
        if limit == 0 {
            return Err(RecallError::LimitTooSmall);
        }
        let workspace = WorkspaceStore::open(&self.config)?;
        require_active_session(&workspace, session_id)?;

        // Short-term lane: base score decays by one rank step per position.
        let short_term_hits: Vec<RecallHit> = workspace
            .search_short_term_memories(session_id, query, limit)?
            .into_iter()
            .enumerate()
            .map(|(position, memory)| RecallHit::ShortTerm {
                memory,
                score: SHORT_TERM_BASE_SCORE - (position as f64 * SHORT_TERM_RANK_STEP),
            })
            .collect();

        // Long-term lane: weighted FTS score plus context boosts.
        let crystals = CrystalStore::open(&self.config)?;
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

        let scored: Vec<(CrystalRecord, f64)> =
            crystals.search_scored(context, query, candidate_limit)?;
        // Metadata-only candidates: crystals with no FTS hit but matching
        // story scopes/semantic tags still enter the pool at the metadata
        // base score. Resolved before ranking so the concept boosts below
        // cover the merged candidate pool, as in the Python call site.
        let metadata_candidates = crystals_matching_metadata(
            &self.config,
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
        let concept_boosts = ConceptStore::open(&self.config)?.recall_boosts_for_crystals(
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
        apply_spreading_activation(&crystals, &mut long_term)?;

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
        let contract = Termbase::open(&self.config, context)?.contract(query)?;

        // RAG lanes: the FTS chunk lane over the context's series with typed
        // metadata boosts, and — when armed — the semantic chunk lane, fused
        // by reciprocal rank over ranks only.
        let rag_store = RagStore::open(&self.config)?;
        let rag_story_scopes = merged_context_values(&context.story_scopes, &context.tags);
        let rag_semantic_tags = merged_context_values(&context.semantic_tags, &context.tags);
        let fts_hits: Vec<RagSearchHit> = rag_store.search(
            &context.series_slug,
            query,
            limit,
            &context.language_tags,
            &rag_story_scopes,
            &rag_semantic_tags,
        )?;

        let (rag_hits, lane_warnings) = match &self.semantic_lane {
            None => (advisory_hits(fts_hits, &contract), Vec::new()),
            Some(lane) => {
                let run = lane.run(&self.config, context, query, limit);
                if run.degraded {
                    // Missing semantic state: the FTS results ride along
                    // untouched, exactly as in the unarmed service, with the
                    // structured degraded-mode warning on the response.
                    (advisory_hits(fts_hits, &contract), run.warnings)
                } else {
                    let fts_ranked: Vec<i64> = fts_hits.iter().map(|hit| hit.chunk.id).collect();
                    let semantic_ranked: Vec<i64> =
                        run.records.iter().map(|record| record.id).collect();
                    // The FTS hit is the preferred carrier (it holds the
                    // boost context); semantic-only chunks carry the semantic
                    // reason.
                    let mut carriers: std::collections::HashMap<i64, RagSearchHit> = fts_hits
                        .into_iter()
                        .map(|hit| (hit.chunk.id, hit))
                        .collect();
                    for record in run.records {
                        carriers.entry(record.id).or_insert(RagSearchHit {
                            chunk: record,
                            score: 0.0,
                            reason: SEMANTIC_MATCH_REASON.to_string(),
                        });
                    }
                    let fused = rrf_fuse(&fts_ranked, &semantic_ranked)
                        .into_iter()
                        .map(|(chunk_id, score)| {
                            let hit = carriers
                                .remove(&chunk_id)
                                .expect("fused ids always have a lane carrier");
                            RecallHit::Rag {
                                conflicts_with_rule_ids: conflicting_rule_ids(
                                    &hit.chunk.text,
                                    &contract,
                                ),
                                chunk: hit.chunk,
                                score,
                                reason: hit.reason,
                            }
                        })
                        .collect();
                    (fused, run.warnings)
                }
            }
        };

        let mut selected = merge_ranked_items(memory, rag_hits, limit);

        let recall_id = next_recall_id();
        record_recall_ledger(&self.config, session_id, query, &recall_id, &mut selected)?;
        Ok(RecallResponse {
            recall_id,
            deterministic_contract: contract,
            hits: selected,
            warnings: lane_warnings,
        })
    }
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

fn require_active_session(workspace: &WorkspaceStore, session_id: i64) -> Result<(), RecallError> {
    match workspace.get_session(session_id) {
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
    _context: &TranslationContext,
    context_story_scopes: &std::collections::HashSet<&str>,
    context_semantic_tags: &std::collections::HashSet<&str>,
) -> Result<Vec<CrystalRecord>, RecallError> {
    if context_story_scopes.is_empty() && context_semantic_tags.is_empty() {
        return Ok(Vec::new());
    }
    let store = CrystalStore::open(config)?;
    let mut matches: std::collections::HashMap<i64, CrystalRecord> =
        std::collections::HashMap::new();
    for scope in context_story_scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("chapter:"))
    {
        // Chapter scopes map onto crystal story scopes verbatim.
        let _ = scope;
    }
    for crystal in store.list_all_candidates(200)? {
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

    let connection = open_migrated(&crystals.config().database_path())?;
    let mut statement = connection.prepare(
        "select case when source_crystal_id = ?1 then target_crystal_id
                        else source_crystal_id end as neighbor_id, weight
         from crystal_links
         where source_crystal_id = ?1 or target_crystal_id = ?1
         limit ?2",
    )?;
    for (trigger_id, trigger_score) in triggers {
        let rows = statement.query_map(
            rusqlite::params![trigger_id, SPREADING_NEIGHBOR_LIMIT],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )?;
        for row in rows {
            let (neighbor_id, weight) = row?;
            if in_pool.contains(&neighbor_id) {
                continue;
            }
            in_pool.insert(neighbor_id);
            let crystal = crystals.get(neighbor_id)?;
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
    query: &str,
    recall_id: &str,
    hits: &mut [RecallHit],
) -> Result<(), RecallError> {
    let mut connection = open_migrated(Path::new(&config.database_path()))?;
    let transaction = connection.transaction()?;
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
                        crystal.crystal_type,
                        crystal.text,
                        crystal.source_credibility,
                        crystal.rule_intent,
                        crystal_id,
                        now
                    ],
                )?;
                let memory_id = transaction.last_insert_rowid();
                transaction.execute(
                    "insert into short_term_memories_fts(rowid, text) values (?1, ?2)",
                    rusqlite::params![memory_id, crystal.text],
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

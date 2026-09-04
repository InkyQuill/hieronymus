use std::path::Path;

use crate::crystals::CrystalStore;
use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::memory_models::{CrystalRecord, ShortTermMemoryRecord, TranslationContext};
use crate::rag::RagStore;
use crate::rag_models::RagChunkRecord;
use crate::workspace::WorkspaceStore;

const SHORT_TERM_BASE_SCORE: f64 = 0.30;
const SHORT_TERM_RANK_STEP: f64 = 0.01;
const STORY_SCOPE_BOOST: f64 = 0.25;
const SEMANTIC_TAG_BOOST: f64 = 0.18;
const ACTIVE_RULE_BOOST: f64 = 0.20;
const LOW_CONFIDENCE_THOUGHT_PENALTY: f64 = 0.12;
const RECALL_REASON: &str = "fts";
const LONG_TERM_METADATA_REASON: &str = "metadata";

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
    Rag(#[from] crate::rag::RagError),
}

/// One ranked recall hit (ADR 0011: flat graded list; the deterministic
/// contract is returned separately and never mixed into this ordering).
#[derive(Debug, Clone, PartialEq)]
pub enum RecallHit {
    LongTerm {
        crystal: CrystalRecord,
        score: f64,
        reason: String,
    },
    ShortTerm {
        memory: ShortTermMemoryRecord,
        score: f64,
    },
    Rag {
        chunk: RagChunkRecord,
        score: f64,
        reason: String,
    },
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
/// `_merge_ranked_items` semantics (an absent semantic lane is the supported
/// degraded mode).
pub struct RecallService {
    config: HieronymusConfig,
}

impl RecallService {
    pub fn open(config: &HieronymusConfig) -> Result<Self, RecallError> {
        open_migrated(Path::new(&config.database_path()))?;
        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn recall(
        &self,
        session_id: i64,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallHit>, RecallError> {
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

        let mut long_term: Vec<RecallHit> = crystals
            .search_scored(context, query, candidate_limit)?
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
                if crystal.crystal_type == "rule" && crystal.status == "active" {
                    ranked_score += ACTIVE_RULE_BOOST;
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
                }
            })
            .collect();

        // Metadata-only candidates: crystals with no FTS hit but matching
        // story scopes/semantic tags still enter the pool at the metadata
        // base score.
        let long_term_ids: std::collections::HashSet<i64> = long_term
            .iter()
            .filter_map(|hit| match hit {
                RecallHit::LongTerm { crystal, .. } => Some(crystal.id),
                _ => None,
            })
            .collect();
        for crystal in crystals_matching_metadata(
            &self.config,
            context,
            &context_story_scopes,
            &context_semantic_tags,
        )? {
            if !long_term_ids.contains(&crystal.id) {
                long_term.push(RecallHit::LongTerm {
                    crystal,
                    score: 0.10,
                    reason: LONG_TERM_METADATA_REASON.to_string(),
                });
            }
        }

        long_term.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));

        // One ranked memory pool (long-term + short-term), as in the Python
        // recall which sorts all memory items together before the merge.
        let mut memory: Vec<RecallHit> = long_term;
        memory.extend(short_term_hits);
        memory.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));

        // RAG lane: FTS search over the context's series with typed metadata
        // boosts from the context's language tags, story scopes, and tags.
        let rag_store = RagStore::open(&self.config)?;
        let rag_story_scopes = merged_context_values(&context.story_scopes, &context.tags);
        let rag_semantic_tags = merged_context_values(&context.semantic_tags, &context.tags);
        let rag_hits: Vec<RecallHit> = rag_store
            .search(
                &context.series_slug,
                query,
                limit,
                &context.language_tags,
                &rag_story_scopes,
                &rag_semantic_tags,
            )?
            .into_iter()
            .map(|hit| RecallHit::Rag {
                chunk: hit.chunk,
                score: hit.score,
                reason: hit.reason,
            })
            .collect();

        let selected = merge_ranked_items(memory, rag_hits, limit);

        record_activations(&self.config, session_id, query, &selected)?;
        Ok(selected)
    }
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

fn record_activations(
    config: &HieronymusConfig,
    session_id: i64,
    query: &str,
    hits: &[RecallHit],
) -> Result<(), RecallError> {
    let connection = open_migrated(Path::new(&config.database_path()))?;
    let now = chrono::Utc::now().to_rfc3339();
    for (position, hit) in hits.iter().enumerate() {
        if let RecallHit::LongTerm {
            crystal,
            score,
            reason,
        } = hit
        {
            connection.execute(
                "insert into crystal_activations(
                   crystal_id, session_id, recall_query, rank, score, reason,
                   cycle_id, created_at
                 )
                 values (?1, ?2, ?3, ?4, ?5, ?6, null, ?7)",
                rusqlite::params![
                    crystal.id,
                    session_id,
                    query,
                    (position + 1) as i64,
                    score,
                    reason,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

use std::path::Path;

use crate::crystals::CrystalStore;
use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::memory_models::{CrystalRecord, ShortTermMemoryRecord, TranslationContext};
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
}

impl RecallHit {
    pub fn source(&self) -> &'static str {
        match self {
            RecallHit::LongTerm { .. } => "long_term",
            RecallHit::ShortTerm { .. } => "short_term",
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            RecallHit::LongTerm { score, .. } | RecallHit::ShortTerm { score, .. } => *score,
        }
    }

    pub fn item_id(&self) -> i64 {
        match self {
            RecallHit::LongTerm { crystal, .. } => crystal.id,
            RecallHit::ShortTerm { memory, .. } => memory.id,
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
    // Higher score first; long-term before short-term on ties; then id.
    let source_preference = match hit.source() {
        "long_term" => 0,
        _ => 1,
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

/// Recall service: bounded multi-lane retrieval with the FTS lane only for
/// now (RAG lane joins behind the same merge once the RAG slice lands; an
/// absent semantic lane is the supported degraded mode).
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

        // Merge: protected active rules survive first, then the ranked pool,
        // bounded by the limit (the RAG lane joins the pool in its slice).
        let mut protected: Vec<RecallHit> = Vec::new();
        let mut pool: Vec<RecallHit> = Vec::new();
        for hit in long_term {
            if hit.is_protected_active_rule() {
                protected.push(hit);
            } else {
                pool.push(hit);
            }
        }
        for hit in &short_term_hits {
            if !hit.is_protected_active_rule() {
                pool.push(hit.clone());
            }
        }
        pool.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));

        let mut selected: Vec<RecallHit> = protected.into_iter().take(limit).collect();
        let remaining = limit - selected.len();
        selected.extend(pool.into_iter().take(remaining));

        record_activations(&self.config, session_id, query, &selected)?;
        Ok(selected)
    }
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

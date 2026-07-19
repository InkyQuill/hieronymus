use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::domain::{
    Crystal, CrystalStore, MemorySource, RecallResult, ShortTermMemory, TranslationContext,
    WorkspaceStore,
};

use super::Result;

const SHORT_TERM_BASE_SCORE: f64 = 0.30;
const SHORT_TERM_RANK_STEP: f64 = 0.01;

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub source: MemorySource,
    pub id: i64,
    pub score: f64,
    pub text: String,
    pub reason: String,
    pub metadata: Value,
    pub crystal: Option<Crystal>,
    pub source_crystal_id: Option<i64>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
}

impl Candidate {
    pub fn long_term(crystal: Crystal, score: f64, reason: &str) -> Self {
        let metadata = json!({
            "crystal_type": crystal.crystal_type,
            "title": crystal.title,
            "scope_type": crystal.scope_type,
            "scope_key": crystal.scope_key,
            "series_slug": crystal.series_slug,
            "source_language": crystal.source_language,
            "target_language": crystal.target_language,
            "source_credibility": crystal.source_credibility,
            "rule_intent": crystal.rule_intent,
            "language_tags": crystal.language_tags,
            "story_scopes": crystal.story_scopes,
            "semantic_tags": crystal.semantic_tags,
            "concept_ids": crystal.concept_ids,
        });
        let language_tags = crystal.language_tags.clone();
        let story_scopes = crystal.story_scopes.clone();
        let semantic_tags = crystal.semantic_tags.clone();
        Self {
            source: MemorySource::LongTerm,
            id: crystal.id,
            score,
            text: crystal.text.clone(),
            reason: reason.to_owned(),
            metadata,
            crystal: Some(crystal),
            source_crystal_id: None,
            language_tags,
            story_scopes,
            semantic_tags,
        }
    }

    fn short_term(memory: ShortTermMemory, index: usize) -> Self {
        let language_tags = memory.language_tags.clone();
        let story_scopes = memory.story_scopes.clone();
        let semantic_tags = memory.semantic_tags.clone();
        Self {
            source: MemorySource::ShortTerm,
            id: memory.id,
            score: (SHORT_TERM_BASE_SCORE - index as f64 * SHORT_TERM_RANK_STEP).max(0.0),
            text: memory.text.clone(),
            reason: "active session short-term memory match".to_owned(),
            metadata: Value::Object(memory.metadata.clone()),
            crystal: None,
            source_crystal_id: memory.source_crystal_id,
            language_tags,
            story_scopes,
            semantic_tags,
        }
    }

    pub fn into_result(self, rank: usize) -> RecallResult {
        RecallResult {
            source: self.source,
            rank,
            score: self.score,
            text: self.text,
            reason: self.reason,
            id: self.id,
            metadata: self.metadata,
        }
    }
}

pub(crate) async fn collect_base_candidates(
    pool: &SqlitePool,
    session_id: i64,
    ctx: &TranslationContext,
    query: &str,
    limit: usize,
) -> Result<Vec<Candidate>> {
    let crystal_store = CrystalStore::new(pool);
    let workspace = WorkspaceStore::new(pool);
    let (rule_result, long_result, short_result) = tokio::join!(
        crystal_store.search_rule_intent_scored(ctx, query, limit),
        crystal_store.search_scored(ctx, query, limit),
        workspace.search_short_term(session_id, query, limit),
    );
    let rules = rule_result?;
    let long = long_result?;
    let short = short_result?;

    // The independent rule-intent query prevents the general FTS candidate cap
    // from discarding a rule before its boost is applied. Both lanes are still
    // fused before boost-aware ranking and final truncation; no slot is reserved.
    Ok(rules
        .into_iter()
        .chain(long)
        .map(|(crystal, score)| Candidate::long_term(crystal, score, "weighted search match"))
        .chain(
            short
                .into_iter()
                .enumerate()
                .map(|(index, memory)| Candidate::short_term(memory, index)),
        )
        .collect())
}

pub(crate) fn long_term_candidate_limit(limit: usize) -> usize {
    limit
        .saturating_mul(4)
        .max(limit.saturating_add(10))
        .min(50)
}

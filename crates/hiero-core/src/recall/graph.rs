use std::collections::{BTreeMap, BTreeSet};

use sqlx::SqlitePool;

use crate::domain::{Crystal, CrystalStore, TranslationContext};

use super::{Result, SPREADING_ACTIVATION_THRESHOLD, SPREADING_ATTENUATION, candidates::Candidate};

pub(crate) async fn expand_one_hop(
    pool: &SqlitePool,
    ctx: &TranslationContext,
    candidates: &[Candidate],
    per_source_limit: usize,
) -> Result<Vec<Candidate>> {
    let store = CrystalStore::new(pool);
    let mut expanded = Vec::new();
    for source in deduplicate_graph_sources(candidates)
        .into_iter()
        .filter(|candidate| {
            candidate.crystal.is_some() && candidate.score > SPREADING_ACTIVATION_THRESHOLD
        })
    {
        let source_crystal = source
            .crystal
            .as_ref()
            .expect("graph candidates are filtered to crystals");
        let mut seen = BTreeSet::new();
        for (neighbor, link, weight) in store
            .linked_bounded(source_crystal.id, ctx, per_source_limit)
            .await?
        {
            let neighbor_id = if link.source_crystal_id == source_crystal.id {
                link.target_crystal_id
            } else {
                link.source_crystal_id
            };
            if !seen.insert(neighbor_id) {
                continue;
            }
            if visible_in_context(&neighbor, ctx) {
                expanded.push(Candidate::long_term(
                    neighbor,
                    source.score * weight * SPREADING_ATTENUATION,
                    "linked spreading activation",
                ));
            }
        }
    }
    Ok(expanded)
}

fn deduplicate_graph_sources(candidates: &[Candidate]) -> Vec<&Candidate> {
    let mut by_id = BTreeMap::<i64, &Candidate>::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.source == crate::domain::MemorySource::LongTerm)
    {
        by_id
            .entry(candidate.id)
            .and_modify(|existing| {
                if candidate.score.total_cmp(&existing.score).is_gt() {
                    *existing = candidate;
                }
            })
            .or_insert(candidate);
    }
    by_id.into_values().collect()
}

fn visible_in_context(crystal: &Crystal, ctx: &TranslationContext) -> bool {
    let coherent_scope = (crystal.scope_type == "global"
        && crystal.scope_key.is_empty()
        && crystal.series_slug.is_empty())
        || (crystal.scope_type == "series"
            && crystal.series_slug == ctx.series_slug
            && crystal.scope_key == ctx.scope_key);
    coherent_scope
        && matches!(crystal.status.as_str(), "active" | "candidate")
        && (crystal.source_language.is_empty() || crystal.source_language == ctx.source_language)
        && (crystal.target_language.is_empty() || crystal.target_language == ctx.target_language)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::deduplicate_graph_sources;
    use crate::{domain::MemorySource, recall::candidates::Candidate};

    fn candidate(id: i64, score: f64) -> Candidate {
        Candidate {
            source: MemorySource::LongTerm,
            id,
            score,
            text: String::new(),
            reason: String::new(),
            metadata: json!({}),
            crystal: None,
            source_crystal_id: None,
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
        }
    }

    #[test]
    fn graph_sources_are_deduplicated_by_id_with_best_score_and_stable_order() {
        let candidates = vec![candidate(2, 0.8), candidate(1, 0.7), candidate(2, 0.9)];
        let deduplicated = deduplicate_graph_sources(&candidates);
        assert_eq!(
            deduplicated
                .iter()
                .map(|candidate| (candidate.id, candidate.score))
                .collect::<Vec<_>>(),
            [(1, 0.7), (2, 0.9)]
        );
    }
}

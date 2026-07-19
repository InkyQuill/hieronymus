use std::collections::BTreeSet;

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
    for source in candidates.iter().filter(|candidate| {
        candidate.crystal.is_some() && candidate.score > SPREADING_ACTIVATION_THRESHOLD
    }) {
        let source_crystal = source
            .crystal
            .as_ref()
            .expect("graph candidates are filtered to crystals");
        let mut seen = BTreeSet::new();
        for (neighbor, link, weight) in store
            .linked_bounded(source_crystal.id, per_source_limit)
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

use std::{cmp::Ordering, collections::BTreeMap};

use crate::{
    domain::{MemorySource, TranslationContext},
    values::SOURCE_CREDIBILITY_CONFIDENCE,
};

use super::{RULE_INTENT_BOOST, candidates::Candidate};

const DEFAULT_SOURCE_CREDIBILITY: f64 = 0.35;
const STORY_SCOPE_BOOST: f64 = 0.25;
const SEMANTIC_TAG_BOOST: f64 = 0.18;
const SHORT_TERM_LANGUAGE_TAG_BOOST: f64 = 0.05;
const SHORT_TERM_STORY_SCOPE_BOOST: f64 = 0.10;
const SHORT_TERM_SEMANTIC_TAG_BOOST: f64 = 0.10;

pub(crate) fn score_base_candidates(
    ctx: &TranslationContext,
    candidates: Vec<Candidate>,
) -> Vec<Candidate> {
    candidates
        .into_iter()
        .map(|mut candidate| {
            if let Some(crystal) = candidate.crystal.as_ref() {
                candidate.score += SOURCE_CREDIBILITY_CONFIDENCE
                    .get(crystal.source_credibility.as_str())
                    .copied()
                    .unwrap_or(DEFAULT_SOURCE_CREDIBILITY);
                if !crystal.rule_intent.trim().is_empty() {
                    candidate.score += RULE_INTENT_BOOST;
                }
                if intersects(&crystal.story_scopes, &ctx.story_scopes, &ctx.tags) {
                    candidate.score += STORY_SCOPE_BOOST;
                }
                if intersects(&crystal.semantic_tags, &ctx.semantic_tags, &ctx.tags) {
                    candidate.score += SEMANTIC_TAG_BOOST;
                }
            } else {
                if intersects(&candidate.language_tags, &ctx.language_tags, &ctx.tags) {
                    candidate.score += SHORT_TERM_LANGUAGE_TAG_BOOST;
                }
                if intersects(&candidate.story_scopes, &ctx.story_scopes, &ctx.tags) {
                    candidate.score += SHORT_TERM_STORY_SCOPE_BOOST;
                }
                if intersects(&candidate.semantic_tags, &ctx.semantic_tags, &ctx.tags) {
                    candidate.score += SHORT_TERM_SEMANTIC_TAG_BOOST;
                }
            }
            candidate
        })
        .collect()
}

pub(crate) fn rank_and_limit(
    candidates: impl IntoIterator<Item = Candidate>,
    limit: usize,
) -> Vec<Candidate> {
    let mut long_term = BTreeMap::<i64, Candidate>::new();
    let mut short_term = BTreeMap::<i64, Candidate>::new();
    for candidate in candidates {
        let target = match candidate.source {
            MemorySource::LongTerm => &mut long_term,
            MemorySource::ShortTerm | MemorySource::Rag => &mut short_term,
        };
        target
            .entry(candidate.id)
            .and_modify(|existing| {
                if candidate_order(&candidate, existing) == Ordering::Less {
                    *existing = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    let long_ids = long_term
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut ranked = long_term
        .into_values()
        .chain(short_term.into_values().filter(|candidate| {
            candidate
                .source_crystal_id
                .is_none_or(|crystal_id| !long_ids.contains(&crystal_id))
        }))
        .collect::<Vec<_>>();
    ranked.sort_by(candidate_order);
    ranked.truncate(limit);
    ranked
}

fn candidate_order(left: &Candidate, right: &Candidate) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| source_preference(left.source).cmp(&source_preference(right.source)))
        .then_with(|| left.id.cmp(&right.id))
}

const fn source_preference(source: MemorySource) -> u8 {
    match source {
        MemorySource::LongTerm => 0,
        MemorySource::ShortTerm => 1,
        MemorySource::Rag => 2,
    }
}

fn intersects(values: &[String], primary: &[String], legacy: &[String]) -> bool {
    values
        .iter()
        .any(|value| primary.contains(value) || legacy.contains(value))
}

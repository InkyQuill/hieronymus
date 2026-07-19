use std::{cmp::Ordering, collections::BTreeMap};

/// Fuses rank positions without comparing incomparable lexical and vector scores.
#[must_use]
pub fn reciprocal_rank_fusion(
    fts_results: &[(i64, f64)],
    vector_results: &[(i64, f32)],
    k: f64,
) -> Vec<(i64, f64)> {
    let k = if k.is_finite() && k >= 0.0 { k } else { 60.0 };
    let mut scores = BTreeMap::<i64, f64>::new();
    for (rank, (id, _)) in fts_results.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }
    for (rank, (id, _)) in vector_results.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }
    let mut fused = scores.into_iter().collect::<Vec<_>>();
    fused.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    fused
}

use crate::{db::CrystalRecord, values::SOURCE_CREDIBILITY_CONFIDENCE};

pub const ARCHIVE_STRENGTH_THRESHOLD: f64 = 0.05;
const RULE_INTENT_DECAY_FACTOR: f64 = 0.5;
const DEFAULT_SOURCE_CREDIBILITY: f64 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreDelta {
    pub strength: f64,
    pub confidence: f64,
}

/// Applies the canonical confidence boundary to an untrusted-output penalty.
#[must_use]
pub fn apply_malformed_confidence_penalty(confidence: f64, penalty: f64) -> f64 {
    let confidence = if confidence.is_finite() {
        confidence
    } else {
        0.0
    };
    let penalty = if penalty.is_finite() && penalty > 0.0 {
        penalty
    } else {
        0.0
    };
    (confidence - penalty).clamp(0.0, 1.0)
}

pub static IMMEDIATE_EVENT_DELTAS: phf::Map<&str, (f64, f64)> = phf::phf_map! {
    "confirmed_by_user" => (0.15, 0.20),
    "contradicted_by_user" => (-0.20, -0.25),
    "deleted_by_user" => (-0.50, -0.35),
    "recalled_useful" => (0.06, 0.04),
    "recalled_miss" => (-0.05, -0.03),
};

pub static PASSIVE_EVENT_DELTAS: phf::Map<&str, (f64, f64)> = phf::phf_map! {
    "cited" => (0.03, 0.02),
    "used_in_translation" => (0.05, 0.02),
    "passed_review" => (0.07, 0.05),
    "caused_correction" => (-0.10, -0.12),
    "superseded" => (-0.12, -0.05),
    "recalled_again" => (0.02, 0.0),
};

/// Applies the canonical score boundary.
///
/// Non-finite deltas are ignored instead of allowing invalid floating-point
/// values to reach SQLite. Negative deltas on rule-intent memories are
/// dampened by `1.0 - 0.5 * credibility`: credibility is clamped to
/// `[0, 1]`, so rules retain between half and all of the ordinary decay.
#[must_use]
pub fn apply_score_delta(crystal: &CrystalRecord, delta: ScoreDelta) -> (f64, f64, String) {
    let credibility = SOURCE_CREDIBILITY_CONFIDENCE
        .get(crystal.source_credibility.as_str())
        .copied()
        .unwrap_or(DEFAULT_SOURCE_CREDIBILITY)
        .clamp(0.0, 1.0);
    let dampening = if crystal.rule_intent.trim().is_empty() {
        1.0
    } else {
        1.0 - RULE_INTENT_DECAY_FACTOR * credibility
    };
    let strength_delta = finite_delta(delta.strength, dampening);
    let confidence_delta = finite_delta(delta.confidence, dampening);
    let strength = (crystal.strength + strength_delta).clamp(0.0, 1.0);
    let confidence = (crystal.confidence + confidence_delta).clamp(0.0, 1.0);
    let status = if crystal.status == "archived" || confidence == 0.0 {
        "archived"
    } else {
        crystal.status.as_str()
    };
    (strength, confidence, status.to_owned())
}

fn finite_delta(delta: f64, dampening: f64) -> f64 {
    if !delta.is_finite() {
        0.0
    } else if delta < 0.0 {
        delta * dampening
    } else {
        delta
    }
}

pub(crate) fn event_delta(event_type: &str) -> Option<(ScoreDelta, bool)> {
    IMMEDIATE_EVENT_DELTAS
        .get(event_type)
        .map(|&(strength, confidence)| {
            (
                ScoreDelta {
                    strength,
                    confidence,
                },
                true,
            )
        })
        .or_else(|| {
            PASSIVE_EVENT_DELTAS
                .get(event_type)
                .map(|&(strength, confidence)| {
                    (
                        ScoreDelta {
                            strength,
                            confidence,
                        },
                        false,
                    )
                })
        })
}

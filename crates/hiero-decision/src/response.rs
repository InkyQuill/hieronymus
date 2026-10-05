// Dispatch derived from decision-model-sdk/src/de.rs; probability validation
// and per-answer recovery are Hieronymus additions.
use crate::{Error, MAX_RESPONSE_BYTES, Request};
use serde_json::{Map, Value};
pub(crate) fn decode(request: &Request, body: &[u8]) -> Result<Value, Error> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::InvalidResponse);
    }
    let mut value = crate::json::decode(body)?;
    let answers = value
        .get_mut("answers")
        .ok_or(Error::InvalidResponse)?
        .as_object_mut()
        .ok_or(Error::InvalidResponse)?;
    let questions = request.payload()["questions"]
        .as_object()
        .ok_or(Error::InvalidRequest)?;
    answers.retain(|name, answer| questions.get(name).is_some_and(|q| valid(q, answer)));
    Ok(value)
}
fn probability(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
}
fn valid(question: &Value, answer: &Value) -> bool {
    if answer["type"] != question["type"] {
        return false;
    }
    match question["type"].as_str() {
        Some("noul") => probability(&answer["noul"]).is_some(),
        Some("choice") => {
            let Some(criteria) = question["criteria"].as_object() else {
                return false;
            };
            let Some(distribution) = distribution(answer, criteria.len()) else {
                return false;
            };
            if !criteria.keys().all(|key| distribution.contains_key(key)) {
                return false;
            }
            let Some(chosen) = answer["choice"].as_str() else {
                return false;
            };
            let Some(pick) = distribution.get(chosen).and_then(probability) else {
                return false;
            };
            distribution
                .values()
                .all(|p| probability(p).is_some_and(|n| n <= pick + 0.0001))
        }
        Some("score") => {
            let Some(levels) = question["criteria"].as_array() else {
                return false;
            };
            let Some(distribution) = distribution(answer, levels.len()) else {
                return false;
            };
            let Some(legend) = answer["legend"].as_object() else {
                return false;
            };
            if legend.len() != levels.len() {
                return false;
            }
            let mut weighted = 0.0;
            for (index, level) in levels.iter().enumerate() {
                let key = index.to_string();
                if legend.get(&key) != Some(level) {
                    return false;
                }
                let Some(p) = distribution.get(&key).and_then(probability) else {
                    return false;
                };
                weighted += index as f64 * p;
            }
            answer["score"].as_f64().is_some_and(|score| {
                score.is_finite()
                    && score >= 0.0
                    && score <= (levels.len() - 1) as f64
                    && (score - weighted).abs() <= 0.0001
            })
        }
        _ => false,
    }
}
fn distribution(answer: &Value, count: usize) -> Option<&Map<String, Value>> {
    probability(&answer["confidence"])?;
    let distribution = answer["probabilities"].as_object()?;
    if distribution.len() != count {
        return None;
    }
    let sum = distribution
        .values()
        .try_fold(0.0, |sum, p| Some(sum + probability(p)?))?;
    ((sum - 1.0).abs() <= 0.0001).then_some(distribution)
}

use std::{collections::HashSet, hash::Hash};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("expected a JSON object")]
pub struct JsonObjectError;

#[must_use]
pub fn clamp_score(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[must_use]
pub fn normalize_tuple<T: Eq + Hash + Clone>(items: &[T]) -> Vec<T> {
    let mut seen = HashSet::with_capacity(items.len());
    items
        .iter()
        .filter(|item| seen.insert(*item))
        .cloned()
        .collect()
}

pub fn json_object(value: &Value) -> Result<Map<String, Value>, JsonObjectError> {
    value.as_object().cloned().ok_or(JsonObjectError)
}

#[must_use]
pub fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

pub const SOURCE_CREDIBILITY_CONFIDENCE: phf::Map<&str, f64> = phf::phf_map! {
    "rumor" => 0.15,
    "thought" => 0.2,
    "observation" => 0.35,
    "source_text" => 0.7,
    "user_suggestion" => 0.8,
    "expert" => 0.85,
    "user_rule" => 0.95,
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        SOURCE_CREDIBILITY_CONFIDENCE, clamp_score, json_object, normalize_tuple, utc_now,
    };

    #[test]
    fn clamp_score_limits_values_to_the_unit_interval() {
        let cases = [
            (-1.0, 0.0),
            (0.0, 0.0),
            (0.25, 0.25),
            (1.0, 1.0),
            (2.0, 1.0),
        ];

        for (value, expected) in cases {
            assert_eq!(clamp_score(value), expected);
        }
    }

    #[test]
    fn normalize_tuple_deduplicates_while_preserving_first_seen_order() {
        let items = ["beta", "alpha", "beta", "gamma", "alpha"];

        assert_eq!(normalize_tuple(&items), vec!["beta", "alpha", "gamma"]);
    }

    #[test]
    fn json_object_returns_an_owned_object_map() {
        let value = json!({"name": "Hieronymus"});

        let object = json_object(&value).expect("object values should be accepted");

        assert_eq!(object.get("name"), Some(&json!("Hieronymus")));
    }

    #[test]
    fn json_object_rejects_arrays_and_scalars() {
        for value in [json!([]), json!("scalar"), json!(42), json!(null)] {
            assert_eq!(
                json_object(&value).expect_err("non-objects should be rejected"),
                super::JsonObjectError
            );
        }
    }

    #[test]
    fn utc_now_uses_utc_and_serializes_with_a_z_suffix() {
        let now = utc_now();

        assert_eq!(now.offset(), &chrono::Utc);
        let serialized = serde_json::to_string(&now).expect("UTC timestamps should serialize");
        assert!(serialized.ends_with("Z\""));
    }

    #[test]
    fn source_credibility_confidence_matches_the_canonical_weights() {
        let expected = [
            ("rumor", 0.15),
            ("thought", 0.2),
            ("observation", 0.35),
            ("source_text", 0.7),
            ("user_suggestion", 0.8),
            ("expert", 0.85),
            ("user_rule", 0.95),
        ];

        for (label, weight) in expected {
            assert_eq!(SOURCE_CREDIBILITY_CONFIDENCE.get(label), Some(&weight));
        }
    }
}

// Adapted from decision-model-sdk/src/question.rs::validate and named-question
// preparation at c5d4459. See ../UPSTREAM.md for retained license/provenance.
use crate::Error;
use serde_json::Value;

/// Validated named questions with structured instructions and extension fields.
///
/// Duplicate request names in an already-parsed Value are resolved by the caller.
/// Raw response bytes reject duplicate keys at every depth.
pub struct Request {
    payload: Value,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").finish_non_exhaustive()
    }
}
impl Request {
    /// Validate state, model and question shapes before sending anything.
    pub fn new(payload: &Value) -> Result<Self, Error> {
        let model = payload["model"].as_str().ok_or(Error::InvalidRequest)?;
        if model.trim().is_empty() || !content(&payload["state"]) {
            return Err(Error::InvalidRequest);
        }
        let questions = payload["questions"]
            .as_object()
            .filter(|q| !q.is_empty())
            .ok_or(Error::InvalidRequest)?;
        for (name, question) in questions {
            if name.is_empty() || !question.is_object() || !content(&question["instructions"]) {
                return Err(Error::InvalidRequest);
            }
            let valid = match question["type"].as_str() {
                Some("noul") => question.get("criteria").is_none_or(|criteria| {
                    criteria.as_object().is_some_and(|map| {
                        map.iter().all(|(name, value)| {
                            matches!(name.as_str(), "true" | "false") && content(value)
                        })
                    })
                }),
                Some("choice") => question["criteria"].as_object().is_some_and(|map| {
                    !map.is_empty()
                        && map.iter().all(|(name, value)| {
                            !name.is_empty() && (value.is_null() || content(value))
                        })
                }),
                Some("score") => question["criteria"]
                    .as_array()
                    .is_some_and(|levels| levels.len() >= 2 && levels.iter().all(content)),
                _ => false,
            };
            if !valid {
                return Err(Error::InvalidRequest);
            }
        }
        Ok(Self {
            payload: serde_json::json!({
                "model":model,"state":payload["state"],"questions":questions
            }),
        })
    }
    /// The envelope passed to the application's transport.
    pub fn payload(&self) -> &Value {
        &self.payload
    }
    /// Decode bounded JSON, omitting invalid or unrequested answers by name.
    ///
    /// Valid siblings survive; callers retain fallback and authority rules.
    /// Model/usage metadata is optional for historical Hieronymus fixtures.
    pub fn decode(&self, body: &[u8]) -> Result<Value, Error> {
        crate::response::decode(self, body)
    }
}
fn content(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_))
}

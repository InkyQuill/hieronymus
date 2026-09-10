//! `POST /recall/feedback` — the native (bearer + Host) recall-feedback
//! route (ADR 0011 §Recall Feedback). One route per the controller ruling:
//! the MCP registry stays untouched, and the feedback store API plus the
//! `hiero recall-feedback` CLI share this contract.
//!
//! Request body: `{"recall_id", "useful": [activation ids], "miss":
//! [activation ids], "idempotency_key"}`. The first application answers
//! `200 {"recall_id", "applied": true, "useful", "miss"}`; a replay (same
//! idempotency key or already-scored activations) is an explicit no-op with
//! `applied: false`. Activation ids that do not belong to the `recall_id`
//! fail `400 activation_mismatch`; an unknown recall invocation fails `404
//! unknown_recall_id`.

use serde_json::{Value, json};

use hieronymus::feedback::{FeedbackError, FeedbackStore, RecallFeedback};

use super::super::DaemonRuntime;
use super::super::http::{Request, Response};
use super::{bearer_matches, host_is_valid, invalid_host, not_found, request_body, unauthorized};

pub(super) fn handle(request: &Request, runtime: &DaemonRuntime) -> Response {
    if request.method.as_str() != "POST" {
        return not_found();
    }
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    if !bearer_matches(request, runtime) {
        return unauthorized();
    }
    let Some(body) = request_body(request) else {
        return Response::json(400, &json!({"error": "recall feedback must be an object"}));
    };
    let request = match parse_feedback(&body) {
        Ok(feedback) => feedback,
        Err(message) => return Response::json(400, &json!({"error": message})),
    };
    let store = FeedbackStore::open(&runtime.config);
    let store = match store {
        Ok(store) => store,
        Err(error) => return Response::json(500, &json!({"error": error.to_string()})),
    };
    match store.record_recall_outcome(&request) {
        Ok(outcome) => Response::json(
            200,
            &json!({
                "recall_id": request.recall_id,
                "applied": outcome.applied,
                "useful": outcome.useful_count,
                "miss": outcome.miss_count,
            }),
        ),
        Err(FeedbackError::UnknownRecall(_)) => {
            Response::json(404, &json!({"error": "unknown_recall_id"}))
        }
        Err(FeedbackError::ActivationMismatch { mismatched, .. }) => Response::json(
            400,
            &json!({"error": "activation_mismatch", "activation_ids": mismatched}),
        ),
        Err(error) => Response::json(400, &json!({"error": error.to_string()})),
    }
}

fn parse_feedback(body: &Value) -> Result<RecallFeedback, String> {
    let string_field = |name: &str| -> Result<String, String> {
        body.get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{name} must be a string"))
    };
    let ids = |name: &str| -> Result<Vec<i64>, String> {
        match body.get(name) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_i64()
                        .ok_or_else(|| format!("{name} must contain activation ids"))
                })
                .collect(),
            Some(_) => Err(format!("{name} must be a list of activation ids")),
        }
    };
    Ok(RecallFeedback {
        recall_id: string_field("recall_id")?,
        useful_activation_ids: ids("useful")?,
        missed_activation_ids: ids("miss")?,
        idempotency_key: string_field("idempotency_key")?,
    })
}

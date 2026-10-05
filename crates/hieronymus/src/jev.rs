//! Local System One protocol over the application's bounded provider transport.
use crate::provider_http::{HttpError, ProviderTransport};
use hiero_decision::{MAX_RESPONSE_BYTES, Request};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// Send named questions once; domain consumers retain their authority rules.
pub fn ask(
    key: &str,
    payload: &Value,
    timeout: Duration,
    transport: Arc<dyn ProviderTransport>,
) -> Result<Value, &'static str> {
    if timeout.is_zero() {
        return Err("transport_error");
    }
    if key.trim().is_empty() || !key.bytes().all(|b| (32..=126).contains(&b)) {
        return Err("configuration_error");
    }
    if payload["model"]
        .as_str()
        .is_none_or(|model| model.trim().is_empty())
    {
        return Err("configuration_error");
    }
    let request = Request::new(payload).map_err(|_| "invalid_request")?;
    let headers = vec![("authorization".into(), format!("Bearer {key}"))];
    let response = transport
        .post_json_bounded(
            "https://api.typesafe.ai/v1/systemone",
            &headers,
            request.payload(),
            timeout,
            MAX_RESPONSE_BYTES,
        )
        .map_err(|error| match error {
            HttpError::TooLarge { .. } => "invalid_response",
            _ => "transport_error",
        })?;
    if !(200..300).contains(&response.status) {
        return Err("http_error");
    }
    request
        .decode(response.body.as_bytes())
        .map_err(|_| "invalid_response")
}

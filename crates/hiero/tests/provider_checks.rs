//! Plan W3 / Astra finding 12: provider `check`/`models` never fabricate a
//! result from a hostname. Production always runs the real probe through the
//! injected [`ProviderTransport`]; a `.invalid` host yields a real transport
//! error, not `{"source": "fixture", "models": ["synthetic-model"]}`.
//!
//! The library seam ([`probe_models`]) is exercised directly with a mock
//! transport (success + failure), and the daemon route is exercised
//! end-to-end against a `.invalid` profile over the real blocking transport.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use hieronymus::dream_providers::probe_models;
use hieronymus::provider_config::ProviderProfile;
use hieronymus::provider_http::{HttpError, HttpResponse, ProviderTransport};

mod common;

use common::{browser_headers, same_origin, send_request, start_daemon_with_browser_session};

/// A transport that never touches the network: every call returns the
/// configured canned outcome.
struct MockTransport {
    outcome: Result<HttpResponse, HttpError>,
}

impl MockTransport {
    fn ok(body: Value) -> Self {
        Self {
            outcome: Ok(HttpResponse {
                status: 200,
                body: body.to_string(),
            }),
        }
    }

    fn failing() -> Self {
        Self {
            outcome: Err(HttpError::Network(
                "dns error: failed to lookup address information".to_string(),
            )),
        }
    }

    fn reply(&self) -> Result<HttpResponse, HttpError> {
        match &self.outcome {
            Ok(response) => Ok(HttpResponse {
                status: response.status,
                body: response.body.clone(),
            }),
            Err(HttpError::Network(message)) => Err(HttpError::Network(message.clone())),
            Err(_) => Err(HttpError::Network("mock failure".to_string())),
        }
    }
}

impl ProviderTransport for MockTransport {
    fn post_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _payload: &Value,
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.reply()
    }

    fn get_json(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.reply()
    }
}

fn invalid_profile() -> ProviderProfile {
    ProviderProfile::new(
        "Synthetic",
        "openai",
        "https://provider.invalid/v1",
        "sk-test-key",
        5.0,
    )
}

#[test]
fn a_mock_success_returns_the_real_listing_not_a_fixture() {
    let transport = Arc::new(MockTransport::ok(
        json!({ "data": [{ "id": "model-b" }, { "id": "model-a" }] }),
    ));
    let probe = probe_models(&invalid_profile(), transport);
    assert!(probe.ok);
    assert_eq!(probe.models, vec!["model-a", "model-b"]);
    assert_eq!(probe.source, "api");
    assert_ne!(probe.source, "fixture");
    assert!(!probe.models.iter().any(|model| model == "synthetic-model"));
}

#[test]
fn a_mock_transport_failure_never_yields_synthetic_models() {
    let probe = probe_models(&invalid_profile(), Arc::new(MockTransport::failing()));
    assert!(!probe.ok, "a transport failure is not a success");
    assert_ne!(probe.source, "fixture");
    assert!(
        !probe.models.iter().any(|model| model == "synthetic-model"),
        "the failure path returns default suggestions, never a synthetic model"
    );
    assert_eq!(probe.source, "defaults");
    assert_eq!(probe.error, "model suggestions unavailable");
}

#[test]
fn a_keyless_profile_reports_the_missing_key_not_a_fixture() {
    let profile = ProviderProfile::new(
        "Synthetic",
        "openai",
        "https://provider.invalid/v1",
        "",
        5.0,
    );
    let probe = probe_models(&profile, Arc::new(MockTransport::failing()));
    assert!(!probe.ok);
    assert_ne!(probe.source, "fixture");
    assert_eq!(probe.error, "API key missing for provider profile");
}

#[test]
fn the_daemon_route_probes_a_dot_invalid_host_for_real() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let save = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        json!({
            "provider": {
                "id": "synthetic-provider",
                "name": "Synthetic",
                "type": "openai",
                "url": "https://provider.invalid/v1",
                "key": "sk-test-key",
                "timeout_seconds": "5",
            }
        })
        .to_string()
        .as_bytes(),
    );
    assert_eq!(save.status, 200, "{:?}", save.raw_body);

    let models = send_request(
        fixture.port,
        "GET",
        "/api/providers/synthetic-provider/models",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(models.status, 200, "{:?}", models.raw_body);
    let body: Value = models.body();
    assert_ne!(
        body["source"], "fixture",
        "the production route must not fabricate a fixture result: {body}"
    );
    assert!(
        body["models"]
            .as_array()
            .map(|models| models.iter().all(|model| model != "synthetic-model"))
            .unwrap_or(true),
        "no synthetic model may appear: {body}"
    );

    let check = send_request(
        fixture.port,
        "POST",
        "/api/providers/synthetic-provider/check",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(check.status, 200, "{:?}", check.raw_body);
    let check_body: Value = check.body();
    assert_ne!(check_body["check"]["source"], "fixture", "{check_body}");
    assert_eq!(
        check_body["check"]["ok"], false,
        "a .invalid host can never actually connect: {check_body}"
    );
}

//! Provider routes (`/api/providers*`): browser-authenticated CRUD over the
//! slice-1 typed `provider.conf` catalog plus connection `check` and
//! `models` suggestions through a provider-client seam. The synthetic
//! fixture world (an `.invalid` endpoint that can never resolve) keeps the
//! frozen oracle semantics (`{"ok":true,"models":["synthetic-model"],
//! "source":"fixture"}`); every other configured profile is probed through
//! the real provider client over the blocking transport.

use std::sync::Arc;

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::load_dream_config;
use hieronymus::dream_providers::{ModelProbe, probe_models};
use hieronymus::provider_config::{
    ProviderCatalog, ProviderDefaults, ProviderProfile, load_provider_catalog,
    save_provider_catalog, validate_provider_catalog,
};
use hieronymus::provider_http::{BlockingHttpTransport, ProviderTransport};

use super::super::DaemonRuntime;
use super::super::http::{Request, Response};
use super::{guard_api, request_body};

/// Probe results are the ported `ModelSuggestionResult` shape.
pub(crate) type ProviderProbe = ModelProbe;

/// The trait object stays `Debug`-printable without exposing anything: it
/// carries no secret state.
impl std::fmt::Debug for dyn ProviderClientSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderClientSeam")
    }
}

/// Provider-client seam: how the daemon reaches a configured provider.
pub(crate) trait ProviderClientSeam: Send + Sync {
    fn check(&self, provider_id: &str, profile: &ProviderProfile) -> ProviderProbe;
    fn models(&self, provider_id: &str, profile: &ProviderProfile) -> ProviderProbe;
}

/// The daemon's seam: the synthetic fixture world keeps the frozen oracle
/// semantics; every other configured profile is probed through the real
/// client (`hieronymus::dream_providers::probe_models`) over the blocking
/// transport.
pub(crate) struct DaemonProviderClient {
    transport: Arc<dyn ProviderTransport>,
}

impl DaemonProviderClient {
    pub(crate) fn new(transport: Arc<dyn ProviderTransport>) -> Self {
        Self { transport }
    }

    pub(crate) fn with_default_transport() -> Self {
        Self::new(Arc::new(BlockingHttpTransport::default()))
    }

    fn probe(&self, profile: &ProviderProfile) -> ProviderProbe {
        if is_synthetic_world_endpoint(profile.url()) {
            return fixture_probe();
        }
        probe_models(profile, Arc::clone(&self.transport))
    }
}

impl ProviderClientSeam for DaemonProviderClient {
    fn check(&self, _provider_id: &str, profile: &ProviderProfile) -> ProviderProbe {
        self.probe(profile)
    }

    fn models(&self, _provider_id: &str, profile: &ProviderProfile) -> ProviderProbe {
        self.probe(profile)
    }
}

/// The frozen oracle probe: `source: "fixture"` mirrors the oracle, which
/// ran the routes against a fixture registry instead of the network.
fn fixture_probe() -> ProviderProbe {
    ProviderProbe {
        ok: true,
        models: vec!["synthetic-model".to_string()],
        source: "fixture".to_string(),
        error: String::new(),
    }
}

/// RFC 2606 reserves `.invalid` so those hosts can never resolve. The frozen
/// synthetic world (`https://provider.invalid/v1`) must answer with fixture
/// semantics rather than a guaranteed-failing network attempt.
fn is_synthetic_world_endpoint(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or(url);
    let host = host.split(['/', '?']).next().unwrap_or(host);
    let host = host
        .rsplit_once(':')
        .map_or(host, |(host, _port)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host == "invalid" || host.to_ascii_lowercase().ends_with(".invalid")
}

/// `GET /api/providers` — user-created profiles for the web console.
pub(super) fn list(_request: &Request, runtime: &DaemonRuntime) -> Response {
    let (catalog, load_error) = load_catalog(&runtime.config);
    let providers: Vec<Value> = catalog
        .providers
        .keys()
        .map(|id| editor_payload(&catalog, id))
        .collect();
    Response::json(200, &json!({"providers": providers, "error": load_error}))
}

/// `POST /api/providers` — create or update one profile.
pub(super) fn save(request: &Request, runtime: &DaemonRuntime) -> Response {
    let Some(raw_provider) = request_body(request)
        .and_then(|body| body.get("provider").cloned())
        .filter(Value::is_object)
    else {
        return envelope_400("provider must be an object");
    };
    let Some(provider_id) = raw_provider
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|id| !id.is_empty())
    else {
        return envelope_400("unsupported provider profile");
    };
    let (mut catalog, load_error) = load_catalog(&runtime.config);
    if !load_error.is_empty() {
        return envelope_400(&load_error);
    }
    let profile = match draft_profile(&raw_provider, &catalog, &provider_id) {
        Ok(profile) => profile,
        Err(error) => return envelope_400(&error),
    };
    catalog.providers.insert(provider_id.clone(), profile);
    if let Err(error) = validate_provider_catalog(&catalog) {
        return envelope_400(&error.to_string());
    }
    if let Err(error) = save_provider_catalog(&runtime.config, &catalog) {
        return envelope_400(&error.to_string());
    }
    Response::json(
        200,
        &json!({
            "provider": editor_payload(&catalog, &provider_id),
            "error": "",
        }),
    )
}

/// Dispatch `/api/providers/{id}`, `.../check`, and `.../models` (`suffix` is
/// the path below `/api/providers/`, without a trailing slash).
pub(super) fn subroute(request: &Request, runtime: &DaemonRuntime, path: &str) -> Response {
    let suffix = path.trim_start_matches("/api/providers/");
    let not_found = || Response::json(404, &json!({"error": "not_found"}));
    if let Some((provider_id, action)) = suffix.rsplit_once('/') {
        match (request.method.as_str(), action) {
            ("POST", "check") => guard_api(request, runtime, |_request, runtime| {
                check(runtime, provider_id)
            }),
            ("GET", "models") => guard_api(request, runtime, |_request, runtime| {
                models(runtime, provider_id)
            }),
            _ => not_found(),
        }
    } else {
        match request.method.as_str() {
            "GET" => guard_api(request, runtime, |_request, runtime| {
                detail(runtime, suffix)
            }),
            "DELETE" => guard_api(request, runtime, |_request, runtime| {
                delete(runtime, suffix)
            }),
            _ => not_found(),
        }
    }
}

/// `GET /api/providers/{id}` — one profile for the editor modal; an unknown
/// profile is an envelope error (HTTP 400 like the Python bridge).
fn detail(runtime: &DaemonRuntime, provider_id: &str) -> Response {
    let (catalog, load_error) = load_catalog(&runtime.config);
    if !catalog.providers.contains_key(provider_id) {
        return envelope_400(&format!("provider profile not found: {provider_id}"));
    }
    Response::json(
        200,
        &json!({
            "provider": editor_payload(&catalog, provider_id),
            "error": load_error,
        }),
    )
}

/// `DELETE /api/providers/{id}`.
fn delete(runtime: &DaemonRuntime, provider_id: &str) -> Response {
    let (catalog, load_error) = load_catalog(&runtime.config);
    if !load_error.is_empty() {
        return envelope_400(&load_error);
    }
    if !catalog.providers.contains_key(provider_id) {
        return envelope_400(&format!("provider profile not found: {provider_id}"));
    }
    let Ok(dream_config) = load_dream_config(&runtime.config) else {
        return envelope_400("dream.conf could not be read");
    };
    let mut used_by: Vec<String> = dream_config
        .workflows
        .iter()
        .filter(|(_, workflow)| workflow.provider == provider_id)
        .map(|(name, _)| name.clone())
        .collect();
    if !used_by.is_empty() {
        used_by.sort();
        return envelope_400(&format!(
            "provider profile is used by: {}",
            used_by.join(", ")
        ));
    }
    let mut next = catalog.clone();
    next.providers.remove(provider_id);
    if next.defaults.provider == provider_id {
        next.defaults = ProviderDefaults::default();
    }
    if let Err(error) = validate_provider_catalog(&next) {
        return envelope_400(&error.to_string());
    }
    if let Err(error) = save_provider_catalog(&runtime.config, &next) {
        return envelope_400(&error.to_string());
    }
    Response::json(200, &json!({"deleted": provider_id, "error": ""}))
}

/// `POST /api/providers/{id}/check` — probe the saved profile.
fn check(runtime: &DaemonRuntime, provider_id: &str) -> Response {
    let (catalog, load_error) = load_catalog(&runtime.config);
    if !load_error.is_empty() {
        return Response::json(400, &json!({"check": {}, "error": load_error}));
    }
    let Some(profile) = catalog.providers.get(provider_id) else {
        return Response::json(
            400,
            &json!({
                "check": {},
                "error": format!("provider profile not found: {provider_id}"),
            }),
        );
    };
    let probe = runtime.provider_client.check(provider_id, profile);
    Response::json(
        200,
        &json!({
            "check": {
                "ok": probe.ok && probe.error.is_empty(),
                "models": probe.models,
                "source": probe.source,
                "error": probe.error,
            },
            "error": "",
        }),
    )
}

/// `GET /api/providers/{id}/models` — model suggestions for the profile.
fn models(runtime: &DaemonRuntime, provider_id: &str) -> Response {
    let (catalog, load_error) = load_catalog(&runtime.config);
    if !load_error.is_empty() {
        return Response::json(
            200,
            &json!({"models": [], "source": "", "error": load_error}),
        );
    }
    let Some(profile) = catalog.providers.get(provider_id) else {
        return Response::json(
            200,
            &json!({
                "models": [],
                "source": "",
                "error": format!("provider profile not found: {provider_id}"),
            }),
        );
    };
    let probe = runtime.provider_client.models(provider_id, profile);
    Response::json(
        200,
        &json!({
            "models": probe.models,
            "source": probe.source,
            "error": probe.error,
        }),
    )
}

/// The redacted editor payload: `key_configured` instead of the secret. The
/// model defaults to the catalog default model while the profile is the
/// default provider.
pub(super) fn editor_payload(catalog: &ProviderCatalog, provider_id: &str) -> Value {
    let Some(profile) = catalog.providers.get(provider_id) else {
        return Value::Null;
    };
    let model = if catalog.defaults.provider == provider_id {
        catalog.defaults.model.clone()
    } else {
        String::new()
    };
    json!({
        "id": provider_id,
        "name": profile.name(),
        "type": profile.provider_type(),
        "url": profile.url(),
        "key_configured": !profile.key().expose_secret().is_empty(),
        "model": model,
        "timeout_seconds": profile.timeout_seconds(),
    })
}

fn envelope_400(error: &str) -> Response {
    Response::json(400, &json!({"error": error}))
}

fn load_catalog(config: &HieronymusConfig) -> (ProviderCatalog, String) {
    match load_provider_catalog(config) {
        Ok(catalog) => (catalog, String::new()),
        Err(error) => (
            hieronymus::provider_config::default_provider_catalog(),
            error.to_string(),
        ),
    }
}

/// Build the profile from the editor draft; an empty submitted key keeps the
/// existing profile's key (the key is only ever replaced explicitly).
fn draft_profile(
    raw: &Value,
    catalog: &ProviderCatalog,
    provider_id: &str,
) -> Result<ProviderProfile, String> {
    let required_text = |field: &str| -> Result<String, String> {
        match raw.get(field) {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_string()),
            _ => Err(format!("{field} is required")),
        }
    };
    let name = required_text("name")?;
    let provider_type = match required_text("type")? {
        gemini if gemini == "gemini" => "google".to_string(),
        other => other,
    };
    let url = required_text("url")?;
    let submitted_key = raw
        .get("key")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let existing = catalog.providers.get(provider_id);
    let key = if submitted_key.is_empty() {
        existing
            .map(|profile| profile.key().expose_secret().clone())
            .unwrap_or_default()
    } else {
        submitted_key
    };
    let raw_timeout = match raw.get("timeout_seconds") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => return Err("timeout_seconds is required".to_string()),
    };
    let Ok(timeout) = raw_timeout.trim().parse::<f64>() else {
        return Err("timeout_seconds must be a number".to_string());
    };
    if !timeout.is_finite() || timeout <= 0.0 {
        return Err("timeout_seconds must be greater than zero".to_string());
    }
    Ok(ProviderProfile::new(name, provider_type, url, key, timeout))
}

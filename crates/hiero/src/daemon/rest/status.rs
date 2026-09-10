//! `GET /status` — the native daemon status route (bearer + Host, no Origin
//! requirement). Payload ports the Python `status_payload`: providers,
//! dreaming autostart state, housekeeping, and adapter availability.

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::{DreamConfig, default_dream_config, load_dream_config};
use hieronymus::provider_config::{ProviderCatalog, load_provider_catalog};

use crate::daemon::daemon_version;
use crate::daemon::http::{Request, Response};

use super::super::DaemonRuntime;
use super::{bearer_matches, host_is_valid, invalid_host, unauthorized};

pub(super) fn handle(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    if !bearer_matches(request, runtime) {
        return unauthorized();
    }
    Response::json(200, &status_payload(runtime))
}

/// The authenticated status contract: the frozen Python `status_payload` keys
/// plus the ADR 0009 process-identity fields.
///
/// `instance_id` and `protocol_revision` are the Rust delta (recorded in
/// `daemon_rest_routes::status_route_matches_frozen_target` as an explicit
/// addition, never a change to the frozen keys). ADR 0009 requires stale
/// discovery to be detected "by authenticated health probing and
/// process-instance comparison, never by PID existence alone", which needs the
/// live instance id and protocol revision to be readable from an authenticated
/// endpoint — `GET /health` stays minimal and unauthenticated.
///
/// The bearer token appears nowhere in this payload; the sentinel-secret rule
/// holds for every field here and for every error this route can return.
pub(super) fn status_payload(runtime: &DaemonRuntime) -> Value {
    let (providers, providers_error) = provider_statuses(&runtime.config);
    let dreaming = dreaming_payload(&runtime.config);
    let pending = dreaming["pending_short_term_memories"]
        .as_i64()
        .unwrap_or(0);
    json!({
        "running": true,
        "pid": runtime.record.pid,
        "host": runtime.bound_address.ip().to_string(),
        "port": runtime.bound_address.port(),
        "version": daemon_version(),
        "instance_id": runtime.record.instance_id,
        "protocol_revision": crate::daemon::registry::PROTOCOL_REVISION,
        "started_at": runtime.record.started_at,
        "data_root": runtime.config.data_root().to_string_lossy(),
        "database_path": runtime.config.database_path().to_string_lossy(),
        "config_path": runtime.config.config_root().to_string_lossy(),
        "providers": providers,
        "providers_error": providers_error,
        "dreaming": dreaming,
        "mcp_adapter": {"available": true, "mode": "local-http"},
        "semantic": semantic_payload(runtime),
        "housekeeping": {"last_cycle": Value::Null, "pending": pending > 0},
    })
}

/// The required semantic readiness surface (Task S2): the supervised
/// controller's state plus, when it has one, its actionable failure detail.
/// An FTS-only lane surfaces as `failed` — never as ready — so strict
/// consumers gate on `require_semantic_ready`.
fn semantic_payload(runtime: &DaemonRuntime) -> Value {
    let snapshot = runtime.semantic.snapshot();
    let mut payload = match snapshot.state {
        crate::daemon::semantic_worker::RequiredSemanticState::Acquiring => {
            json!({"state": "acquiring", "detail": Value::Null})
        }
        crate::daemon::semantic_worker::RequiredSemanticState::Rebuilding => {
            json!({"state": "rebuilding", "detail": Value::Null})
        }
        crate::daemon::semantic_worker::RequiredSemanticState::Ready => {
            json!({"state": "ready", "detail": Value::Null})
        }
        crate::daemon::semantic_worker::RequiredSemanticState::Failed(reason) => {
            json!({"state": "failed", "detail": reason})
        }
    };
    payload["configuration_revision"] = json!(snapshot.configuration_revision);
    payload["configuration"] = json!(snapshot.configuration);
    payload["identity"] = json!(snapshot.identity.as_ref().map(|identity| json!({
        "provider": identity.provider(), "model": identity.model(), "revision": identity.revision(),
        "dimensions": identity.dimensions(), "normalization": identity.normalization(), "processing": identity.tokenizer(),
    })));
    payload
}

/// Builtin dream providers in the frozen order, then any catalog-only
/// profiles. A missing catalog profile is "provider profile missing".
fn provider_statuses(config: &HieronymusConfig) -> (Vec<Value>, String) {
    let builtin: [(&str, &str); 4] = [
        ("deterministic", "Deterministic"),
        ("openai", "OpenAI compatible"),
        ("gemini", "Gemini"),
        ("anthropic", "Anthropic"),
    ];
    let dream_config = match load_dream_config(config) {
        Ok(dream_config) => dream_config,
        Err(error) => return (Vec::new(), error.to_string()),
    };
    let catalog = match load_provider_catalog(config) {
        Ok(catalog) => catalog,
        Err(error) => return (Vec::new(), error.to_string()),
    };
    let mut names: Vec<String> = builtin
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    for name in catalog.providers.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    let mut statuses = Vec::with_capacity(names.len());
    for name in &names {
        let display_name = builtin
            .iter()
            .find(|(builtin, _)| builtin == name)
            .map(|(_, display)| (*display).to_string())
            .unwrap_or_else(|| name.clone());
        if name == "deterministic" {
            statuses.push(json!({
                "name": name,
                "display_name": display_name,
                "configured": true,
                "model": "",
                "api_key_present": false,
                "base_url": "",
                "timeout_seconds": 0.0,
                "error": "",
            }));
            continue;
        }
        let Some(profile) = catalog.providers.get(name) else {
            statuses.push(json!({
                "name": name,
                "display_name": display_name,
                "configured": false,
                "model": "",
                "api_key_present": false,
                "base_url": "",
                "timeout_seconds": 0.0,
                "error": "provider profile missing",
            }));
            continue;
        };
        let mut model = model_for_profile(&dream_config, name);
        if model.is_empty() && catalog.defaults.provider == *name {
            model = catalog.defaults.model.clone();
        }
        let api_key_present = !profile.key().expose_secret().trim().is_empty();
        let (configured, error) = if profile.provider_type() != "ollama" && !api_key_present {
            (false, "API key missing for provider profile".to_string())
        } else if model.trim().is_empty() {
            (false, "model is empty for provider profile".to_string())
        } else {
            (true, String::new())
        };
        statuses.push(json!({
            "name": name,
            "display_name": display_name,
            "configured": configured,
            "model": model,
            "api_key_present": api_key_present,
            "base_url": profile.url(),
            "timeout_seconds": profile.timeout_seconds(),
            "error": error,
        }));
    }
    (statuses, String::new())
}

/// The workflow model for a profile: any enabled workflow with a non-empty
/// model wins, then any workflow at all.
fn model_for_profile(dream_config: &DreamConfig, profile_name: &str) -> String {
    for workflow in dream_config.workflows.values() {
        if workflow.enabled
            && workflow.provider == profile_name
            && !workflow.model.trim().is_empty()
        {
            return workflow.model.clone();
        }
    }
    for workflow in dream_config.workflows.values() {
        if workflow.provider == profile_name && !workflow.model.trim().is_empty() {
            return workflow.model.clone();
        }
    }
    String::new()
}

/// Port of `DreamAutostart.status`: thresholds, pending counts, and the
/// persisted autostart bookkeeping (`dream-autostart.json`).
fn dreaming_payload(config: &HieronymusConfig) -> Value {
    let (dream_config, catalog) = safe_config_state(config);
    let state = load_autostart_state(config);
    let (pending_completed_sessions, pending_short_term_memories) = pending_counts(config);
    let cycle_active = hieronymus::dream_locks::read_dream_cycle_state(config);
    json!({
        "enabled": dream_config.enabled,
        "active_provider": active_provider(&dream_config, &catalog),
        "schedule_interval_minutes": dream_config.schedule_interval_minutes,
        "min_pending_short_term_memories": dream_config.min_pending_short_term_memories,
        "max_pending_short_term_memories": dream_config.max_pending_short_term_memories,
        "max_short_term_memories_per_cycle": dream_config.max_short_term_memories_per_cycle,
        "not_enough_memories_cycle_threshold": dream_config.not_enough_memories_cycle_threshold,
        "pending_completed_sessions": pending_completed_sessions,
        "pending_short_term_memories": pending_short_term_memories,
        "last_started_at": state.last_started_at,
        "last_error": state.last_error,
        "last_skipped_at": state.last_skipped_at,
        "last_skip_reason": state.last_skip_reason,
        "not_enough_memories_skipped_count": state.not_enough_memories_skipped_count,
        "skipped_count": state.not_enough_memories_skipped_count,
        "cycle_active": cycle_active.is_some(),
        "active_cycle": cycle_active.map(|state| {
            json!({
                "owner": state.owner,
                "pid": state.pid,
                "started_at": state.started_at,
            })
        }),
    })
}

/// A dream config/catalog pair that never fails: broken files fall back to
/// the defaults (their errors surface through the dedicated error fields of
/// the routes that need them).
pub(super) fn safe_config_state(config: &HieronymusConfig) -> (DreamConfig, ProviderCatalog) {
    let dream_config = load_dream_config(config).unwrap_or_else(|_| default_dream_config());
    let catalog = hieronymus::provider_config::load_provider_catalog(config)
        .unwrap_or_else(|_| hieronymus::provider_config::default_provider_catalog());
    (dream_config, catalog)
}

struct AutostartState {
    last_started_at: Value,
    last_error: String,
    last_skipped_at: Value,
    last_skip_reason: String,
    not_enough_memories_skipped_count: i64,
}

fn load_autostart_state(config: &HieronymusConfig) -> AutostartState {
    let default = AutostartState {
        last_started_at: Value::Null,
        last_error: String::new(),
        last_skipped_at: Value::Null,
        last_skip_reason: String::new(),
        not_enough_memories_skipped_count: 0,
    };
    let Ok(text) = std::fs::read_to_string(config.dream_autostart_path()) else {
        return default;
    };
    let Ok(payload) = serde_json::from_str::<Value>(&text) else {
        return default;
    };
    AutostartState {
        last_started_at: payload
            .get("last_started_at")
            .cloned()
            .unwrap_or(Value::Null),
        last_error: payload
            .get("last_error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        last_skipped_at: payload
            .get("last_skipped_at")
            .cloned()
            .unwrap_or(Value::Null),
        last_skip_reason: payload
            .get("last_skip_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        not_enough_memories_skipped_count: payload
            .get("not_enough_memories_skipped_count")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    }
}

/// Completed sessions and their unarchived short-term memories that dreaming
/// has not consumed yet.
pub(super) fn pending_counts(config: &HieronymusConfig) -> (i64, i64) {
    let Ok(connection) = hieronymus::db::open_migrated(&config.database_path()) else {
        return (0, 0);
    };
    connection
        .query_row(
            "select
               count(distinct task_sessions.id),
               count(short_term_memories.id)
             from task_sessions
             join short_term_memories
               on short_term_memories.session_id = task_sessions.id
             where task_sessions.status = 'completed'
               and task_sessions.cycle_id is null
               and short_term_memories.archived_at is null",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, 0))
}

/// Disabled dreaming is always deterministic; otherwise the crystallization
/// workflow's provider (or the default provider) runs it.
fn active_provider(dream_config: &DreamConfig, catalog: &ProviderCatalog) -> String {
    if !dream_config.enabled {
        return "deterministic".to_string();
    }
    let effective = |provider: &str| {
        if provider.is_empty() {
            catalog.defaults.provider.clone()
        } else {
            provider.to_string()
        }
    };
    if let Some(workflow) = dream_config.workflows.get("knowledge_crystals")
        && workflow.enabled
    {
        return effective(&workflow.provider);
    }
    for workflow in dream_config.workflows.values() {
        if workflow.enabled {
            return effective(&workflow.provider);
        }
    }
    "deterministic".to_string()
}

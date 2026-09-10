//! Admin routes (`/api/admin/*`): the dashboard payload, view snapshots, and
//! admin actions over the slice-2 stores and the slice-5 dream registry.
//! Payloads port the Python `AdminStore`/`AdminBridge` shapes, verified
//! against the frozen fixture targets.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{DreamConfig, default_dream_config, load_dream_config};

use super::super::DaemonRuntime;
use super::super::http::{Request, Response};
use super::parse_query;
use super::request_body;
use crate::daemon::daemon_display_version;
use crate::daemon::dream_worker::DreamRequest;

const ADMIN_VIEWS: [&str; 10] = [
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Memory",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Dream Audits",
    "Audit Log",
];

const ADMIN_VIEW_KEYS: [&str; 10] = [
    "concepts",
    "renderings",
    "crystals",
    "lessons",
    "short_term_memory",
    "short_term_sessions",
    "dream_runs",
    "proposals",
    "dream_audits",
    "audit_log",
];

/// One keyboard command definition: (id, label, hint, key, group, views,
/// requires_selection).
type AdminCommand = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    bool,
);

/// Keyboard command metadata, verbatim from the frozen oracle.
const ADMIN_COMMANDS: [AdminCommand; 13] = [
    (
        "add_memory",
        "Add Memory",
        "Create a new crystal in the current memory view.",
        "a",
        "Memory",
        &["Crystals", "Lessons"],
        false,
    ),
    (
        "edit_memory",
        "Edit Memory",
        "Edit the selected crystal or lesson text.",
        "e",
        "Memory",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "delete_selected",
        "Delete Selected",
        "Delete or archive the selected row after confirmation.",
        "d",
        "Memory",
        &["Concepts", "Crystals", "Lessons"],
        true,
    ),
    (
        "merge_selected",
        "Merge Selected",
        "Merge the selected concept or crystal into another item.",
        "m",
        "Memory",
        &["Concepts", "Crystals"],
        true,
    ),
    (
        "split_crystal",
        "Split Crystal",
        "Split the selected crystal or lesson into two memories.",
        "s",
        "Memory",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "reinforce_crystal",
        "Reinforce Crystal",
        "Increase strength/confidence for the selected crystal or lesson.",
        "+",
        "Memory",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "decay_crystal",
        "Decay Crystal",
        "Decrease strength/confidence for the selected crystal or lesson.",
        "-",
        "Memory",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "approve_proposal",
        "Approve Proposal",
        "Approve the selected compatibility proposal.",
        "a",
        "Proposals",
        &["Proposals"],
        true,
    ),
    (
        "reject_proposal",
        "Reject Proposal",
        "Reject the selected compatibility proposal.",
        "x",
        "Proposals",
        &["Proposals"],
        true,
    ),
    (
        "inspect_provenance",
        "Inspect Provenance",
        "Load provenance for the selected crystal or lesson.",
        "p",
        "Inspect",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "inspect_recall_reasons",
        "Inspect Recall Reasons",
        "Load recall reason data for the selected crystal or lesson.",
        "r",
        "Inspect",
        &["Crystals", "Lessons"],
        true,
    ),
    (
        "run_manual_dreaming",
        "Run Manual Dreaming",
        "Run dreaming manually and select the resulting dream run.",
        "D",
        "Dreaming",
        &["Dream Runs"],
        false,
    ),
    (
        "review_dream_output",
        "Review Dream Output",
        "Load the review payload for the selected dream run.",
        "enter",
        "Dreaming",
        &["Dream Runs"],
        true,
    ),
];

/// `GET /api/admin/dashboard` — the full admin bootstrap payload.
pub(super) fn dashboard(_request: &Request, runtime: &DaemonRuntime) -> Response {
    let config = &runtime.config;
    let mut payload = json!({
        "views": ADMIN_VIEWS,
        "view_keys": ADMIN_VIEW_KEYS,
        "view_labels": view_labels(),
        "view_options": view_options(),
        "command_options": command_options(),
        "default_view": "Crystals",
        "default_view_key": "crystals",
        "header": header_payload(),
        "stats": stats_payload(config),
        "service": service_payload(),
        "snapshot": snapshot_value(config, "Crystals", ""),
        "config_editor": config_editor_payload(config),
    });
    for (key, value) in dashboard_status_payload(config) {
        payload[key] = value;
    }
    Response::json(200, &payload)
}

/// `GET /api/admin/snapshot` — stats, snapshot, and live status.
pub(super) fn snapshot(_request: &Request, runtime: &DaemonRuntime, query: &str) -> Response {
    let params: BTreeMap<String, String> = parse_query(query).into_iter().collect();
    let view = view_label(params.get("view").map(String::as_str).unwrap_or("Crystals"));
    let config = &runtime.config;
    let snapshot =
        match crate::application::admin::snapshot(config, &view, &admin_query_from_params(&params))
        {
            Ok(value) => value,
            Err(crate::application::AppError::Invalid(message)) => {
                return Response::json(400, &json!({"error": message}));
            }
            Err(error) => {
                return Response::json(500, &json!({"error": error.to_string()}));
            }
        };
    let mut payload = json!({
        "stats": stats_payload(config),
        "snapshot": snapshot,
    });
    for (key, value) in dashboard_status_payload(config) {
        payload[key] = value;
    }
    Response::json(200, &payload)
}

/// `POST /api/admin/actions/{action}` — the typed, audited admin write
/// surface (plan W3). The domain logic and the audit row live in
/// [`crate::application::admin::run_action`]; this handler decodes the body,
/// passes the transport-authenticated actor (never a JSON field), and wraps
/// the outcome with the refreshed `stats`/`snapshot`/status projections
/// (frozen `reinforce_crystal` envelope preserved).
pub(super) fn action(request: &Request, runtime: &DaemonRuntime, action: &str) -> Response {
    let body = request_body(request).unwrap_or_else(|| json!({}));
    match crate::application::admin::run_action(
        &runtime.application,
        crate::daemon::BEARER_ACTOR,
        action,
        &body,
    ) {
        Ok(outcome) => {
            let config = &runtime.config;
            let view = outcome
                .get("view")
                .and_then(Value::as_str)
                .unwrap_or("Crystals");
            let selected_id = outcome
                .get("selected_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut payload = json!({
                "result": outcome.get("result").cloned().unwrap_or_else(|| json!({})),
                "stats": stats_payload(config),
                "snapshot": snapshot_value(config, view, selected_id),
            });
            for extra in ["provenance", "reasons", "review", "run"] {
                if let Some(value) = outcome.get(extra) {
                    payload[extra] = value.clone();
                }
            }
            for (key, value) in dashboard_status_payload(config) {
                payload[key] = value;
            }
            Response::json(200, &payload)
        }
        Err(crate::application::AppError::Authority(error)) => {
            Response::json(409, &json!({"error": error}))
        }
        Err(crate::application::AppError::Coherent(error)) => {
            Response::json(409, &json!({"error": error.to_string()}))
        }
        Err(crate::application::AppError::NotImplemented(_)) => {
            Response::json(404, &json!({"error": "unknown_admin_action"}))
        }
        Err(
            crate::application::AppError::Invalid(message)
            | crate::application::AppError::Domain(message),
        ) => Response::json(400, &json!({"error": message})),
    }
}

/// `POST /api/admin/actions/run_manual_dreaming` — hands the request to the
/// daemon's dream controller and answers immediately. The run executes on
/// the controller's supervised worker with the configured provider lanes
/// (task D5), coalescing into any run already active; its lifecycle is
/// published on the admin event hub by the worker (`dream_started`, then
/// `dream_phase_progress` / `dream_completed` / `dream_failed`, ported from
/// Python `_start_manual_dreaming`). The run is fail-closed: any failure is
/// recorded on the run row and the event stream, never propagated here.
pub(super) fn run_manual_dreaming(_request: &Request, runtime: &DaemonRuntime) -> Response {
    match runtime.dream.request(DreamRequest {
        all: true,
        manual: true,
    }) {
        Ok(_ticket) => Response::json(200, &json!({"started": true, "status": "running"})),
        // Only reachable while the daemon is shutting down.
        Err(error) => Response::json(503, &json!({"error": error})),
    }
}

// ---------------------------------------------------------------------------
// Payload builders
// ---------------------------------------------------------------------------

/// `short_term_status` + `dream_status` + `dream_config_error`, ported from
/// `AdminStore.dashboard_status_payload`.
pub(super) fn dashboard_status_payload(config: &HieronymusConfig) -> Vec<(&'static str, Value)> {
    let (dream_config, dream_config_error) = match load_dream_config(config) {
        Ok(dream_config) => (dream_config, String::new()),
        Err(error) => (default_dream_config(), error.to_string()),
    };
    let pending = pending_admin_count(config);
    let drain = drain_progress(config, pending);
    let mut dream_status = dream_status_payload(config, &dream_config);
    if !dream_config_error.is_empty() {
        dream_status["reason"] = json!(dream_config_error);
    }
    vec![
        ("dream_config_error", json!(dream_config_error)),
        ("dream_status", dream_status),
        (
            "short_term_status",
            json!({
                "pending_count": pending,
                "min_pending_short_term_memories": dream_config.min_pending_short_term_memories,
                "max_pending_short_term_memories": dream_config.max_pending_short_term_memories,
                "urgent": pending >= dream_config.max_pending_short_term_memories,
                "drain_in_progress": drain.in_progress,
                "drain_completed": drain.completed,
                "drain_remaining": drain.remaining,
                "drain_total": drain.total,
                "drain_progress": drain.progress,
            }),
        ),
    ]
}

/// Completed-session short-term memories that have not been archived
/// (`AdminStore.pending_completed_short_term_memory_count`).
fn pending_admin_count(config: &HieronymusConfig) -> i64 {
    query_count(
        config,
        "select count(*)
         from short_term_memories
         join task_sessions
           on task_sessions.id = short_term_memories.session_id
         where task_sessions.status = 'completed'
           and short_term_memories.archived_at is null",
    )
}

fn query_count(config: &HieronymusConfig, sql: &str) -> i64 {
    let Ok(connection) = open_migrated(&config.database_path()) else {
        return 0;
    };
    connection.query_row(sql, [], |row| row.get(0)).unwrap_or(0)
}

struct DrainProgress {
    in_progress: bool,
    completed: i64,
    remaining: i64,
    total: i64,
    progress: f64,
}

/// Port of `_dream_drain_progress`: a running dream consumes the pending
/// memories phase by phase.
fn drain_progress(config: &HieronymusConfig, pending: i64) -> DrainProgress {
    let idle = DrainProgress {
        in_progress: false,
        completed: 0,
        remaining: pending,
        total: pending,
        progress: 0.0,
    };
    let Ok(connection) = open_migrated(&config.database_path()) else {
        return idle;
    };
    let running_phase: Option<i64> = connection
        .query_row(
            "select dream_run_id from dream_phase_runs
             where status = 'running' order by id desc limit 1",
            [],
            |row| row.get(0),
        )
        .ok();
    let running_run_id = running_phase.or_else(|| {
        connection
            .query_row(
                "select id from dream_runs where status = 'running' order by id desc limit 1",
                [],
                |row| row.get(0),
            )
            .ok()
    });
    let Some(run_id) = running_run_id else {
        if hieronymus::dream_locks::read_dream_cycle_state(config).is_some() {
            // An OS-locked cycle without visible run rows yet: in progress,
            // nothing completed.
            return DrainProgress {
                in_progress: true,
                completed: 0,
                remaining: pending,
                total: pending,
                progress: 0.0,
            };
        }
        return idle;
    };
    let completed: i64 = connection
        .query_row(
            "select coalesce(sum(input_count), 0) from dream_phase_runs
             where dream_run_id = ?1 and status = 'completed'",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let total = completed + pending;
    let progress = if total == 0 {
        1.0
    } else {
        completed as f64 / total as f64
    };
    DrainProgress {
        in_progress: true,
        completed,
        remaining: pending,
        total,
        progress: round4(progress),
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// Port of `AdminStore._dream_status`: DISABLED/IDLE/WORKING from the run
/// rows and the OS dream-cycle state.
fn dream_status_payload(config: &HieronymusConfig, dream_config: &DreamConfig) -> Value {
    let Ok(connection) = open_migrated(&config.database_path()) else {
        return idle_dream_status(dream_config);
    };
    let mut run_id: Option<i64> = None;
    let mut cycle_id: Option<i64> = None;
    let mut phase: Option<String> = None;
    if let Ok((dream_run_id, dream_cycle_id, current_phase)) = connection.query_row(
        "select p.dream_run_id, r.cycle_id, p.phase
         from dream_phase_runs as p
         join dream_runs as r on r.id = p.dream_run_id
         where p.status = 'running'
         order by p.id desc limit 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ) {
        run_id = Some(dream_run_id);
        cycle_id = Some(dream_cycle_id);
        phase = Some(current_phase);
    }
    if run_id.is_none()
        && let Ok((dream_run_id, dream_cycle_id)) = connection.query_row(
            "select id, cycle_id from dream_runs where status = 'running'
             order by id desc limit 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    {
        run_id = Some(dream_run_id);
        cycle_id = Some(dream_cycle_id);
        phase = connection
            .query_row(
                "select phase from dream_phase_runs where dream_run_id = ?1
                     order by case status when 'running' then 0 else 1 end, id desc limit 1",
                [dream_run_id],
                |row| row.get(0),
            )
            .ok();
    }
    let active_cycle = hieronymus::dream_locks::read_dream_cycle_state(config);
    if active_cycle.is_none() && run_id.is_none() {
        return idle_dream_status(dream_config);
    }
    let current_phase = phase.unwrap_or_else(|| "starting".to_string());
    json!({
        "state": "WORKING",
        "current_phase": current_phase,
        "progress": phase_progress(config, &current_phase),
        "run_id": run_id,
        "cycle_id": cycle_id,
        "owner": active_cycle
            .as_ref()
            .map(|state| state.owner.clone())
            .unwrap_or_default(),
        "started_at": active_cycle
            .as_ref()
            .map(|state| state.started_at.clone())
            .unwrap_or_default(),
    })
}

fn idle_dream_status(dream_config: &DreamConfig) -> Value {
    json!({
        "state": if dream_config.enabled { "IDLE" } else { "DISABLED" },
        "current_phase": "",
        "progress": 0.0,
        "run_id": Value::Null,
        "cycle_id": Value::Null,
        "owner": "",
        "started_at": "",
    })
}

fn phase_progress(config: &HieronymusConfig, current_phase: &str) -> f64 {
    if current_phase == "starting" {
        return 0.0;
    }
    if current_phase == "maintenance" {
        return 0.9;
    }
    let drain = drain_progress(config, pending_admin_count(config));
    if drain.total > 0 {
        return drain.progress;
    }
    0.5
}

/// The eight admin counters (`AdminStore.stats`).
fn stats_payload(config: &HieronymusConfig) -> Value {
    let count = |connection: &Connection, table: &str, extra: &str| -> i64 {
        connection
            .query_row(
                &format!("select count(*) from {table} {extra}"),
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    };
    let empty = || {
        json!({
            "series": 0,
            "crystals": 0,
            "lessons": 0,
            "short_term_memories": 0,
            "sessions": 0,
            "dream_runs": 0,
            "pending_proposals": 0,
            "audit_events": 0,
        })
    };
    let Ok(connection) = open_migrated(&config.database_path()) else {
        return empty();
    };
    json!({
        "series": count(&connection, "series", ""),
        "crystals": count(&connection, "crystals", ""),
        "lessons": count(&connection, "crystals", "where crystal_type = 'lesson'"),
        "short_term_memories": count(
            &connection,
            "short_term_memories",
            "where archived_at is null",
        ),
        "sessions": count(&connection, "task_sessions", ""),
        "dream_runs": count(&connection, "dream_runs", ""),
        "pending_proposals": count(
            &connection,
            "strict_concept_proposals",
            "where status = 'pending'",
        ),
        "audit_events": count(&connection, "audit_log", ""),
    })
}

/// The admin view projection, delegated to the read-only domain module
/// [`crate::application::admin`]. Used by the callers that only ever pass a
/// selection (`dashboard`, `reinforce_crystal`); `Crystals`/`Lessons` stay
/// byte-identical to the frozen fixture.
fn snapshot_value(config: &HieronymusConfig, view: &str, selected_id: &str) -> Value {
    let mut query = serde_json::Map::new();
    if !selected_id.is_empty() {
        query.insert("selected_id".to_string(), json!(selected_id));
    }
    crate::application::admin::snapshot(config, view, &Value::Object(query))
        .unwrap_or_else(|error| json!({"error": error.to_string()}))
}

/// The `GET /api/admin/snapshot` query surface: selection plus the bounded
/// paging and series scope that [`crate::application::admin::snapshot`] reads
/// (`AdminSnapshotQuery` in the frontend). Values pass through as strings; the
/// domain module clamps and parses them.
fn admin_query_from_params(params: &BTreeMap<String, String>) -> Value {
    let mut query = serde_json::Map::new();
    for key in ["selected_id", "id", "limit", "offset", "series", "context"] {
        if let Some(value) = params.get(key).filter(|value| !value.is_empty()) {
            query.insert(key.to_string(), json!(value));
        }
    }
    Value::Object(query)
}

/// Header metadata for the console (port of `header_status_payload`).
fn header_payload() -> Value {
    json!({
        "product": "Hieronymus",
        "version": daemon_display_version(),
        "tagline": "Remembers things for you.",
        "logo": {
            "text": "\u{1FAB6}",
            "name": "feather",
            "alt": "Hieronymus feather logo",
        },
    })
}

/// The daemon is the only local service in this slice; there is no external
/// service state, so the admin console reports it as not running (the Python
/// `ServiceManager` "no-state" outcome).
fn service_payload() -> Value {
    json!({"running": false, "reason": "no-state"})
}

/// Port of `AdminStore.config_editor_payload`: the redacted dream config, the
/// provider catalog, prompts, thresholds, and the model-cache warnings.
fn config_editor_payload(config: &HieronymusConfig) -> Value {
    let (dream_config, dream_config_error) = match load_dream_config(config) {
        Ok(dream_config) => (dream_config, String::new()),
        Err(error) => (default_dream_config(), error.to_string()),
    };
    let (catalog, provider_config_error) =
        match hieronymus::provider_config::load_provider_catalog(config) {
            Ok(catalog) => (catalog, String::new()),
            Err(error) => (
                hieronymus::provider_config::default_provider_catalog(),
                error.to_string(),
            ),
        };
    let dream_payload = hieronymus::dream_config::redacted_dream_config_payload(&dream_config);
    let catalog_payload = hieronymus::provider_config::redacted_provider_catalog_payload(&catalog);
    let dream_json = serde_json::to_value(&dream_payload).unwrap_or_else(|_| json!({}));
    let workflows = dream_json
        .get("workflows")
        .cloned()
        .unwrap_or_else(|| json!({}));
    json!({
        "config": dream_json,
        "config_error": dream_config_error,
        "provider_config_error": provider_config_error,
        "providers": serde_json::to_value(&catalog_payload).unwrap_or_else(|_| json!({})),
        "workflows": serde_json::to_value(&workflows).unwrap_or_else(|_| json!({})),
        "prompts": {"general": dream_config.general_prompt},
        "thresholds": {
            "min_pending_short_term_memories": dream_config.min_pending_short_term_memories,
            "max_pending_short_term_memories": dream_config.max_pending_short_term_memories,
            "max_short_term_memories_per_cycle": dream_config.max_short_term_memories_per_cycle,
            "not_enough_memories_cycle_threshold": dream_config.not_enough_memories_cycle_threshold,
            "max_changed_crystals_per_cycle": dream_config.max_changed_crystals_per_cycle,
            "max_related_concepts_per_cycle": dream_config.max_related_concepts_per_cycle,
            "max_related_crystals_per_concept": dream_config.max_related_crystals_per_concept,
            "max_total_affected_crystals": dream_config.max_total_affected_crystals,
        },
        "model_cache": super::settings::model_cache_payload(config),
        "model_cache_warnings": model_cache_warnings(config, &dream_config),
    })
}

/// Workflow providers that are not configured are reported per workflow (the
/// only warning class reachable in this slice: the model cache is written by
/// a later provider slice).
fn model_cache_warnings(config: &HieronymusConfig, dream_config: &DreamConfig) -> Value {
    let catalog = hieronymus::provider_config::load_provider_catalog(config)
        .unwrap_or_else(|_| hieronymus::provider_config::default_provider_catalog());
    let warnings: Vec<Value> = dream_config
        .workflows
        .iter()
        .filter(|(_, workflow)| !catalog.providers.contains_key(&workflow.provider))
        .map(|(name, workflow)| {
            json!({
                "workflow": name,
                "provider": workflow.provider,
                "code": "provider_missing",
                "message": "workflow provider is not configured",
            })
        })
        .collect();
    Value::Array(warnings)
}

fn view_labels() -> Value {
    ADMIN_VIEW_KEYS
        .iter()
        .zip(ADMIN_VIEWS)
        .map(|(key, label)| (key.to_string(), json!(label)))
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

fn view_options() -> Vec<Value> {
    ADMIN_VIEW_KEYS
        .iter()
        .zip(ADMIN_VIEWS)
        .map(|(key, label)| json!({"key": key, "label": label}))
        .collect()
}

fn command_options() -> Vec<Value> {
    ADMIN_COMMANDS
        .iter()
        .map(|(id, label, hint, key, group, views, requires_selection)| {
            json!({
                "id": id,
                "label": label,
                "hint": hint,
                "key": key,
                "group": group,
                "views": views,
                "requires_selection": requires_selection,
            })
        })
        .collect()
}

/// Normalize a view key to its label form (`admin_view_label`); labels pass
/// through.
fn view_label(view: &str) -> String {
    ADMIN_VIEW_KEYS
        .iter()
        .position(|key| *key == view)
        .map(|position| ADMIN_VIEWS[position].to_string())
        .unwrap_or_else(|| view.to_string())
}

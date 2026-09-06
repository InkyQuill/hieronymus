//! Admin routes (`/api/admin/*`): the dashboard payload, view snapshots, and
//! admin actions over the slice-2 stores and the slice-5 dream registry.
//! Payloads port the Python `AdminStore`/`AdminBridge` shapes, verified
//! against the frozen fixture targets.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{DreamConfig, default_dream_config, load_dream_config};
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{DreamError, DreamRunRecord, DreamService};

use super::super::DaemonRuntime;
use super::super::events;
use super::super::http::{Request, Response};
use super::parse_query;
use super::request_body;
use crate::daemon::daemon_display_version;

/// The views the snapshot serves in this slice. The frozen contracts address
/// the `Crystals` view; `Lessons` shares its row model. The remaining views
/// need stores that are ported in later slices and fail closed for now.
const SUPPORTED_VIEWS: [&str; 2] = ["Crystals", "Lessons"];

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

/// Immediate user-feedback deltas: `confirmed_by_user` moves a crystal by
/// (+0.15 strength, +0.20 confidence), clamped to [0, 1].
const REINFORCE_DELTAS: (f64, f64) = (0.15, 0.20);

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
    let selected_id = params
        .get("selected_id")
        .or_else(|| params.get("id"))
        .map(String::as_str)
        .unwrap_or_default();
    if !SUPPORTED_VIEWS.contains(&view.as_str()) {
        return Response::json(
            400,
            &json!({"error": format!("unsupported admin view: {view}")}),
        );
    }
    let config = &runtime.config;
    let mut payload = json!({
        "stats": stats_payload(config),
        "snapshot": snapshot_value(config, &view, selected_id),
    });
    for (key, value) in dashboard_status_payload(config) {
        payload[key] = value;
    }
    Response::json(200, &payload)
}

/// `POST /api/admin/actions/{action}`.
pub(super) fn action(request: &Request, runtime: &DaemonRuntime, action: &str) -> Response {
    match action {
        "reinforce_crystal" => reinforce_crystal(request, runtime),
        _ => Response::json(404, &json!({"error": "unknown_admin_action"})),
    }
}

/// `POST /api/admin/actions/run_manual_dreaming` — starts a dream run through
/// the slice-5 `DreamService` seam in the background and answers
/// immediately. The run is fail-closed: any failure is recorded on the run
/// row, never propagated to the caller. The run's lifecycle is published on
/// the admin event hub (`dream_started` synchronously, then
/// `dream_phase_progress` / `dream_completed` / `dream_failed` as the run
/// progresses, ported from Python `_start_manual_dreaming`).
pub(super) fn run_manual_dreaming(_request: &Request, runtime: &DaemonRuntime) -> Response {
    let config = runtime.config.clone();
    // Published before the route answers, so subscribers see the run begin
    // even when the background work fails immediately.
    runtime
        .events
        .publish("dream_started", json!({"trigger": "manual"}));
    let finished = Arc::new(AtomicBool::new(false));
    {
        let config = config.clone();
        let events = Arc::clone(&runtime.events);
        let finished = Arc::clone(&finished);
        std::thread::spawn(move || monitor_dream_progress(&config, &events, &finished));
    }
    let events = Arc::clone(&runtime.events);
    std::thread::spawn(move || {
        // The monitor stops when this drops — on completion and on unwind,
        // so it can never outlive the run.
        let _finished = FinishFlag(Arc::clone(&finished));
        // Explicit deterministic injection (the pre-D5 seam): the configured
        // provider lanes arrive with the D5 controller.
        let outcome = DreamService::open(&config, WorkflowResolver::deterministic())
            .and_then(|service| service.run_all("admin", true, false));
        match outcome {
            Ok(record) => {
                // Port of AdminStore.run_manual_dreaming's audit entry.
                if let Ok(connection) = open_migrated(&config.database_path()) {
                    let _ = connection.execute(
                        "insert into audit_log(action, entity_type, entity_id, note, created_at)
                         values ('run', 'dream', ?1, ?2, ?3)",
                        rusqlite::params![
                            record.id.to_string(),
                            format!(
                                "Manual dream run {} with provider {}",
                                record.cycle_id, record.provider
                            ),
                            now(),
                        ],
                    );
                }
                events.publish(
                    "dream_completed",
                    json!({"trigger": "manual", "result": dream_run_payload(&record)}),
                );
            }
            Err(error) => {
                events.publish(
                    "dream_failed",
                    json!({"trigger": "manual", "error": redacted_error(&config, &error)}),
                );
            }
        }
    });
    Response::json(200, &json!({"started": true, "status": "running"}))
}

/// Sets its flag on drop: the monitor's stop signal survives any early
/// return or unwind in the run thread.
struct FinishFlag(Arc<AtomicBool>);

impl Drop for FinishFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// How often the dream monitor samples the run/phase registry (the Python
/// monitor slept 0.2s between samples).
const DREAM_PROGRESS_POLL: Duration = Duration::from_millis(200);

/// Port of the Python dream monitor: while a manual run is in flight, watch
/// the slice-5 run/phase registry (the durable `dream_runs`/`dream_phase_runs`
/// rows the `DreamService` writes) and publish `dream_phase_progress`
/// whenever the running phase row changes.
fn monitor_dream_progress(
    config: &HieronymusConfig,
    events: &events::AdminEventHub,
    finished: &AtomicBool,
) {
    let mut seen_phase_id = 0;
    while !finished.load(Ordering::Acquire) {
        if let Some((phase_id, run_id, cycle_id, phase)) = latest_running_phase(config)
            && phase_id != seen_phase_id
        {
            seen_phase_id = phase_id;
            events.publish(
                "dream_phase_progress",
                json!({"run_id": run_id, "cycle_id": cycle_id, "phase": phase}),
            );
        }
        std::thread::sleep(DREAM_PROGRESS_POLL);
    }
}

/// The currently running phase row of the dream registry, newest first:
/// (phase row id, run id, cycle id, phase name).
fn latest_running_phase(config: &HieronymusConfig) -> Option<(i64, i64, i64, String)> {
    let connection = open_migrated(&config.database_path()).ok()?;
    connection
        .query_row(
            "select p.id, p.dream_run_id, r.cycle_id, p.phase
             from dream_phase_runs as p
             join dream_runs as r on r.id = p.dream_run_id
             where p.status = 'running'
             order by p.id desc limit 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok()
}

/// The `dream_completed` result payload: the serialized dream-run record
/// (Python `dataclass_to_json(run)`, field names verbatim).
fn dream_run_payload(record: &DreamRunRecord) -> Value {
    json!({
        "id": record.id,
        "cycle_id": record.cycle_id,
        "status": record.status,
        "provider": record.provider,
        "input_count": record.input_count,
        "created_crystal_count": record.created_crystal_count,
        "proposal_count": record.proposal_count,
        "error": record.error,
    })
}

/// Error text for the `dream_failed` event, redacted exactly like the
/// dreaming core so configured provider keys never leave the process.
fn redacted_error(config: &HieronymusConfig, error: &DreamError) -> String {
    let message = error.to_string();
    hieronymus::provider_config::load_provider_catalog(config)
        .map(|catalog| {
            let keys: Vec<&str> = catalog
                .providers
                .values()
                .map(|profile| profile.key().expose_secret().as_str())
                .collect();
            hieronymus::secret::redact_values(&message, &keys)
        })
        .unwrap_or(message)
}

/// `POST /api/admin/actions/reinforce_crystal` — one `confirmed_by_user`
/// feedback event plus an audit entry, then the refreshed payload.
fn reinforce_crystal(request: &Request, runtime: &DaemonRuntime) -> Response {
    let Some(body) = request_body(request) else {
        return action_error("id must be an integer");
    };
    let Some(crystal_id) = body.get("id").and_then(Value::as_i64) else {
        return action_error("id must be an integer");
    };
    let evidence = body
        .get("evidence")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("Reinforced from admin bridge");
    let config = &runtime.config;
    if let Err(error) = apply_reinforcement(config, crystal_id, evidence) {
        return action_error(&error);
    }
    let mut payload = json!({
        "result": json!({
            "entity_type": "crystal",
            "entity_id": crystal_id,
            "action": "reinforce",
            "message": "Crystal reinforced",
        }),
        "stats": stats_payload(config),
        "snapshot": snapshot_value(config, "Crystals", &crystal_id.to_string()),
    });
    for (key, value) in dashboard_status_payload(config) {
        payload[key] = value;
    }
    Response::json(200, &payload)
}

fn action_error(message: &str) -> Response {
    Response::json(400, &json!({"error": message}))
}

/// The reinforce transaction: feedback event, clamped score update, audit
/// entry. `confirmed_by_user` raises strength by 0.15 and confidence by 0.20.
fn apply_reinforcement(
    config: &HieronymusConfig,
    crystal_id: i64,
    evidence: &str,
) -> Result<(), String> {
    let mut connection = open_migrated(&config.database_path()).map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let existing: Option<(f64, f64, String)> = transaction
        .query_row(
            "select strength, confidence, status from crystals where id = ?1",
            [crystal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| e.to_string())?;
    let Some((strength, confidence, status)) = existing else {
        return Err(format!("unknown crystal: {crystal_id}"));
    };
    let (strength_delta, confidence_delta) = REINFORCE_DELTAS;
    let now = now();
    transaction
        .execute(
            "insert into memory_events(
               crystal_id, session_id, event_type, source_role, evidence,
               strength_delta, confidence_delta, applied, created_at
             )
             values (?1, null, 'confirmed_by_user', 'user', ?2, ?3, ?4, 1, ?5)",
            rusqlite::params![crystal_id, evidence, strength_delta, confidence_delta, now],
        )
        .map_err(|e| e.to_string())?;
    transaction
        .execute(
            "update crystals set strength = ?1, confidence = ?2, status = ?3, updated_at = ?4
             where id = ?5",
            rusqlite::params![
                clamp_score(strength + strength_delta),
                clamp_score(confidence + confidence_delta),
                status,
                now,
                crystal_id
            ],
        )
        .map_err(|e| e.to_string())?;
    transaction
        .execute(
            "insert into audit_log(action, entity_type, entity_id, note, created_at)
             values ('reinforce', 'crystal', ?1, ?2, ?3)",
            rusqlite::params![crystal_id.to_string(), evidence, now],
        )
        .map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}

fn clamp_score(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

/// The Crystals/Lessons snapshot (`AdminBridge._snapshot` semantics: the
/// detail body is the crystal text).
fn snapshot_value(config: &HieronymusConfig, view: &str, selected_id: &str) -> Value {
    let kind_filter: Option<&'static str> = if view == "Lessons" {
        Some("lesson")
    } else {
        None
    };
    let rows = crystal_rows(config, kind_filter);
    let selected = select_row(&rows, selected_id);
    let detail = match &selected {
        Some(row) => {
            let crystal_id = row["id"].as_i64().unwrap_or_default();
            let crystal_text = open_migrated(&config.database_path())
                .ok()
                .and_then(|connection| {
                    connection
                        .query_row(
                            "select text from crystals where id = ?1",
                            [crystal_id],
                            |row| row.get::<_, String>(0),
                        )
                        .ok()
                })
                .unwrap_or_default();
            json!({
                "title": row["label"].clone(),
                "subtitle": format!(
                    "{} / {}",
                    row["kind"].as_str().unwrap_or_default(),
                    row["status"].as_str().unwrap_or_default()
                ),
                "body": crystal_text,
                "fields": [
                    ["Series", row["scope"].clone()],
                    ["Language", row["language_pair"].clone()],
                    ["Quality", row["quality_label"].clone()],
                ],
            })
        }
        None => json!({
            "title": view,
            "subtitle": "No rows",
            "body": "",
            "fields": [],
        }),
    };
    let selected = selected.unwrap_or(Value::Null);
    json!({
        "view": view,
        "rows": rows,
        "selected": selected,
        "detail": detail,
        "filters": [],
    })
}

fn crystal_rows(config: &HieronymusConfig, kind: Option<&'static str>) -> Vec<Value> {
    let Ok(connection) = open_migrated(&config.database_path()) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "select id, crystal_type, title, text, status, series_slug, scope_key, scope_type,
                source_language, target_language, tags_json, confidence, strength
         from crystals
         where (?1 is null or crystal_type = ?1)
         order by id
         limit 200",
    ) else {
        return Vec::new();
    };
    let rows = statement.query_map([kind], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, f64>(11)?,
            row.get::<_, f64>(12)?,
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.flatten()
        .map(
            |(
                id,
                kind,
                title,
                text,
                status,
                series_slug,
                scope_key,
                scope_type,
                source_language,
                target_language,
                tags_json,
                confidence,
                strength,
            )| {
                let label = if title.is_empty() {
                    excerpt(&text)
                } else {
                    title
                };
                let scope = if !series_slug.is_empty() {
                    series_slug
                } else if !scope_key.is_empty() {
                    scope_key
                } else {
                    scope_type
                };
                json!({
                    "id": id,
                    "kind": kind,
                    "label": label,
                    "status": status,
                    "scope": scope,
                    "language_pair": format!("{source_language} -> {target_language}"),
                    "quality_label": quality_label(confidence, strength),
                    "tags": serde_json::from_str::<Vec<String>>(&tags_json)
                        .unwrap_or_default(),
                })
            },
        )
        .collect()
}

fn select_row(rows: &[Value], selected_id: &str) -> Option<Value> {
    let first = rows.first()?.clone();
    if selected_id.is_empty() {
        return Some(first);
    }
    rows.iter()
        .find(|row| {
            row["id"]
                .as_i64()
                .is_some_and(|id| id.to_string() == selected_id)
        })
        .cloned()
        .or(Some(first))
}

fn quality_label(confidence: f64, strength: f64) -> String {
    format!("{} conf / {} str", percent(confidence), percent(strength))
}

/// Python `f"{round(float(value) * 100):.0f}%"` (round-half-even over the
/// binary value, which `{:.0}` formatting reproduces).
fn percent(value: f64) -> String {
    format!("{:.0}%", value * 100.0)
}

fn excerpt(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 80;
    if normalized.chars().count() <= LIMIT {
        return normalized;
    }
    let cut: String = normalized.chars().take(LIMIT - 1).collect();
    format!("{cut}...")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// The monitor publishes a `dream_phase_progress` event whenever the
    /// running phase row in the slice-5 registry changes.
    #[test]
    fn dream_progress_monitor_publishes_phase_changes() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let connection = open_migrated(&config.database_path()).unwrap();
        connection
            .execute(
                "insert into dream_runs(cycle_id, status, provider, created_at)
                 values (7, 'running', 'test', '2026-09-04T00:00:00Z')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "insert into dream_phase_runs(
                   dream_run_id, phase, provider_profile, provider_type, model, status, created_at
                 )
                 values (1, 'knowledge_crystals', 'test', 'test', 'test', 'running',
                         '2026-09-04T00:00:00Z')",
                [],
            )
            .unwrap();
        drop(connection);

        let hub = events::AdminEventHub::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            hub.subscribe(Arc::new(move |event: &events::AdminEvent| {
                seen.lock().unwrap().push(event.clone());
                true
            }));
        }
        let finished = Arc::new(AtomicBool::new(false));
        let monitor = {
            let config = config.clone();
            let hub = hub;
            let finished = Arc::clone(&finished);
            std::thread::spawn(move || monitor_dream_progress(&config, &hub, &finished))
        };

        let wait_for = |count: usize| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while seen.lock().unwrap().len() < count {
                assert!(
                    Instant::now() < deadline,
                    "monitor never published event {count}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait_for(1);
        // A new running phase row is the change the monitor reports next.
        let connection = open_migrated(&config.database_path()).unwrap();
        connection
            .execute(
                "insert into dream_phase_runs(
                   dream_run_id, phase, provider_profile, provider_type, model, status, created_at
                 )
                 values (1, 'persistence', 'test', 'test', 'test', 'running',
                         '2026-09-04T00:00:00Z')",
                [],
            )
            .unwrap();
        drop(connection);
        wait_for(2);
        finished.store(true, Ordering::Release);
        monitor.join().unwrap();

        let published = seen.lock().unwrap().clone();
        assert_eq!(
            published.len(),
            2,
            "unchanged phase rows are not republished"
        );
        assert_eq!(published[0].event_type, "dream_phase_progress");
        assert_eq!(
            published[0].payload,
            json!({"run_id": 1, "cycle_id": 7, "phase": "knowledge_crystals"})
        );
        assert_eq!(
            published[1].payload,
            json!({"run_id": 1, "cycle_id": 7, "phase": "persistence"})
        );
    }
}

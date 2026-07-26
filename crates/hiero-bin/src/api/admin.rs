use std::future::Future;

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    routing::{get, post},
};
use hiero_core::db::{
    ConceptProposalRecord, ConceptRecord, CrystalRecord, DreamRunRecord, ShortTermMemoryRecord,
    TaskSessionRecord,
};
use hiero_core::domain::{
    ConceptProposalStore, ConceptStore, CrystalStore, FeedbackError, FeedbackEvent, FeedbackStore,
    WorkspaceStore,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::daemon::{AppState, DAEMON_DREAM_CLEANUP_DEADLINE, RequestId};

use super::{
    contracts::{
        ActionMessage, AdminActionRequest, AdminActionResult, AdminDashboard, AdminDetail,
        AdminDreamStatus, AdminHeader, AdminRow, AdminShortTermStatus, AdminSnapshot,
        AdminSnapshotResponse, ManualDreamResponse,
    },
    error::ApiError,
};

const SNAPSHOT_LIMIT: i64 = 200;

const VIEWS: [&str; 7] = [
    "Crystals",
    "Lessons",
    "Concepts",
    "Proposals",
    "Short-Term Memories",
    "Short-Term Sessions",
    "Dream Runs",
];

struct DreamRunningGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for DreamRunningGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub trait FeedbackRecorder: Clone + Send + Sync {
    fn record(
        &self,
        event: FeedbackEvent,
    ) -> impl Future<Output = Result<(), FeedbackError>> + Send;
    fn delete_by_user(
        &self,
        crystal_id: i64,
    ) -> impl Future<Output = Result<(), FeedbackError>> + Send;
}

#[derive(Clone)]
struct SqliteFeedback {
    pool: SqlitePool,
}

impl FeedbackRecorder for SqliteFeedback {
    async fn record(&self, event: FeedbackEvent) -> Result<(), FeedbackError> {
        FeedbackStore::new(&self.pool)
            .record(event)
            .await
            .map(|_| ())
    }

    async fn delete_by_user(&self, crystal_id: i64) -> Result<(), FeedbackError> {
        FeedbackStore::new(&self.pool)
            .delete_by_user(crystal_id, "web_admin", Some("Deleted from web admin"))
            .await
            .map(|_| ())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("action requires confirmation")]
    ConfirmationRequired,
    #[error("unknown admin action")]
    UnknownAction,
    #[error(transparent)]
    Feedback(#[from] FeedbackError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub struct AdminApi<R> {
    feedback: R,
    pool: SqlitePool,
}

impl<R: FeedbackRecorder> AdminApi<R> {
    pub const fn new(feedback: R, pool: SqlitePool) -> Self {
        Self { feedback, pool }
    }

    pub async fn run_action(
        &self,
        action: &str,
        request: AdminActionRequest,
    ) -> Result<ActionMessage, AdminError> {
        let (event_type, message, evidence) = match action {
            "reinforce_crystal" => (
                "confirmed_by_user",
                "Crystal reinforced",
                "Reinforced from web admin",
            ),
            "decay_crystal" => (
                "contradicted_by_user",
                "Crystal decayed",
                "Decayed from web admin",
            ),
            "delete_crystal" => {
                if request.confirmed != Some(true) {
                    return Err(AdminError::ConfirmationRequired);
                }
                (
                    "deleted_by_user",
                    "Crystal deleted",
                    "Deleted from web admin",
                )
            }
            _ => return Err(AdminError::UnknownAction),
        };
        if event_type == "deleted_by_user" {
            self.feedback.delete_by_user(request.id.get()).await?;
        } else {
            self.feedback
                .record(FeedbackEvent {
                    crystal_id: request.id.get(),
                    event_type: event_type.into(),
                    source_role: "web_admin".into(),
                    evidence: Some(evidence.into()),
                    session_id: None,
                })
                .await?;
        }
        Ok(ActionMessage {
            message: message.into(),
        })
    }

    pub async fn snapshot(
        &self,
        view: &str,
        selected_id: Option<&str>,
    ) -> Result<AdminSnapshot, sqlx::Error> {
        let records = match view {
            "Crystals" => crystal_items(&self.pool, false).await?,
            "Lessons" => crystal_items(&self.pool, true).await?,
            "Concepts" => concept_items(&self.pool).await?,
            "Proposals" => proposal_items(&self.pool).await?,
            "Short-Term Memories" => short_term_memory_items(&self.pool).await?,
            "Short-Term Sessions" => session_items(&self.pool).await?,
            "Dream Runs" => dream_run_items(&self.pool).await?,
            _ => Vec::new(),
        };
        let selected_id = selected_id.and_then(|value| value.parse::<i64>().ok());
        let selected_record = match selected_id {
            Some(selected_id)
                if records
                    .iter()
                    .all(|record| record.row.id.as_i64() != Some(selected_id)) =>
            {
                selected_item(&self.pool, view, selected_id).await?
            }
            _ => None,
        };
        Ok(build_snapshot(view, selected_id, records, selected_record))
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dashboard", get(dashboard))
        .route("/snapshot", get(snapshot))
        .route("/actions/run_manual_dreaming", post(run_manual_dreaming))
        .route("/actions/{action}", post(run_action))
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    view: String,
    #[serde(default)]
    selected_id: Option<String>,
}

async fn dashboard(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<AdminDashboard>, ApiError> {
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64)>(
        "SELECT
            (SELECT count(*) FROM series),
            (SELECT count(*) FROM crystals),
            (SELECT count(*) FROM crystals WHERE crystal_type = 'lesson'),
            (SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL),
            (SELECT count(*) FROM task_sessions),
            (SELECT count(*) FROM dream_runs),
            (SELECT count(*) FROM concept_proposals WHERE status = 'pending'),
            (SELECT count(*) FROM memory_events)",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM short_term_memories
         JOIN task_sessions ON task_sessions.id = short_term_memories.session_id
         WHERE task_sessions.status = 'completed'
           AND short_term_memories.archived_at IS NULL",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let running = sqlx::query_as::<_, (i64, i64, Option<String>)>(
        "SELECT r.id, r.cycle_id, p.phase
         FROM dream_runs AS r
         LEFT JOIN dream_phase_runs AS p
           ON p.dream_run_id = r.id AND p.status = 'running'
         WHERE r.status = 'running'
         ORDER BY p.id DESC
         LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let config = state.config.clone();
    let (dream_config, active_cycle) = tokio::task::spawn_blocking(move || {
        let dream = hiero_core::dreaming::DreamConfig::load(&config)?;
        let active = hiero_core::dreaming::read_dream_cycle_state(&config);
        Ok::<_, hiero_core::dreaming::DreamConfigError>((dream, active))
    })
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let working = running.is_some() || active_cycle.is_some();
    let (run_id, cycle_id, current_phase) = running
        .map(|(run_id, cycle_id, phase)| {
            (
                Some(run_id),
                Some(cycle_id),
                phase.unwrap_or_else(|| "starting".into()),
            )
        })
        .unwrap_or((
            None,
            None,
            if working {
                "starting".into()
            } else {
                String::new()
            },
        ));
    let completed = if let Some(run_id) = run_id {
        sqlx::query_scalar::<_, i64>(
            "SELECT coalesce(sum(input_count), 0)
             FROM dream_phase_runs
             WHERE dream_run_id = ? AND status = 'completed'",
        )
        .bind(run_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?
    } else {
        0
    };
    let drain_total = completed + pending_count;
    let drain_progress = if working && drain_total == 0 {
        1.0
    } else if drain_total == 0 {
        0.0
    } else {
        completed as f64 / drain_total as f64
    };
    let urgent = pending_count >= dream_config.max_pending_short_term_memories as i64;
    let dream_progress = if !working || current_phase == "starting" {
        0.0
    } else if current_phase == "maintenance" {
        0.9
    } else {
        drain_progress
    };
    let owner = active_cycle
        .as_ref()
        .map(|cycle| cycle.owner.clone())
        .unwrap_or_default();
    let started_at = active_cycle
        .as_ref()
        .map(|cycle| cycle.started_at.to_rfc3339())
        .unwrap_or_default();
    Ok(Json(AdminDashboard {
        header: AdminHeader {
            product: "Hieronymus".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            tagline: "Local-first literary translation memory".into(),
        },
        stats: [
            ("series".into(), stats.0),
            ("crystals".into(), stats.1),
            ("lessons".into(), stats.2),
            ("short_term_memories".into(), stats.3),
            ("sessions".into(), stats.4),
            ("dream_runs".into(), stats.5),
            ("pending_proposals".into(), stats.6),
            ("audit_events".into(), stats.7),
        ]
        .into(),
        views: VIEWS.into_iter().map(str::to_owned).collect(),
        short_term_status: AdminShortTermStatus {
            state: if working {
                "DRAINING"
            } else if urgent {
                "URGENT"
            } else {
                "IDLE"
            }
            .into(),
            pending_count,
            min_pending_short_term_memories: dream_config.min_pending_short_term_memories,
            max_pending_short_term_memories: dream_config.max_pending_short_term_memories,
            urgent,
            drain_in_progress: working,
            drain_completed: completed,
            drain_remaining: pending_count,
            drain_total,
            drain_progress,
        },
        dream_status: AdminDreamStatus {
            state: if working {
                "WORKING"
            } else if dream_config.enabled {
                "IDLE"
            } else {
                "DISABLED"
            }
            .into(),
            current_phase,
            progress: dream_progress,
            run_id,
            cycle_id,
            owner,
            started_at,
        },
    }))
}

async fn snapshot(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<AdminSnapshotResponse>, ApiError> {
    if !VIEWS.contains(&query.view.as_str()) {
        return Err(ApiError::bad_request(request_id, "unknown admin view"));
    }
    let api = AdminApi::new(
        SqliteFeedback {
            pool: state.pool.clone(),
        },
        state.pool.clone(),
    );
    let snapshot = api
        .snapshot(&query.view, query.selected_id.as_deref())
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Json(AdminSnapshotResponse { snapshot }))
}

struct AdminItem {
    row: AdminRow,
    body: String,
    fields: Vec<(String, String)>,
}

fn build_snapshot(
    view: &str,
    selected_id: Option<i64>,
    records: Vec<AdminItem>,
    selected_fallback: Option<AdminItem>,
) -> AdminSnapshot {
    let selected_record = selected_id
        .and_then(|selected_id| {
            records
                .iter()
                .find(|record| record.row.id.as_i64() == Some(selected_id))
        })
        .or(selected_fallback.as_ref());
    let selected = selected_record.map(|record| record.row.clone());
    let detail = selected_record.map_or_else(
        || AdminDetail {
            title: view.to_owned(),
            subtitle: "No record selected".into(),
            body: String::new(),
            fields: Vec::new(),
        },
        |record| AdminDetail {
            title: record.row.label.clone(),
            subtitle: format!("{} · {}", record.row.kind, record.row.status),
            body: record.body.clone(),
            fields: record.fields.clone(),
        },
    );
    AdminSnapshot {
        view: view.to_owned(),
        rows: records.into_iter().map(|record| record.row).collect(),
        selected,
        detail,
    }
}

#[derive(sqlx::FromRow)]
struct SelectedRecord {
    id: i64,
    kind: String,
    label: String,
    status: String,
    scope: String,
    language_pair: String,
    quality_label: String,
    tags_json: String,
    body: String,
}

async fn selected_item(
    pool: &SqlitePool,
    view: &str,
    id: i64,
) -> Result<Option<AdminItem>, sqlx::Error> {
    let query = match view {
        "Crystals" => {
            "SELECT id, 'Crystal' AS kind, title AS label, status, CASE WHEN scope_key = '' THEN scope_type ELSE scope_type || ': ' || scope_key END AS scope, CASE WHEN source_language = '' AND target_language = '' THEN '' ELSE source_language || ' → ' || target_language END AS language_pair, printf('%.0f%% strength · %.0f%% confidence', strength * 100, confidence * 100) AS quality_label, tags_json, text AS body FROM crystals WHERE id = ? AND crystal_type != 'lesson'"
        }
        "Lessons" => {
            "SELECT id, 'Lesson' AS kind, title AS label, status, CASE WHEN scope_key = '' THEN scope_type ELSE scope_type || ': ' || scope_key END AS scope, CASE WHEN source_language = '' AND target_language = '' THEN '' ELSE source_language || ' → ' || target_language END AS language_pair, printf('%.0f%% strength · %.0f%% confidence', strength * 100, confidence * 100) AS quality_label, tags_json, text AS body FROM crystals WHERE id = ? AND crystal_type = 'lesson'"
        }
        "Concepts" => {
            "SELECT id, 'Concept' AS kind, canonical_name AS label, status, CASE WHEN scope_key = '' THEN scope_type ELSE scope_type || ': ' || scope_key END AS scope, '' AS language_pair, printf('%.0f%% confidence', confidence * 100) AS quality_label, '[]' AS tags_json, description AS body FROM concepts WHERE id = ?"
        }
        "Proposals" => {
            "SELECT id, 'Proposal' AS kind, concept_text AS label, status, series_slug AS scope, CASE WHEN source_language = '' AND target_language = '' THEN '' ELSE source_language || ' → ' || target_language END AS language_pair, '' AS quality_label, '[]' AS tags_json, rationale AS body FROM concept_proposals WHERE id = ?"
        }
        "Short-Term Memories" => {
            "SELECT id, kind, text AS label, CASE WHEN archived_at IS NULL THEN 'pending' ELSE 'archived' END AS status, 'session ' || session_id AS scope, '' AS language_pair, coalesce(source_credibility, '') AS quality_label, '[]' AS tags_json, text AS body FROM short_term_memories WHERE id = ?"
        }
        "Short-Term Sessions" => {
            "SELECT id, 'Session' AS kind, volume || ' / ' || chapter AS label, status, series_slug AS scope, CASE WHEN source_language = '' AND target_language = '' THEN '' ELSE source_language || ' → ' || target_language END AS language_pair, task_type AS quality_label, '[]' AS tags_json, '' AS body FROM task_sessions WHERE id = ?"
        }
        "Dream Runs" => {
            "SELECT id, 'Dream Run' AS kind, 'Cycle ' || cycle_id AS label, status, provider AS scope, '' AS language_pair, created_crystal_count || ' crystals · ' || proposal_count || ' proposals' AS quality_label, '[]' AS tags_json, error AS body FROM dream_runs WHERE id = ?"
        }
        _ => return Ok(None),
    };
    sqlx::query_as::<_, SelectedRecord>(query)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|record| {
            record.map(|record| AdminItem {
                row: AdminRow {
                    id: record.id.into(),
                    kind: record.kind,
                    label: record.label,
                    status: record.status,
                    scope: record.scope,
                    language_pair: record.language_pair,
                    quality_label: record.quality_label,
                    tags: serde_json::from_str(&record.tags_json).unwrap_or_default(),
                },
                body: record.body,
                fields: Vec::new(),
            })
        })
}

async fn crystal_items(pool: &SqlitePool, lessons: bool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records = if lessons {
        sqlx::query_as::<_, CrystalRecord>(
            "SELECT * FROM crystals WHERE crystal_type = 'lesson' ORDER BY id DESC LIMIT ?",
        )
        .bind(SNAPSHOT_LIMIT)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, CrystalRecord>(
            "SELECT * FROM crystals WHERE crystal_type != 'lesson' ORDER BY id DESC LIMIT ?",
        )
        .bind(SNAPSHOT_LIMIT)
        .fetch_all(pool)
        .await?
    };
    Ok(records
        .into_iter()
        .map(|record| {
            let tags = serde_json::from_str(&record.tags_json).unwrap_or_default();
            let fields = vec![
                ("Type".into(), record.crystal_type.clone()),
                ("Scope".into(), scope(&record.scope_type, &record.scope_key)),
                ("Strength".into(), quality(record.strength)),
                ("Confidence".into(), quality(record.confidence)),
            ];
            AdminItem {
                row: AdminRow {
                    id: record.id.into(),
                    kind: if lessons { "Lesson" } else { "Crystal" }.into(),
                    label: record.title.clone(),
                    status: record.status,
                    scope: scope(&record.scope_type, &record.scope_key),
                    language_pair: language_pair(&record.source_language, &record.target_language),
                    quality_label: format!(
                        "{} strength · {} confidence",
                        quality(record.strength),
                        quality(record.confidence)
                    ),
                    tags,
                },
                body: record.text,
                fields,
            }
        })
        .collect())
}

async fn concept_items(pool: &SqlitePool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records =
        sqlx::query_as::<_, ConceptRecord>("SELECT * FROM concepts ORDER BY id DESC LIMIT ?")
            .bind(SNAPSHOT_LIMIT)
            .fetch_all(pool)
            .await?;
    Ok(records
        .into_iter()
        .map(|record| AdminItem {
            row: AdminRow {
                id: record.id.into(),
                kind: "Concept".into(),
                label: record.canonical_name.clone(),
                status: record.status,
                scope: scope(&record.scope_type, &record.scope_key),
                language_pair: String::new(),
                quality_label: format!("{} confidence", quality(record.confidence)),
                tags: Vec::new(),
            },
            body: record.description,
            fields: vec![
                ("Scope".into(), scope(&record.scope_type, &record.scope_key)),
                ("Confidence".into(), quality(record.confidence)),
            ],
        })
        .collect())
}

async fn proposal_items(pool: &SqlitePool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records = sqlx::query_as::<_, ConceptProposalRecord>(
        "SELECT * FROM concept_proposals ORDER BY id DESC LIMIT ?",
    )
    .bind(SNAPSHOT_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(records
        .into_iter()
        .map(|record| AdminItem {
            row: AdminRow {
                id: record.id.into(),
                kind: "Proposal".into(),
                label: record.concept_text.clone(),
                status: record.status,
                scope: record.series_slug.clone(),
                language_pair: language_pair(&record.source_language, &record.target_language),
                quality_label: String::new(),
                tags: Vec::new(),
            },
            body: record.rationale,
            fields: vec![
                ("Source form".into(), record.source_form),
                ("Canonical rendering".into(), record.canonical_rendering),
            ],
        })
        .collect())
}

async fn short_term_memory_items(pool: &SqlitePool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records = sqlx::query_as::<_, ShortTermMemoryRecord>(
        "SELECT * FROM short_term_memories ORDER BY id DESC LIMIT ?",
    )
    .bind(SNAPSHOT_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(records
        .into_iter()
        .map(|record| {
            let status = if record.archived_at.is_some() {
                "archived"
            } else {
                "pending"
            };
            AdminItem {
                row: AdminRow {
                    id: record.id.into(),
                    kind: record.kind.clone(),
                    label: record.text.clone(),
                    status: status.into(),
                    scope: format!("session {}", record.session_id),
                    language_pair: String::new(),
                    quality_label: record.source_credibility.clone().unwrap_or_default(),
                    tags: Vec::new(),
                },
                body: record.text,
                fields: vec![
                    ("Source".into(), record.source_role),
                    ("Source reference".into(), record.source_ref),
                ],
            }
        })
        .collect())
}

async fn session_items(pool: &SqlitePool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records = sqlx::query_as::<_, TaskSessionRecord>(
        "SELECT * FROM task_sessions ORDER BY id DESC LIMIT ?",
    )
    .bind(SNAPSHOT_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(records
        .into_iter()
        .map(|record| AdminItem {
            row: AdminRow {
                id: record.id.into(),
                kind: "Session".into(),
                label: format!("{} / {}", record.volume, record.chapter),
                status: record.status,
                scope: record.series_slug.clone(),
                language_pair: language_pair(&record.source_language, &record.target_language),
                quality_label: record.task_type.clone(),
                tags: Vec::new(),
            },
            body: String::new(),
            fields: vec![
                ("Series".into(), record.series_slug),
                ("Task".into(), record.task_type),
            ],
        })
        .collect())
}

async fn dream_run_items(pool: &SqlitePool) -> Result<Vec<AdminItem>, sqlx::Error> {
    let records =
        sqlx::query_as::<_, DreamRunRecord>("SELECT * FROM dream_runs ORDER BY id DESC LIMIT ?")
            .bind(SNAPSHOT_LIMIT)
            .fetch_all(pool)
            .await?;
    Ok(records
        .into_iter()
        .map(|record| AdminItem {
            row: AdminRow {
                id: record.id.into(),
                kind: "Dream Run".into(),
                label: format!("Cycle {}", record.cycle_id),
                status: record.status,
                scope: record.provider.clone(),
                language_pair: String::new(),
                quality_label: format!(
                    "{} crystals · {} proposals",
                    record.created_crystal_count, record.proposal_count
                ),
                tags: Vec::new(),
            },
            body: record.error,
            fields: vec![
                ("Provider".into(), record.provider),
                ("Inputs".into(), record.input_count.to_string()),
            ],
        })
        .collect())
}

fn scope(scope_type: &str, scope_key: &str) -> String {
    if scope_key.is_empty() {
        scope_type.to_owned()
    } else {
        format!("{scope_type}: {scope_key}")
    }
}

fn language_pair(source: &str, target: &str) -> String {
    if source.is_empty() && target.is_empty() {
        String::new()
    } else {
        format!("{source} → {target}")
    }
}

fn quality(value: f64) -> String {
    format!("{:.0}%", value * 100.0)
}

async fn run_action(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(action): Path<String>,
    request: Result<Json<AdminActionRequest>, JsonRejection>,
) -> Result<Json<AdminActionResult>, ApiError> {
    let Json(request) =
        request.map_err(|_| ApiError::bad_request(request_id.clone(), "invalid action request"))?;
    if matches!(
        action.as_str(),
        "deprecate_crystal" | "archive_concept" | "remove_short_term_memory" | "close_session"
    ) && request.confirmed != Some(true)
    {
        return Err(ApiError::bad_request(
            request_id,
            "action requires confirmation",
        ));
    }

    let result = match action.as_str() {
        "reinforce_crystal" | "decay_crystal" | "delete_crystal" => {
            let api = AdminApi::new(
                SqliteFeedback {
                    pool: state.pool.clone(),
                },
                state.pool.clone(),
            );
            let message = api
                .run_action(&action, request.clone())
                .await
                .map_err(|error| match error {
                    AdminError::ConfirmationRequired => {
                        ApiError::bad_request(request_id.clone(), "action requires confirmation")
                    }
                    AdminError::UnknownAction => {
                        ApiError::not_found(request_id.clone(), "unknown admin action")
                    }
                    AdminError::Feedback(error) => ApiError::internal(request_id.clone(), &error),
                    AdminError::Database(error) => ApiError::internal(request_id.clone(), &error),
                })?;
            action_result(&message.message, "Crystals")
        }
        "deprecate_crystal" => {
            CrystalStore::new(&state.pool)
                .archive(request.id.get())
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Crystal deprecated", "Crystals")
        }
        "approve_proposal" | "reject_proposal" => {
            let store = ConceptProposalStore::new(&state.pool);
            let result = if action == "approve_proposal" {
                store.approve(request.id.get()).await
            } else {
                store.reject(request.id.get()).await
            };
            result.map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result(
                if action == "approve_proposal" {
                    "Proposal approved"
                } else {
                    "Proposal rejected"
                },
                "Proposals",
            )
        }
        "archive_concept" => {
            ConceptStore::new(&state.pool)
                .archive(request.id.get(), "Archived from web admin")
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Concept archived", "Concepts")
        }
        "remove_short_term_memory" => {
            WorkspaceStore::new(&state.pool)
                .archive(request.id.get())
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Short-term memory removed", "Short-Term Memories")
        }
        "close_session" => {
            WorkspaceStore::new(&state.pool)
                .complete_session(request.id.get())
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Session closed", "Short-Term Sessions")
        }
        "reinforce_concept" | "decay_concept" => {
            let store = ConceptStore::new(&state.pool);
            let result = if action == "reinforce_concept" {
                store.reinforce(request.id.get()).await
            } else {
                store.decay(request.id.get()).await
            };
            result.map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result(
                if action == "reinforce_concept" {
                    "Concept reinforced"
                } else {
                    "Concept decayed"
                },
                "Concepts",
            )
        }
        _ => return Err(ApiError::not_found(request_id, "unknown admin action")),
    };
    let selected_id = request.id.get().to_string();
    let api = AdminApi::new(
        SqliteFeedback {
            pool: state.pool.clone(),
        },
        state.pool.clone(),
    );
    let mut result = result;
    result.snapshot = api
        .snapshot(&result.snapshot.view, Some(&selected_id))
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    state.events.notify_refresh();
    Ok(Json(result))
}

fn action_result(message: &str, view: &str) -> AdminActionResult {
    AdminActionResult {
        result: ActionMessage {
            message: message.into(),
        },
        snapshot: AdminSnapshot::empty(view),
    }
}

async fn run_manual_dreaming(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ManualDreamResponse>, ApiError> {
    use std::sync::atomic::Ordering;

    let config = state.config.clone();
    let (dream_config, catalog) = tokio::task::spawn_blocking(move || {
        let dream = hiero_core::dreaming::DreamConfig::load(&config)?;
        let catalog = hiero_core::provider::ProviderCatalog::load(config.provider_config_path())
            .map_err(hiero_core::dreaming::DreamConfigError::File)?;
        Ok::<_, hiero_core::dreaming::DreamConfigError>((dream, catalog))
    })
    .await
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?
    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;

    if state
        .dream_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(Json(ManualDreamResponse {
            started: false,
            status: "running".into(),
        }));
    }

    let cache = hiero_core::provider::ModelCache::new(
        128,
        1_000_000,
        std::time::Duration::from_secs(24 * 60 * 60),
    );
    let registry = std::sync::Arc::new(hiero_core::provider::ProviderRegistry::production(
        cache,
        hiero_core::provider::ReqwestTransportOptions::default(),
    ));
    let resolver_catalog = std::sync::Arc::new(catalog.clone());
    let resolver = std::sync::Arc::new(move |workflow: &hiero_core::dreaming::WorkflowProfile| {
        registry
            .resolve(&resolver_catalog, &workflow.provider, &workflow.model)
            .map(std::sync::Arc::from)
            .map_err(hiero_core::dreaming::DreamPhaseError::Provider)
    });
    let pool = state.pool.clone();
    let config = state.config.clone();
    let events = state.events.clone();
    let running = DreamRunningGuard(state.dream_running.clone());
    let accepted = state
        .workers
        .spawn_result(async move {
            let _running = running;
            let service = hiero_core::dreaming::DreamService::new_with_catalog(
                &pool,
                &config,
                &dream_config,
                resolver,
                catalog,
            )
            .with_cleanup_deadline(DAEMON_DREAM_CLEANUP_DEADLINE);
            let result = service
                .run_all(hiero_core::dreaming::CycleOptions {
                    owner: "web_admin".into(),
                    ..hiero_core::dreaming::CycleOptions::default()
                })
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from);
            events.notify_refresh();
            result
        })
        .await;
    if !accepted {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "shutting_down",
            "service is shutting down",
            request_id,
        ));
    }
    state.events.notify_refresh();
    Ok(Json(ManualDreamResponse {
        started: true,
        status: "running".into(),
    }))
}

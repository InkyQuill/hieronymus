use std::future::Future;

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
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
use serde_json::Value;
use sqlx::SqlitePool;

use crate::daemon::{AppState, RequestId};

use super::{
    contracts::{
        ActionMessage, AdminActionRequest, AdminActionResult, AdminDashboard, AdminDetail,
        AdminHeader, AdminRow, AdminSnapshot, AdminSnapshotResponse, ManualDreamResponse,
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
    ) -> Result<AdminActionResult, AdminError> {
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
        self.feedback
            .record(FeedbackEvent {
                crystal_id: request.id,
                event_type: event_type.into(),
                source_role: "web_admin".into(),
                evidence: Some(evidence.into()),
                session_id: None,
            })
            .await?;
        let selected_id = request.id.to_string();
        Ok(AdminActionResult {
            result: ActionMessage {
                message: message.into(),
            },
            snapshot: self.snapshot("Crystals", Some(&selected_id)).await?,
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
        Ok(build_snapshot(view, selected_id, records))
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
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT (SELECT count(*) FROM crystals), (SELECT count(*) FROM concepts), (SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL), (SELECT count(*) FROM task_sessions WHERE status = 'active')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Json(AdminDashboard {
        header: AdminHeader {
            product: "Hieronymus".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            tagline: "Local-first literary translation memory".into(),
        },
        stats: [
            ("crystals".into(), stats.0),
            ("concepts".into(), stats.1),
            ("pending_short_term_memories".into(), stats.2),
            ("active_sessions".into(), stats.3),
        ]
        .into(),
        views: VIEWS.into_iter().map(str::to_owned).collect(),
        short_term_status: [("pending".into(), Value::from(stats.2))].into(),
        dream_status: [(
            "state".into(),
            Value::from(
                if state
                    .dream_running
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    "RUNNING"
                } else {
                    "IDLE"
                },
            ),
        )]
        .into(),
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

fn build_snapshot(view: &str, selected_id: Option<&str>, records: Vec<AdminItem>) -> AdminSnapshot {
    let selected_id = selected_id.and_then(|value| value.parse::<i64>().ok());
    let selected_record = selected_id.and_then(|selected_id| {
        records
            .iter()
            .find(|record| record.row.id.as_i64() == Some(selected_id))
    });
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
    Json(request): Json<AdminActionRequest>,
) -> Result<Json<AdminActionResult>, ApiError> {
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
            let result =
                api.run_action(&action, request.clone())
                    .await
                    .map_err(|error| match error {
                        AdminError::ConfirmationRequired => ApiError::bad_request(
                            request_id.clone(),
                            "action requires confirmation",
                        ),
                        AdminError::UnknownAction => {
                            ApiError::not_found(request_id.clone(), "unknown admin action")
                        }
                        AdminError::Feedback(error) => {
                            ApiError::internal(request_id.clone(), &error)
                        }
                        AdminError::Database(error) => {
                            ApiError::internal(request_id.clone(), &error)
                        }
                    })?;
            if action == "delete_crystal" {
                CrystalStore::new(&state.pool)
                    .archive(request.id)
                    .await
                    .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            }
            result
        }
        "deprecate_crystal" => {
            CrystalStore::new(&state.pool)
                .archive(request.id)
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Crystal deprecated", "Crystals")
        }
        "approve_proposal" | "reject_proposal" => {
            let store = ConceptProposalStore::new(&state.pool);
            let result = if action == "approve_proposal" {
                store.approve(request.id).await
            } else {
                store.reject(request.id).await
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
                .archive(request.id, "Archived from web admin")
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Concept archived", "Concepts")
        }
        "remove_short_term_memory" => {
            WorkspaceStore::new(&state.pool)
                .archive(request.id)
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Short-term memory removed", "Short-Term Memories")
        }
        "close_session" => {
            WorkspaceStore::new(&state.pool)
                .complete_session(request.id)
                .await
                .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
            action_result("Session closed", "Short-Term Sessions")
        }
        "reinforce_concept" | "decay_concept" => {
            let store = ConceptStore::new(&state.pool);
            let result = if action == "reinforce_concept" {
                store.reinforce(request.id).await
            } else {
                store.decay(request.id).await
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
    let selected_id = request.id.to_string();
    let api = AdminApi::new(
        SqliteFeedback {
            pool: state.pool.clone(),
        },
        state.pool,
    );
    let mut result = result;
    result.snapshot = api
        .snapshot(&result.snapshot.view, Some(&selected_id))
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
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
    let running = DreamRunningGuard(state.dream_running.clone());
    tokio::spawn(async move {
        let _running = running;
        let service = hiero_core::dreaming::DreamService::new_with_catalog(
            &pool,
            &config,
            &dream_config,
            resolver,
            catalog,
        );
        if let Err(error) = service
            .run_all(hiero_core::dreaming::CycleOptions {
                owner: "web_admin".into(),
                ..hiero_core::dreaming::CycleOptions::default()
            })
            .await
        {
            tracing::error!(%error, "manual dream run failed");
        }
    });
    Ok(Json(ManualDreamResponse {
        started: true,
        status: "running".into(),
    }))
}

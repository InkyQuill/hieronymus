use std::time::Duration;

use axum::{
    Json,
    extract::{Extension, State},
};
use hiero_core::doctor::{DoctorReport, run_doctor};
use serde::Serialize;

use crate::daemon::{AppState, RequestId};

use super::error::ApiError;

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    ok: bool,
    service: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct StatusResponse {
    running: bool,
    doctor: DoctorReport,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShutdownResponse {
    ok: bool,
    stopping: bool,
}

pub(crate) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "hieronymus",
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub(crate) async fn status(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<StatusResponse>, ApiError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(|error| ApiError::internal(request_id.clone(), &error))?;
    let doctor = tokio::time::timeout(Duration::from_secs(5), run_doctor(&state.config))
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Json(StatusResponse {
        running: true,
        doctor,
    }))
}

pub(crate) async fn shutdown(State(state): State<AppState>) -> Json<ShutdownResponse> {
    let _ = state.shutdown.send(());
    Json(ShutdownResponse {
        ok: true,
        stopping: true,
    })
}

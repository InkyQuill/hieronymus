use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::daemon::RequestId;

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: &'static str,
    code: &'static str,
    request_id: String,
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: RequestId,
}

impl ApiError {
    pub(crate) fn new(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        request_id: RequestId,
    ) -> Self {
        Self {
            status,
            code,
            message,
            request_id,
        }
    }

    pub(crate) fn internal(
        request_id: RequestId,
        source: &(impl std::fmt::Display + ?Sized),
    ) -> Self {
        tracing::error!(request_id = %request_id.0, error = %source, "request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
            request_id,
        )
    }

    pub(crate) fn bad_request(request_id: RequestId, message: &'static str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            message,
            request_id,
        )
    }

    pub(crate) fn not_found(request_id: RequestId, message: &'static str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message, request_id)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: self.message,
                code: self.code,
                request_id: self.request_id.0,
            }),
        )
            .into_response()
    }
}

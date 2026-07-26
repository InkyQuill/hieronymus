pub mod error;
pub mod system;

use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::daemon::RequestId;

use self::error::ApiError;

pub(crate) async fn placeholder(Extension(request_id): Extension<RequestId>) -> Response {
    ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "route is not implemented yet",
        request_id,
    )
    .into_response()
}

pub(crate) async fn not_found(Extension(request_id): Extension<RequestId>) -> Response {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "route not found",
        request_id,
    )
    .into_response()
}

pub(crate) async fn method_not_allowed(Extension(request_id): Extension<RequestId>) -> Response {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "method not allowed",
        request_id,
    )
    .into_response()
}

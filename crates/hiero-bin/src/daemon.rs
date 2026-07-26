use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{
        HeaderValue, StatusCode,
        header::{HOST, ORIGIN},
        uri::Authority,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use hiero_core::{config::HieronymusConfig, db};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::api::{
    self,
    error::ApiError,
    system::{health, shutdown, status},
};

const BODY_LIMIT: usize = 1_000_000;
const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<HieronymusConfig>,
    pub shutdown: broadcast::Sender<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct RequestId(pub(crate) String);

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(api::placeholder))
        .route("/admin", get(api::placeholder))
        .route("/config", get(api::placeholder))
        .route("/assets/{*path}", get(api::placeholder))
        .route("/api/mcp/{operation}", post(api::placeholder))
        .route("/api/{*path}", any(api::placeholder))
        .route("/ws/admin", get(api::placeholder))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/shutdown", post(shutdown))
        .route("/mcp", any(api::placeholder))
        .method_not_allowed_fallback(api::method_not_allowed)
        .fallback(api::not_found)
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(assign_request_id))
                .layer(TraceLayer::new_for_http())
                .layer(middleware::from_fn(limit_request_body))
                .layer(middleware::from_fn(validate_host_and_origin)),
        )
        .with_state(state)
}

pub async fn serve<S>(config: HieronymusConfig, port: u16, shutdown: S) -> Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    ensure!(port != 0, "daemon port 0 is not allowed");
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind daemon to {address}"))?;
    let pool = db::connect(&config)
        .await
        .context("failed to open daemon database")?;
    let (shutdown_sender, mut shutdown_receiver) = broadcast::channel(4);
    let router = build_router(AppState {
        pool,
        config: Arc::new(config),
        shutdown: shutdown_sender,
    });

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            tokio::select! {
                () = shutdown => {}
                _ = shutdown_receiver.recv() => {}
            }
        })
        .await
        .context("daemon server failed")
}

async fn assign_request_id(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map_or_else(|| Uuid::new_v4().to_string(), ToOwned::to_owned);
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        REQUEST_ID_HEADER,
        HeaderValue::from_str(&request_id).expect("validated or generated request ID is a header"),
    );
    response
}

async fn limit_request_body(request: Request, next: Next) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .expect("request ID middleware runs before the body limit");
    let (parts, body) = request.into_parts();
    match to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Err(_) => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "request body exceeds 1000000 bytes",
            request_id,
        )
        .into_response(),
    }
}

async fn validate_host_and_origin(request: Request, next: Next) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .expect("request ID middleware runs before security validation");
    let Some(host) = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Authority::from_str(value).ok())
    else {
        return forbidden(request_id);
    };
    if !is_loopback_authority(&host) {
        return forbidden(request_id);
    }

    if let Some(origin) = request.headers().get(ORIGIN) {
        let valid_origin = origin
            .to_str()
            .ok()
            .and_then(|value| value.parse::<axum::http::Uri>().ok())
            .filter(|uri| uri.scheme_str() == Some("http"))
            .and_then(|uri| uri.authority().cloned())
            .is_some_and(|origin| same_authority(&host, &origin));
        if !valid_origin {
            return forbidden(request_id);
        }
    }

    next.run(request).await
}

fn forbidden(request_id: RequestId) -> Response {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "request host or origin is not allowed",
        request_id,
    )
    .into_response()
}

fn is_loopback_authority(authority: &Authority) -> bool {
    authority.host().eq_ignore_ascii_case("127.0.0.1")
        || authority.host().eq_ignore_ascii_case("localhost")
}

fn same_authority(host: &Authority, origin: &Authority) -> bool {
    is_loopback_authority(origin)
        && host.host().eq_ignore_ascii_case(origin.host())
        && host.port_u16() == origin.port_u16()
}

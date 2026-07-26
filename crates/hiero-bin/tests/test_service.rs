use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use hiero_bin::daemon::{AppState, build_router, serve};
use hiero_core::{config::HieronymusConfig, db};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::{broadcast, oneshot};
use tower::ServiceExt;

const LOCAL_HOST: &str = "127.0.0.1:9768";
const BODY_LIMIT: usize = 1_000_000;

async fn test_state() -> (AppState, broadcast::Receiver<()>) {
    let pool = db::connect_url("sqlite::memory:?cache=shared")
        .await
        .expect("test database should connect");
    let temp = TempDir::new().expect("temporary directory should be created");
    let config =
        HieronymusConfig::load(Some(temp.path().join("data"))).expect("test config should load");
    let (shutdown, receiver) = broadcast::channel(4);
    (
        AppState {
            pool,
            config: Arc::new(config),
            shutdown,
        },
        receiver,
    )
}

fn request(method: Method, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, LOCAL_HOST)
        .body(body)
        .expect("test request should build")
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response should contain JSON")
}

#[tokio::test]
async fn router_registers_every_phase_005_section_2_route() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let routes = [
        (Method::GET, "/", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/admin", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/config", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/assets/app.js", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/api/providers", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/ws/admin", StatusCode::NOT_IMPLEMENTED),
        (Method::GET, "/health", StatusCode::OK),
        (Method::GET, "/status", StatusCode::OK),
        (Method::POST, "/shutdown", StatusCode::OK),
        (Method::POST, "/mcp", StatusCode::NOT_IMPLEMENTED),
        (
            Method::POST,
            "/api/mcp/series_create",
            StatusCode::NOT_IMPLEMENTED,
        ),
    ];

    for (method, path, expected) in routes {
        let response = router
            .clone()
            .oneshot(request(method, path, Body::empty()))
            .await
            .expect("router should answer");
        assert_eq!(response.status(), expected, "{path}");
    }
}

#[tokio::test]
async fn router_security_enforces_the_loopback_host_and_origin_matrix() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let cases = [
        (Method::GET, LOCAL_HOST, None, "/health", StatusCode::OK),
        (
            Method::GET,
            LOCAL_HOST,
            Some("http://127.0.0.1:9768"),
            "/health",
            StatusCode::OK,
        ),
        (
            Method::GET,
            "evil.example",
            None,
            "/health",
            StatusCode::FORBIDDEN,
        ),
        (
            Method::GET,
            LOCAL_HOST,
            Some("https://evil.example"),
            "/health",
            StatusCode::FORBIDDEN,
        ),
        (
            Method::GET,
            LOCAL_HOST,
            Some("http://localhost:9768"),
            "/health",
            StatusCode::FORBIDDEN,
        ),
        (
            Method::POST,
            LOCAL_HOST,
            None,
            "/mcp",
            StatusCode::NOT_IMPLEMENTED,
        ),
    ];

    for (method, host, origin, path, expected) in cases {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let response = router
            .clone()
            .oneshot(
                builder
                    .body(Body::empty())
                    .expect("test request should build"),
            )
            .await
            .expect("router should answer");
        assert_eq!(response.status(), expected, "host={host} origin={origin:?}");
    }
}

#[tokio::test]
async fn router_rejects_a_request_body_larger_than_one_megabyte() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let accepted = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/mcp/probe",
            Body::from(vec![b'a'; BODY_LIMIT]),
        ))
        .await
        .expect("router should answer");
    assert_eq!(accepted.status(), StatusCode::NOT_IMPLEMENTED);

    let rejected = router
        .oneshot(request(
            Method::POST,
            "/api/mcp/probe",
            Body::from(vec![b'a'; BODY_LIMIT + 1]),
        ))
        .await
        .expect("router should answer");
    assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response_json(rejected).await["error"]["code"],
        "payload_too_large"
    );
}

#[tokio::test]
async fn router_preserves_or_generates_request_ids_on_every_response() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let supplied = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/missing")
                .header(header::HOST, LOCAL_HOST)
                .header("x-request-id", "test-request-123")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(
        supplied.headers()["x-request-id"],
        "test-request-123",
        "the caller's correlation ID should be preserved"
    );
    assert_eq!(
        response_json(supplied).await["error"]["request_id"],
        "test-request-123"
    );

    let generated = router
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    let generated_id = generated.headers()["x-request-id"]
        .to_str()
        .expect("request ID should be ASCII");
    assert!(
        uuid::Uuid::parse_str(generated_id).is_ok(),
        "generated request ID should be a UUID"
    );
}

#[tokio::test]
async fn router_health_is_cheap_and_status_reports_doctor_checks() {
    let (state, _) = test_state().await;
    let router = build_router(state.clone());

    let health = router
        .clone()
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        response_json(health).await,
        json!({
            "ok": true,
            "service": "hieronymus",
            "version": env!("CARGO_PKG_VERSION"),
        })
    );

    let status = router
        .clone()
        .oneshot(request(Method::GET, "/status", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(status.status(), StatusCode::OK);
    let payload = response_json(status).await;
    assert_eq!(payload["running"], true);
    assert!(payload["doctor"]["checks"].is_array());

    state.pool.close().await;
    let health_after_close = router
        .oneshot(request(Method::GET, "/health", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(health_after_close.status(), StatusCode::OK);
}

#[tokio::test]
async fn router_shutdown_signals_only_after_request_authorization() {
    let (state, mut shutdown) = test_state().await;
    let router = build_router(state);

    let rejected = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/shutdown")
                .header(header::HOST, "evil.example")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), shutdown.recv())
            .await
            .is_err()
    );

    let accepted = router
        .oneshot(request(Method::POST, "/shutdown", Body::empty()))
        .await
        .expect("router should answer");
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        response_json(accepted).await,
        json!({"ok": true, "stopping": true})
    );
    tokio::time::timeout(Duration::from_secs(1), shutdown.recv())
        .await
        .expect("shutdown signal should arrive")
        .expect("shutdown channel should remain open");
}

#[tokio::test]
async fn router_internal_errors_are_sanitized_and_correlated() {
    let (state, _) = test_state().await;
    state.pool.close().await;
    let router = build_router(state);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/status")
                .header(header::HOST, LOCAL_HOST)
                .header("x-request-id", "closed-pool-request")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(
        body,
        json!({
            "error": {
                "code": "internal_error",
                "message": "internal server error",
                "request_id": "closed-pool-request",
            }
        })
    );
    assert!(!body.to_string().contains("pool closed"));
}

#[tokio::test]
async fn router_method_errors_use_the_stable_json_envelope() {
    let (state, _) = test_state().await;
    let router = build_router(state);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/health")
                .header(header::HOST, LOCAL_HOST)
                .header("x-request-id", "wrong-method-request")
                .body(Body::empty())
                .expect("test request should build"),
        )
        .await
        .expect("router should answer");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response_json(response).await,
        json!({
            "error": {
                "code": "method_not_allowed",
                "message": "method not allowed",
                "request_id": "wrong-method-request",
            }
        })
    );
}

#[tokio::test]
async fn router_serve_binds_only_to_ipv4_loopback_and_shuts_down_gracefully() {
    let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = reservation
        .local_addr()
        .expect("listener should have an address")
        .port();
    drop(reservation);

    let temp = TempDir::new().expect("temporary directory should be created");
    let config =
        HieronymusConfig::load(Some(temp.path().join("data"))).expect("test config should load");
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(serve(config, port, async move {
        let _ = stopped.await;
    }));

    let stream = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                break stream;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("server should accept loopback connections");
    assert_eq!(
        stream
            .local_addr()
            .expect("client should have a local address")
            .ip()
            .to_string(),
        "127.0.0.1"
    );
    drop(stream);

    stop.send(())
        .expect("shutdown receiver should remain alive");
    server
        .await
        .expect("server task should join")
        .expect("server should shut down cleanly");
}

#[tokio::test]
async fn router_serve_fails_fast_when_the_requested_port_is_occupied() {
    let occupied = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = occupied
        .local_addr()
        .expect("listener should have an address")
        .port();
    let temp = TempDir::new().expect("temporary directory should be created");
    let config =
        HieronymusConfig::load(Some(temp.path().join("data"))).expect("test config should load");

    let error = serve(config, port, std::future::pending())
        .await
        .expect_err("occupied port must fail startup");
    let message = format!("{error:#}");
    assert!(message.contains(&format!("127.0.0.1:{port}")));
    assert!(
        message.contains("in use") || message.contains("Address already in use"),
        "{message}"
    );
}

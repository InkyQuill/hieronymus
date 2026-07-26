use std::{sync::Arc, time::Duration};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use hiero_bin::daemon::{AppState, bind_listener, build_router, load_or_create_auth_token, serve};
use hiero_core::{config::HieronymusConfig, db};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tower::ServiceExt;

const LOCAL_HOST: &str = "127.0.0.1:9768";
const BODY_LIMIT: usize = 1_000_000;
const AUTH_TOKEN: &str = "test-auth-token";

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
            auth_token: Arc::from(AUTH_TOKEN),
            port: 9768,
        },
        receiver,
    )
}

fn request(method: Method, path: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, LOCAL_HOST)
        .header("x-hieronymus-token", AUTH_TOKEN)
        .body(body)
        .expect("test request should build")
}

#[derive(Clone, Copy)]
struct SecurityCase {
    name: &'static str,
    host: Option<&'static str>,
    origin: Option<&'static str>,
    token: Option<&'static str>,
    status: StatusCode,
    error_code: Option<&'static str>,
}

fn security_request(method: Method, path: &str, case: SecurityCase) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(host) = case.host {
        builder = builder.header(header::HOST, host);
    }
    if let Some(origin) = case.origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(token) = case.token {
        builder = builder.header("x-hieronymus-token", token);
    }
    builder
        .body(Body::empty())
        .expect("security test request should build")
}

async fn assert_security_response(response: Response, case: SecurityCase) {
    assert_eq!(response.status(), case.status, "{}", case.name);
    if let Some(error_code) = case.error_code {
        assert_eq!(
            response_json(response).await["error"]["code"],
            error_code,
            "{}",
            case.name
        );
    }
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
async fn router_mcp_security_matrix_is_route_complete() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let cases = [
        SecurityCase {
            name: "MCP correct token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::NOT_IMPLEMENTED,
            error_code: Some("not_implemented"),
        },
        SecurityCase {
            name: "MCP missing token",
            host: Some(LOCAL_HOST),
            origin: None,
            token: None,
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "MCP wrong token",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some("wrong-token"),
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "MCP missing Host",
            host: None,
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP malformed Host authority",
            host: Some("127.0.0.1:not-a-port"),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP wrong loopback Host port",
            host: Some("127.0.0.1:9999"),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "MCP same Origin",
            host: Some(LOCAL_HOST),
            origin: Some("http://127.0.0.1:9768"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::NOT_IMPLEMENTED,
            error_code: Some("not_implemented"),
        },
        SecurityCase {
            name: "MCP foreign Origin",
            host: Some(LOCAL_HOST),
            origin: Some("https://evil.example"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
    ];

    for case in cases {
        let response = router
            .clone()
            .oneshot(security_request(Method::POST, "/mcp", case))
            .await
            .expect("router should answer");
        assert_security_response(response, case).await;
    }
}

#[tokio::test]
async fn router_browser_admin_security_matrix_is_route_complete() {
    let (state, _) = test_state().await;
    let router = build_router(state);
    let cases = [
        SecurityCase {
            name: "admin correct token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some(AUTH_TOKEN),
            status: StatusCode::NOT_IMPLEMENTED,
            error_code: Some("not_implemented"),
        },
        SecurityCase {
            name: "admin missing token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: None,
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "admin wrong token and missing Origin",
            host: Some(LOCAL_HOST),
            origin: None,
            token: Some("wrong-token"),
            status: StatusCode::UNAUTHORIZED,
            error_code: Some("unauthorized"),
        },
        SecurityCase {
            name: "admin missing Host",
            host: None,
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin malformed Host authority",
            host: Some("127.0.0.1:not-a-port"),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin wrong loopback Host port",
            host: Some("127.0.0.1:9999"),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
        SecurityCase {
            name: "admin same Origin without token",
            host: Some(LOCAL_HOST),
            origin: Some("http://127.0.0.1:9768"),
            token: None,
            status: StatusCode::NOT_IMPLEMENTED,
            error_code: Some("not_implemented"),
        },
        SecurityCase {
            name: "admin foreign Origin with correct token",
            host: Some(LOCAL_HOST),
            origin: Some("https://evil.example"),
            token: Some(AUTH_TOKEN),
            status: StatusCode::FORBIDDEN,
            error_code: Some("forbidden"),
        },
    ];

    for case in cases {
        let response = router
            .clone()
            .oneshot(security_request(Method::GET, "/api/providers", case))
            .await
            .expect("router should answer");
        assert_security_response(response, case).await;
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
                .header("x-hieronymus-token", AUTH_TOKEN)
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
async fn router_shutdown_security_matrix_is_route_complete_and_side_effect_safe() {
    let cases = [
        (
            SecurityCase {
                name: "shutdown correct token and missing Origin",
                host: Some(LOCAL_HOST),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::OK,
                error_code: None,
            },
            true,
        ),
        (
            SecurityCase {
                name: "shutdown missing token",
                host: Some(LOCAL_HOST),
                origin: None,
                token: None,
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown wrong token",
                host: Some(LOCAL_HOST),
                origin: None,
                token: Some("wrong-token"),
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown missing Host",
                host: None,
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown malformed Host authority",
                host: Some("127.0.0.1:not-a-port"),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown wrong loopback Host port",
                host: Some("127.0.0.1:9999"),
                origin: None,
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown same Origin with correct token",
                host: Some(LOCAL_HOST),
                origin: Some("http://127.0.0.1:9768"),
                token: Some(AUTH_TOKEN),
                status: StatusCode::OK,
                error_code: None,
            },
            true,
        ),
        (
            SecurityCase {
                name: "shutdown same Origin without token",
                host: Some(LOCAL_HOST),
                origin: Some("http://127.0.0.1:9768"),
                token: None,
                status: StatusCode::UNAUTHORIZED,
                error_code: Some("unauthorized"),
            },
            false,
        ),
        (
            SecurityCase {
                name: "shutdown foreign Origin with correct token",
                host: Some(LOCAL_HOST),
                origin: Some("https://evil.example"),
                token: Some(AUTH_TOKEN),
                status: StatusCode::FORBIDDEN,
                error_code: Some("forbidden"),
            },
            false,
        ),
    ];

    for (case, should_signal) in cases {
        let (state, mut shutdown) = test_state().await;
        let response = build_router(state)
            .oneshot(security_request(Method::POST, "/shutdown", case))
            .await
            .expect("router should answer");
        if case.error_code.is_some() {
            assert_security_response(response, case).await;
        } else {
            assert_eq!(response.status(), case.status, "{}", case.name);
            assert_eq!(
                response_json(response).await,
                json!({"ok": true, "stopping": true}),
                "{}",
                case.name
            );
        }

        let signal = tokio::time::timeout(Duration::from_millis(20), shutdown.recv()).await;
        let received_shutdown = matches!(signal, Ok(Ok(())));
        assert_eq!(received_shutdown, should_signal, "{}", case.name);
    }
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
                .header("x-hieronymus-token", AUTH_TOKEN)
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
                .header("x-hieronymus-token", AUTH_TOKEN)
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
async fn router_binding_helper_exposes_the_exact_ipv4_loopback_address() {
    let reservation = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("test port should be reservable");
    let port = reservation
        .local_addr()
        .expect("listener should have an address")
        .port();
    drop(reservation);

    let listener = bind_listener(port)
        .await
        .expect("daemon listener should bind");
    assert_eq!(
        listener.local_addr().expect("listener has an address"),
        format!("127.0.0.1:{port}")
            .parse()
            .expect("expected address should parse")
    );
}

#[test]
fn router_auth_token_file_is_created_once_with_private_permissions() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let path = temp.path().join("config").join("auth-token");

    let first = load_or_create_auth_token(&path).expect("token should be created");
    let second = load_or_create_auth_token(&path).expect("token should be reused");

    assert_eq!(first, second);
    assert!(first.len() >= 64);
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(path)
            .expect("token metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn router_auth_token_file_rejects_non_private_existing_permissions() {
    let temp = TempDir::new().expect("temporary directory should be created");
    let path = temp.path().join("auth-token");
    std::fs::write(&path, AUTH_TOKEN).expect("fixture token should be written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("fixture permissions should be set");

    let error = load_or_create_auth_token(&path).expect_err("public token file must be rejected");

    assert!(error.to_string().contains("0600"));
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

//! Route handling for the daemon's HTTP surface:
//! - `GET /health` — minimal unauthenticated liveness (no data);
//! - `POST /mcp` — bearer-token authenticated stateless MCP (JSON or
//!   request-scoped SSE);
//! - `POST /shutdown` — authenticated graceful shutdown;
//! - `/status`, `/auth/*`, `/api/*` — the REST surface with launch-grant
//!   session auth (see `rest`);
//! - `GET /ws/admin` — the admin websocket (see `ws`);
//! - `/`, `/admin(/..)`, `/config(/..)`, `/assets/..` — the unauthenticated,
//!   Host-validated static SPA routes over the embedded asset set (see
//!   `assets`).
//!
//! Check order on authenticated routes mirrors the frozen route cases:
//! method/path → Host → credential → protocol rules.

use serde_json::{Value, json};

use super::DaemonRuntime;
use super::assets;
use super::http::{Request, Response, header};
use super::protocol;
use super::registry::PROTOCOL_REVISION;
use super::rest;
use super::ws;

/// The outcome of routing one request: a plain response, or an upgraded
/// connection the accept loop must hand to the websocket session.
pub(crate) enum Dispatch {
    Respond(Response),
    Upgrade(ws::Session),
}

pub(crate) fn dispatch(request: &Request, runtime: &DaemonRuntime) -> Dispatch {
    let (path, query) = split_target(&request.target);
    match (request.method.as_str(), path.as_str()) {
        ("GET", "/health") => Dispatch::Respond(handle_health(request, runtime)),
        ("POST", "/mcp") => Dispatch::Respond(handle_mcp(request, runtime)),
        ("POST", "/shutdown") => Dispatch::Respond(handle_shutdown(request, runtime)),
        // The native routes are single-method; anything else is not a route.
        (_, "/health") | (_, "/mcp") | (_, "/shutdown") => Dispatch::Respond(not_found()),
        ("GET", "/ws/admin") => ws::handle(request, runtime),
        (_, "/ws/admin") => Dispatch::Respond(not_found()),
        // Static SPA routes: unauthenticated, Host-validated only.
        ("GET", "/") => Dispatch::Respond(handle_static_index(request, runtime)),
        ("GET", path) if is_client_side_route(path) => {
            Dispatch::Respond(handle_static_index(request, runtime))
        }
        ("GET", path) if path.starts_with("/assets/") => {
            Dispatch::Respond(handle_static_asset(request, runtime, path))
        }
        (_, "/") => Dispatch::Respond(not_found()),
        (_, path) if is_client_side_route(path) || path.starts_with("/assets/") => {
            Dispatch::Respond(not_found())
        }
        (_, other) => Dispatch::Respond(rest::handle(request, other, &query, runtime)),
    }
}

/// The SPA's client-side routes: unknown deep links under them fall back to
/// the embedded `index.html` (spec §HTTP And Frontend Contracts).
fn is_client_side_route(path: &str) -> bool {
    path == "/admin"
        || path.starts_with("/admin/")
        || path == "/config"
        || path.starts_with("/config/")
}

/// Split the request target into path and query (`/api/x?a=b`); no percent
/// decoding, matching the Python route dispatch.
fn split_target(target: &str) -> (String, String) {
    match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.to_string(), String::new()),
    }
}

fn host_is_valid(request: &Request, runtime: &DaemonRuntime) -> bool {
    header(&request.headers, "host") == Some(runtime.bound_address.to_string().as_str())
}

fn bearer_matches(request: &Request, runtime: &DaemonRuntime) -> bool {
    let expected = format!("Bearer {}", runtime.bearer.expose_secret());
    header(&request.headers, "authorization") == Some(expected.as_str())
}

fn not_found() -> Response {
    Response::json(404, &json!({"error": "not_found"}))
}

fn handle_health(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return Response::json(400, &json!({"error": "invalid_host"}));
    }
    // Minimal by contract: no versions, no paths, no user data.
    Response::json(200, &json!({"ok": true}))
}

/// `GET /` and every unknown client-side route: the asset set's
/// `index.html`; without a console build the Python
/// `web_console_not_built` outcome stands.
fn handle_static_index(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return rest::invalid_host();
    }
    match runtime.assets.lookup("index.html") {
        Some(asset) => asset_response(asset),
        None => Response::json(404, &json!({"error": "web_console_not_built"})),
    }
}

/// `GET /assets/{path}`: that exact asset, or 404 for missing actual assets
/// (never an index fallback). The URL path maps into the asset set
/// root-relative (`/assets/app.js` -> `assets/app.js`), matching the Python
/// asset dispatch.
fn handle_static_asset(request: &Request, runtime: &DaemonRuntime, path: &str) -> Response {
    if !host_is_valid(request, runtime) {
        return rest::invalid_host();
    }
    let name = path.strip_prefix('/').filter(|name| !name.is_empty());
    match name.and_then(|name| runtime.assets.lookup(name)) {
        Some(asset) => asset_response(asset),
        None => not_found(),
    }
}

fn asset_response(asset: assets::Asset) -> Response {
    Response {
        status: 200,
        content_type: asset.content_type,
        body: asset.body,
        extra_headers: Vec::new(),
    }
}

fn handle_shutdown(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return Response::json(400, &json!({"error": "invalid_host"}));
    }
    if !bearer_matches(request, runtime) {
        return Response::json(401, &json!({"error": "unauthorized"}));
    }
    runtime
        .stop
        .store(true, std::sync::atomic::Ordering::Release);
    Response::json(200, &json!({"ok": true, "stopping": true}))
}

fn handle_mcp(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return Response::json(400, &json!({"error": "invalid_host"}));
    }
    if !bearer_matches(request, runtime) {
        return Response::json(401, &json!({"error": "unauthorized"}));
    }
    let content_type_is_json = header(&request.headers, "content-type")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
    if !content_type_is_json {
        return Response::json(
            400,
            &protocol::error_response(Value::Null, -32602, "Invalid request metadata", None),
        );
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(_) => {
            return Response::json(
                400,
                &protocol::error_response(Value::Null, -32700, "Parse error", None),
            );
        }
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    if header(&request.headers, "mcp-session-id").is_some()
        || header(&request.headers, "last-event-id").is_some()
    {
        return Response::json(400, &protocol::invalid_params(id));
    }
    if let Some(message) = protocol::mirrored_header_error(
        &|name| header(&request.headers, name).map(str::to_string),
        &body,
        &runtime.registry,
    ) {
        return Response::json(400, &protocol::error_response(id, -32020, &message, None));
    }
    if protocol::validate_request(&body, true).is_err() {
        return Response::json(400, &protocol::invalid_params(id));
    }
    let requested_version = body
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str);
    match requested_version {
        Some(version) if version == PROTOCOL_REVISION => {}
        Some(version) => {
            return Response::json(400, &protocol::unsupported_version_response(id, version));
        }
        None => return Response::json(400, &protocol::invalid_params(id)),
    }
    let response = protocol::process_request(&runtime.registry, &body);
    let is_error = response.get("error").is_some();
    let status = if is_error { 400 } else { 200 };
    let wants_sse = header(&request.headers, "accept")
        .is_some_and(|accept| accept.trim().eq_ignore_ascii_case("text/event-stream"));
    if wants_sse && !is_error {
        Response::sse(&response)
    } else {
        Response::json(status, &response)
    }
}

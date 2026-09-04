//! Route handling for the daemon's skeleton HTTP surface:
//! - `GET /health` — minimal unauthenticated liveness (no data);
//! - `POST /mcp` — bearer-token authenticated stateless MCP (JSON or
//!   request-scoped SSE);
//! - `POST /shutdown` — authenticated graceful shutdown.
//!
//! Check order on authenticated routes mirrors the frozen route cases:
//! method/path → Host → bearer → protocol rules.

use serde_json::{json, Value};

use super::http::{header, Request, Response};
use super::protocol;
use super::registry::PROTOCOL_REVISION;
use super::DaemonRuntime;

pub(crate) fn handle(request: &Request, runtime: &DaemonRuntime) -> Response {
    match (request.method.as_str(), request.target.as_str()) {
        ("GET", "/health") => handle_health(request, runtime),
        ("POST", "/mcp") => handle_mcp(request, runtime),
        ("POST", "/shutdown") => handle_shutdown(request, runtime),
        _ => Response::json(404, &json!({"error": "not_found"})),
    }
}

fn host_is_valid(request: &Request, runtime: &DaemonRuntime) -> bool {
    header(&request.headers, "host") == Some(runtime.bound_address.to_string().as_str())
}

fn bearer_matches(request: &Request, runtime: &DaemonRuntime) -> bool {
    let expected = format!("Bearer {}", runtime.bearer.expose_secret());
    header(&request.headers, "authorization") == Some(expected.as_str())
}

fn handle_health(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return Response::json(400, &json!({"error": "invalid_host"}));
    }
    // Minimal by contract: no versions, no paths, no user data.
    Response::json(200, &json!({"ok": true}))
}

fn handle_shutdown(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return Response::json(400, &json!({"error": "invalid_host"}));
    }
    if !bearer_matches(request, runtime) {
        return Response::json(401, &json!({"error": "unauthorized"}));
    }
    runtime.stop.store(true, std::sync::atomic::Ordering::Release);
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
    if let Some(message) =
        protocol::mirrored_header_error(&|name| {
            header(&request.headers, name).map(str::to_string)
        }, &body, &runtime.registry)
    {
        return Response::json(
            400,
            &protocol::error_response(id, -32020, &message, None),
        );
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
            return Response::json(
                400,
                &protocol::unsupported_version_response(id, version),
            );
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

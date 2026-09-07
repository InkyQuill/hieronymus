//! The REST surface of the daemon (ADR 0012 as amended 2026-09-03, ADR 0015):
//!
//! - `GET /status` — native, bearer + Host;
//! - `POST /recall/feedback` — native, bearer + Host: recall-feedback store
//!   surface (ADR 0011; one new route, outside the frozen registry);
//! - `POST /auth/launch-grant` — native, bearer + Host: mints a one-time
//!   launch grant for local `hiero config`/`hiero admin` commands;
//! - `POST /auth/launch-grant/exchange` — browser: swaps the grant for a
//!   `hieronymus_session` cookie (the separate CSRF layer is waived);
//! - `/api/*` — browser routes: valid session cookie, valid `Host`, valid
//!   `Origin` on every request (reads included).
//!
//! Check order on every surface: method/path → Host → credential → Origin
//! (the frozen route cases pin `invalid_host` before `unauthorized` before
//! `forbidden_origin`). `POST /api/mcp/{operation}` is removed per ADR 0015
//! and always reports `404 {"error":"not_found"}`.

pub(crate) mod admin;
pub(crate) mod feedback;
pub(crate) mod providers;
mod semantic;
pub(crate) mod settings;
pub(crate) mod status;

use serde_json::{Value, json};

use super::DaemonRuntime;
use super::http::{Request, Response, header};
use super::sessions::{ExchangeOutcome, session_cookie_header, session_from_cookie_header};

/// Entry point from the daemon route table: `path` is the target without the
/// query string.
pub(crate) fn handle(
    request: &Request,
    path: &str,
    query: &str,
    runtime: &DaemonRuntime,
) -> Response {
    match path {
        "/status" => status::handle(request, runtime),
        "/authority/host-event" if request.method == "POST" => host_event(request, runtime),
        "/api/authority/correct" if request.method == "POST" => {
            guard_api(request, runtime, console_correction)
        }
        "/api/authority/selection" if request.method == "POST" => {
            guard_api(request, runtime, console_selection)
        }
        "/semantic/configure" => semantic::handle(request, runtime, false),
        "/semantic/acquire" => semantic::handle(request, runtime, true),
        "/recall/feedback" => feedback::handle(request, runtime),
        "/auth/launch-grant" => handle_grant_mint(request, runtime),
        "/auth/launch-grant/exchange" => handle_grant_exchange(request, runtime),
        "/api/providers" => match request.method.as_str() {
            "GET" => guard_api(request, runtime, providers::list),
            "POST" => guard_api(request, runtime, providers::save),
            _ => not_found(),
        },
        "/api/settings/dream" => settings_route(request, runtime, settings::Kind::Dream),
        "/api/settings/ingest" => settings_route(request, runtime, settings::Kind::Ingest),
        "/api/settings/release" => settings_route(request, runtime, settings::Kind::Release),
        "/api/admin/dashboard" => match request.method.as_str() {
            "GET" => guard_api(request, runtime, admin::dashboard),
            _ => not_found(),
        },
        "/api/admin/snapshot" => match request.method.as_str() {
            "GET" => guard_api(request, runtime, |request, runtime| {
                admin::snapshot(request, runtime, query)
            }),
            _ => not_found(),
        },
        "/api/admin/actions/run_manual_dreaming" => match request.method.as_str() {
            "POST" => guard_api(request, runtime, admin::run_manual_dreaming),
            _ => not_found(),
        },
        path if path.starts_with("/api/admin/actions/") => match request.method.as_str() {
            "POST" => {
                let action = path.trim_start_matches("/api/admin/actions/");
                guard_api(request, runtime, |request, runtime| {
                    admin::action(request, runtime, action)
                })
            }
            _ => not_found(),
        },
        path if path.starts_with("/api/providers/") => providers::subroute(request, runtime, path),
        // The private operation bridge is removed (ADR 0015).
        path if path.starts_with("/api/mcp/") || path == "/api/mcp" => not_found(),
        _ => not_found(),
    }
}

fn settings_route(request: &Request, runtime: &DaemonRuntime, kind: settings::Kind) -> Response {
    match request.method.as_str() {
        "GET" => guard_api(request, runtime, |request, runtime| {
            settings::get(request, runtime, kind)
        }),
        "POST" => guard_api(request, runtime, |request, runtime| {
            settings::save(request, runtime, kind)
        }),
        _ => not_found(),
    }
}

fn not_found() -> Response {
    Response::json(404, &json!({"error": "not_found"}))
}

pub(super) fn invalid_host() -> Response {
    Response::json(400, &json!({"error": "invalid_host"}))
}

pub(super) fn unauthorized() -> Response {
    Response::json(401, &json!({"error": "unauthorized"}))
}

pub(super) fn forbidden_origin() -> Response {
    Response::json(403, &json!({"error": "forbidden_origin"}))
}

/// A browser-context request on `/ws/admin` without a websocket handshake
/// (Python reference error body).
pub(super) fn websocket_upgrade_required() -> Response {
    Response::json(400, &json!({"error": "websocket_upgrade_required"}))
}

pub(super) fn host_is_valid(request: &Request, runtime: &DaemonRuntime) -> bool {
    header(&request.headers, "host") == Some(runtime.bound_address.to_string().as_str())
}

pub(super) fn bearer_matches(request: &Request, runtime: &DaemonRuntime) -> bool {
    let expected = format!("Bearer {}", runtime.bearer.expose_secret());
    header(&request.headers, "authorization") == Some(expected.as_str())
}

/// The browser context check: the `Origin` header must name this daemon
/// exactly. Absent origins fail closed — this is the guard for mutations, the
/// grant exchange, and the WS upgrade (the CSRF layer is waived, so it is the
/// only cross-site guard for state changes).
pub(super) fn origin_is_valid(request: &Request, runtime: &DaemonRuntime) -> bool {
    let origin = header(&request.headers, "origin");
    Some(format!("http://{}", runtime.bound_address).as_str()) == origin
}

/// The safe-read variant of the browser Origin check (plan W1): a request with
/// **no** `Origin` passes, because legitimate top-level navigations and many
/// same-origin `GET`s omit the header, but an explicit **foreign** `Origin` is
/// still rejected. Only authenticated `GET`/`HEAD` reads use this; every
/// state-changing route keeps [`origin_is_valid`].
pub(super) fn origin_is_absent_or_valid(request: &Request, runtime: &DaemonRuntime) -> bool {
    match header(&request.headers, "origin") {
        None => true,
        Some(_) => origin_is_valid(request, runtime),
    }
}

/// The presented `hieronymus_session` cookie value, if any.
pub(super) fn presented_session(request: &Request) -> Option<String> {
    header(&request.headers, "cookie")
        .and_then(session_from_cookie_header)
        .map(str::to_string)
}

/// Browser-route guard: Host → session cookie → Origin, then the handler.
///
/// The `Origin` rule splits by method (plan W1): safe reads accept a missing
/// `Origin` so a top-level browser navigation into the console works, but
/// still reject an explicit foreign `Origin`; every non-safe method requires
/// the exact allowed `Origin`. Only `GET` reaches `guard_api` from the current
/// route table; `HEAD` is listed alongside it as forward-looking (a `HEAD`
/// route would be a safe read too) and costs nothing.
fn guard_api(
    request: &Request,
    runtime: &DaemonRuntime,
    handler: impl Fn(&Request, &DaemonRuntime) -> Response,
) -> Response {
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    let authorized = presented_session(request)
        .is_some_and(|session| runtime.sessions.session_is_valid(&session));
    if !authorized {
        return unauthorized();
    }
    let is_safe_read = matches!(request.method.as_str(), "GET" | "HEAD");
    let origin_ok = if is_safe_read {
        origin_is_absent_or_valid(request, runtime)
    } else {
        origin_is_valid(request, runtime)
    };
    if !origin_ok {
        return forbidden_origin();
    }
    handler(request, runtime)
}

/// Mint a launch grant: the minimal native (bearer + Host) path the
/// `hiero admin` / `hiero config` commands call. The grant is returned only in
/// this response body; the CLI then carries it to the browser in a URL
/// fragment that `bootstrap.ts` scrubs before any network call (see
/// `sessions` and plan `2026-09-05-rust-port-console`). It never appears in a
/// query string or a log line.
fn handle_grant_mint(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    if !credential_matches(request, &runtime.console_credential) {
        return unauthorized();
    }
    match runtime.sessions.mint_grant() {
        Ok(grant) => Response::json(200, &json!({"launch_grant": grant.expose_secret()})),
        Err(_) => Response::json(500, &json!({"error": "launch_grant_unavailable"})),
    }
}

/// Exchange a one-time launch grant for the browser session cookie.
/// Order: Host → grant → Origin; a consumed grant reports reuse, an unknown
/// or expired one fails closed with a clear error body. A cross-site request
/// burns the grant but never receives the session.
fn handle_grant_exchange(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    let presented = request_body(request)
        .and_then(|body| body.get("launch_grant").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let outcome = match runtime.sessions.exchange_grant(&presented) {
        Ok(outcome) => outcome,
        Err(_) => return Response::json(500, &json!({"error": "launch_grant_unavailable"})),
    };
    let session = match outcome {
        ExchangeOutcome::Exchanged(session) => session,
        ExchangeOutcome::AlreadyUsed => {
            return Response::json(401, &json!({"error": "launch_grant_already_used"}));
        }
        ExchangeOutcome::Unknown => {
            return Response::json(401, &json!({"error": "launch_grant_invalid"}));
        }
    };
    if !origin_is_valid(request, runtime) {
        return forbidden_origin();
    }
    Response::json(200, &json!({ "ok": true }))
        .with_header("Set-Cookie", session_cookie_header(&session))
}

/// Parse the request body as a JSON object; `None` when absent or not an
/// object (mirrors the Python bridge treating non-object bodies as empty).
pub(super) fn request_body(request: &Request) -> Option<Value> {
    if request.body.is_empty() {
        return None;
    }
    serde_json::from_slice(&request.body)
        .ok()
        .filter(Value::is_object)
}

/// Decode browser form-encoded query values once; path decoding is separate.
pub(super) fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            let value = url::form_urlencoded::parse(pair.as_bytes())
                .next()
                .map(|(_, value)| value.into_owned())
                .unwrap_or_default();
            (key.to_string(), value)
        })
        .collect()
}

/// Credential check shared by separately authenticated local authority routes.
fn credential_matches(request: &Request, credential: &hieronymus::secret::Secret<String>) -> bool {
    header(&request.headers, "authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| value == credential.expose_secret())
}

fn host_event(request: &Request, runtime: &DaemonRuntime) -> Response {
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    if !credential_matches(request, &runtime.host_event_credential) {
        return unauthorized();
    }
    correction_response(
        request,
        runtime,
        crate::trusted_ingress::Principal::HostEvent,
    )
}
fn console_correction(request: &Request, runtime: &DaemonRuntime) -> Response {
    // Cookie was authenticated by guard_api. Store a hash, never a live cookie credential.
    let identity = crate::trusted_ingress::hash(&presented_session(request).unwrap_or_default());
    correction_response(
        request,
        runtime,
        crate::trusted_ingress::Principal::Console(identity),
    )
}
fn correction_response(
    request: &Request,
    runtime: &DaemonRuntime,
    principal: crate::trusted_ingress::Principal,
) -> Response {
    let Some(body) = request_body(request) else {
        return Response::json(400, &json!({"error":"invalid_request"}));
    };
    match crate::application::authority::user_correction(&runtime.application, principal, &body) {
        Ok(value) => Response::json(200, &value),
        Err(crate::application::AppError::Authority(error)) => {
            Response::json(409, &json!({"error":error}))
        }
        Err(error) => Response::json(400, &json!({"error":error.to_string()})),
    }
}

fn console_selection(request: &Request, runtime: &DaemonRuntime) -> Response {
    let Some(body) = request_body(request) else {
        return Response::json(400, &json!({"error":"invalid_request"}));
    };
    match crate::application::authority::user_selection(&runtime.application, &body) {
        Ok(value) => Response::json(200, &value),
        Err(crate::application::AppError::Authority(error)) => {
            Response::json(409, &json!({"error":error}))
        }
        Err(error) => Response::json(400, &json!({"error":error.to_string()})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_use_browser_form_encoding_once() {
        assert_eq!(
            parse_query("view=Crystals&selected_id=1"),
            vec![
                ("view".to_string(), "Crystals".to_string()),
                ("selected_id".to_string(), "1".to_string())
            ]
        );
        assert_eq!(parse_query(""), Vec::new());
        assert_eq!(
            parse_query("view=Dream+Runs&literal=%2B&once=%252B&path=a%2Fb&key+name=value"),
            vec![
                ("view".into(), "Dream Runs".into()),
                ("literal".into(), "+".into()),
                ("once".into(), "%2B".into()),
                ("path".into(), "a/b".into()),
                ("key+name".into(), "value".into()),
            ]
        );
    }
}

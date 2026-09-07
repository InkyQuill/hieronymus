//! REST route contract tests against the frozen fixture targets
//! (`compatibility/fixtures/http/route-cases.json`, `target` blocks). Per the
//! controller ruling on ADR 0012 (as amended 2026-09-03), the stale CSRF
//! machinery in the frozen targets (`invalid-csrf`/`missing-csrf` failures,
//! `403 csrf_failed`, csrf tokens) is NOT binding and is never implemented or
//! asserted: browser mutations authenticate with the session cookie plus a
//! valid `Origin`/`Host` only.

mod common;

use common::{
    RawResponse, RouteFixture, browser_headers, route_target, same_origin,
    seed_synthetic_admin_data, send_request, start_daemon_on_ephemeral_port,
    start_daemon_with_browser_session, substitute_route_placeholders,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// Failure ids frozen before the CSRF waiver; the amended auth never serves
/// them and the tests must not assert them.
fn is_stale_csrf_case(failure: &Value) -> bool {
    failure["id"].as_str().is_some_and(|id| {
        id == "invalid-csrf"
            || id == "missing-csrf"
            || failure["response"]["body"]["error"] == json!("csrf_failed")
    })
}

/// Send the fixture request block (already substituted) against the daemon.
/// The stale CSRF header is dropped: the amended daemon has no CSRF layer.
fn execute_fixture_request(request: &Value, fixture: &RouteFixture) -> RawResponse {
    let method = request["method"].as_str().unwrap();
    let mut path = request["path"].as_str().unwrap().to_string();
    if let Some(query) = request["query"].as_object() {
        let pairs: Vec<String> = query
            .iter()
            .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
            .collect();
        if !pairs.is_empty() {
            path.push('?');
            path.push_str(&pairs.join("&"));
        }
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in request["headers"].as_object().unwrap() {
        if name.eq_ignore_ascii_case("x-csrf-token") {
            continue;
        }
        let value = value.as_str().unwrap_or_default().to_string();
        if value == "Bearer compat-secret-do-not-log" {
            headers.push((name.clone(), format!("Bearer {}", fixture.bearer)));
        } else {
            headers.push((name.clone(), value));
        }
    }
    let body: Vec<u8> = match &request["body"] {
        Value::Null => Vec::new(),
        Value::String(text) => text.clone().into_bytes(),
        other => serde_json::to_vec(other).unwrap(),
    };
    send_request(fixture.port, method, &path, &headers, &body)
}

fn assert_json_response(route_id: &str, case: &str, response: &RawResponse, expected: &Value) {
    assert_eq!(
        response.content_type, JSON_CONTENT_TYPE,
        "{route_id}/{case} content type"
    );
    assert_eq!(
        response.body(),
        *expected,
        "{route_id}/{case} body mismatch"
    );
}

/// Assert every non-stale frozen failure case of a route contract.
fn assert_frozen_failures(route_id: &str, target: &Value, fixture: &RouteFixture) {
    for failure in target["failures"].as_array().unwrap() {
        if is_stale_csrf_case(failure) {
            continue;
        }
        let id = failure["id"].as_str().unwrap_or("unnamed");
        let response = execute_fixture_request(&failure["request"], fixture);
        assert_eq!(
            u64::from(response.status),
            failure["response"]["status"].as_u64().unwrap(),
            "{route_id}/{id} status"
        );
        assert_json_response(route_id, id, &response, &failure["response"]["body"]);
    }
}

/// Assert the frozen success case of a route contract; `fixup` adjusts the
/// normalized oracle body for values that are only string placeholders in the
/// fixture (numeric pid/port on the status route).
fn assert_frozen_success(
    route_id: &str,
    target: &Value,
    fixture: &RouteFixture,
    fixup: impl Fn(&mut Value),
) -> RawResponse {
    let success = &target["success"];
    let response = execute_fixture_request(&success["request"], fixture);
    assert_eq!(
        u64::from(response.status),
        success["response"]["status"].as_u64().unwrap(),
        "{route_id} success status"
    );
    let mut expected = success["response"]["body"].clone();
    fixup(&mut expected);
    assert_json_response(route_id, "success", &response, &expected);
    response
}

fn assert_frozen_contract(route_id: &str, fixture: &RouteFixture, fixup: impl Fn(&mut Value)) {
    let target = substitute_route_placeholders(&route_target(route_id), fixture);
    assert_frozen_failures(route_id, &target, fixture);
    assert_frozen_success(route_id, &target, fixture, fixup);
}

/// The ADR-backed Rust delta on the authenticated `/status` payload: exactly
/// these keys are added to the frozen contract, and nothing frozen changes.
///
/// ADR 0009 requires stale discovery to be detected "by authenticated health
/// probing and process-instance comparison, never by PID existence alone".
/// That comparison needs the live process instance and MCP revision to be
/// readable from an authenticated endpoint (`GET /health` stays minimal and
/// unauthenticated), so R5 adds them here. S2 adds the semantic readiness
/// surface (`semantic.state`, the required-gate DTO). The frozen fixture
/// file itself is untouched; this list is the recorded expectation change.
const STATUS_RUST_ADDITIONS: [&str; 3] = ["instance_id", "protocol_revision", "semantic"];

#[test]
fn status_route_matches_frozen_target() {
    let (fixture, _root, daemon) = start_daemon_with_browser_session();
    let target = substitute_route_placeholders(&route_target("http.route.get.status"), &fixture);
    assert_frozen_failures("http.route.get.status", &target, &fixture);
    let record = daemon.discovery_record();
    // The S2 semantic addition is a live readiness verdict (state machine, not
    // a fixture constant); it is normalized from a probe of the same daemon
    // and pinned separately by the semantic_execution typed-DTO tests.
    let probe = execute_fixture_request(&target["success"]["request"], &fixture).body();
    let semantic = probe["semantic"].clone();
    assert!(
        semantic.is_object() && semantic["state"].is_string(),
        "semantic status surface missing: {semantic}"
    );
    // The oracle normalizes pid/port to string placeholders; the daemon serves
    // them as numbers. The added keys are appended explicitly, so any
    // *other* drift from the frozen body still fails this assertion.
    assert_frozen_success("http.route.get.status", &target, &fixture, |body| {
        body["version"] = json!(env!("CARGO_PKG_VERSION"));
        body["pid"] = json!(fixture.pid);
        body["port"] = json!(fixture.port);
        body["instance_id"] = json!(record.instance_id);
        body["protocol_revision"] = json!(common::PROTOCOL_REVISION);
        body["semantic"] = semantic.clone();
    });

    // Every frozen key survives, and the additions are exactly the declared
    // ones — no unannounced field can slip into the authenticated DTO.
    let frozen = route_target("http.route.get.status")["success"]["response"]["body"].clone();
    let response = execute_fixture_request(&target["success"]["request"], &fixture);
    let served = response.body();
    for key in frozen.as_object().unwrap().keys() {
        assert!(served.get(key).is_some(), "frozen key {key} disappeared");
    }
    for key in served.as_object().unwrap().keys() {
        assert!(
            frozen.get(key).is_some() || STATUS_RUST_ADDITIONS.contains(&key.as_str()),
            "undeclared addition to the authenticated status DTO: {key}"
        );
    }
    // Sentinel secret: the credential appears nowhere in the payload.
    assert!(
        !served.to_string().contains(daemon.bearer().expose_secret()),
        "the status payload must never contain the bearer token"
    );
}

#[test]
fn launch_grant_exchange_matches_frozen_target_and_waives_csrf() {
    let (fixture, _root, daemon) = start_daemon_with_browser_session();
    let port = fixture.port;
    let grant_body = |grant: &str| format!(r#"{{"launch_grant": "{grant}"}}"#).into_bytes();
    let mint_headers = vec![(
        "Authorization".to_string(),
        format!(
            "Bearer {}",
            common::console_credential(&daemon).expose_secret()
        ),
    )];
    // The oracle treats every frozen case as starting from a live grant, so
    // each case below mints its own.
    let mint = || {
        let minted = send_request(port, "POST", "/auth/launch-grant", &mint_headers, b"");
        assert_eq!(minted.status, 200, "grant mint must succeed");
        minted.body()["launch_grant"].as_str().unwrap().to_string()
    };

    // cross-origin (frozen): a valid grant with a foreign Origin is rejected
    // before the session is handed out.
    let cross_origin = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), "https://attacker.invalid".to_string()),
        ],
        &grant_body(&mint()),
    );
    assert_eq!(cross_origin.status, 403);
    assert_json_response(
        "http.route.post.auth.launch-grant.exchange",
        "cross-origin",
        &cross_origin,
        &json!({"error": "forbidden_origin"}),
    );

    // invalid-host (frozen): host is checked before everything else.
    let invalid_host = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Host".to_string(), "attacker.invalid".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &grant_body(&fixture.grant),
    );
    assert_eq!(invalid_host.status, 400);
    assert_json_response(
        "http.route.post.auth.launch-grant.exchange",
        "invalid-host",
        &invalid_host,
        &json!({"error": "invalid_host"}),
    );

    // grant-reuse (frozen): the one-time grant is consumed by the first
    // exchange; a second attempt reports it as used.
    let fresh = mint();
    let first = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &grant_body(&fresh),
    );
    assert_eq!(first.status, 200, "first exchange must consume the grant");
    let reuse = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &grant_body(&fresh),
    );
    assert_eq!(reuse.status, 401);
    assert_json_response(
        "http.route.post.auth.launch-grant.exchange",
        "grant-reuse",
        &reuse,
        &json!({"error": "launch_grant_already_used"}),
    );

    // Unknown grants fail closed with a clear error body.
    let unknown = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &grant_body("hieronymus-unknown-grant"),
    );
    assert_eq!(unknown.status, 401);
    assert_json_response(
        "http.route.post.auth.launch-grant.exchange",
        "unknown-grant",
        &unknown,
        &json!({"error": "launch_grant_invalid"}),
    );

    // A missing Origin is not a valid Origin (fail closed).
    let missing_origin = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[("Content-Type".to_string(), "application/json".to_string())],
        &grant_body(&mint()),
    );
    assert_eq!(missing_origin.status, 403);
    assert_eq!(
        missing_origin.body(),
        json!({"error": "forbidden_origin"}),
        "missing origin"
    );

    // The amended success shape: session cookie, no CSRF machinery.
    let fresh = mint();
    let success = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &grant_body(&fresh),
    );
    assert_eq!(success.status, 200);
    assert_eq!(success.content_type, JSON_CONTENT_TYPE);
    let cookie = success
        .headers
        .get("set-cookie")
        .expect("exchange must set the session cookie");
    assert!(
        cookie.starts_with("hieronymus_session=")
            && cookie.contains("Path=/")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict"),
        "unexpected Set-Cookie: {cookie}"
    );
    let token = cookie
        .strip_prefix("hieronymus_session=")
        .and_then(|rest| rest.split(';').next())
        .unwrap();
    assert!(token.len() >= 32, "session token must be random material");
    assert_eq!(
        success.body(),
        json!({"ok": true}),
        "the waived CSRF layer must not leak tokens in the body"
    );
    assert!(
        success.body().get("csrf_token").is_none(),
        "csrf_token is waived machinery (ADR 0012 amendment)"
    );

    // The session cookie authenticates same-origin /api requests.
    let authorized = send_request(
        port,
        "GET",
        "/api/providers",
        &[
            ("Cookie".to_string(), format!("hieronymus_session={token}")),
            ("Origin".to_string(), same_origin(port)),
        ],
        b"",
    );
    assert_eq!(
        authorized.status, 200,
        "exchanged session must authorize /api"
    );
}

#[test]
fn api_provider_list_matches_frozen_target() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    assert_frozen_contract("http.route.get.api.providers", &fixture, |_| {});
}

#[test]
fn api_provider_routes_match_frozen_targets() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();

    // Saving the synthetic provider first: the detail/check/models/delete
    // contracts address it.
    assert_frozen_contract("http.route.post.api.providers", &fixture, |_| {});
    assert_frozen_contract("http.route.get.api.providers.id", &fixture, |_| {});

    // Astra finding 12 / plan W3: the `check`/`models` success bodies for the
    // synthetic `provider.invalid` profile change — production now runs the
    // real probe, so a `.invalid` host reports a genuine transport failure
    // instead of `{"source": "fixture", "models": ["synthetic-model"]}`. The
    // frozen auth/CSRF/host failure cases are unchanged and still asserted;
    // the frozen fixture bytes are untouched. The reconciled contract per
    // ADR 0012 / Astra 12 is `compatibility/rust/provider-checks.json`.
    for route_id in [
        "http.route.post.api.providers.id.check",
        "http.route.get.api.providers.id.models",
    ] {
        let target = substitute_route_placeholders(&route_target(route_id), &fixture);
        assert_frozen_failures(route_id, &target, &fixture);
        let response = execute_fixture_request(&target["success"]["request"], &fixture);
        assert_eq!(u64::from(response.status), 200, "{route_id} success status");
        let body = response.body();
        let check = body.get("check").unwrap_or(&body);
        assert_ne!(check["source"], json!("fixture"), "{route_id}: {body}");
        assert_eq!(check["source"], json!("defaults"), "{route_id}: {body}");
        assert!(
            check["models"]
                .as_array()
                .is_some_and(|list| list.iter().all(|model| model != "synthetic-model")),
            "{route_id} must not fabricate a synthetic model: {body}"
        );
    }

    assert_frozen_contract("http.route.delete.api.providers.id", &fixture, |_| {});

    // The deletion is durable: the profile is gone afterwards.
    let missing = send_request(
        fixture.port,
        "GET",
        "/api/providers/synthetic-provider",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(missing.status, 400);
    assert_eq!(
        missing.body(),
        json!({"error": "provider profile not found: synthetic-provider"})
    );

    // Domain validation failures are 400 envelope errors.
    let invalid = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        br#"{"provider": {"id": "broken", "name": "", "type": "openai", "url": "", "timeout_seconds": "30"}}"#,
    );
    assert_eq!(invalid.status, 400);
    assert!(
        invalid.body()["error"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "invalid provider payload must carry an error: {:?}",
        invalid.raw_body
    );
}

#[test]
fn api_settings_routes_match_frozen_targets() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    assert_frozen_contract("http.route.get.api.settings.dream", &fixture, |_| {});
    assert_frozen_contract("http.route.post.api.settings.dream", &fixture, |_| {});
    assert_frozen_contract("http.route.get.api.settings.ingest", &fixture, |_| {});
    assert_frozen_contract("http.route.post.api.settings.ingest", &fixture, |_| {});
    assert_frozen_contract("http.route.get.api.settings.release", &fixture, |_| {});
    assert_frozen_contract("http.route.post.api.settings.release", &fixture, |_| {});
}

#[test]
fn api_admin_snapshot_matches_frozen_target() {
    let (fixture, root, _daemon) = start_daemon_with_browser_session();
    let crystal_id = seed_synthetic_admin_data(root.path());
    assert_eq!(crystal_id, 1, "the oracle snapshot selects crystal id 1");
    assert_frozen_contract("http.route.get.api.admin.snapshot", &fixture, |_| {});
}

#[test]
fn api_admin_actions_match_frozen_targets() {
    let (fixture, root, _daemon) = start_daemon_with_browser_session();
    let crystal_id = seed_synthetic_admin_data(root.path());
    assert_eq!(crystal_id, 1);

    let target = substitute_route_placeholders(
        &route_target("http.route.post.api.admin.actions.action"),
        &fixture,
    );
    assert_frozen_failures(
        "http.route.post.api.admin.actions.action",
        &target,
        &fixture,
    );
    assert_frozen_success(
        "http.route.post.api.admin.actions.action",
        &target,
        &fixture,
        |_| {},
    );

    // Unknown actions are 404 envelope errors.
    let unknown = send_request(
        fixture.port,
        "POST",
        "/api/admin/actions/teleport_crystal",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        br#"{"id": 1}"#,
    );
    assert_eq!(unknown.status, 404);
    assert_eq!(unknown.content_type, JSON_CONTENT_TYPE);
    assert_eq!(unknown.body(), json!({"error": "unknown_admin_action"}));
}

#[test]
fn api_admin_dashboard_matches_frozen_target_shape() {
    let (fixture, root, _daemon) = start_daemon_with_browser_session();
    seed_synthetic_admin_data(root.path());

    let mut expected = substitute_route_placeholders(
        &route_target("http.route.get.api.admin.dashboard")["success"]["response"]["body"],
        &fixture,
    );
    expected["header"]["version"] = json!(format!("v{}α", env!("CARGO_PKG_VERSION")));
    let response = send_request(
        fixture.port,
        "GET",
        "/api/admin/dashboard",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        b"",
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, JSON_CONTENT_TYPE);
    let body = response.body();

    // Shape: the exact top-level key set of the frozen oracle.
    let expected_keys: Vec<String> = expected.as_object().unwrap().keys().cloned().collect();
    let actual_keys: Vec<String> = body.as_object().unwrap().keys().cloned().collect();
    assert_eq!(actual_keys, expected_keys, "dashboard top-level keys");

    // Static payloads must match the oracle exactly.
    for section in [
        "command_options",
        "views",
        "view_keys",
        "view_labels",
        "view_options",
        "default_view",
        "default_view_key",
        "header",
        "service",
    ] {
        assert_eq!(body[section], expected[section], "dashboard.{section}");
    }

    // Domain payloads over the seeded stores must match the oracle exactly.
    for section in [
        "stats",
        "snapshot",
        "dream_status",
        "short_term_status",
        "dream_config_error",
    ] {
        assert_eq!(body[section], expected[section], "dashboard.{section}");
    }

    // The config editor payload: exact keys and sections.
    let editor = &body["config_editor"];
    let expected_editor_keys: Vec<String> = expected["config_editor"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let actual_editor_keys: Vec<String> = editor.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        actual_editor_keys, expected_editor_keys,
        "config_editor keys"
    );
    for section in [
        "config",
        "config_error",
        "provider_config_error",
        "providers",
        "workflows",
        "prompts",
        "thresholds",
        "model_cache",
        "model_cache_warnings",
    ] {
        assert_eq!(
            editor[section], expected["config_editor"][section],
            "config_editor.{section}"
        );
    }
}

#[test]
fn run_manual_dreaming_starts_a_dream_run() {
    let (fixture, root, _daemon) = start_daemon_with_browser_session();
    let target = substitute_route_placeholders(
        &route_target("http.route.post.api.admin.actions.run_manual_dreaming"),
        &fixture,
    );
    assert_frozen_failures(
        "http.route.post.api.admin.actions.run_manual_dreaming",
        &target,
        &fixture,
    );

    let response = send_request(
        fixture.port,
        "POST",
        "/api/admin/actions/run_manual_dreaming",
        &browser_headers(&fixture, &[("Origin", same_origin(fixture.port))]),
        br#"{}"#,
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, JSON_CONTENT_TYPE);
    assert_eq!(
        response.body(),
        json!({"started": true, "status": "running"})
    );

    // The run goes through the DreamService seam in the background: a durable
    // run row appears (fail-closed: no pending memories, empty completed run).
    let deadline = Instant::now() + Duration::from_secs(5);
    let database = root.path().join("hieronymus.sqlite");
    loop {
        let connection = rusqlite::Connection::open(&database).unwrap();
        let runs: i64 = connection
            .query_row("select count(*) from dream_runs", [], |row| row.get(0))
            .unwrap();
        drop(connection);
        if runs > 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the manual dreaming route never started a dream run"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn private_mcp_bridge_is_removed_per_adr_0015() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let removed = send_request(
        fixture.port,
        "POST",
        "/api/mcp/status",
        &[("Origin".to_string(), same_origin(fixture.port))],
        br#"{}"#,
    );
    assert_eq!(removed.status, 404);
    assert_eq!(removed.content_type, JSON_CONTENT_TYPE);
    assert_eq!(removed.body(), json!({"error": "not_found"}));
}

#[test]
fn api_session_and_origin_guards_reject_unknown_sessions() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let forged = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &[(
            "Cookie".to_string(),
            "hieronymus_session=not-a-real-session".to_string(),
        )],
        b"",
    );
    assert_eq!(forged.status, 401);
    assert_eq!(forged.body(), json!({"error": "unauthorized"}));

    // Origin checks guard reads too: a valid session with a foreign origin is
    // 403 before any domain work happens.
    let foreign = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &browser_headers(
            &fixture,
            &[("Origin", "https://attacker.invalid".to_string())],
        ),
        b"",
    );
    assert_eq!(foreign.status, 403);
    assert_eq!(foreign.body(), json!({"error": "forbidden_origin"}));
}

#[test]
fn bearer_mint_route_never_serves_grants_without_the_token() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let unauthorized = send_request(port, "POST", "/auth/launch-grant", &[], b"");
    assert_eq!(unauthorized.status, 401);
    assert_eq!(unauthorized.body(), json!({"error": "unauthorized"}));
}

#[test]
fn browser_form_encoded_multiword_views_are_decoded_once() {
    let (fixture, _root, daemon) = start_daemon_with_browser_session();
    for encoded in [
        "Dream+Runs",
        "Dream%20Runs",
        "Short-Term+Memory",
        "Audit+Log",
    ] {
        let response = send_request(
            daemon.local_addr().port(),
            "GET",
            &format!("/api/admin/snapshot?view={encoded}"),
            &browser_headers(&fixture, &[]),
            b"",
        );
        assert_eq!(
            response.status,
            200,
            "browser view {encoded}: {}",
            response.body()
        );
    }
    daemon.shutdown().unwrap();
}

#[test]
fn ordinary_mcp_bearer_cannot_mint_console_authority() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let response = send_request(
        port,
        "POST",
        "/auth/launch-grant",
        &[
            ("Host".into(), format!("127.0.0.1:{port}")),
            ("Origin".into(), format!("http://127.0.0.1:{port}")),
            (
                "Authorization".into(),
                format!("Bearer {}", daemon.bearer().expose_secret()),
            ),
        ],
        br#"{"actor_kind":"explicit_user","source_role":"user"}"#,
    );
    assert_eq!(response.status, 401);
    daemon.shutdown().unwrap();
}

#[test]
fn ordinary_mcp_bearer_cannot_submit_host_events() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/authority/host-event",
        &[(
            "Authorization".into(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        )],
        br#"{"source_role":"user","event_id":"copied-transcript"}"#,
    );
    assert_eq!(response.status, 401);
}

#[test]
fn public_mcp_producers_then_console_and_bridge_correction() {
    let (root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let invoke = |name: &str, value: Value| {
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":value,"_meta":{"io.modelcontextprotocol/protocolVersion":common::PROTOCOL_REVISION,"io.modelcontextprotocol/clientCapabilities":{}}}});
        let response = send_request(
            port,
            "POST",
            "/mcp",
            &common::mcp_headers(&daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(response.status, 200, "{name}: {}", response.body());
        let body = response.body();
        assert_eq!(body["result"]["isError"], false, "{name}: {body}");
        body["result"]["structuredContent"].clone()
    };
    let (draft, mut event) = common::authority::prepared_with(invoke, root.path());
    let concept_id = draft["concept_id"].clone();
    let activated = invoke("hieronymus_decide", draft);
    assert!(activated.get("Applied").is_some(), "{activated}");
    let (_, cookie) = common::browser_session(&daemon);
    let console_headers = vec![
        ("Cookie".into(), format!("hieronymus_session={cookie}")),
        ("Origin".into(), same_origin(port)),
    ];
    for (index, case) in [
        "hash",
        "language",
        "scope",
        "multiple",
        "quote",
        "unsupported",
    ]
    .iter()
    .enumerate()
    {
        let mut invalid = event.clone();
        invalid["event_id"] = json!(format!("invalid-{case}"));
        invalid["decision_id"] = json!(format!("13000000-0000-4000-8000-{index:012}"));
        match *case {
            "hash" => invalid["selected_sources"][0]["content_hash"] = json!("changed"),
            "language" => invalid["source_language"] = Value::Null,
            "scope" => invalid["applicability"] = Value::Null,
            "multiple" => {
                invalid["selected_sources"] =
                    json!([event["selected_sources"][0], event["selected_sources"][0]])
            }
            "quote" => invalid["text"] = json!("translate \"Alec\" as \"Б\""),
            _ => invalid["source_language"] = json!("xx"),
        }
        let outcome = send_request(
            port,
            "POST",
            "/api/authority/correct",
            &console_headers,
            &serde_json::to_vec(&invalid).unwrap(),
        );
        assert_eq!(
            outcome.body()["status"],
            "tentative",
            "{case}: {}",
            outcome.body()
        );
        event["expected_revision"] = outcome.body()["resulting_revision"].clone();
        if *case == "hash" {
            assert_eq!(outcome.body()["detail"], "changed_source_hash_or_span");
        }
    }
    let mut stale = event.clone();
    stale["event_id"] = json!("stale-observation");
    stale["decision_id"] = json!("13000000-0000-4000-8000-000000000010");
    stale["expected_revision"] = json!(1);
    let stale_result = send_request(
        port,
        "POST",
        "/api/authority/correct",
        &console_headers,
        &serde_json::to_vec(&stale).unwrap(),
    );
    assert_eq!(stale_result.status, 409);
    assert_eq!(
        stale_result.body()["error"]["RevisionConflict"]["current_revision"],
        event["expected_revision"]
    );
    let mut form = event.clone();
    form["text"] = Value::Null;
    form["structured"] = json!({"kind":"rendering","canonical":"Б","approved_variants":["Alias"],"forbidden_variants":["Bad"]});
    let corrected = send_request(
        port,
        "POST",
        "/api/authority/correct",
        &console_headers,
        &serde_json::to_vec(&form).unwrap(),
    );
    assert_eq!(corrected.status, 200, "{}", corrected.body());
    assert!(
        corrected.body().get("Applied").is_some(),
        "{}",
        corrected.body()
    );
    let receipt = corrected.body()["Applied"]["receipt"].clone();
    for (rendering, valid) in [("Б", true), ("А", false)] {
        let result = invoke(
            "hieronymus_termbase_validate",
            json!({"series_slug":"book","raw_text":"Alex","translated_text":rendering,"volume":"I","chapter":"1","story_timeline_id":event["applicability"]["timeline_id"],"story_scene_key":"a","required_decision_id":receipt["decision_id"]}),
        );
        assert_eq!(
            result["results"].as_array().unwrap().is_empty(),
            valid,
            "{rendering}: {result}"
        );
    }
    event["expected_revision"] = receipt["resulting_revision"].clone();
    event["selected_rule"]["id"] = receipt["affected_rules"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()[0]
        .clone();
    event["selected_rule"]["revision"] = receipt["affected_rules"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()[1]
        .clone();
    event["decision_id"] = json!("11000000-0000-4000-8000-000000000003");
    event["event_id"] = json!("bridge-event-2");
    event["text"] = json!("translate this as В");
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let token = hiero::daemon::discovery::read_local_credential(
        &config,
        hiero::daemon::discovery::LocalCredential::HostEvent,
    )
    .unwrap();
    let bridge = send_request(
        port,
        "POST",
        "/authority/host-event",
        &[(
            "Authorization".into(),
            format!("Bearer {}", token.expose_secret()),
        )],
        &serde_json::to_vec(&event).unwrap(),
    );
    assert_eq!(bridge.status, 200, "{}", bridge.body());
    assert!(bridge.body().get("Applied").is_some(), "{}", bridge.body());
    for (rendering, valid) in [("В", true), ("Alias", true), ("Б", false), ("Bad", false)] {
        let result = invoke(
            "hieronymus_termbase_validate",
            json!({"series_slug":"book","raw_text":"Alex","translated_text":rendering,"volume":"I","chapter":"1","story_timeline_id":event["applicability"]["timeline_id"],"story_scene_key":"a","required_decision_id":bridge.body()["Applied"]["receipt"]["decision_id"]}),
        );
        assert_eq!(
            result["results"].as_array().unwrap().is_empty(),
            valid,
            "{rendering}: {result}"
        );
    }
    let memory = invoke(
        "hieronymus_short_term_add",
        json!({"session_id":event["session_id"],"kind":"note","text":"Alex suspects it. Alex knows it.","claims":[{"text":"Alex suspects it.","concept_id":concept_id,"applicability":event["applicability"]},{"text":"Alex knows it.","concept_id":concept_id,"applicability":event["applicability"]}]}),
    );
    let selection_body =
        json!({"series_id":1,"target":{"source":"short_term","id":memory["memory_id"]}});
    let selection = send_request(
        port,
        "POST",
        "/api/authority/selection",
        &console_headers,
        &serde_json::to_vec(&selection_body).unwrap(),
    );
    assert_eq!(selection.status, 200, "{}", selection.body());
    let selection = selection.body();
    assert_eq!(selection["claims"].as_array().unwrap().len(), 2);
    let mut fact = event.clone();
    fact["expected_revision"] = selection["expected_revision"].clone();
    fact["selected_sources"] = json!([]);
    fact["selected_rule"] = Value::Null;
    fact["target_language"] = Value::Null;
    fact["text"] = json!("that memory is wrong");
    fact["selected_claims"] = json!(
        selection["claims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| json!({"id":c["claim_id"],"revision":c["revision"]}))
            .collect::<Vec<_>>()
    );
    fact["event_id"] = json!("ambiguous-claims");
    fact["decision_id"] = json!("11000000-0000-4000-8000-000000000005");
    let ambiguous = send_request(
        port,
        "POST",
        "/api/authority/correct",
        &console_headers,
        &serde_json::to_vec(&fact).unwrap(),
    );
    assert_eq!(ambiguous.body()["status"], "tentative");
    fact["expected_revision"] = ambiguous.body()["resulting_revision"].clone();
    fact["selected_claims"] = json!([fact["selected_claims"][0]]);
    fact["event_id"] = json!("selected-claim");
    fact["decision_id"] = json!("11000000-0000-4000-8000-000000000006");
    let invalidated = send_request(
        port,
        "POST",
        "/api/authority/correct",
        &console_headers,
        &serde_json::to_vec(&fact).unwrap(),
    );
    assert_eq!(invalidated.status, 200, "{}", invalidated.body());
    assert!(
        invalidated.body().get("Applied").is_some(),
        "{}",
        invalidated.body()
    );
    fact["expected_revision"] =
        invalidated.body()["Applied"]["receipt"]["resulting_revision"].clone();
    fact["selected_claims"] = json!([{"id":selection["claims"][1]["claim_id"],"revision":selection["claims"][1]["revision"]}]);
    fact["event_id"] = json!("qualified-claim");
    fact["decision_id"] = json!("11000000-0000-4000-8000-000000000007");
    fact["text"] = json!(r#"qualify that memory as "Mira suspects this; \u2603""#);
    let qualified = send_request(
        port,
        "POST",
        "/api/authority/correct",
        &console_headers,
        &serde_json::to_vec(&fact).unwrap(),
    );
    assert_eq!(qualified.status, 200, "{}", qualified.body());
    assert!(
        qualified.body().get("Applied").is_some(),
        "{}",
        qualified.body()
    );
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let qualification: String = db
        .query_row(
            "select qualification from claim_effects where effect='qualified'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(qualification, "Mira suspects this; ☃");
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    // Same trusted event cannot be rebound to another operation or observed revision.
    event["text"] = json!("translate this as Forged");
    let replay = send_request(
        port,
        "POST",
        "/authority/host-event",
        &[(
            "Authorization".into(),
            format!("Bearer {}", token.expose_secret()),
        )],
        &serde_json::to_vec(&event).unwrap(),
    );
    assert_eq!(replay.status, 409);
}

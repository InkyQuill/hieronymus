//! Contract tests for the daemon's HTTP MCP surface, driven by the frozen
//! fixtures:
//! - `compatibility/fixtures/mcp/protocol.json` (target tools/list + tools/call)
//! - `compatibility/fixtures/http/route-cases.json` (http.route.post.mcp and
//!   http.route.get.health targets: host, bearer, mirrored headers, and error
//!   mapping)
//!
//! The daemon runs in-process on an ephemeral loopback port; requests are sent
//! as raw HTTP so status lines and headers are part of the assertion surface.

mod common;

use common::{
    PROTOCOL_REVISION, mcp_headers, mcp_protocol, route_target, send_request,
    start_daemon_on_ephemeral_port, substitute_placeholders,
};
use serde_json::{Value, json};

fn post_mcp(
    daemon: &hiero::daemon::Daemon,
    headers: &[(String, String)],
    body: &Value,
) -> common::RawResponse {
    send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        headers,
        &serde_json::to_vec(body).unwrap(),
    )
}

fn body_bytes(body: &Value) -> Vec<u8> {
    serde_json::to_vec(body).unwrap()
}

/// Replay a fixture request's headers verbatim. `<PORT>` and
/// `<INVALID_BEARER_TOKEN>` are already substituted; the only placeholder the
/// daemon cannot know is the per-installation bearer, swapped in here. The
/// `Host` header is part of the contract (the `invalid-host` cases depend on
/// it) and must be transmitted exactly as frozen.
fn expected_headers_from_fixture(
    request: &Value,
    daemon: &hiero::daemon::Daemon,
) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for (name, value) in request["headers"].as_object().unwrap() {
        let value = value.as_str().unwrap();
        if value == "Bearer compat-secret-do-not-log" {
            headers.push((
                name.clone(),
                format!("Bearer {}", daemon.bearer().expose_secret()),
            ));
        } else {
            headers.push((name.clone(), value.to_string()));
        }
    }
    headers
}

// ------------------------------------------------------- frozen MCP contract

#[test]
fn tools_list_matches_active_rust_registry_in_frozen_envelope() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let protocol = current_protocol();
    let request = protocol["target"]["tools_list"]["request"].clone();
    let expected = protocol["target"]["tools_list"]["response"].clone();

    let response = post_mcp(
        &daemon,
        &mcp_headers(&daemon, &[("Mcp-Method", "tools/list")]),
        &request,
    );

    assert_eq!(response.status, 200, "{:?}", response.body());
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    assert_eq!(response.body(), expected);
}

#[test]
fn tools_call_status_round_trip_matches_frozen_fixture_exactly() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let protocol = current_protocol();
    let request = protocol["target"]["tools_call"]["request"].clone();
    let expected = protocol["target"]["tools_call"]["response"].clone();

    let response = post_mcp(
        &daemon,
        &mcp_headers(
            &daemon,
            &[
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "hieronymus_status"),
            ],
        ),
        &request,
    );

    assert_eq!(response.status, 200, "{:?}", response.body());
    assert_eq!(response.body(), expected);
}

// -------------------------------------------------- frozen route-case target

#[test]
fn route_case_failures_reproduce_the_frozen_http_contract() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let target = route_target("http.route.post.mcp");

    for failure in target["failures"].as_array().unwrap() {
        let id = failure["id"].as_str().unwrap();
        let request = substitute_placeholders(&failure["request"], port);
        let headers = expected_headers_from_fixture(&request, &daemon);
        let body = match &request["body"] {
            Value::Null => Vec::new(),
            value => body_bytes(value),
        };
        let response = send_request(
            port,
            request["method"].as_str().unwrap(),
            request["path"].as_str().unwrap(),
            &headers,
            &body,
        );

        let expected_status = failure["response"]["status"].as_u64().unwrap();
        let expected_body = substitute_placeholders(&failure["response"]["body"], port);
        assert_eq!(
            response.status as u64,
            expected_status,
            "case {id}: body was {:?}",
            response.body()
        );
        assert_eq!(
            response.body(),
            expected_body,
            "case {id} response body does not match the frozen contract"
        );
    }
}

#[test]
fn route_case_successes_keep_the_frozen_envelope() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let target = route_target("http.route.post.mcp");
    let protocol = current_protocol();

    for success in target["successes"].as_array().unwrap() {
        let id = success["id"].as_str().unwrap();
        let request = substitute_placeholders(&success["request"], port);
        let headers = expected_headers_from_fixture(&request, &daemon);
        let response = send_request(
            port,
            request["method"].as_str().unwrap(),
            request["path"].as_str().unwrap(),
            &headers,
            &body_bytes(&request["body"]),
        );

        assert_eq!(response.status, 200, "case {id}: {:?}", response.body());
        assert_eq!(
            response.body()["jsonrpc"],
            request["body"]["jsonrpc"],
            "case {id}"
        );
        assert_eq!(response.body()["id"], request["body"]["id"], "case {id}");
        match id {
            "tools-list" => {
                assert_eq!(response.body()["result"]["cacheScope"], json!("private"));
                assert_eq!(response.body()["result"]["ttlMs"], json!(0));
                assert_eq!(response.body()["result"]["resultType"], json!("complete"));
                // The route-case freezes the envelope; the manifest owns the
                // registry content.
                assert_eq!(
                    response.body()["result"]["tools"],
                    protocol["target"]["tools_list"]["response"]["result"]["tools"]
                );
            }
            "tools-call" => {
                assert_eq!(response.body()["result"]["isError"], json!(false));
                assert_eq!(response.body()["result"]["resultType"], json!("complete"));
                assert!(
                    !response.body()["result"]["content"]
                        .as_array()
                        .unwrap()
                        .is_empty(),
                    "case {id}: content must not be empty"
                );
            }
            other => panic!("unexpected success case {other}"),
        }
    }
}

// ------------------------------------------------------------ transport bits

#[test]
fn request_scoped_sse_wraps_the_same_result() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let protocol = current_protocol();
    let request = protocol["target"]["tools_list"]["request"].clone();
    let expected = protocol["target"]["tools_list"]["response"].clone();

    let mut headers = mcp_headers(&daemon, &[("Mcp-Method", "tools/list")]);
    for header in &mut headers {
        if header.0 == "Accept" {
            header.1 = "text/event-stream".to_string();
        }
    }
    let response = post_mcp(&daemon, &headers, &request);

    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    assert_eq!(response.content_type, "text/event-stream");
    let text = String::from_utf8(response.raw_body.clone()).unwrap();
    let data = text
        .strip_prefix("event: message\ndata: ")
        .unwrap()
        .strip_suffix("\n\n")
        .unwrap();
    let payload: Value = serde_json::from_str(data).unwrap();
    assert_eq!(payload, expected);
}

#[test]
fn session_and_event_headers_are_rejected() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let request = mcp_protocol()["target"]["tools_list"]["request"].clone();

    for (header_name, header_value) in [
        ("Mcp-Session-Id", "0198f0aa-0000-0000-0000-000000000000"),
        ("Last-Event-ID", "42"),
    ] {
        let response = post_mcp(
            &daemon,
            &mcp_headers(
                &daemon,
                &[("Mcp-Method", "tools/list"), (header_name, header_value)],
            ),
            &request,
        );
        assert_eq!(response.status, 400, "{header_name}: {:?}", response.body());
        assert_eq!(
            response.body()["error"]["code"],
            json!(-32602),
            "{header_name}"
        );
        assert_eq!(
            response.body()["error"]["message"],
            json!("Invalid request metadata"),
            "{header_name}"
        );
    }
}

#[test]
fn health_route_matches_frozen_contract() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let target = route_target("http.route.get.health");

    let success = send_request(port, "GET", "/health", &[], b"");
    assert_eq!(success.status, 200);
    assert_eq!(
        success.body(),
        substitute_placeholders(&target["success"]["response"]["body"], port)
    );

    let failure = send_request(
        port,
        "GET",
        "/health",
        &[("Host".to_string(), "attacker.invalid".to_string())],
        b"",
    );
    assert_eq!(failure.status, 400);
    assert_eq!(
        failure.body(),
        json!({"error": "invalid_host"}),
        "health invalid-host body must match the frozen contract"
    );
}

// ------------------------------------------------------- registry dispatch

#[test]
fn not_yet_ported_tool_returns_clean_jsonrpc_error() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let protocol = current_protocol();
    let mut request = protocol["target"]["tools_call"]["request"].clone();
    request["id"] = json!(77);
    // Every advertised tool has a concrete handler since M5; an unknown name
    // is the honest leftover that must still fail with a clean JSON-RPC
    // error (never a panic, never a leaked diagnostic).
    request["params"]["name"] = json!("hieronymus_nonexistent");

    let response = post_mcp(
        &daemon,
        &mcp_headers(
            &daemon,
            &[
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "hieronymus_nonexistent"),
            ],
        ),
        &request,
    );

    assert_eq!(response.status, 400, "{:?}", response.body());
    assert_eq!(response.body()["jsonrpc"], json!("2.0"));
    assert_eq!(response.body()["id"], json!(77));
    let error = &response.body()["error"];
    assert_eq!(error["code"], json!(-32602));
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("hieronymus_nonexistent"), "{message}");
    assert!(
        !message.contains("Bearer"),
        "diagnostics must not leak credentials"
    );
}

#[test]
fn served_registry_preserves_frozen_names_and_active_rust_schemas() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let request = mcp_protocol()["target"]["tools_list"]["request"].clone();
    let expected = current_protocol()["target"]["tools_list"]["response"].clone();

    let response = post_mcp(
        &daemon,
        &mcp_headers(&daemon, &[("Mcp-Method", "tools/list")]),
        &request,
    );

    let response_body = response.body();
    let tools = response_body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools, expected["result"]["tools"].as_array().unwrap());
    // Names and order must equal the manifest snapshot.
    let snapshot = common::fixture("compatibility/snapshots/mcp.json");
    let mut snapshot_names: Vec<&str> = snapshot["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    let served_names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    snapshot_names.extend([
        "hieronymus_decide",
        "hieronymus_correct",
        "hieronymus_order_register",
        "hieronymus_evidence_capture",
    ]);
    assert_eq!(served_names, snapshot_names);
    assert_eq!(
        served_names.len(),
        snapshot["derived_tool_count"].as_u64().unwrap() as usize + 4
    );
    assert_eq!(
        PROTOCOL_REVISION,
        snapshot["protocol_revision"].as_str().unwrap()
    );
}

// Frozen transport envelopes remain the baseline; active Rust tools/list
// carries the versioned input additions qualified in application_authority_context.
fn current_protocol() -> Value {
    let mut protocol = mcp_protocol();
    protocol["target"]["tools_list"]["response"]["result"]["tools"] =
        serde_json::to_value(hiero::daemon::registry::McpRegistry::embedded().list_tools())
            .unwrap();
    for key in ["tools_list", "tools_call"] {
        protocol["target"][key]["response"]["result"]["_meta"] = json!({"io.modelcontextprotocol/serverInfo":{"name":"hieronymus","version":env!("CARGO_PKG_VERSION")}});
    }
    protocol
}

#[test]
fn modern_discovery_origin_and_optional_call_flow() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let mut body = json!({"jsonrpc":"2.0","id":0,"method":"server/discover","params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"codex-mcp-client","title":"Codex","version":"0.147.0"},
        "io.modelcontextprotocol/clientCapabilities":{"elicitation":{"form":{},"url":{}}}}}});
    let mut headers = mcp_headers(&daemon, &[("Mcp-Method", "server/discover")]);
    headers.push(("Origin".into(), "https://hostile.example".into()));
    assert_eq!(post_mcp(&daemon, &headers, &body).status, 403);
    headers.pop();
    headers.push(("Origin".into(), format!("http://{}", daemon.local_addr())));
    let response = post_mcp(&daemon, &headers, &body);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body()["result"]["supportedVersions"],
        json!([PROTOCOL_REVISION])
    );
    body["method"] = json!("tools/list");
    body["id"] = json!("server-discover-probe-1");
    let response = post_mcp(
        &daemon,
        &mcp_headers(&daemon, &[("Mcp-Method", "tools/list")]),
        &body,
    );
    assert!(response.body()["result"]["tools"].as_array().is_some());
    body["method"] = json!("tools/call");
    body["params"]["name"] = json!("hieronymus_status");
    let response = post_mcp(
        &daemon,
        &mcp_headers(
            &daemon,
            &[
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "=?base64?aGllcm9ueW11c19zdGF0dXM=?="),
            ],
        ),
        &body,
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.body()["result"]["isError"], false);
    body["method"] = json!("unknown/method");
    assert_eq!(
        post_mcp(
            &daemon,
            &mcp_headers(&daemon, &[("Mcp-Method", "unknown/method")]),
            &body
        )
        .status,
        404
    );
}

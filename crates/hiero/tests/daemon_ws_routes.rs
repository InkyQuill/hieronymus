//! Admin websocket route contract tests against the frozen fixture target
//! (`websocket.route.get.ws.admin`, `target` block): guard order
//! invalid-host → session → Origin, the RFC 6455 handshake with the
//! fixture-frozen `Connection: Upgrade` / `Upgrade: websocket` header set,
//! end-to-end dream event delivery, resume replay, and subscriber cleanup.
//! Per the Task 2 ruling, stale CSRF machinery is never asserted.

mod common;

use common::{
    RawResponse, RouteFixture, WS_FIXTURE_ACCEPT, browser_headers, route_target, same_origin,
    send_request, start_daemon_with_browser_session, substitute_route_placeholders, wait_until,
    ws_connect, ws_upgrade_headers,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const FRAME_POLL: Duration = Duration::from_millis(20);

/// Send the frozen fixture request block (already substituted) as a plain
/// HTTP request. The recorded logical body is ignored by the server for
/// guard failures, exactly like the REST runner.
fn execute_fixture_request(request: &Value, fixture: &RouteFixture) -> RawResponse {
    let method = request["method"].as_str().unwrap();
    let path = request["path"].as_str().unwrap();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in request["headers"].as_object().unwrap() {
        let value = value
            .as_str()
            .unwrap_or_default()
            .replace("<EXPIRED_SESSION>", "expired-session-placeholder");
        headers.push((name.clone(), value));
    }
    send_request(fixture.port, method, path, &headers, b"")
}

/// Read the next server frame as a JSON event envelope before the deadline.
fn read_event(client: &mut common::WsClient, deadline: Instant, context: &str) -> (u64, Value) {
    let (opcode, payload) = client
        .read_frame(deadline)
        .unwrap_or_else(|| panic!("no {context} frame arrived before the deadline"));
    assert_eq!(opcode, 0x1, "{context} must arrive as a text frame");
    let event: Value = serde_json::from_slice(&payload).unwrap_or_else(|error| {
        panic!(
            "{context} frame is not JSON: {error}; bytes: {:02x?}; text: {:?}",
            payload,
            String::from_utf8_lossy(&payload)
        )
    });
    let event_id = event["event_id"]
        .as_u64()
        .unwrap_or_else(|| panic!("{context} frame carries no numeric event_id: {event}"));
    (event_id, event)
}

#[test]
fn ws_admin_matches_frozen_target() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let target =
        substitute_route_placeholders(&route_target("websocket.route.get.ws.admin"), &fixture);

    // Every frozen failure: invalid-host → session → Origin guard order.
    for failure in target["failures"].as_array().unwrap() {
        let id = failure["id"].as_str().unwrap_or("unnamed");
        let response = execute_fixture_request(&failure["request"], &fixture);
        assert_eq!(
            u64::from(response.status),
            failure["response"]["status"].as_u64().unwrap(),
            "websocket.route.get.ws.admin/{id} status"
        );
        assert_eq!(
            response.content_type, JSON_CONTENT_TYPE,
            "{id} content type"
        );
        assert_eq!(
            response.body(),
            failure["response"]["body"],
            "websocket.route.get.ws.admin/{id} body"
        );
    }

    // Frozen success: 101 with exactly the frozen header set plus the
    // RFC 6455 accept key.
    let success_headers: Vec<(String, String)> = target["success"]["request"]["headers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
        .collect();
    let (handshake, mut client) = ws_connect(fixture.port, &success_headers);
    assert_eq!(handshake.status, 101, "handshake must switch protocols");
    assert_eq!(
        handshake.headers.get("connection").map(String::as_str),
        Some("Upgrade"),
        "frozen Connection header"
    );
    assert_eq!(
        handshake.headers.get("upgrade").map(String::as_str),
        Some("websocket"),
        "frozen Upgrade header"
    );
    assert_eq!(
        handshake
            .headers
            .get("sec-websocket-accept")
            .map(String::as_str),
        Some(WS_FIXTURE_ACCEPT),
        "RFC 6455 accept key for the frozen Sec-WebSocket-Key"
    );

    // The fixture records the logical resume request as the upgrade body; per
    // the controller ruling it rides as the client's first text frame. A
    // fresh daemon retained nothing at or below id 41, so nothing replays.
    client.send_text(r#"{"resume_from_event_id": 41}"#);
    client.close();
}

#[test]
fn ws_admin_requires_websocket_upgrade_headers() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let port = fixture.port;
    // Authorized browser context, but no Upgrade header and no key.
    let missing_upgrade = send_request(
        port,
        "GET",
        "/ws/admin",
        &browser_headers(&fixture, &[("Origin", same_origin(port))]),
        b"",
    );
    assert_eq!(missing_upgrade.status, 400);
    assert_eq!(missing_upgrade.content_type, JSON_CONTENT_TYPE);
    assert_eq!(
        missing_upgrade.body(),
        json!({"error": "websocket_upgrade_required"})
    );

    // Upgrade header without a Sec-WebSocket-Key (Python reference behavior).
    let missing_key = send_request(
        port,
        "GET",
        "/ws/admin",
        &browser_headers(
            &fixture,
            &[
                ("Origin", same_origin(port)),
                ("Upgrade", "websocket".to_string()),
            ],
        ),
        b"",
    );
    assert_eq!(missing_key.status, 400);
    assert_eq!(
        missing_key.body(),
        json!({"error": "websocket_upgrade_required"})
    );

    // A non-websocket Upgrade value fails the same way.
    let wrong_upgrade = send_request(
        port,
        "GET",
        "/ws/admin",
        &browser_headers(
            &fixture,
            &[
                ("Origin", same_origin(port)),
                ("Upgrade", "h2c".to_string()),
                ("Sec-WebSocket-Key", common::WS_FIXTURE_KEY.to_string()),
            ],
        ),
        b"",
    );
    assert_eq!(wrong_upgrade.status, 400);
    assert_eq!(
        wrong_upgrade.body(),
        json!({"error": "websocket_upgrade_required"})
    );
}

#[test]
fn ws_admin_delivers_dream_events_end_to_end() {
    let (fixture, _root, daemon) = start_daemon_with_browser_session();
    let port = fixture.port;
    let (_, mut client) = ws_connect(port, &ws_upgrade_headers(&fixture));
    // No resume frame: the stream runs live from the next event.

    let response = send_request(
        port,
        "POST",
        "/api/admin/actions/run_manual_dreaming",
        &browser_headers(&fixture, &[("Origin", same_origin(port))]),
        br#"{}"#,
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body(),
        json!({"started": true, "status": "running"})
    );

    // dream_started is published before the route answers, so it is the
    // first frame.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (first_id, started) = read_event(&mut client, deadline, "dream_started");
    assert_eq!(
        first_id, 1,
        "event ids are daemon-lifetime monotonic from 1"
    );
    assert_eq!(started["version"], json!(1), "frozen protocol version");
    assert_eq!(started["event_type"], json!("dream_started"));
    assert_eq!(started["payload"], json!({"trigger": "manual"}));

    // The deterministic run finishes shortly after: dream_completed carries
    // the serialized run record; any interleaved frame can only be a phase
    // progress event.
    let mut last_id = first_id;
    let mut frames: Vec<Value> = vec![started];
    let completed = loop {
        let (event_id, event) = read_event(&mut client, deadline, "subsequent dream");
        assert!(
            event_id > last_id,
            "event ids must increase: {event_id} after {last_id}"
        );
        last_id = event_id;
        match event["event_type"].as_str().unwrap_or_default() {
            "dream_completed" => break event,
            "dream_phase_progress" => {
                let payload = &event["payload"];
                assert!(payload["run_id"].is_i64(), "phase payload run_id: {event}");
                assert!(
                    payload["cycle_id"].is_i64(),
                    "phase payload cycle_id: {event}"
                );
                assert!(payload["phase"].is_string(), "phase payload phase: {event}");
                frames.push(event);
            }
            other => panic!("unexpected event type on the admin stream: {other}"),
        }
    };
    assert_eq!(completed["payload"]["trigger"], json!("manual"));
    let result = &completed["payload"]["result"];
    for key in [
        "id",
        "cycle_id",
        "status",
        "provider",
        "input_count",
        "created_crystal_count",
        "proposal_count",
        "error",
    ] {
        assert!(result.get(key).is_some(), "dream result.{key} missing");
    }
    assert_eq!(
        result["status"],
        json!("completed"),
        "empty deterministic run"
    );

    // Secret discipline: the session cookie and bearer never ride frames.
    let rendered = serde_json::to_string(&frames).unwrap();
    let bearer = daemon.bearer().expose_secret();
    assert!(
        !rendered.contains(&fixture.session),
        "session leaked into frames"
    );
    assert!(
        !rendered.contains(bearer.as_str()),
        "bearer leaked into frames"
    );

    client.close();
}

#[test]
fn ws_admin_resume_replays_retained_events() {
    let (fixture, root, _daemon) = start_daemon_with_browser_session();
    let port = fixture.port;

    // One completed manual run: dream_started, phase events, dream_completed.
    let response = send_request(
        port,
        "POST",
        "/api/admin/actions/run_manual_dreaming",
        &browser_headers(&fixture, &[("Origin", same_origin(port))]),
        br#"{}"#,
    );
    assert_eq!(response.status, 200);
    let database = root.path().join("hieronymus.sqlite");
    let deadline = Instant::now() + Duration::from_secs(10);
    while rusqlite::Connection::open(&database)
        .and_then(|connection| {
            connection.query_row(
                "select count(*) from dream_runs where status = 'completed'",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .unwrap_or(0)
        < 1
    {
        assert!(
            Instant::now() < deadline,
            "manual dream run never completed"
        );
        std::thread::sleep(FRAME_POLL);
    }

    // Resume from 0 replays the whole retained history contiguously.
    let (_, mut client) = ws_connect(port, &ws_upgrade_headers(&fixture));
    client.send_text(r#"{"resume_from_event_id": 0}"#);
    let deadline = Instant::now() + Duration::from_secs(10);
    let (first_id, first) = read_event(&mut client, deadline, "first replayed");
    assert_eq!(first_id, 1, "replay starts at the first retained event");
    assert_eq!(first["event_type"], json!("dream_started"));
    let mut last_id = first_id;
    let completed_id = loop {
        let (event_id, event) = read_event(&mut client, deadline, "replayed");
        assert_eq!(event_id, last_id + 1, "replay must be contiguous, no gaps");
        last_id = event_id;
        if event["event_type"] == json!("dream_completed") {
            break event_id;
        }
        assert_eq!(
            event["event_type"],
            json!("dream_phase_progress"),
            "only dream events are retained"
        );
    };
    client.close();

    // Resume from the last event replays nothing; a partial window replays
    // only the later events.
    let (_, mut current_client) = ws_connect(port, &ws_upgrade_headers(&fixture));
    current_client.send_text(&format!(r#"{{"resume_from_event_id": {completed_id}}}"#));
    // The daemon publishes nothing further here; a frame now would be a bug.
    let quiet = Instant::now() + Duration::from_millis(300);
    assert!(
        current_client.read_frame(quiet).is_none(),
        "current clients must not receive a replay"
    );

    let (_, mut partial_client) = ws_connect(port, &ws_upgrade_headers(&fixture));
    partial_client.send_text(r#"{"resume_from_event_id": 1}"#);
    // Read the partial replay to the end of the retained window: contiguity
    // is what matters; phase events may or may not have been sampled for a
    // run this fast.
    let mut last = 1;
    loop {
        let (event_id, event) = read_event(&mut partial_client, deadline, "partial replay");
        assert_eq!(event_id, last + 1, "partial replay must be contiguous");
        last = event_id;
        if event["event_type"] == json!("dream_completed") {
            break;
        }
        assert_eq!(event["event_type"], json!("dream_phase_progress"));
    }
    assert_eq!(
        last, completed_id,
        "the partial replay ends at the same newest event"
    );
    partial_client.close();
}

#[test]
fn ws_client_close_unsubscribes_from_the_hub() {
    let (fixture, _root, daemon) = start_daemon_with_browser_session();
    assert_eq!(
        daemon.admin_subscriber_count(),
        0,
        "fresh daemon has no subscribers"
    );

    let (_, client) = ws_connect(fixture.port, &ws_upgrade_headers(&fixture));
    assert!(
        wait_until(
            || daemon.admin_subscriber_count() == 1,
            Duration::from_secs(5)
        ),
        "the upgrade must subscribe the client"
    );

    client.close();
    assert!(
        wait_until(
            || daemon.admin_subscriber_count() == 0,
            Duration::from_secs(5)
        ),
        "client close must unsubscribe from the hub"
    );

    // The daemon keeps serving after the disconnect.
    let health = send_request(fixture.port, "GET", "/health", &[], b"");
    assert_eq!(health.status, 200);
}

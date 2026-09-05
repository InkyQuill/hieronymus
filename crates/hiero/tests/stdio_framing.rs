//! Stdio framing contract tests: the frozen newline-delimited JSON-RPC
//! exchanges from `compatibility/fixtures/mcp/protocol.json` (target.stdio),
//! served through the registry, the `hiero mcp` adapter end-to-end, and the
//! adapter's fail-closed behavior without discovery.

mod common;

use common::mcp_protocol;
use hiero::daemon::protocol::{PROTOCOL_REVISION, process_request};
use hiero::daemon::registry::McpRegistry;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn frozen_exchanges() -> Vec<(String, String)> {
    mcp_protocol()["target"]["stdio"]["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|exchange| {
            (
                exchange["request_line"].as_str().unwrap().to_string(),
                exchange["response_line"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn stdio_exchanges_match_the_frozen_wire_lines_byte_for_byte() {
    let registry = McpRegistry::embedded();
    for (index, (request_line, expected_line)) in frozen_exchanges().into_iter().enumerate() {
        let request: Value = serde_json::from_str(request_line.trim_end())
            .unwrap_or_else(|error| panic!("exchange {index} request is not JSON: {error}"));
        let response = process_request(&registry, &request);
        let actual = format!("{}\n", serde_json::to_string(&response).unwrap());
        assert_eq!(actual, expected_line, "exchange {index} wire line mismatch");
    }
}

#[test]
fn stdio_exchange_results_match_the_target_fixtures() {
    let protocol = mcp_protocol();
    let registry = McpRegistry::embedded();
    for (index, (request_line, _)) in frozen_exchanges().into_iter().enumerate() {
        let request: Value = serde_json::from_str(request_line.trim_end()).unwrap();
        let response = process_request(&registry, &request);
        let expected = if index == 0 {
            &protocol["target"]["tools_list"]["response"]
        } else {
            &protocol["target"]["tools_call"]["response"]
        };
        assert_eq!(response, *expected, "exchange {index}");
    }
}

#[test]
fn stdio_adapter_proxies_to_a_live_daemon_end_to_end() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let discovery_path = root.path().join("daemon.json");
    let mut record = None;
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&discovery_path) {
            record = serde_json::from_str::<Value>(&text).ok();
            if record.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let record = record.expect("daemon must publish discovery before the adapter starts");

    let mut adapter = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    for (request_line, expected_line) in frozen_exchanges() {
        adapter
            .stdin
            .as_mut()
            .unwrap()
            .write_all(request_line.as_bytes())
            .unwrap();
        let mut line = String::new();
        BufReader::new(adapter.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(
            line, expected_line,
            "adapter must proxy the exact wire line"
        );
    }

    adapter.kill().unwrap();
    let _ = adapter.wait();
    daemon.kill().unwrap();
    let _ = daemon.wait();
    // Sanity: discovery pointed at the daemon the adapter used.
    assert_eq!(record["protocol_version"], json!(PROTOCOL_REVISION));
}

#[test]
fn stdio_adapter_fails_closed_without_discovery() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("no running local service discovered"),
        "{stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "a failed adapter must not write protocol output"
    );
}

#[test]
fn stdio_adapter_without_autostart_flag_reports_unreachable_daemon() {
    let root = tempfile::tempdir().unwrap();

    // A stale discovery record for a daemon that is not running (the owner
    // explicitly tolerates stale discovery in this slice).
    let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let dead_port = holder.local_addr().unwrap().port();
    drop(holder);
    let record = json!({
        "discovery_version": 1,
        "protocol_version": PROTOCOL_REVISION,
        "host": "127.0.0.1",
        "port": dead_port,
        "pid": 1,
        "instance_id": "ab".repeat(16),
        "started_at": "2026-09-04T00:00:00+00:00"
    });
    std::fs::write(
        root.path().join("daemon.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    std::fs::write(root.path().join("daemon.token"), b"test-token-0001\n").unwrap();

    let exchanges = frozen_exchanges();
    let (request_line, _) = exchanges.first().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request_line.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unreachable"), "{stderr}");
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["jsonrpc"], json!("2.0"));
    assert_eq!(response["id"], json!(1));
    assert_eq!(response["error"]["code"], json!(-32603));
    let message = response["error"]["message"].as_str().unwrap();
    assert!(!message.contains("test-token-0001"), "{message}");
}

#[test]
fn stdio_adapter_derives_mirrored_headers_from_the_body() {
    // The adapter is a plain proxy: point it at a live daemon, send a
    // tools/call, and confirm the daemon accepts the derived Mcp-Method and
    // Mcp-Name headers (any header mismatch would fail with -32020).
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());

    let exchanges = frozen_exchanges();
    let (request_line, expected_line) = exchanges[1].clone();

    let mut adapter = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    adapter
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request_line.as_bytes())
        .unwrap();
    let mut line = String::new();
    BufReader::new(adapter.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();

    assert_eq!(line, expected_line);
    adapter.kill().unwrap();
    let _ = adapter.wait();
    daemon.shutdown().unwrap();
}

#[test]
fn stdio_adapter_start_daemon_flag_boots_the_daemon_it_proxies_to() {
    // Explicit opt-in: with an empty data root and --start-daemon, the adapter
    // spawns a daemon, waits for its discovery record, and proxies normally.
    // (Without the flag the same setup fails closed — covered above.)
    let root = tempfile::tempdir().unwrap();
    let mut adapter = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "mcp",
            "--data-root",
            root.path().to_str().unwrap(),
            "--start-daemon",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let exchanges = frozen_exchanges();
    let (request_line, expected_line) = exchanges[0].clone();
    adapter
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request_line.as_bytes())
        .unwrap();
    let mut line = String::new();
    BufReader::new(adapter.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(
        line, expected_line,
        "autostart path must proxy the wire line"
    );

    adapter.kill().unwrap();
    let _ = adapter.wait();

    // The autostarted daemon outlives its parent; stop it via the pid it
    // published so no stray listener survives the test run.
    let record: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("daemon.json")).unwrap()).unwrap();
    let pid = record["pid"].as_u64().unwrap().to_string();
    Command::new("kill")
        .arg(&pid)
        .status()
        .expect("kill must be available");
}

#[test]
fn stdio_adapter_wraps_route_level_daemon_errors_as_jsonrpc() {
    // A daemon restart regenerates the bearer token, so an adapter holding the
    // stale credential receives the daemon's route-level
    // `401 {"error":"unauthorized"}` — not a JSON-RPC envelope. The adapter
    // must wrap it so the NDJSON stream stays protocol-clean.
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The daemon writes the token before publishing discovery, so existence
    // of the record implies both files are ready for the adapter.
    let discovery_path = root.path().join("daemon.json");
    let mut ready = false;
    for _ in 0..100 {
        if discovery_path.exists() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        ready,
        "daemon must publish discovery before the adapter starts"
    );

    // The daemon keeps its in-memory token, so overwriting the file before
    // the adapter starts makes the adapter deterministically read a stale
    // credential (invalidating after `spawn` would race the adapter's
    // startup read under load).
    std::fs::write(root.path().join("daemon.token"), b"stale-token\n").unwrap();

    let mut adapter = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let exchanges = frozen_exchanges();
    let (request_line, _) = exchanges[0].clone();
    adapter
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request_line.as_bytes())
        .unwrap();
    let mut line = String::new();
    BufReader::new(adapter.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();

    let response: Value =
        serde_json::from_str(line.trim_end()).expect("adapter output must stay a JSON line");
    assert_eq!(response["jsonrpc"], json!("2.0"), "line was: {line}");
    assert_eq!(response["id"], json!(1), "line was: {line}");
    assert_eq!(response["error"]["code"], json!(-32603), "line was: {line}");
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("unauthorized"), "{message}");
    assert!(
        !message.contains("stale-token"),
        "diagnostics must not echo credentials: {message}"
    );

    adapter.kill().unwrap();
    let _ = adapter.wait();
    daemon.kill().unwrap();
    let _ = daemon.wait();
}

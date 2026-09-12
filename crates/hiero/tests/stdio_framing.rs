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

fn current_response(mut value: Value) -> Value {
    if value.get("result").is_some() {
        value["result"]["_meta"] = json!({"io.modelcontextprotocol/serverInfo":{"name":"hieronymus","version":env!("CARGO_PKG_VERSION")}});
    }
    if value["result"].get("tools").is_some() {
        value["result"]["tools"] =
            serde_json::to_value(McpRegistry::embedded().list_tools()).unwrap();
    }
    value
}
fn frozen_exchanges() -> Vec<(String, String)> {
    mcp_protocol()["target"]["stdio"]["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|exchange| {
            (
                exchange["request_line"].as_str().unwrap().to_string(),
                format!(
                    "{}\n",
                    current_response(
                        serde_json::from_str(exchange["response_line"].as_str().unwrap()).unwrap()
                    )
                ),
            )
        })
        .collect()
}

#[test]
fn stdio_exchanges_preserve_historical_payload_with_current_metadata() {
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
fn stdio_exchange_results_include_current_registry_and_metadata() {
    let mut protocol = mcp_protocol();
    for key in ["tools_list", "tools_call"] {
        protocol["target"][key]["response"] =
            current_response(protocol["target"][key]["response"].clone());
    }
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

    // A stale discovery record must fail the authenticated startup check.
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

    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("no running local service discovered"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
    assert!(!stderr.contains("test-token-0001"), "{stderr}");
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

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
#[cfg(target_os = "linux")]
fn stdio_adapter_start_daemon_flag_goes_through_the_service_integration() {
    // Explicit opt-in autostart now starts the per-user SERVICE, never a raw
    // `hiero daemon` child (ADR 0009: the managed role is the service; the
    // foreground `hiero daemon` is for supervisors and debugging).
    //
    // The test isolates the service integration completely: HOME points at a
    // temp directory (so `default_unit_dir` resolves there) and PATH is
    // emptied (so `manager_enabled` is false and systemd is never contacted).
    // The install step still runs, which is exactly the evidence we want.
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "mcp",
            "--data-root",
            root.path().to_str().unwrap(),
            "--start-daemon",
        ])
        .env("HOME", home.path())
        .env("PATH", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("local service"),
        "autostart must report the service integration: {stderr}"
    );
    // The unit was rendered through the service integration ...
    let unit = home.path().join(".config/systemd/user/hieronymus.service");
    assert!(unit.exists(), "autostart must install the per-user unit");
    // ... and no raw daemon was ever spawned behind the host's back.
    assert!(
        !root.path().join("daemon.json").exists(),
        "autostart must not spawn a raw `hiero daemon` child"
    );
}

#[test]
fn stdio_adapter_rejects_invalid_startup_credentials() {
    // Invalid discovery credentials must be rejected before any MCP traffic.
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

    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("rejected this installation's credential"),
        "{stderr}"
    );
    assert!(!stderr.contains("stale-token"), "{stderr}");

    daemon.kill().unwrap();
    let _ = daemon.wait();
}

#[test]
fn captured_modern_discovery_list_call_and_notifications_over_stdio() {
    let root = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon(root.path());
    let mut adapter = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["mcp", "--data-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(adapter.stdout.take().unwrap());
    let mut input = adapter.stdin.take().unwrap();
    let mut request = json!({"jsonrpc":"2.0","id":0,"method":"server/discover","params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"codex-mcp-client","title":"Codex","version":"0.147.0"},"io.modelcontextprotocol/clientCapabilities":{"elicitation":{"form":{},"url":{}}}}}});
    for method in ["server/discover", "tools/list", "tools/call"] {
        writeln!(input,"{}",json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"nonexistent"}})).unwrap();
        request["method"] = json!(method);
        if method == "tools/call" {
            request["params"]["name"] = json!("hieronymus_status");
        }
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], request["id"]);
        assert_eq!(response["result"]["resultType"], "complete");
        if method == "server/discover" {
            assert_eq!(
                response["result"]["supportedVersions"],
                json!([PROTOCOL_REVISION])
            );
        }
        request["id"] = json!("server-discover-probe-1");
    }
    request["id"] = Value::Null;
    writeln!(input, "{request}").unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
        -32600
    );
    drop(input);
    assert!(adapter.wait().unwrap().success());
    daemon.shutdown().unwrap();
}

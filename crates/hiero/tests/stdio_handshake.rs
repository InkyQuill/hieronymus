//! Exercise ordinary host messages through the real stdio executable and daemon.
mod common;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Host {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<Value>,
}

impl Host {
    fn start(root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args(["mcp", "--data-root"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, responses) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let value = serde_json::from_str(&line.unwrap()).unwrap();
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            input: child.stdin.take(),
            child,
            responses,
        }
    }

    fn send(&mut self, request: Value) {
        writeln!(self.input.as_mut().unwrap(), "{request}").unwrap();
    }

    fn receive(&self) -> Value {
        self.responses
            .recv_timeout(Duration::from_secs(10))
            .expect("adapter response")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize(version: &str) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":version,"capabilities":{"roots":{"listChanged":true}},
        "clientInfo":{"name":"standard-mcp-repro","version":"1"}
    }})
}

#[test]
fn standard_host_initializes_discovers_and_calls_tools_without_internal_metadata() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    for version in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
        let mut host = Host::start(root.path());
        host.send(initialize(version));
        let response = host.receive();
        assert_eq!(response["result"]["protocolVersion"], version, "{response}");
        assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
        assert_eq!(response["result"]["serverInfo"]["name"], "hieronymus");
        host.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        host.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        let response = host.receive();
        assert_eq!(response["id"], 2, "notifications must not get replies");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tool schemas");
        assert!(tools.iter().any(|tool| tool["name"] == "hieronymus_status"));
        host.send(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":"hieronymus_status","arguments":{}
            }}),
        );
        let response = host.receive();
        assert!(response.get("error").is_none(), "{response}");
        assert!(response["result"]["content"].is_array(), "{response}");
        assert_ne!(response["result"]["isError"], true, "{response}");
        assert_eq!(
            response["result"]["structuredContent"]["service"]["mode"],
            "local-http"
        );
        host.send(json!({"jsonrpc":"2.0","id":4,"method":"ping"}));
        assert_eq!(host.receive(), json!({"jsonrpc":"2.0","id":4,"result":{}}));
    }
    daemon.shutdown().unwrap();
}

#[test]
fn unsupported_host_version_negotiates_a_supported_version() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let mut host = Host::start(root.path());
    host.send(initialize("2099-01-01"));
    assert_eq!(host.receive()["result"]["protocolVersion"], "2025-11-25");
    daemon.shutdown().unwrap();
}

#[test]
fn malformed_initialization_does_not_poison_session_and_duplicate_is_rejected() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let mut host = Host::start(root.path());
    for field in ["protocolVersion", "capabilities", "clientInfo"] {
        let mut request = initialize("2025-06-18");
        request["params"].as_object_mut().unwrap().remove(field);
        host.send(request);
        assert_eq!(host.receive()["error"]["code"], -32602);
    }
    host.send(initialize("2025-06-18"));
    assert_eq!(host.receive()["result"]["protocolVersion"], "2025-06-18");
    host.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert_eq!(host.receive()["error"]["code"], -32600);
    host.send(initialize("2025-11-25"));
    assert_eq!(host.receive()["error"]["code"], -32600);
    host.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    for params in [json!(null), json!([]), json!({"_meta":null})] {
        host.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":params}));
        assert_eq!(host.receive()["error"]["code"], -32602);
    }
    host.send(json!({"jsonrpc":"2.0","id":4,"method":"resources/list"}));
    assert_eq!(host.receive()["error"]["code"], -32601);
    host.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/list","params":{}}));
    assert!(host.receive()["result"]["tools"].is_array());
    daemon.shutdown().unwrap();
}

#[test]
fn pipelined_handshake_orders_state_before_dispatch_and_ignores_host_protocol_metadata() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let mut host = Host::start(root.path());
    host.send(initialize("2025-06-18"));
    host.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    host.send(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{
            "io.modelcontextprotocol/protocolVersion":"2025-06-18",
            "io.modelcontextprotocol/clientCapabilities":{"sampling":{}},
            "progressToken":"test-list"
        }}}),
    );
    assert_eq!(host.receive()["result"]["protocolVersion"], "2025-06-18");
    let response = host.receive();
    assert!(response["result"]["tools"].is_array(), "{response}");
    daemon.shutdown().unwrap();
}

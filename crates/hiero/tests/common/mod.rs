//! Shared helpers for the `hiero` daemon contract tests: raw loopback HTTP,
//! frozen fixture loading, and placeholder substitution. Deliberately
//! independent of the production HTTP client so contract checks stay
//! end-to-end.

#![allow(dead_code)]

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PROTOCOL_REVISION: &str = "2026-07-28";
pub const INVALID_BEARER: &str = "hieronymus-invalid-bearer";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn fixture(relative: &str) -> Value {
    let bytes = std::fs::read(repo_root().join(relative)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn mcp_protocol() -> Value {
    fixture("compatibility/fixtures/mcp/protocol.json")
}

pub fn route_target(route_id: &str) -> Value {
    route_cases()["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["contract_id"] == route_id)
        .unwrap_or_else(|| panic!("route {route_id} is missing from route-cases.json"))["target"]
        .clone()
}

pub fn route_cases() -> Value {
    fixture("compatibility/fixtures/http/route-cases.json")
}

/// Replace fixture placeholders: `<PORT>` with the daemon's bound port and
/// `<INVALID_BEARER_TOKEN>` with a fixed wrong credential.
pub fn substitute_placeholders(value: &Value, port: u16) -> Value {
    match value {
        Value::String(text) => Value::String(
            text.replace("<PORT>", &port.to_string())
                .replace("<INVALID_BEARER_TOKEN>", INVALID_BEARER),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_placeholders(item, port))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), substitute_placeholders(value, port)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub struct RawResponse {
    pub status: u16,
    pub content_type: String,
    pub raw_body: Vec<u8>,
}

impl RawResponse {
    pub fn body(&self) -> Value {
        serde_json::from_slice(&self.raw_body)
            .unwrap_or_else(|error| panic!("response body is not JSON: {error}: {:?}", self.raw_body))
    }
}

pub fn send_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> RawResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> RawResponse {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("response has no header separator: {:?}", raw));
    let head = std::str::from_utf8(&raw[..separator]).unwrap();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_type = headers.get("content-type").cloned().unwrap_or_default();
    let full_body = &raw[separator + 4..];
    let body = match headers.get("content-length").and_then(|v| v.parse().ok()) {
        Some(length) => &full_body[..length],
        None => full_body,
    };
    RawResponse {
        status,
        content_type,
        raw_body: body.to_vec(),
    }
}

pub fn start_daemon_on_ephemeral_port() -> (tempfile::TempDir, hiero::daemon::Daemon) {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    (root, daemon)
}

pub fn start_daemon(root: &Path) -> hiero::daemon::Daemon {
    hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.to_path_buf()),
        port: 0,
    })
    .unwrap()
}

/// Headers every authorized POST /mcp request carries, mirroring the frozen
/// route-case success requests.
pub fn mcp_headers(daemon: &hiero::daemon::Daemon, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "Accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
        (
            "MCP-Protocol-Version".to_string(),
            PROTOCOL_REVISION.to_string(),
        ),
    ];
    for (name, value) in extra {
        headers.push(((*name).to_string(), (*value).to_string()));
    }
    headers
}

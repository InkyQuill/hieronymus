//! `hiero mcp`: the stdio MCP adapter (ADR 0009/0015). It exchanges
//! newline-delimited JSON-RPC on stdin/stdout, discovers and authenticates to
//! the local daemon, and proxies every request to `POST /mcp`. It never opens
//! SQLite and never invents protocol behavior; the daemon is the contract
//! enforcement point. Diagnostics go to stderr and are bounded.
//!
//! Startup: read the discovery record and the token from the data root. If
//! discovery is missing and `--start-daemon` was given, the adapter starts
//! `hiero daemon` in the background and waits for its record; the flag
//! defaults to false (an MCP host that wants autostart opts in explicitly).

use std::io::{BufRead, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use hieronymus::data_root::load_config;
use hieronymus::secret::Secret;

use crate::client;
use crate::daemon::discovery::{
    read_discovery, read_token, DiscoveryError, DiscoveryRecord, CredentialError,
};
use crate::daemon::registry::PROTOCOL_REVISION;

const MAX_LINE_BYTES: usize = 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 480;
const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct StdioOptions {
    pub data_root: Option<PathBuf>,
    /// Explicit opt-in to spawning the daemon; default false (documented).
    pub start_daemon: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StdioError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("stdio i/o failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("refusing to connect to a non-loopback discovery address: {0}")]
    NotLoopback(String),
    #[error("discovery record is malformed: {0}")]
    Malformed(&'static str),
    #[error("daemon is not running and --start-daemon is not allowed")]
    DaemonNotRunning,
    #[error("cannot start the daemon: {0}")]
    Spawn(String),
    #[error("timed out waiting for the daemon discovery record")]
    AutostartTimeout,
}

pub fn run_stdio_adapter(options: &StdioOptions) -> Result<(), StdioError> {
    let config = load_config(options.data_root.as_deref());
    let record = match read_discovery(&config) {
        Ok(record) => record,
        Err(DiscoveryError::Missing { .. }) if options.start_daemon => {
            start_daemon_and_wait(&config)?
        }
        Err(error) => return Err(error.into()),
    };
    let token = read_token(&config)?;
    let address = endpoint_address(&record)?;
    proxy_stdin(&address, &token)
}

fn endpoint_address(record: &DiscoveryRecord) -> Result<SocketAddr, StdioError> {
    let ip: IpAddr = record
        .host
        .parse()
        .map_err(|_| StdioError::Malformed("host is not an IP address"))?;
    if !ip.is_loopback() {
        return Err(StdioError::NotLoopback(record.host.clone()));
    }
    Ok(SocketAddr::new(ip, record.port))
}

fn start_daemon_and_wait(
    config: &hieronymus::data_root::HieronymusConfig,
) -> Result<DiscoveryRecord, StdioError> {
    let executable = std::env::current_exe()
        .map_err(|error| StdioError::Spawn(error.to_string()))?;
    let data_root = config.data_root().to_path_buf();
    let mut command = Command::new(&executable);
    command
        .arg("daemon")
        .arg("--data-root")
        .arg(&data_root)
        .arg("--port")
        .arg("0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .map_err(|error| StdioError::Spawn(error.to_string()))?;

    let deadline = Instant::now() + AUTOSTART_TIMEOUT;
    loop {
        match read_discovery(config) {
            Ok(record) => return Ok(record),
            Err(DiscoveryError::Missing { .. }) if Instant::now() >= deadline => {
                return Err(StdioError::AutostartTimeout);
            }
            Err(DiscoveryError::Missing { .. }) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn proxy_stdin(address: &SocketAddr, token: &Secret<String>) -> Result<(), StdioError> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            return Ok(());
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() > MAX_LINE_BYTES {
            let _ = writeln!(
                writer,
                "{}",
                serde_json::json!({
                    "jsonrpc": "2.0", "id": null, "error": {
                        "code": -32602, "message": "request exceeds size limit"
                    }
                })
            );
            let _ = writer.flush();
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Err(_) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0", "id": null, "error": {
                            "code": -32700, "message": "Parse error"
                        }
                    })
                );
                let _ = writer.flush();
            }
            Ok(body) => {
                let response = proxy_request(address, token, &body);
                let _ = writeln!(writer, "{response}");
                let _ = writer.flush();
            }
        }
    }
}

/// Forward one request: derive the mirrored headers from the body, POST it,
/// and return the daemon's JSON-RPC response. Transport failures become a
/// clean JSON-RPC error (and a bounded stderr diagnostic) so a batch session
/// does not corrupt its framing.
fn proxy_request(
    address: &SocketAddr,
    token: &Secret<String>,
    body: &serde_json::Value,
) -> serde_json::Value {
    let id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
    match forward(address, token, body) {
        Ok((_status, response_body)) => {
            serde_json::from_slice(&response_body).unwrap_or_else(|_| {
                crate::daemon::protocol::error_response(
                    id,
                    -32603,
                    "daemon returned a non-JSON response",
                    None,
                )
            })
        }
        Err(error) => {
            diagnostic(&format!("daemon unreachable: {error}"));
            crate::daemon::protocol::error_response(
                id,
                -32603,
                "daemon is unreachable; start it with 'hiero daemon'",
                None,
            )
        }
    }
}

fn forward(
    address: &SocketAddr,
    token: &Secret<String>,
    body: &serde_json::Value,
) -> Result<(u16, Vec<u8>), client::ClientError> {
    let method = body.get("method").and_then(serde_json::Value::as_str);
    let mut headers = vec![
        ("Host".to_string(), address.to_string()),
        (
            "Authorization".to_string(),
            format!("Bearer {}", token.expose_secret()),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let version = body
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(PROTOCOL_REVISION);
    headers.push(("MCP-Protocol-Version".to_string(), version.to_string()));
    if let Some(method) = method {
        headers.push(("Mcp-Method".to_string(), method.to_string()));
        if matches!(method, "tools/call" | "resources/read" | "prompts/get") {
            if let Some(name) = body
                .pointer("/params/name")
                .and_then(serde_json::Value::as_str)
            {
                headers.push(("Mcp-Name".to_string(), name.to_string()));
            }
        }
    }
    let payload = serde_json::to_vec(body)
        .map_err(|_| client::ClientError::Protocol("request cannot serialize"))?;
    client::post_json(*address, "/mcp", &headers, &payload)
}

/// One bounded line on stderr; never includes the token (secrets are
/// redacted by construction — errors here carry only transport text).
fn diagnostic(message: &str) {
    let mut text: String = message.chars().take(DIAGNOSTIC_LIMIT).collect();
    if message.chars().count() > DIAGNOSTIC_LIMIT {
        text.push('…');
    }
    eprintln!("hiero mcp: {text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_must_be_loopback() {
        let mut record = DiscoveryRecord {
            discovery_version: 1,
            protocol_version: PROTOCOL_REVISION.to_string(),
            host: "10.1.2.3".to_string(),
            port: 9768,
            pid: 1,
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        let error = endpoint_address(&record).unwrap_err();
        assert!(error.to_string().contains("non-loopback"), "{error}");
        record.host = "127.0.0.1".to_string();
        assert_eq!(endpoint_address(&record).unwrap().port(), 9768);
    }
}

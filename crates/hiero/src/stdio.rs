//! `hiero mcp`: the stdio MCP adapter (ADR 0009/0015). It exchanges
//! newline-delimited JSON-RPC on stdin/stdout, discovers and authenticates to
//! the local daemon, and proxies every request to `POST /mcp`. It never opens
//! SQLite and never invents protocol behavior; the daemon is the contract
//! enforcement point. Diagnostics go to stderr and are bounded.
//!
//! Startup: run the shared authenticated discovery probe (ADR 0009 — a live
//! endpoint is one that answers the authenticated `GET /status` with the same
//! process instance the record claims, never a PID or a bare TCP connect). If
//! no live daemon answers and `--start-daemon` was given, the adapter starts
//! the per-user **service** — not a raw `hiero daemon` child — and waits for a
//! live endpoint; the flag defaults to false (an MCP host that wants autostart
//! opts in explicitly). A discovery record the probe proved stale is repaired
//! on that path only, and only after that proof.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use hieronymus::data_root::load_config;

use crate::daemon::discovery::{CredentialError, DiscoveryError};
use crate::lifecycle;

const MAX_LINE_BYTES: usize = 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 480;

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
    /// Keeps the historical wording: MCP hosts and the frozen CLI behavior
    /// match on this prefix.
    #[error("no running local service discovered: {0} (start it with `hiero start`)")]
    DaemonNotRunning(String),
    #[error("cannot start the local service: {0}")]
    Spawn(String),
}

pub fn run_stdio_adapter(options: &StdioOptions) -> Result<(), StdioError> {
    let config = load_config(options.data_root.as_deref());
    let client = lifecycle::connect(&config, options.start_daemon).map_err(|error| {
        if options.start_daemon {
            StdioError::Spawn(error.to_string())
        } else {
            StdioError::DaemonNotRunning(error.to_string())
        }
    })?;
    proxy_stdin(&client)
}

fn proxy_stdin(client: &lifecycle::DaemonClient) -> Result<(), StdioError> {
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
                let response = proxy_request(client, &body);
                let _ = writeln!(writer, "{response}");
                let _ = writer.flush();
            }
        }
    }
}

/// Forward one request: derive the mirrored headers from the body, POST it,
/// and return the daemon's JSON-RPC response. Anything the daemon sends that
/// is not a JSON-RPC envelope (transport failures, non-JSON bodies, and
/// route-level errors like `401 {"error":"unauthorized"}`) becomes a clean
/// JSON-RPC error (plus a bounded stderr diagnostic) so a batch session never
/// corrupts its NDJSON framing.
fn proxy_request(client: &lifecycle::DaemonClient, body: &serde_json::Value) -> serde_json::Value {
    let id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
    match client.forward_mcp(body) {
        Ok((_status, response_body)) => {
            match serde_json::from_slice::<serde_json::Value>(&response_body) {
                // Well-formed JSON-RPC envelopes (including protocol errors
                // such as -32020) are forwarded verbatim.
                Ok(response) if response.get("jsonrpc").is_some() => response,
                // Route-level daemon errors — e.g. a 401 after a daemon
                // restart regenerated the credential — are not JSON-RPC.
                Ok(response) => {
                    diagnostic("daemon returned a non-JSON-RPC body");
                    crate::daemon::protocol::error_response(
                        id,
                        -32603,
                        &route_error_text(&response),
                        None,
                    )
                }
                Err(_) => crate::daemon::protocol::error_response(
                    id,
                    -32603,
                    "daemon returned a non-JSON response",
                    None,
                ),
            }
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

/// The daemon's route-level error text (`{"error":"unauthorized"}`) surfaced
/// inside the wrapping `-32603` envelope.
fn route_error_text(response: &serde_json::Value) -> String {
    match response.get("error").and_then(serde_json::Value::as_str) {
        Some(text) => format!("daemon rejected the request: {text}"),
        None => "daemon returned a non-JSON-RPC response".to_owned(),
    }
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

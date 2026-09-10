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
    proxy_frames(
        &mut reader,
        &mut std::io::stdout(),
        &|body, cancellation| proxy_request(client, body, cancellation),
    )
}
fn proxy_frames<R: BufRead, W: Write + Send, F>(
    reader: &mut R,
    writer: &mut W,
    forward: &F,
) -> Result<(), StdioError>
where
    F: Fn(&serde_json::Value, &crate::client::Cancellation) -> serde_json::Value + Sync,
{
    use crate::daemon::protocol::{error_response, request_id};
    use std::sync::{Arc, Mutex, mpsc};
    type Work = (serde_json::Value, Arc<crate::client::Cancellation>);
    const MAX_INFLIGHT: usize = 16;
    let writer = Mutex::new(writer);
    let inflight = Mutex::new(std::collections::HashMap::<
        String,
        Arc<crate::client::Cancellation>,
    >::new());
    let finished = std::sync::Condvar::new();
    let (sender, receiver) = mpsc::sync_channel::<Work>(MAX_INFLIGHT);
    let receiver = Mutex::new(receiver);
    let emit = |value: &serde_json::Value| -> Result<(), std::io::Error> {
        let mut writer = writer.lock().unwrap();
        writeln!(writer, "{value}")?;
        writer.flush()
    };
    std::thread::scope(|scope| -> Result<(), StdioError> {
        for _ in 0..4 {
            let receiver = &receiver;
            let inflight = &inflight;
            let emit = &emit;
            let finished = &finished;
            scope.spawn(move || {
                loop {
                    let work = receiver.lock().unwrap().recv();
                    let Ok((body, cancellation)) = work else {
                        break;
                    };
                    // Drop the receiver lock before forwarding; other workers can run.
                    let response = if cancellation.is_cancelled() {
                        None
                    } else {
                        Some(forward(&body, &cancellation))
                    };
                    let mut active = inflight.lock().unwrap();
                    if !cancellation.is_cancelled()
                        && let Some(response) = response
                    {
                        let _ = emit(&response);
                    }
                    active.remove(&request_id(&body).to_string());
                    finished.notify_all();
                }
            });
        }
        let reading = (|| -> Result<(), StdioError> {
            while let Some(frame) = read_frame(reader)? {
                let bytes = match frame {
                    Frame::TooLarge => {
                        emit(&error_response(
                            serde_json::Value::Null,
                            -32600,
                            "request exceeds size limit",
                            None,
                        ))?;
                        continue;
                    }
                    Frame::Json(bytes) => bytes,
                };
                if bytes.is_empty() {
                    continue;
                }
                let body: serde_json::Value = match serde_json::from_slice(&bytes) {
                    Ok(body) => body,
                    Err(_) => {
                        emit(&error_response(
                            serde_json::Value::Null,
                            -32700,
                            "Parse error",
                            None,
                        ))?;
                        continue;
                    }
                };
                if body.get("id").is_none()
                    && body.get("jsonrpc") == Some(&serde_json::json!("2.0"))
                    && body.get("method").is_some_and(serde_json::Value::is_string)
                {
                    if body["method"] == "notifications/cancelled"
                        && let Some(id) = body
                            .pointer("/params/requestId")
                            .filter(|id| crate::daemon::protocol::valid_id(id))
                        && let Some(cancel) = inflight.lock().unwrap().get(&id.to_string())
                    {
                        cancel.cancel();
                    }
                    continue;
                }
                let id = request_id(&body);
                let key = id.to_string();
                let mut active = inflight.lock().unwrap();
                if active.len() >= MAX_INFLIGHT || active.contains_key(&key) {
                    emit(&error_response(
                        id,
                        -32600,
                        "too many or duplicate in-flight requests",
                        None,
                    ))?;
                    continue;
                }
                let cancellation = Arc::new(crate::client::Cancellation::default());
                active.insert(key, cancellation.clone());
                sender
                    .send((body, cancellation))
                    .map_err(|_| std::io::Error::other("MCP worker stopped"))?;
            }
            Ok(())
        })();
        if reading.is_err() {
            for cancel in inflight.lock().unwrap().values() {
                cancel.cancel();
            }
        }
        // Drain fast accepted requests on EOF, then cancel remaining sockets promptly.
        drop(sender);
        let (active, _) = finished
            .wait_timeout_while(
                inflight.lock().unwrap(),
                std::time::Duration::from_millis(250),
                |active| !active.is_empty(),
            )
            .unwrap();
        for cancellation in active.values() {
            cancellation.cancel();
        }
        drop(active);
        reading
    })
}

/// Forward one request: derive the mirrored headers from the body, POST it,
/// and return the daemon's JSON-RPC response. Anything the daemon sends that
/// is not a JSON-RPC envelope (transport failures, non-JSON bodies, and
/// route-level errors like `401 {"error":"unauthorized"}`) becomes a clean
/// JSON-RPC error (plus a bounded stderr diagnostic) so a batch session never
/// corrupts its NDJSON framing.
fn proxy_request(
    client: &lifecycle::DaemonClient,
    body: &serde_json::Value,
    cancellation: &crate::client::Cancellation,
) -> serde_json::Value {
    let id = crate::daemon::protocol::request_id(body);
    match client.forward_mcp_cancellable(body, cancellation) {
        Ok((_status, response_body)) => decode_response(&response_body, &id).unwrap_or_else(|_| {
            crate::daemon::protocol::error_response(
                id,
                -32603,
                "daemon returned an invalid or mismatched response",
                None,
            )
        }),
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

/// One bounded line on stderr; never includes the token (secrets are
/// redacted by construction — errors here carry only transport text).
fn diagnostic(message: &str) {
    let mut text: String = message.chars().take(DIAGNOSTIC_LIMIT).collect();
    if message.chars().count() > DIAGNOSTIC_LIMIT {
        text.push('…');
    }
    eprintln!("hiero mcp: {text}");
}

fn decode_response(bytes: &[u8], id: &serde_json::Value) -> Result<serde_json::Value, ()> {
    fn final_response(value: &serde_json::Value, id: &serde_json::Value) -> bool {
        value.get("jsonrpc") == Some(&serde_json::json!("2.0"))
            && value.get("id") == Some(id)
            && value.get("method").is_none()
            && (value.get("result").is_some() ^ value.get("error").is_some())
    }
    if let Ok(value) = serde_json::from_slice(bytes) {
        return if final_response(&value, id) {
            Ok(value)
        } else {
            Err(())
        };
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ())?
        .replace("\r\n", "\n");
    let mut final_value = None;
    for event in text.split("\n\n") {
        let data = event
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|s| s.strip_prefix(' ').unwrap_or(s))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&data).map_err(|_| ())?;
        if final_response(&value, id) {
            if final_value.is_some() {
                return Err(());
            }
            final_value = Some(value);
        } else if final_value.is_some()
            || value.get("jsonrpc") != Some(&serde_json::json!("2.0"))
            || value.get("id").is_some()
            || !matches!(
                value.get("method").and_then(serde_json::Value::as_str),
                Some("notifications/progress" | "notifications/message")
            )
        {
            return Err(());
        }
    }
    final_value.ok_or(())
}
enum Frame {
    TooLarge,
    Json(Vec<u8>),
}
fn read_frame(reader: &mut impl BufRead) -> Result<Option<Frame>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    let mut consumed = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        consumed = true;
        let newline = chunk.iter().position(|b| *b == b'\n');
        let count = newline.map_or(chunk.len(), |n| n + 1);
        let content_count = newline.unwrap_or(chunk.len());
        if bytes.len() + content_count > MAX_LINE_BYTES {
            oversized = true;
            bytes.clear();
        }
        if !oversized {
            bytes.extend_from_slice(&chunk[..content_count]);
        }
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    if !consumed {
        return Ok(None);
    }
    if oversized {
        return Ok(Some(Frame::TooLarge));
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(Some(Frame::Json(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn notifications_cancel_inflight_without_responses_and_eof_drains() {
        let input=b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"slow\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":1}}\n{\"jsonrpc\":\"2.0\",\"method\":\"unknown/notification\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"fast\"}\n";
        let mut output = Vec::new();
        proxy_frames(
            &mut std::io::Cursor::new(input),
            &mut output,
            &|body, cancel| {
                if body["method"] == "slow" {
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(200);
                    while !cancel.is_cancelled() && std::time::Instant::now() < deadline {
                        std::thread::yield_now();
                    }
                }
                json!({"jsonrpc":"2.0","id":body["id"],"result":{}})
            },
        )
        .unwrap();
        let lines = String::from_utf8(output).unwrap();
        let lines = lines.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(lines[0]).unwrap()["id"],
            2
        );
    }
    #[test]
    fn scoped_sse_only_consumes_matching_final_response() {
        let stream = b": keepalive\r\n\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"resultType\":\"complete\"}}\r\n\r\n";
        assert_eq!(
            decode_response(stream, &json!(7)).unwrap()["result"]["resultType"],
            "complete"
        );
        assert!(decode_response(stream, &json!(8)).is_err());
        assert!(
            decode_response(
                b"data: {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"evil\"}\n\n",
                &json!(7)
            )
            .is_err()
        );
        assert!(
            decode_response(b"{\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{}}", &json!(7)).is_err()
        );
    }
    #[test]
    fn bounded_framing_drains_oversized_input() {
        let bytes = [vec![b'a'; MAX_LINE_BYTES + 100], b"\n{}\n".to_vec()].concat();
        let mut reader = std::io::Cursor::new(bytes);
        assert!(matches!(
            read_frame(&mut reader).unwrap(),
            Some(Frame::TooLarge)
        ));
        assert!(matches!(read_frame(&mut reader).unwrap(),Some(Frame::Json(v)) if v==b"{}"));
        assert!(read_frame(&mut reader).unwrap().is_none());
    }
}

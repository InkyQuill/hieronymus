//! The CLI-side daemon boundary (plan M5): headless `hiero` commands reach
//! daemon capabilities through this module instead of opening stores
//! directly. It reuses the existing loopback HTTP client seam
//! ([`crate::client::post_json`]) with the published discovery record and the
//! per-installation bearer token — the same credentials the stdio adapter
//! uses — so every CLI write lands in the daemon's audited path.
//!
//! R5 swap point: the runtime plan replaces this module's internals with
//! `lifecycle::DaemonClient`. Callers keep discovering and posting through
//! this one seam, so the swap changes no CLI behavior.
//!
//! "Daemon not running" is honest: a missing discovery record is a typed
//! error carrying the remediation (start the daemon), matching the stdio
//! adapter diagnostics. Nothing here stops or locks a daemon; spawning one
//! happens only on the caller's explicit opt-in ([`DaemonClient::connect_opt_in`]).

use std::net::{IpAddr, SocketAddr};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::secret::Secret;
use serde_json::Value;

use crate::client::{self, ClientError};
use crate::daemon::discovery::{
    CredentialError, DiscoveryError, DiscoveryRecord, read_discovery, read_token,
};
use crate::daemon::registry::PROTOCOL_REVISION;

/// How long one autostart wait may take when a command explicitly opts in.
const AUTOSTART_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum DaemonClientError {
    #[error(
        "no running local service discovered ({path} is missing); \
         start it with 'hiero daemon' or 'hiero service start'"
    )]
    NotRunning { path: std::path::PathBuf },
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error("daemon credential is unavailable: {0}")]
    Credential(#[from] CredentialError),
    #[error("refusing to connect to a non-loopback discovery address: {0}")]
    NotLoopback(String),
    #[error("daemon is unreachable: {0}")]
    Connect(#[from] ClientError),
    #[error("the daemon rejected the request (HTTP {status}): {body}")]
    Rejected { status: u16, body: String },
    #[error("cannot start the daemon: {0}")]
    Autostart(String),
}

/// A discovered, authenticated connection to the local daemon.
pub struct DaemonClient {
    address: SocketAddr,
    token: Secret<String>,
}

impl std::fmt::Debug for DaemonClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonClient")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl DaemonClient {
    /// Discover the local daemon from the data root. A missing discovery
    /// record is [`DaemonClientError::NotRunning`] (with remediation); a
    /// non-loopback record is refused exactly like the stdio adapter. With
    /// `start_daemon` the caller explicitly opts into spawning one (the same
    /// documented opt-in `hiero mcp --start-daemon` has); the default is to
    /// report the fact honestly instead.
    pub fn connect_opt_in(
        config: &HieronymusConfig,
        start_daemon: bool,
    ) -> Result<Self, DaemonClientError> {
        match Self::connect(config) {
            Ok(client) => Ok(client),
            Err(DaemonClientError::NotRunning { .. }) if start_daemon => {
                start_daemon_and_wait(config)?;
                Self::connect(config)
            }
            Err(error) => Err(error),
        }
    }

    /// Discover the local daemon from the data root. A missing discovery
    /// record is [`DaemonClientError::NotRunning`] (with remediation); a
    /// non-loopback record is refused exactly like the stdio adapter.
    pub fn connect(config: &HieronymusConfig) -> Result<Self, DaemonClientError> {
        let record = match read_discovery(config) {
            Ok(record) => record,
            // The one honest "not running" diagnostic; other discovery
            // failures (unreadable record) keep their typed text.
            Err(DiscoveryError::Missing { path }) => {
                return Err(DaemonClientError::NotRunning { path });
            }
            Err(error) => return Err(error.into()),
        };
        let token = read_token(config)?;
        Ok(Self {
            address: endpoint_address(&record)?,
            token,
        })
    }

    /// The discovered endpoint (test/diagnostics support).
    pub fn endpoint(&self) -> SocketAddr {
        self.address
    }

    /// POST one JSON payload to a native daemon route and return the parsed
    /// JSON object answer. Non-2xx responses are typed rejections carrying
    /// the daemon's error body (e.g. `activation_mismatch`), so callers can
    /// surface them without scraping text.
    pub fn post(&self, path: &str, payload: &Value) -> Result<Value, DaemonClientError> {
        let body = serde_json::to_vec(payload)
            .map_err(|_| ClientError::Protocol("request cannot serialize"))?;
        let headers = self.headers(None::<[(String, String); 0]>);
        let (status, response) = client::post_json(self.address, path, &headers, &body)?;
        let value: Value = serde_json::from_slice(&response)
            .map_err(|_| ClientError::Protocol("response body is not JSON"))?;
        if !(200..300).contains(&status) {
            return Err(DaemonClientError::Rejected {
                status,
                body: value.to_string(),
            });
        }
        Ok(value)
    }

    /// One stateless `tools/call` over the authenticated `/mcp` route: the
    /// headless adapter for every advertised tool (series/session, recall,
    /// dream, RAG, termbase, graph). Returns the MCP result envelope; JSON-RPC
    /// protocol errors are typed rejections. The mirrored headers follow the
    /// daemon's protocol rules exactly like the stdio adapter.
    pub fn call_tool(&self, name: &str, arguments: &Value) -> Result<Value, DaemonClientError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": PROTOCOL_REVISION,
                    "io.modelcontextprotocol/clientCapabilities": {},
                },
            },
        });
        let payload = serde_json::to_vec(&body)
            .map_err(|_| ClientError::Protocol("request cannot serialize"))?;
        let headers = self.headers(Some([
            (
                "MCP-Protocol-Version".to_string(),
                PROTOCOL_REVISION.to_string(),
            ),
            ("Mcp-Method".to_string(), "tools/call".to_string()),
            ("Mcp-Name".to_string(), name.to_string()),
        ]));
        let (status, response) = client::post_json(self.address, "/mcp", &headers, &payload)?;
        let value: Value = serde_json::from_slice(&response)
            .map_err(|_| ClientError::Protocol("response body is not JSON"))?;
        if !(200..300).contains(&status) {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown protocol error")
                .to_string();
            return Err(DaemonClientError::Rejected {
                status,
                body: message,
            });
        }
        Ok(value)
    }

    /// Headers every authenticated request carries: Host for the daemon's
    /// CSRF-equivalent check, the bearer credential, and JSON content types.
    /// `extra` carries the MCP mirrored headers on the `/mcp` path.
    fn headers(
        &self,
        extra: Option<impl IntoIterator<Item = (String, String)>>,
    ) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Host".to_string(), self.address.to_string()),
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.token.expose_secret()),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
        ];
        if let Some(extra) = extra {
            headers.extend(extra);
        }
        headers
    }
}

/// Same loopback discipline as the stdio adapter: the discovery record may
/// only name a loopback endpoint.
fn endpoint_address(record: &DiscoveryRecord) -> Result<SocketAddr, DaemonClientError> {
    let ip: IpAddr = record
        .host
        .parse()
        .map_err(|_| DaemonClientError::NotLoopback(record.host.clone()))?;
    if !ip.is_loopback() {
        return Err(DaemonClientError::NotLoopback(record.host.clone()));
    }
    Ok(SocketAddr::new(ip, record.port))
}

/// Explicit opt-in autostart (the `hiero mcp --start-daemon` behavior): spawn
/// the daemon in the background over this data root and wait for its
/// discovery record.
fn start_daemon_and_wait(config: &HieronymusConfig) -> Result<(), DaemonClientError> {
    use std::process::Command;
    use std::time::{Duration, Instant};

    let executable = std::env::current_exe().map_err(|error| {
        DaemonClientError::Autostart(format!("cannot locate the running binary: {error}"))
    })?;
    Command::new(&executable)
        .arg("daemon")
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--port")
        .arg("0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| DaemonClientError::Autostart(error.to_string()))?;

    let deadline = Instant::now() + AUTOSTART_TIMEOUT;
    loop {
        match std::fs::exists(config.daemon_discovery_path()) {
            Ok(true) => return Ok(()),
            Ok(false) if Instant::now() >= deadline => {
                return Err(DaemonClientError::Autostart(
                    "timed out waiting for the daemon discovery record".to_string(),
                ));
            }
            Ok(false) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(DaemonClientError::Autostart(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hieronymus::data_root::HieronymusConfig;

    #[test]
    fn missing_discovery_reports_not_running_with_remediation() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let error = DaemonClient::connect(&config).unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("no running local service discovered"),
            "{text}"
        );
        assert!(text.contains("hiero daemon"), "{text}");
    }

    #[test]
    fn non_loopback_discovery_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let record = DiscoveryRecord {
            discovery_version: crate::daemon::discovery::DISCOVERY_VERSION,
            protocol_version: crate::daemon::registry::PROTOCOL_REVISION.to_string(),
            host: "10.9.9.9".to_string(),
            port: 9768,
            pid: std::process::id(),
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        crate::daemon::discovery::write_discovery(&config, &record).unwrap();
        crate::daemon::discovery::write_token(
            &config,
            &crate::daemon::discovery::generate_bearer_token().unwrap(),
        )
        .unwrap();
        let error = DaemonClient::connect(&config).unwrap_err();
        assert!(error.to_string().contains("non-loopback"), "{error}");
    }
}

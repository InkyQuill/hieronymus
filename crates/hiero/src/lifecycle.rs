//! Authenticated lifecycle: `hiero start | stop | restart | status`
//! (ADR 0009 §Decision, security design spec §Lifecycle And Discovery).
//!
//! Two things live here:
//!
//! - [`probe`], the one discovery-health check the whole binary shares. ADR
//!   0009 is explicit: "stale discovery state is detected by authenticated
//!   health probing and process-instance comparison, never by PID existence
//!   alone." A record only counts as live when the endpoint answers the
//!   *authenticated* `GET /status` and the instance id it reports equals the
//!   one in the record. A foreign listener that inherited the port, or a
//!   record left behind by a crashed daemon, therefore reads as stale — a PID
//!   or a bare TCP connect proves neither.
//! - [`DaemonClient`] plus the four lifecycle commands. `stop` requests
//!   authenticated graceful shutdown through the discovered endpoint (the ADR
//!   0009 wording) and only falls back to the service manager when no daemon
//!   answers. `start` installs/starts the per-user service; `hiero daemon`
//!   stays the foreground role for supervisors and debugging.
//!
//! Nothing here ever prints, logs, or embeds the bearer token: errors carry
//! paths and reasons only.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::secret::Secret;

use crate::client::{ClientError, request_json_within};
use crate::daemon::discovery::{
    self, CredentialError, DiscoveryError, DiscoveryRecord, read_discovery, read_token,
};
use crate::daemon::registry::PROTOCOL_REVISION;
use crate::service::{self, ServiceOptions};

/// Per-probe deadline. Short on purpose: a probe runs in interactive commands
/// (`doctor`, the agent hook, `hiero status`), and an unresponsive listener
/// must not stall them.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long `stop` waits for the daemon to finish its graceful shutdown.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(30);

/// How long `connect(.., start_if_absent = true)` waits for a started service
/// to publish a live, authenticated endpoint.
const START_WAIT: Duration = Duration::from_secs(20);

const POLL: Duration = Duration::from_millis(100);

/// The verdict of one discovery-health probe.
#[derive(Debug)]
pub enum DiscoveryHealth {
    /// No discovery record exists.
    NoRecord { path: PathBuf },
    /// The record exists but is not valid JSON.
    Unreadable { path: PathBuf },
    /// The record advertises a non-loopback endpoint (ADR 0012).
    NotLoopback { record: DiscoveryRecord },
    /// A record exists but the credential next to it does not.
    NoCredential {
        record: DiscoveryRecord,
        detail: String,
    },
    /// Nothing answered, or what answered did not speak HTTP.
    Unreachable {
        record: DiscoveryRecord,
        detail: String,
    },
    /// Something answered but rejected our installation credential — it is
    /// not this installation's daemon.
    Unauthenticated {
        record: DiscoveryRecord,
        status: u16,
    },
    /// The endpoint authenticated but its status payload could not be
    /// understood (empty body, not an object, missing identity fields).
    ///
    /// Deliberately **not** a stale record: something answered *and* accepted
    /// this installation's credential, so it is almost certainly our daemon
    /// having a bad moment. Deleting its discovery record over one malformed
    /// response would be the wrong repair.
    Unparseable {
        record: DiscoveryRecord,
        detail: String,
    },
    /// The endpoint authenticated but reports a different process instance:
    /// the record belongs to a daemon that is gone.
    InstanceMismatch {
        record: DiscoveryRecord,
        live_instance: String,
    },
    /// The endpoint is this installation's daemon but speaks another MCP
    /// revision than the record claims.
    ProtocolMismatch {
        record: DiscoveryRecord,
        live_protocol: String,
    },
    /// Authenticated, and the process instance matches the record.
    Live {
        record: DiscoveryRecord,
        status: Box<Value>,
    },
}

impl DiscoveryHealth {
    pub fn is_live(&self) -> bool {
        matches!(self, DiscoveryHealth::Live { .. })
    }

    /// The record this verdict is about, when there was one.
    pub fn record(&self) -> Option<&DiscoveryRecord> {
        match self {
            DiscoveryHealth::NoRecord { .. } | DiscoveryHealth::Unreadable { .. } => None,
            DiscoveryHealth::NotLoopback { record }
            | DiscoveryHealth::NoCredential { record, .. }
            | DiscoveryHealth::Unreachable { record, .. }
            | DiscoveryHealth::Unauthenticated { record, .. }
            | DiscoveryHealth::Unparseable { record, .. }
            | DiscoveryHealth::InstanceMismatch { record, .. }
            | DiscoveryHealth::ProtocolMismatch { record, .. }
            | DiscoveryHealth::Live { record, .. } => Some(record),
        }
    }

    /// Whether a record exists that the authenticated comparison **proved**
    /// does not belong to a live daemon. Only such a record may be repaired.
    ///
    /// `Unparseable` is excluded on purpose: an endpoint that answered and
    /// accepted our credential is our daemon, whatever its body looked like.
    /// So are `NoCredential` and `NotLoopback`, which prove nothing about the
    /// endpoint at all.
    pub fn is_stale_record(&self) -> bool {
        matches!(
            self,
            DiscoveryHealth::Unreachable { .. }
                | DiscoveryHealth::Unauthenticated { .. }
                | DiscoveryHealth::InstanceMismatch { .. }
        )
    }

    /// A one-line explanation. Never contains a credential.
    pub fn detail(&self) -> String {
        match self {
            DiscoveryHealth::NoRecord { path } => {
                format!("no discovery record at {}", path.display())
            }
            DiscoveryHealth::Unreadable { path } => {
                format!("discovery record at {} is not valid JSON", path.display())
            }
            DiscoveryHealth::NotLoopback { record } => format!(
                "discovery record advertises non-loopback host {}",
                record.host
            ),
            DiscoveryHealth::NoCredential { detail, .. } => detail.clone(),
            DiscoveryHealth::Unreachable { record, detail } => format!(
                "nothing answered at {}:{} ({detail})",
                record.host, record.port
            ),
            DiscoveryHealth::Unauthenticated { record, status } => format!(
                "the listener at {}:{} rejected this installation's credential (HTTP {status}); \
                 it is not this daemon",
                record.host, record.port
            ),
            DiscoveryHealth::Unparseable { record, detail } => format!(
                "the daemon at {}:{} authenticated but its status payload was not understood \
                 ({detail})",
                record.host, record.port
            ),
            DiscoveryHealth::InstanceMismatch {
                record,
                live_instance,
            } => format!(
                "the daemon at {}:{} reports instance {live_instance}, \
                 but the discovery record claims {}",
                record.host, record.port, record.instance_id
            ),
            DiscoveryHealth::ProtocolMismatch {
                record,
                live_protocol,
            } => format!(
                "the daemon at {}:{} serves protocol {live_protocol}, \
                 but the discovery record claims {}",
                record.host, record.port, record.protocol_version
            ),
            DiscoveryHealth::Live { record, .. } => format!(
                "daemon is live at {}:{} (instance {}, pid {})",
                record.host, record.port, record.instance_id, record.pid
            ),
        }
    }

    fn into_client_error(self) -> ClientError {
        match self {
            DiscoveryHealth::NoCredential { detail, .. } => ClientError::Credential(detail),
            other => ClientError::NotDiscovered(other.detail()),
        }
    }
}

/// The shared authenticated discovery-health probe (ADR 0009). Read-only: it
/// never starts, writes, or deletes anything.
pub fn probe(config: &HieronymusConfig) -> DiscoveryHealth {
    let record = match read_discovery(config) {
        Ok(record) => record,
        Err(DiscoveryError::Missing { path }) => return DiscoveryHealth::NoRecord { path },
        Err(DiscoveryError::Unreadable { path }) => return DiscoveryHealth::Unreadable { path },
    };
    let address = match loopback_address(&record) {
        Some(address) => address,
        None => return DiscoveryHealth::NotLoopback { record },
    };
    let token = match read_token(config) {
        Ok(token) => token,
        Err(error) => {
            let detail = match error {
                CredentialError::Missing { path } => {
                    format!("no installation credential at {}", path.display())
                }
                CredentialError::Empty { path } => {
                    format!("the installation credential at {} is empty", path.display())
                }
            };
            return DiscoveryHealth::NoCredential { record, detail };
        }
    };
    match authenticated_status(address, &token) {
        Err(error) => DiscoveryHealth::Unreachable {
            record,
            detail: error.to_string(),
        },
        Ok((status, _)) if status != 200 => DiscoveryHealth::Unauthenticated { record, status },
        Ok((_, body)) => {
            // A 200 that is not a JSON object, or that carries no identity, is
            // a malformed answer from something that already authenticated as
            // us — not evidence that the record is stale. Keep it out of the
            // repairable verdicts so one bad response never deletes a live
            // daemon's discovery record.
            if !body.is_object() {
                return DiscoveryHealth::Unparseable {
                    record,
                    detail: "the status response was not a JSON object".to_string(),
                };
            }
            let Some(live_instance) = body.get("instance_id").and_then(Value::as_str) else {
                return DiscoveryHealth::Unparseable {
                    record,
                    detail: "the status response carried no instance_id".to_string(),
                };
            };
            let live_instance = live_instance.to_string();
            if live_instance != record.instance_id {
                return DiscoveryHealth::InstanceMismatch {
                    record,
                    live_instance,
                };
            }
            let live_protocol = body
                .get("protocol_revision")
                .and_then(Value::as_str)
                .unwrap_or("<absent>")
                .to_string();
            if live_protocol != record.protocol_version {
                return DiscoveryHealth::ProtocolMismatch {
                    record,
                    live_protocol,
                };
            }
            DiscoveryHealth::Live {
                record,
                status: Box::new(body),
            }
        }
    }
}

fn loopback_address(record: &DiscoveryRecord) -> Option<SocketAddr> {
    let ip: IpAddr = record.host.parse().ok()?;
    ip.is_loopback().then(|| SocketAddr::new(ip, record.port))
}

fn authenticated_status(
    address: SocketAddr,
    token: &Secret<String>,
) -> Result<(u16, Value), ClientError> {
    let (status, body) = request_json_within(
        "GET",
        address,
        "/status",
        &bearer_headers(address, token),
        b"",
        PROBE_TIMEOUT,
    )?;
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    Ok((status, parsed))
}

fn bearer_headers(address: SocketAddr, token: &Secret<String>) -> Vec<(String, String)> {
    vec![
        ("Host".to_string(), address.to_string()),
        (
            "Authorization".to_string(),
            format!("Bearer {}", token.expose_secret()),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ]
}

/// An authenticated connection to the discovered daemon.
pub struct DaemonClient {
    address: SocketAddr,
    bearer: Secret<String>,
    record: DiscoveryRecord,
}

impl std::fmt::Debug for DaemonClient {
    /// Deliberately credential-free.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonClient")
            .field("address", &self.address)
            .field("instance_id", &self.record.instance_id)
            .finish_non_exhaustive()
    }
}

impl DaemonClient {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn record(&self) -> &DiscoveryRecord {
        &self.record
    }

    /// POST a JSON body to an authenticated route and return the response.
    pub fn post(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.send("POST", path, body)
    }

    /// GET an authenticated route.
    pub fn get(&self, path: &str) -> Result<Value, ClientError> {
        self.send("GET", path, &Value::Null)
    }

    fn send(&self, method: &str, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.send_with_timeout(method, path, body, Duration::from_secs(10))
    }

    /// Explicit acquisition uses a bounded deadline for both asset downloads.
    pub fn post_with_timeout(
        &self,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        self.send_with_timeout("POST", path, body, timeout)
    }

    fn send_with_timeout(
        &self,
        method: &str,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let payload = match body {
            Value::Null => Vec::new(),
            other => serde_json::to_vec(other)
                .map_err(|_| ClientError::Protocol("request cannot serialize"))?,
        };
        let (status, raw) = request_json_within(
            method,
            self.address,
            path,
            &bearer_headers(self.address, &self.bearer),
            &payload,
            timeout,
        )?;
        let parsed = serde_json::from_slice::<Value>(&raw).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let detail = parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unexpected response")
                .to_string();
            return Err(ClientError::Status { status, detail });
        }
        Ok(parsed)
    }
}

/// Connect to the discovered daemon. With `start_if_absent` the per-user
/// service is installed/started first when no live daemon answers, and a
/// discovery record the authenticated comparison proved stale is repaired
/// (removed) before that — never on a bare PID or connect failure alone.
pub fn connect(
    config: &HieronymusConfig,
    start_if_absent: bool,
) -> Result<DaemonClient, ClientError> {
    let health = probe(config);
    if let DiscoveryHealth::Live { record, .. } = health {
        return open(config, record);
    }
    if !start_if_absent {
        return Err(health.into_client_error());
    }
    // The authenticated comparison failed, so this record does not describe a
    // live daemon: removing it is a repair, not a race. `remove_discovery`
    // re-reads and only deletes a record that still carries this instance id,
    // so a daemon that published in the meantime is never disturbed.
    if health.is_stale_record()
        && let Some(record) = health.record()
    {
        discovery::remove_discovery(config, &record.instance_id);
    }
    let options = default_service_options(config)
        .map_err(|error| ClientError::NotDiscovered(error.to_string()))?;
    ensure_service_started(&options).map_err(ClientError::NotDiscovered)?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        if let DiscoveryHealth::Live { record, .. } = probe(config) {
            return open(config, record);
        }
        if Instant::now() >= deadline {
            return Err(ClientError::NotDiscovered(
                "the local service was started but never published a live endpoint".to_string(),
            ));
        }
        std::thread::sleep(POLL);
    }
}

fn open(config: &HieronymusConfig, record: DiscoveryRecord) -> Result<DaemonClient, ClientError> {
    let address = loopback_address(&record).ok_or_else(|| {
        ClientError::NotDiscovered(format!("non-loopback discovery host {}", record.host))
    })?;
    let bearer = read_token(config).map_err(|error| ClientError::Credential(error.to_string()))?;
    Ok(DaemonClient {
        address,
        bearer,
        record,
    })
}

/// The per-user service definition for this data root and this binary.
pub fn default_service_options(config: &HieronymusConfig) -> std::io::Result<ServiceOptions> {
    Ok(ServiceOptions {
        data_root: config.data_root().to_path_buf(),
        unit_dir: service::default_unit_dir(),
        binary: std::env::current_exe()?,
        use_manager: true,
    })
}

/// Install the unit when it is missing, then start it. A failure keeps the
/// steps that already succeeded in its message: an operator must be able to
/// see that the unit *was* written even though the manager refused to start
/// it.
fn ensure_service_started(options: &ServiceOptions) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let fail = |lines: &[String], message: String| {
        let mut text = String::new();
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str(&message);
        text
    };
    if !options.unit_path().exists() {
        match service::install(options) {
            Ok(installed) => lines.extend(installed),
            Err(error) => return Err(fail(&lines, error.to_string())),
        }
    }
    match service::start(options) {
        Ok(started) => lines.extend(started),
        Err(error) => return Err(fail(&lines, error.to_string())),
    }
    Ok(lines)
}

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("{0}")]
    Service(String),
    #[error("{0}")]
    Client(#[from] ClientError),
    #[error("the daemon accepted the shutdown request but did not stop within {0:?}")]
    ShutdownTimeout(Duration),
}

/// `hiero start`: install (idempotently) and start the per-user service.
/// `hiero daemon` remains the foreground role.
pub fn start(options: &ServiceOptions) -> Result<Vec<String>, LifecycleError> {
    ensure_service_started(options).map_err(LifecycleError::Service)
}

/// `hiero stop`: authenticated graceful shutdown through the discovered
/// endpoint (ADR 0009). A daemon started in the foreground and a daemon under
/// the service manager are both reached this way; the service manager is only
/// asked when no daemon answers the authenticated probe.
pub fn stop(
    config: &HieronymusConfig,
    options: &ServiceOptions,
) -> Result<Vec<String>, LifecycleError> {
    let health = probe(config);
    let Some(record) = health.record().cloned().filter(|_| health.is_live()) else {
        let mut lines = vec![format!(
            "no running local daemon answered the authenticated probe: {}",
            health.detail()
        )];
        if !options.unit_path().exists() {
            lines.push(format!(
                "no service unit at {}; nothing to stop",
                options.unit_path().display()
            ));
            return Ok(lines);
        }
        lines.push("asking the service manager to stop the unit instead".to_string());
        lines.extend(
            service::stop(options).map_err(|error| LifecycleError::Service(error.to_string()))?,
        );
        return Ok(lines);
    };
    let client = open(config, record.clone())?;
    // `POST /shutdown` sets the stop flag before it answers, so the daemon can
    // legitimately tear the connection down before (or while) its 200 reaches
    // us. A dropped response here means the request landed — the shutdown is
    // already under way — so it must not be reported as a failure. Whether it
    // really stopped is settled by `wait_until_stopped` below, which watches
    // the discovery record rather than trusting either outcome.
    let accepted = match client.post("/shutdown", &json!({})) {
        Ok(response) => {
            if response
                .get("stopping")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "stopping"
            } else {
                "acknowledged"
            }
        }
        Err(error) if disconnected_mid_shutdown(&error) => "accepted (connection closed)",
        Err(error) => return Err(error.into()),
    };
    let mut lines = vec![format!(
        "shutdown requested: {}:{} (instance {}, pid {}) — {accepted}",
        record.host, record.port, record.instance_id, record.pid,
    )];
    wait_until_stopped(config, &record)?;
    lines.push("daemon stopped and released its discovery record".to_string());
    Ok(lines)
}

/// Whether an error from a `POST /shutdown` that was already written is the
/// daemon closing the connection as it stops, rather than a real failure.
///
/// Only applies after the request reached the socket: a *connect* failure is
/// `ClientError::Connect` and is still an error, because nothing was sent.
fn disconnected_mid_shutdown(error: &ClientError) -> bool {
    match error {
        // The response never arrived, or arrived truncated.
        ClientError::Protocol(
            "response headers are incomplete" | "response body is incomplete" | "empty response",
        ) => true,
        ClientError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
        ),
        _ => false,
    }
}

/// The daemon removes its own record and releases data-root ownership as the
/// last two shutdown steps, so a record that is gone (or now belongs to a
/// different instance) is the completion signal.
fn wait_until_stopped(
    config: &HieronymusConfig,
    record: &DiscoveryRecord,
) -> Result<(), LifecycleError> {
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    loop {
        match read_discovery(config) {
            Err(_) => return Ok(()),
            Ok(current) if current.instance_id != record.instance_id => return Ok(()),
            Ok(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(LifecycleError::ShutdownTimeout(SHUTDOWN_WAIT));
        }
        std::thread::sleep(POLL);
    }
}

/// `hiero restart`: the authenticated stop followed by the service start.
///
/// A failed stop aborts deliberately — starting a second daemon over one that
/// is still running would only be refused by the ownership guard — so the
/// error names the manual recovery instead.
pub fn restart(
    config: &HieronymusConfig,
    options: &ServiceOptions,
) -> Result<Vec<String>, LifecycleError> {
    let mut lines = stop(config, options).map_err(|error| {
        LifecycleError::Service(format!(
            "{error}\nrestart stopped here so a second daemon is never started over a running \
             one; recover with `hiero service stop` and then `hiero start`"
        ))
    })?;
    lines.extend(start(options)?);
    Ok(lines)
}

/// `hiero status`: the authenticated status of the discovered daemon plus the
/// probe verdict that produced it.
#[derive(Debug)]
pub struct LifecycleStatus {
    pub running: bool,
    pub detail: String,
    pub record: Option<DiscoveryRecord>,
    pub status: Option<Value>,
}

impl LifecycleStatus {
    pub fn to_json(&self) -> Value {
        json!({
            "running": self.running,
            "detail": self.detail,
            "discovery": self.record.as_ref().map(|record| json!({
                "host": record.host,
                "port": record.port,
                "pid": record.pid,
                "instance_id": record.instance_id,
                "protocol_version": record.protocol_version,
                "started_at": record.started_at,
            })),
            "status": self.status,
        })
    }

    pub fn render_human(&self) -> String {
        let mut text = if self.running {
            format!("daemon: running ({})\n", self.detail)
        } else {
            format!("daemon: not running ({})\n", self.detail)
        };
        if let Some(status) = &self.status {
            let field = |name: &str| {
                status
                    .get(name)
                    .map(|value| match value.as_str() {
                        Some(text) => text.to_string(),
                        None => value.to_string(),
                    })
                    .unwrap_or_else(|| "-".to_string())
            };
            text.push_str(&format!("  version: {}\n", field("version")));
            text.push_str(&format!(
                "  protocol revision: {}\n",
                field("protocol_revision")
            ));
            text.push_str(&format!("  instance: {}\n", field("instance_id")));
            text.push_str(&format!("  started at: {}\n", field("started_at")));
            text.push_str(&format!("  data root: {}\n", field("data_root")));
        } else {
            text.push_str(&format!(
                "  this binary serves protocol revision {PROTOCOL_REVISION}\n"
            ));
        }
        text
    }
}

pub fn status(config: &HieronymusConfig) -> LifecycleStatus {
    let health = probe(config);
    let detail = health.detail();
    match health {
        DiscoveryHealth::Live { record, status } => LifecycleStatus {
            running: true,
            detail,
            record: Some(record),
            status: Some(*status),
        },
        other => LifecycleStatus {
            running: false,
            detail,
            record: other.record().cloned(),
            status: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_record(port: u16, instance_id: &str) -> DiscoveryRecord {
        DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: PROTOCOL_REVISION.to_string(),
            host: "127.0.0.1".to_string(),
            port,
            pid: std::process::id(),
            instance_id: instance_id.to_string(),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn a_missing_record_is_not_live() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let health = probe(&config);
        assert!(!health.is_live());
        assert!(!health.is_stale_record());
        assert!(
            health.detail().contains("no discovery record"),
            "{health:?}"
        );
    }

    #[test]
    fn a_record_without_a_credential_is_not_live_and_is_not_repairable() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        discovery::write_discovery(&config, &seeded_record(1, &"ab".repeat(16))).unwrap();
        let health = probe(&config);
        assert!(!health.is_live());
        // Without a credential nothing was *proved* stale, so no repair.
        assert!(!health.is_stale_record(), "{health:?}");
        assert!(health.detail().contains("credential"), "{health:?}");
    }

    #[test]
    fn a_refused_port_is_a_proved_stale_record() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
        discovery::write_discovery(&config, &seeded_record(1, &"ab".repeat(16))).unwrap();
        let health = probe(&config);
        assert!(!health.is_live());
        assert!(health.is_stale_record(), "{health:?}");
    }

    #[test]
    fn the_debug_form_of_a_client_never_carries_the_credential() {
        let record = seeded_record(9999, &"cd".repeat(16));
        let client = DaemonClient {
            address: "127.0.0.1:9999".parse().unwrap(),
            bearer: Secret::new("super-secret-token".to_string()),
            record,
        };
        let text = format!("{client:?}");
        assert!(!text.contains("super-secret-token"), "{text}");
        assert!(text.contains("cdcd"), "{text}");
    }
}

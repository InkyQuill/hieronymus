//! `hiero daemon`: the foreground local daemon skeleton (ADR 0009). Startup
//! verifies the database (fail closed on unsupported states), binds loopback
//! only (occupied port is an error; no scan), generates the per-installation
//! bearer token, publishes the non-secret discovery record atomically, and
//! serves until SIGINT or an authenticated `POST /shutdown`.

pub mod discovery;
pub mod http;
pub mod protocol;
pub mod registry;
mod server;

use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use hieronymus::data_root::{load_config, HieronymusConfig};
use hieronymus::db::{classify_database, open_migrated};
use hieronymus::secret::Secret;

pub use discovery::DiscoveryRecord;
pub use registry::{McpRegistry, PROTOCOL_REVISION};

/// The default loopback port (ADR 0012); overrides land in discovery so
/// plugins never hard-code this value.
pub const DEFAULT_PORT: u16 = 9768;
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// Options for starting the daemon. `port` 0 binds an ephemeral port (used by
/// tests; the discovered record carries the actual port).
pub struct DaemonOptions {
    pub data_root: Option<PathBuf>,
    pub port: u16,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            data_root: None,
            port: DEFAULT_PORT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(
        "daemon cannot start: database state '{state}' is not supported; \
         run the documented database upgrade path first"
    )]
    UnsupportedDatabase { state: &'static str },
    #[error("daemon cannot open the database: {0}")]
    Database(#[from] hieronymus::db::OpenMigratedError),
    #[error("daemon cannot bind loopback endpoint {address}: {source}")]
    Bind {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("daemon cannot generate random material: {0}")]
    Random(#[from] getrandom::Error),
    #[error("daemon cannot write the bearer token: {source}")]
    TokenWrite { source: std::io::Error },
    #[error("daemon cannot write the discovery record: {source}")]
    DiscoveryWrite { source: std::io::Error },
    #[error("daemon cannot remove the discovery record: {source}")]
    DiscoveryRemove { source: std::io::Error },
    #[error("daemon cannot install the ctrl-c handler: {0}")]
    Signal(String),
    #[error("daemon worker failed: {0}")]
    Worker(String),
}

#[derive(Debug)]
pub(crate) struct DaemonRuntime {
    pub config: HieronymusConfig,
    pub registry: McpRegistry,
    pub bearer: Secret<String>,
    pub bound_address: SocketAddr,
    pub stop: AtomicBool,
    pub instance_id: String,
    /// The daemon holds the database open for its whole lifetime: it owns the
    /// data root (ADR 0009). Nobody reads it on the hot path yet.
    #[allow(dead_code)]
    database: Mutex<rusqlite::Connection>,
}

/// A running daemon. `start` performs startup in order; the accept loop runs
/// on its own thread until [`Daemon::request_shutdown`] (also triggered by
/// `POST /shutdown`).
#[derive(Debug)]
pub struct Daemon {
    runtime: Arc<DaemonRuntime>,
    accept_thread: Option<JoinHandle<()>>,
}

impl Daemon {
    /// Start the daemon: database check, loopback bind, token, discovery.
    pub fn start(options: &DaemonOptions) -> Result<Daemon, DaemonError> {
        let config = load_config(options.data_root.as_deref());

        // Fail closed on unsupported database states before binding anything
        // (ADR 0009: startup never publishes readiness for rejected state).
        let database_path = config.database_path();
        let state = classify_database(&database_path);
        let supported = match &state {
            hieronymus::db::DatabaseState::Empty => true,
            hieronymus::db::DatabaseState::RustSchema { version } => {
                *version == hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION
            }
            _ => false,
        };
        if !supported {
            return Err(DaemonError::UnsupportedDatabase {
                state: state.as_str(),
            });
        }
        let connection = open_migrated(&database_path)?;

        let registry = McpRegistry::embedded();

        let address = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), options.port);
        let listener = TcpListener::bind(address).map_err(|source| DaemonError::Bind {
            address,
            source,
        })?;
        let bound_address = listener.local_addr().map_err(|source| DaemonError::Bind {
            address,
            source,
        })?;

        let bearer = discovery::generate_bearer_token()?;
        discovery::write_token(&config, &bearer).map_err(|source| DaemonError::TokenWrite {
            source,
        })?;

        let instance_id = discovery::generate_instance_id()?;
        let record = DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: PROTOCOL_REVISION.to_string(),
            host: bound_address.ip().to_string(),
            port: bound_address.port(),
            pid: std::process::id(),
            instance_id: instance_id.clone(),
            started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        };
        discovery::write_discovery(&config, &record)
            .map_err(|source| DaemonError::DiscoveryWrite { source })?;

        let runtime = Arc::new(DaemonRuntime {
            config,
            registry,
            bearer,
            bound_address,
            stop: AtomicBool::new(false),
            instance_id,
            database: Mutex::new(connection),
        });
        let accept_thread = spawn_accept_thread(listener, Arc::clone(&runtime));
        Ok(Daemon {
            runtime,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.runtime.bound_address
    }

    pub fn data_root(&self) -> &Path {
        self.runtime.config.data_root()
    }

    /// Test and adapter support: read the live credential.
    pub fn bearer(&self) -> &Secret<String> {
        &self.runtime.bearer
    }

    pub fn discovery_record(&self) -> DiscoveryRecord {
        discovery::read_discovery(&self.runtime.config)
            .expect("the daemon wrote its own discovery record")
    }

    /// Ask the daemon to stop (the `POST /shutdown` path sets the same flag).
    pub fn request_shutdown(&self) {
        self.runtime.stop.store(true, Ordering::Release);
    }

    /// Block until shutdown is requested, then clean up: remove the matching
    /// discovery record and join the accept loop.
    pub fn wait_for_shutdown(mut self) -> Result<(), DaemonError> {
        while !self.runtime.stop.load(Ordering::Acquire) {
            std::thread::sleep(ACCEPT_POLL);
        }
        self.finish_shutdown()
    }

    /// Request shutdown and wait for it.
    pub fn shutdown(self) -> Result<(), DaemonError> {
        self.request_shutdown();
        self.wait_for_shutdown()
    }

    fn finish_shutdown(&mut self) -> Result<(), DaemonError> {
        // Stale discovery from a crashed run is tolerated for now (documented
        // in the port report); a graceful stop removes its own record.
        discovery::remove_discovery(&self.runtime.config, &self.runtime.instance_id);
        if let Some(handle) = self.accept_thread.take() {
            handle
                .join()
                .map_err(|_| DaemonError::Worker("accept loop panicked".to_string()))?;
        }
        Ok(())
    }
}

fn spawn_accept_thread(listener: TcpListener, runtime: Arc<DaemonRuntime>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        loop {
            if runtime.stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let runtime = Arc::clone(&runtime);
                    std::thread::spawn(move || serve_connection(stream, &runtime));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(_) => break,
            }
        }
    })
}

fn serve_connection(mut stream: std::net::TcpStream, runtime: &DaemonRuntime) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(http_io_timeout()));
    let _ = stream.set_write_timeout(Some(http_io_timeout()));
    let request = match http::read_request(&mut stream) {
        Ok(request) => request,
        Err(_) => return,
    };
    let response = server::handle(&request, runtime);
    let _ = http::write_response(&mut stream, &response);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn http_io_timeout() -> Duration {
    Duration::from_secs(5)
}

/// Run the daemon in the foreground: like [`Daemon::start`], plus a ctrl-c
/// handler that triggers the same graceful shutdown.
pub fn run_foreground(options: DaemonOptions) -> Result<(), DaemonError> {
    let daemon = Daemon::start(&options)?;
    let handler_runtime = Arc::clone(&daemon.runtime);
    ctrlc::set_handler(move || {
        handler_runtime.stop.store(true, Ordering::Release);
    })
    .map_err(|error| DaemonError::Signal(error.to_string()))?;
    let record = daemon.discovery_record();
    println!(
        "hiero daemon listening on {} (pid {}); discovery: {}; ctrl-c to stop",
        daemon.local_addr(),
        record.pid,
        daemon
            .runtime
            .config
            .daemon_discovery_path()
            .display()
    );
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    daemon.wait_for_shutdown()
}

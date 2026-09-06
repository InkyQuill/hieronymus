//! `hiero daemon`: the foreground local daemon skeleton (ADR 0009). Startup
//! verifies the database (fail closed on unsupported states), binds loopback
//! only (occupied port is an error; no scan), generates the per-installation
//! bearer token, publishes the non-secret discovery record atomically, and
//! serves until SIGINT or an authenticated `POST /shutdown`.

pub mod assets;
pub mod discovery;
pub mod dream_worker;
mod events;
pub mod http;
pub mod protocol;
pub mod registry;
mod rest;
mod server;
mod sessions;
pub mod workers;
mod ws;

use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use hieronymus::data_root::{HieronymusConfig, load_config};
use hieronymus::db::open_migrated;
use hieronymus::ownership::RootOwnership;
use hieronymus::secret::Secret;
use hieronymus::state_classifier::{StartupState, classify};

use crate::application::Application;
use crate::daemon::dream_worker::DreamController;

pub use assets::Assets;
pub use discovery::DiscoveryRecord;
pub use registry::{McpRegistry, PROTOCOL_REVISION};

use rest::providers::{DaemonProviderClient, ProviderClientSeam};
use sessions::SessionStore;
use workers::{IdleConnections, WorkerGroup};

/// The authenticated actor reported to the application for MCP calls: the
/// per-installation bearer token holder (no per-actor identities exist yet).
pub(crate) const BEARER_ACTOR: &str = "local-bearer";

/// The daemon crate version, served by `GET /status`.
pub(crate) fn daemon_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The display form the admin console header serves (Python
/// `display_version`: pre-1.0 versions carry the alpha mark).
pub(crate) fn daemon_display_version() -> String {
    let version = daemon_version();
    if version.starts_with("0.") {
        format!("v{version}\u{03B1}")
    } else {
        format!("v{version}")
    }
}

/// The default loopback port (ADR 0012); overrides land in discovery so
/// plugins never hard-code this value.
pub const DEFAULT_PORT: u16 = 9768;
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// Options for starting the daemon. `port` 0 binds an ephemeral port (used by
/// tests; the discovered record carries the actual port).
pub struct DaemonOptions {
    pub data_root: Option<PathBuf>,
    pub port: u16,
    /// The embedded console asset set backing the static SPA routes.
    pub assets: Assets,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            data_root: None,
            port: DEFAULT_PORT,
            assets: Assets::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon cannot start: {message} (run `{remediation}`)")]
    InvalidStartupState {
        code: &'static str,
        message: String,
        remediation: String,
    },
    #[error(
        "another process already owns this data root: {message} (stop the running daemon or maintenance command first)"
    )]
    OwnershipHeld { message: String },
    #[error("daemon cannot acquire data-root ownership: {source}")]
    Ownership { source: std::io::Error },
    #[error("daemon cannot open the database: {0}")]
    Database(#[from] hieronymus::db::OpenMigratedError),
    #[error("daemon cannot initialize the application: {0}")]
    Application(#[from] crate::application::AppError),
    #[error("daemon cannot bind loopback endpoint {address}: {source}")]
    Bind {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("daemon cannot generate random material: {0}")]
    Random(#[from] getrandom::Error),
    #[error("daemon cannot write the bearer token: {source}")]
    TokenWrite { source: std::io::Error },
    #[error("daemon cannot use the installation credential: {0}")]
    Credential(#[from] discovery::EnsureTokenError),
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
    /// The one cancellation edge every loop in the daemon observes; shared
    /// with [`DaemonRuntime::workers`].
    pub stop: Arc<AtomicBool>,
    /// Every mutating worker thread the daemon owns: the per-connection
    /// serve threads and the dream controller's worker (S2's semantic
    /// controller registers here too). Joined before discovery removal and
    /// ownership release.
    pub workers: WorkerGroup,
    /// Accepted-but-not-yet-dispatched sockets, so a shutdown can wake reads
    /// that are blocked on a peer that never sent anything.
    pub idle_connections: IdleConnections,
    /// One-time launch grants and browser sessions (in-memory, daemon
    /// lifetime).
    pub sessions: SessionStore,
    /// The admin event hub: websocket event stream with bounded retention
    /// for resume (see `events`).
    pub events: Arc<events::AdminEventHub>,
    /// The embedded console asset set backing the static SPA routes.
    pub assets: assets::Assets,
    /// The provider-client seam for the providers `check`/`models` routes
    /// (the synthetic fixture world answers with frozen oracle semantics;
    /// configured profiles are probed through the real client).
    pub provider_client: Box<dyn ProviderClientSeam>,
    /// The record this daemon published at startup; kept in memory so
    /// diagnostics never depend on the file still existing.
    pub record: DiscoveryRecord,
    /// The application dispatcher backing the ported MCP tools (plan M1).
    pub application: Application,
    /// The supervised dream controller (task D5): every production dream
    /// run — scheduled, admin manual, and MCP — coalesces into its worker.
    /// Its event hub is the runtime's admin event hub.
    pub dream: DreamController,
    /// The daemon holds the database open for its whole lifetime: it owns the
    /// data root (ADR 0009). Every worker that touches it is supervised by
    /// [`DaemonRuntime::workers`], so the handle is quiescent by the time
    /// shutdown rolls back and releases ownership.
    database: Mutex<rusqlite::Connection>,
}

/// A running daemon. `start` performs startup in order; the accept loop runs
/// on its own thread until [`Daemon::request_shutdown`] (also triggered by
/// `POST /shutdown`).
#[derive(Debug)]
pub struct Daemon {
    runtime: Arc<DaemonRuntime>,
    accept_thread: Option<JoinHandle<()>>,
    /// Exclusive data-root ownership (ADR 0009), held for the daemon's whole
    /// lifetime and released on a graceful stop AFTER the discovery record is
    /// removed. Kept outside `DaemonRuntime` so the ctrl-c handler's `Arc`
    /// clone cannot pin it past shutdown.
    ownership: Option<RootOwnership>,
}

impl Daemon {
    /// Start the daemon: bounded startup-state classification, database open,
    /// loopback bind, token, discovery.
    pub fn start(options: &DaemonOptions) -> Result<Daemon, DaemonError> {
        let config = load_config(options.data_root.as_deref());

        // The shared bounded startup-state classifier is the first gate
        // (ADR 0009): it reads schema/config version markers and the
        // cutover-journal state, runs no converters or scans, and never
        // mutates the data root. Only `Fresh` and `Current` proceed;
        // everything else refuses before binding, tokens, or discovery, so a
        // rejected state never publishes readiness. R1 deliberately keeps
        // this ahead of ownership acquisition so a rejected root is never
        // touched.
        let to_startup_error =
            |error: hieronymus::state_classifier::ClassifyError| DaemonError::InvalidStartupState {
                code: error.code(),
                message: error.message().to_string(),
                remediation: error.remediation().to_string(),
            };
        match classify(&config) {
            Ok(StartupState::Fresh | StartupState::Current) => {}
            Err(error) => return Err(to_startup_error(error)),
        }

        // Take exclusive ownership of the data root before opening the
        // database, binding, or publishing anything (ADR 0009). One
        // nonblocking OS `try_lock`: a root another daemon (or an offline
        // maintenance run) already owns refuses here (`WouldBlock`), so a
        // second daemon never overwrites the first daemon's token or
        // discovery record. Any other io error (permissions, ENOSPC, a
        // read-only mount) is a plain startup failure, not a contention.
        // The guard lives on `Daemon` for the whole lifetime and releases
        // on drop.
        let ownership =
            RootOwnership::acquire(&config, "daemon").map_err(|error| match error.kind() {
                std::io::ErrorKind::WouldBlock => DaemonError::OwnershipHeld {
                    message: error.to_string(),
                },
                _ => DaemonError::Ownership { source: error },
            })?;

        // Re-check the cutover-journal gate now that ownership is held. This
        // is the authoritative re-check: it closes the window between the
        // `classify` above and this point against any (future) unlocked
        // journal writer, and a mid-cutover root discovered here refuses
        // before binding or publishing anything.
        if let Err(error) = classify(&config) {
            return Err(to_startup_error(error));
        }

        // `classify` ran a moment ago against files that could in principle
        // change before this line; `open_migrated` is the authoritative
        // re-classification (it re-reads the markers and refuses any
        // non-startable state). The daemon is not serving yet, so a race here
        // fails safe as `DaemonError::Database` without publishing readiness.
        let database_path = config.database_path();
        let connection = open_migrated(&database_path)?;

        let registry = McpRegistry::embedded();
        let application = Application::open(&config)?;

        let address = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), options.port);
        let listener =
            TcpListener::bind(address).map_err(|source| DaemonError::Bind { address, source })?;
        let bound_address = listener
            .local_addr()
            .map_err(|source| DaemonError::Bind { address, source })?;

        // One static per-installation credential (ADR 0012 as amended;
        // astra 11). Ownership is held, so this read-or-mint is exclusive: a
        // plain restart reuses the stored token and never rotates it.
        let bearer = discovery::ensure_installation_token(&config)?;

        let instance_id = discovery::generate_instance_id()?;
        let record = DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: PROTOCOL_REVISION.to_string(),
            host: bound_address.ip().to_string(),
            port: bound_address.port(),
            pid: std::process::id(),
            instance_id,
            started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        };
        discovery::write_discovery(&config, &record)
            .map_err(|source| DaemonError::DiscoveryWrite { source })?;

        let stop = Arc::new(AtomicBool::new(false));
        let mut workers = WorkerGroup::new(Arc::clone(&stop));
        // The dream controller registers its worker with the group, so a
        // graceful stop joins it before discovery is removed and ownership
        // released (task D5; the SIGTERM path needs no extra wiring).
        let dream = dream_worker::DreamController::start(config.clone(), &mut workers)
            .map_err(DaemonError::Worker)?;
        // The controller runs every production dream run; the MCP
        // `hieronymus_dream` dispatch serves through this handle. On a bare
        // `Application::open` (no daemon) it stays absent and the dispatch
        // fails closed.
        application.install_dream_controller(dream.clone());
        let events = dream.events();
        let runtime = Arc::new(DaemonRuntime {
            config,
            registry,
            bearer,
            bound_address,
            workers,
            idle_connections: IdleConnections::default(),
            stop,
            sessions: SessionStore::default(),
            events,
            assets: options.assets.clone(),
            provider_client: Box::new(DaemonProviderClient::with_default_transport()),
            record,
            application,
            dream,
            database: Mutex::new(connection),
        });
        let accept_thread = spawn_accept_thread(listener, Arc::clone(&runtime));
        Ok(Daemon {
            runtime,
            accept_thread: Some(accept_thread),
            ownership: Some(ownership),
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

    /// Diagnostics support: the number of live admin websocket subscribers.
    pub fn admin_subscriber_count(&self) -> usize {
        self.runtime.events.subscriber_count()
    }

    /// The dream controller's status: the active run and the last finished
    /// one (the admin surface's honest controller view; the durable
    /// run/phase registry remains the detailed record).
    pub fn dream_status(&self) -> dream_worker::DreamStatus {
        self.runtime.dream.status()
    }

    /// The discovery record this daemon published at startup.
    pub fn discovery_record(&self) -> DiscoveryRecord {
        self.runtime.record.clone()
    }

    /// Ask the daemon to stop (the `POST /shutdown` path sets the same flag).
    pub fn request_shutdown(&self) {
        self.runtime.stop.store(true, Ordering::Release);
    }

    /// Block until shutdown is requested, then run the graceful stop.
    pub fn wait_for_shutdown(mut self) -> Result<(), DaemonError> {
        while !self.runtime.stop.load(Ordering::Acquire) {
            std::thread::sleep(ACCEPT_POLL);
        }
        self.finish_shutdown()
    }

    /// Request shutdown and wait for it.
    pub fn shutdown(mut self) -> Result<(), DaemonError> {
        self.request_shutdown();
        self.stop()
    }

    /// Graceful stop, by reference. Idempotent: a second call is a clean
    /// no-op, which is what makes both `shutdown()` and the [`Drop`] guard
    /// safe to run in sequence.
    pub fn stop(&mut self) -> Result<(), DaemonError> {
        self.request_shutdown();
        self.finish_shutdown()
    }

    /// Clean up in the order the security design spec fixes: stop admission →
    /// close MCP/WebSocket sessions → signal workers → wait for bounded work →
    /// roll back unfinished transactions → close the semantic index → remove
    /// matching discovery state → release ownership.
    ///
    /// Ownership is released **last, and never while a writer is live**. If a
    /// worker did not join cleanly, the guard and the discovery record are
    /// both kept and this returns `Err`: letting another owner in while a
    /// writer may still hold the database is worse than a noisy failure
    /// (ADR 0009, astra 9).
    ///
    /// A later `stop()` (or the `Drop` guard) retries, and that retry can
    /// succeed. That is not a loosening of the rule: the only way
    /// `stop_and_join` fails is a worker that panicked, and a panicked thread
    /// has terminated. The retry drains a handle list that is provably empty —
    /// every thread admitted to the group has run to completion, panic
    /// included — so nothing is executing when the guard finally drops.
    ///
    /// Idempotent: safe to call from `shutdown`, `wait_for_shutdown`, and the
    /// `Drop` guard.
    fn finish_shutdown(&mut self) -> Result<(), DaemonError> {
        if self.accept_thread.is_none() && self.ownership.is_none() {
            return Ok(());
        }
        // 1. Stop admission. `WorkerGroup::spawn` refuses from here on, and
        //    the accept loop, the websocket sessions, and every worker see the
        //    same edge (one shared flag).
        self.runtime.stop.store(true, Ordering::Release);

        // 2. Close sessions that are waiting on a peer: sockets accepted but
        //    still blocked reading their request head. Live websocket
        //    sessions poll the stop flag themselves and are not torn down
        //    mid-frame.
        self.runtime.idle_connections.wake_all();

        // 3–4. Signal workers and wait for their bounded work. The accept
        //    loop is joined first so nothing new can be admitted, then the
        //    worker group drains.
        let accept_result = match self.accept_thread.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| DaemonError::Worker("accept loop panicked".to_string())),
            None => Ok(()),
        };
        let worker_result = self
            .runtime
            .workers
            .stop_and_join()
            .map_err(DaemonError::Worker);

        // A join failure deliberately returns here: the discovery record and
        // the ownership guard are both kept. The guard is released when this
        // process exits, never while a writer may still be live.
        accept_result.and(worker_result)?;

        // 5. Roll back anything a worker left open. Reachable only now that
        //    every worker joined, so this lock can no longer be contended.
        self.rollback_open_transaction();

        // 6. Close the semantic index. There is no daemon-owned index handle
        //    yet (S2 registers one with the worker group); when there is, it
        //    closes here, after the writers joined and before discovery is
        //    removed.

        // 7. Remove only our own discovery record (a newer daemon's record is
        //    never deleted), then 8. release ownership last.
        discovery::remove_discovery(&self.runtime.config, &self.runtime.record.instance_id);
        drop(self.ownership.take());
        Ok(())
    }

    /// Best-effort rollback of a transaction a worker left open. A poisoned
    /// lock is itself evidence of a panicked writer, and the connection is
    /// dropped with the runtime in that case.
    fn rollback_open_transaction(&self) {
        if let Ok(connection) = self.runtime.database.lock()
            && !connection.is_autocommit()
        {
            let _ = connection.execute_batch("rollback");
        }
    }
}

impl Drop for Daemon {
    /// A `Daemon` dropped without `shutdown()` / `wait_for_shutdown()` (for
    /// instance a panic between `start` and shutdown) still removes its
    /// discovery record before the ownership guard drops, preserving the
    /// "remove discovery → release ownership" order. Best-effort; the normal
    /// path already ran `finish_shutdown` and this call then no-ops.
    fn drop(&mut self) {
        let _ = self.finish_shutdown();
    }
}

/// The accept loop. It only *admits* work: every connection is served on a
/// thread owned by [`DaemonRuntime::workers`], so a graceful stop joins the
/// in-flight requests instead of leaving detached writers behind (the R2
/// carry-over: a detached handler could hold the SQLite connection past
/// ownership release).
///
/// Joining happens outside this thread. The loop never calls `stop_and_join`,
/// so it can never wait on itself.
fn spawn_accept_thread(listener: TcpListener, runtime: Arc<DaemonRuntime>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        loop {
            if runtime.stop.load(Ordering::Acquire) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let connection_runtime = Arc::clone(&runtime);
                    // A refusal means shutdown started between the accept and
                    // here: the stream is dropped with the closure, which is
                    // exactly "stop admitting".
                    let _ = runtime.workers.spawn(Box::new(move |_stop| {
                        serve_connection(stream, &connection_runtime);
                    }));
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
    // Park the socket while the request head is read: a peer that connects
    // and sends nothing would otherwise pin this worker for the whole read
    // timeout. Shutdown wakes every parked read; the timeout remains the
    // backstop.
    let ticket = runtime.idle_connections.park(&stream);
    let request = http::read_request(&mut stream);
    let released = runtime.idle_connections.release(ticket);
    let request = match request {
        Ok(request) => request,
        Err(_) => return,
    };
    // We read a complete request head, but shutdown may have shut this socket
    // down in the window before `release` took the lock. Dispatching now would
    // run the handler — committing any mutation it carries — while the
    // response write silently fails, so a client retrying the interrupted
    // request against the next daemon instance would double-apply it. Drop the
    // request instead: not applying it once is what makes the retry safe.
    if !released && runtime.stop.load(Ordering::Acquire) {
        return;
    }
    match server::dispatch(&request, runtime) {
        server::Dispatch::Respond(response) => {
            let _ = http::write_response(&mut stream, &response);
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        // The upgrade owns the connection from here (the 101 head, frames,
        // and unsubscribe happen inside the session).
        server::Dispatch::Upgrade(session) => session.serve(stream, runtime),
    }
}

fn http_io_timeout() -> Duration {
    Duration::from_secs(5)
}

/// Run the daemon in the foreground: like [`Daemon::start`], plus a signal
/// handler that triggers the same graceful shutdown. With `ctrlc`'s
/// `termination` feature the handler covers SIGTERM (and SIGHUP) as well as
/// SIGINT, so a service manager stopping the unit takes exactly the same
/// drain-and-release path as ctrl-c.
pub fn run_foreground(options: DaemonOptions) -> Result<(), DaemonError> {
    let daemon = Daemon::start(&options)?;
    let handler_stop = Arc::clone(&daemon.runtime.stop);
    ctrlc::set_handler(move || {
        handler_stop.store(true, Ordering::Release);
    })
    .map_err(|error| DaemonError::Signal(error.to_string()))?;
    let record = daemon.discovery_record();
    println!(
        "hiero daemon listening on {} (pid {}); discovery: {}; ctrl-c to stop",
        daemon.local_addr(),
        record.pid,
        daemon.runtime.config.daemon_discovery_path().display()
    );
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    daemon.wait_for_shutdown()
}

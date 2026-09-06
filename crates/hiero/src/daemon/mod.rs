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
pub mod semantic_worker;
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

/// Cleanup for the startup steps that run *after* the first worker thread
/// exists (ADR 0009: no writer may outlive data-root ownership).
///
/// `Daemon::start` spawns supervised workers — the semantic controller, then
/// the dream controller — and then keeps failing: instance id, discovery
/// publication, controller setup. Every one of those `?`s used to drop a
/// `WorkerGroup` that joins nothing and a `RootOwnership` that releases
/// immediately, leaving a live writer on a root another process could take
/// over. This guard owns all three pieces for that window so an early return
/// (or a panic) unwinds through one ordered teardown instead.
///
/// Drop order is the whole point, and it is the same order `finish_shutdown`
/// uses: signal → join every worker → remove our discovery record → release
/// ownership. It is written out explicitly rather than left to field order,
/// because getting it backwards is exactly the bug being fixed.
///
/// On the success path [`StartupGuard::into_parts`] moves the pieces onto the
/// `DaemonRuntime`/`Daemon` that own them from then on, and the drop below
/// becomes a no-op.
struct StartupGuard {
    stop: Arc<AtomicBool>,
    /// `None` only after `into_parts` handed the group to the runtime.
    workers: Option<WorkerGroup>,
    /// `None` only after `into_parts` handed the guard to the daemon.
    ownership: Option<RootOwnership>,
    /// The discovery record published during startup, if publication got that
    /// far: a failed start must not leave a readiness record pointing at a
    /// daemon that never began serving.
    published: Option<(HieronymusConfig, String)>,
}

impl StartupGuard {
    fn new(stop: Arc<AtomicBool>, workers: WorkerGroup, ownership: RootOwnership) -> StartupGuard {
        StartupGuard {
            stop,
            workers: Some(workers),
            ownership: Some(ownership),
            published: None,
        }
    }

    /// The worker group the controllers register with.
    fn workers_mut(&mut self) -> &mut WorkerGroup {
        self.workers
            .as_mut()
            .expect("the startup guard owns its workers until `into_parts`")
    }

    /// Record that the discovery record is now on disk, so a later startup
    /// failure removes it again.
    fn mark_published(&mut self, config: &HieronymusConfig, instance_id: &str) {
        self.published = Some((config.clone(), instance_id.to_string()));
    }

    /// Startup succeeded: hand the pieces to their long-lived owners. The
    /// guard's own `Drop` then finds nothing left to clean up.
    fn into_parts(mut self) -> (Arc<AtomicBool>, WorkerGroup, RootOwnership) {
        let workers = self
            .workers
            .take()
            .expect("the startup guard is disarmed exactly once");
        let ownership = self
            .ownership
            .take()
            .expect("the startup guard is disarmed exactly once");
        self.published = None;
        (Arc::clone(&self.stop), workers, ownership)
    }
}

impl Drop for StartupGuard {
    fn drop(&mut self) {
        if let Some(workers) = self.workers.take() {
            // Signal first, then wait: a worker that already armed a provider
            // or opened the database must be gone before anything else lets
            // another process in. A panicked worker has terminated, so the
            // join result is only diagnostic here — the guard is released
            // either way, and there is no caller left to return it to.
            self.stop.store(true, Ordering::Release);
            if let Err(error) = workers.stop_and_join() {
                eprintln!("hiero daemon startup cleanup: {error}");
            }
            drop(workers);
        }
        // Only now, with every writer joined: unpublish, then release the
        // root. Publishing readiness for a daemon that failed to start would
        // point clients (and `hiero status`) at a dead endpoint.
        if let Some((config, instance_id)) = self.published.take() {
            discovery::remove_discovery(&config, &instance_id);
        }
        drop(self.ownership.take());
    }
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
    /// Shared with the semantic controller, which installs/refreshes the
    /// armed query lane through interior mutability (Task S2).
    pub application: Arc<Application>,
    /// The supervised dream controller (task D5): every production dream
    /// run — scheduled, admin manual, and MCP — coalesces into its worker.
    /// Its event hub is the runtime's admin event hub.
    pub dream: DreamController,
    /// The supervised semantic rebuild worker and readiness state (Task S2).
    pub semantic: semantic_worker::SemanticController,
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
        let application = Arc::new(Application::open(&config)?);

        // Bind and credential validation come *before* any worker exists.
        // Both are ordinary startup refusals — an occupied port (ADR 0009
        // never scans for another one) and an unusable token file are the two
        // most common ones — and neither needs a running worker to decide.
        // Failing here therefore returns without ever having spawned a
        // thread, which is strictly safer than unwinding through a cleanup:
        // there is nothing to detach.
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

        // From here on startup owns worker threads, so every remaining `?`
        // unwinds through `StartupGuard`: signal, join, unpublish, and only
        // then release the root.
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = StartupGuard::new(
            Arc::clone(&stop),
            WorkerGroup::new(Arc::clone(&stop)),
            ownership,
        );

        // The semantic controller (Task S2): one supervised worker under the
        // same stop edge as everything else. Arming resolves through the
        // persisted runtime configuration (`hiero semantic enable --runtime`
        // validated here, retained across restarts); the query lane it arms
        // is installed into the application, and RAG imports queue durable
        // rebuilds back through the controller.
        let semantic = {
            let lane_application = Arc::clone(&application);
            let arm = semantic_worker::resolve_arm(&config);
            semantic_worker::SemanticController::start_with(
                config.clone(),
                guard.workers_mut(),
                arm,
                Box::new(move |lane| lane_application.install_semantic_lane(lane)),
            )
            .map_err(DaemonError::Worker)?
        };
        let hook_controller = semantic.clone();
        application.set_rebuild_hook(Arc::new(move |series| {
            hook_controller.request_rebuild(series)
        }));

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
        guard.mark_published(&config, &record.instance_id);

        // The dream controller registers its worker with the group, so a
        // graceful stop joins it before discovery is removed and ownership
        // released (task D5; the SIGTERM path needs no extra wiring).
        let dream = dream_worker::DreamController::start(config.clone(), guard.workers_mut())
            .map_err(DaemonError::Worker)?;
        // The controller runs every production dream run; the MCP
        // `hieronymus_dream` dispatch serves through this handle. On a bare
        // `Application::open` (no daemon) it stays absent and the dispatch
        // fails closed.
        application.install_dream_controller(dream.clone());
        let events = dream.events();

        // Startup succeeded: the runtime owns the workers and the daemon owns
        // the root guard from here, and nothing below can fail.
        let (stop, workers, ownership) = guard.into_parts();
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
            semantic,
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

    /// Diagnostics support: how many worker handles the group still retains.
    /// Steady state is the long-lived controllers plus whatever requests are
    /// genuinely in flight — completed connection workers are reaped by the
    /// accept loop, so this must not grow with the number of requests served.
    pub fn worker_count(&self) -> usize {
        self.runtime.workers.live_count()
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

        // 6. Close the semantic index: the semantic controller's worker is
        //    joined with the group above, so its provider/index handles are
        //    already gone by the time discovery is removed.

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
/// so it can never wait on itself. It *does* call `reap_finished`, which joins
/// only handles that already ran to completion and gives up immediately if a
/// shutdown holds the join section — the accept loop is never the thread that
/// waits.
///
/// The accept thread is not itself a member of the group, and `finish_shutdown`
/// joins it before it drains the group, so the reap below cannot collide with
/// the shutdown drain for long (and is harmless when it does).
fn spawn_accept_thread(listener: TcpListener, runtime: Arc<DaemonRuntime>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        loop {
            if runtime.stop.load(Ordering::Acquire) {
                break;
            }
            // Retire finished connection workers. Under load this runs once
            // per accepted connection; when idle, once per `ACCEPT_POLL`.
            // Without it the handle list would grow by one entry per request
            // served and only drain at shutdown.
            if let Err(error) = runtime.workers.reap_finished() {
                // A panicked connection worker is a bug worth reporting, not
                // a reason to stop accepting: the panic already unwound that
                // one thread and touched nothing else.
                eprintln!("hiero daemon worker: {error}");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record_for(instance_id: &str) -> DiscoveryRecord {
        DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: PROTOCOL_REVISION.to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            pid: std::process::id(),
            instance_id: instance_id.to_string(),
            started_at: "2026-09-06T00:00:00Z".to_string(),
        }
    }

    /// The drop order the ownership invariant depends on: a startup that
    /// fails after spawning must join every worker, then unpublish, and only
    /// then release the root. The integration suite covers the real
    /// `Daemon::start` paths that reach this; here the guard itself is put
    /// through its teardown directly, because the last post-publish failure
    /// (`DreamController::start`) has no external trigger.
    #[test]
    fn a_dropped_startup_guard_joins_then_unpublishes_then_releases() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let ownership = RootOwnership::acquire(&config, "daemon").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = StartupGuard::new(
            Arc::clone(&stop),
            WorkerGroup::new(Arc::clone(&stop)),
            ownership,
        );

        // A worker that outlives the failure point, like a semantic worker
        // mid-arming, plus the record a partial startup already published.
        let joined = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&joined);
        guard
            .workers_mut()
            .spawn(Box::new(move |_stop| {
                std::thread::sleep(Duration::from_millis(150));
                flag.store(true, Ordering::Release);
            }))
            .unwrap();
        let record = record_for("guarded-instance");
        discovery::write_discovery(&config, &record).unwrap();
        guard.mark_published(&config, &record.instance_id);

        drop(guard);

        assert!(
            joined.load(Ordering::Acquire),
            "the guard must join every worker before it releases the root"
        );
        assert!(
            !config.daemon_discovery_path().exists(),
            "a failed startup must not leave a readiness record behind"
        );
        assert!(
            RootOwnership::acquire(&config, "probe").is_ok(),
            "ownership must be free once the guard has dropped"
        );
    }

    /// The success path: `into_parts` hands everything to its long-lived
    /// owner, so the guard's drop must not signal, join, unpublish, or
    /// release anything.
    #[test]
    fn a_disarmed_startup_guard_leaves_its_parts_running() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let ownership = RootOwnership::acquire(&config, "daemon").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut guard = StartupGuard::new(
            Arc::clone(&stop),
            WorkerGroup::new(Arc::clone(&stop)),
            ownership,
        );
        let record = record_for("live-instance");
        discovery::write_discovery(&config, &record).unwrap();
        guard.mark_published(&config, &record.instance_id);

        let (kept_stop, workers, ownership) = guard.into_parts();

        assert!(!kept_stop.load(Ordering::Acquire), "nothing was cancelled");
        workers
            .spawn(Box::new(|_| {}))
            .expect("the group still admits work");
        assert!(
            config.daemon_discovery_path().exists(),
            "a started daemon keeps its published record"
        );
        assert!(
            RootOwnership::acquire(&config, "probe").is_err(),
            "the caller still holds the root"
        );
        workers.stop_and_join().unwrap();
        drop(ownership);
    }
}

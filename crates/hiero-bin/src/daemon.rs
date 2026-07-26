use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    future::Future,
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    pin::Pin,
    str::FromStr,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{HOST, ORIGIN},
        uri::Authority,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use hiero_core::{
    config::HieronymusConfig,
    db,
    dreaming::{DreamConfig, DreamPhaseError, DreamService, WorkflowProfile, run_background_loop},
    provider::{
        ModelCache, ProviderCatalog, ProviderRegistry, ProviderTransport, ReqwestTransportOptions,
    },
};
use sqlx::SqlitePool;
use subtle::ConstantTimeEq;
use tokio::{
    sync::{Notify, broadcast, mpsc, oneshot},
    task::{Id, JoinError, JoinHandle, JoinSet},
};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::api::{
    self,
    error::ApiError,
    events::{AdminNotifier, admin_event_channel, admin_ws},
    system::{health, shutdown, status},
};
use crate::assets::{AssetSource, is_client_route, serve_assets, serve_client_route, serve_index};
use crate::mcp::{StoreDreamRunner, StoreMcpBackend, http, proxy_operation};

const BODY_LIMIT: usize = 1_000_000;
const REQUEST_ID_HEADER: &str = "x-request-id";
const AUTH_TOKEN_HEADER: &str = "x-hieronymus-token";
const WORKER_CHANNEL_CAPACITY: usize = 64;
const WORKER_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const WORKER_ABORT_GRACE: Duration = Duration::from_secs(1);
pub const DAEMON_DREAM_CLEANUP_DEADLINE: Duration = WORKER_ABORT_GRACE;
const WORKER_SHUTDOWN_TOTAL: Duration = Duration::from_secs(4);
const SERVER_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const LOOPBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RETAINED_WORKER_FAILURES: usize = 16;
const MAX_WORKER_FAILURE_BYTES: usize = 1_024;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<HieronymusConfig>,
    pub shutdown: broadcast::Sender<()>,
    pub auth_token: Arc<str>,
    pub port: u16,
    pub dream_running: Arc<AtomicBool>,
    pub provider_transport: Option<Arc<dyn ProviderTransport>>,
    pub provider_catalog_mutations: Arc<tokio::sync::Mutex<()>>,
    pub workers: WorkerSupervisor,
    pub events: AdminNotifier,
    pub assets: AssetSource,
}

type WorkerFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerKind {
    OneShot,
    Recurring,
}

enum WorkerCommand {
    Spawn {
        kind: WorkerKind,
        task: WorkerFuture,
    },
    Shutdown(oneshot::Sender<Result<()>>),
}

struct WorkerSupervisorInner {
    commands: mpsc::Sender<WorkerCommand>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
    closed: AtomicBool,
    reaper: StdMutex<Option<JoinHandle<()>>>,
    fatal: broadcast::Sender<String>,
}

#[derive(Clone)]
pub struct WorkerSupervisor {
    inner: Arc<WorkerSupervisorInner>,
}

impl Default for WorkerSupervisor {
    fn default() -> Self {
        let (commands, receiver) = mpsc::channel(WORKER_CHANNEL_CAPACITY);
        let active = Arc::new(AtomicUsize::new(0));
        let idle = Arc::new(Notify::new());
        let (fatal, _) = broadcast::channel(4);
        let reaper = tokio::spawn(run_worker_supervisor(
            receiver,
            active.clone(),
            idle.clone(),
            fatal.clone(),
        ));
        Self {
            inner: Arc::new(WorkerSupervisorInner {
                commands,
                active,
                idle,
                closed: AtomicBool::new(false),
                reaper: StdMutex::new(Some(reaper)),
                fatal,
            }),
        }
    }
}

impl WorkerSupervisor {
    pub async fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> bool {
        self.spawn_result(async move {
            task.await;
            Ok(())
        })
        .await
    }

    pub async fn spawn_result(
        &self,
        task: impl Future<Output = Result<()>> + Send + 'static,
    ) -> bool {
        self.spawn_kind(WorkerKind::OneShot, task).await
    }

    pub async fn spawn_recurring(
        &self,
        task: impl Future<Output = Result<()>> + Send + 'static,
    ) -> bool {
        self.spawn_kind(WorkerKind::Recurring, task).await
    }

    async fn spawn_kind(
        &self,
        kind: WorkerKind,
        task: impl Future<Output = Result<()>> + Send + 'static,
    ) -> bool {
        if self.inner.closed.load(Ordering::Acquire) {
            return false;
        }
        self.inner
            .commands
            .send(WorkerCommand::Spawn {
                kind,
                task: Box::pin(task),
            })
            .await
            .is_ok()
    }

    #[must_use]
    pub fn subscribe_fatal(&self) -> broadcast::Receiver<String> {
        self.inner.fatal.subscribe()
    }

    pub async fn receive_fatal(&self, receiver: &mut broadcast::Receiver<String>) -> String {
        let mut lagged = 0_u64;
        loop {
            match receiver.recv().await {
                Ok(failure) if lagged == 0 => {
                    return format!("fatal recurring worker failure: {failure}");
                }
                Ok(failure) => {
                    return format!(
                        "fatal recurring worker failure: {failure}; {lagged} earlier fatal notification(s) were dropped"
                    );
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    lagged = lagged.saturating_add(skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return "fatal recurring worker failure channel closed unexpectedly".into();
                }
            }
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        match tokio::time::timeout(WORKER_SHUTDOWN_TOTAL, self.shutdown_inner()).await {
            Ok(result) => result,
            Err(error) => {
                let reaper = self
                    .inner
                    .reaper
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(reaper) = reaper {
                    reaper.abort();
                }
                Err(anyhow::Error::new(error).context("worker shutdown timed out"))
            }
        }
    }

    async fn shutdown_inner(&self) -> Result<()> {
        let mut result = Ok(());
        if !self.inner.closed.swap(true, Ordering::AcqRel) {
            let (completed, observed) = oneshot::channel();
            if self
                .inner
                .commands
                .send(WorkerCommand::Shutdown(completed))
                .await
                .is_ok()
            {
                result = observed
                    .await
                    .context("worker supervisor stopped before acknowledging shutdown")?;
            }
        }
        if result.is_ok() {
            self.wait_idle().await;
        }
        let reaper = self
            .inner
            .reaper
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(reaper) = reaper {
            reaper
                .await
                .context("worker supervisor task panicked during shutdown")?;
        }
        result
    }

    pub async fn active_count(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    pub async fn wait_idle(&self) {
        loop {
            let idle = self.inner.idle.notified();
            if self.inner.active.load(Ordering::Acquire) == 0 {
                return;
            }
            idle.await;
        }
    }
}

async fn run_worker_supervisor(
    mut commands: mpsc::Receiver<WorkerCommand>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
    fatal: broadcast::Sender<String>,
) {
    let mut tasks = JoinSet::new();
    let mut kinds = HashMap::new();
    let mut failures = Vec::new();
    loop {
        if tasks.is_empty() {
            match commands.recv().await {
                Some(WorkerCommand::Spawn { kind, task }) => {
                    spawn_worker(&mut tasks, &mut kinds, &active, kind, task);
                }
                Some(WorkerCommand::Shutdown(completed)) => {
                    let shutdown = finish_worker_shutdown(
                        &mut commands,
                        &mut tasks,
                        &mut kinds,
                        &active,
                        &idle,
                        &mut failures,
                        &fatal,
                    )
                    .await;
                    let _ =
                        completed.send(combine_worker_results(shutdown, worker_failures(failures)));
                    return;
                }
                None => {
                    if let Err(error) = abort_workers(
                        &mut tasks,
                        &mut kinds,
                        &active,
                        &idle,
                        &mut failures,
                        &fatal,
                    )
                    .await
                    {
                        tracing::error!(?error, "worker supervisor closed uncleanly");
                    }
                    for failure in failures {
                        tracing::error!(failure, "worker task failed after supervisor closed");
                    }
                    return;
                }
            }
        } else {
            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(WorkerCommand::Spawn { kind, task }) => {
                            spawn_worker(&mut tasks, &mut kinds, &active, kind, task);
                        }
                        Some(WorkerCommand::Shutdown(completed)) => {
                            let shutdown = finish_worker_shutdown(
                                &mut commands,
                                &mut tasks,
                                &mut kinds,
                                &active,
                                &idle,
                                &mut failures,
                                &fatal,
                            ).await;
                            let _ = completed.send(combine_worker_results(
                                shutdown,
                                worker_failures(failures),
                            ));
                            return;
                        }
                        None => {
                            if let Err(error) = abort_workers(
                                &mut tasks,
                                &mut kinds,
                                &active,
                                &idle,
                                &mut failures,
                                &fatal,
                            ).await {
                                tracing::error!(?error, "worker supervisor closed uncleanly");
                            }
                            for failure in failures {
                                tracing::error!(
                                    failure,
                                    "worker task failed after supervisor closed"
                                );
                            }
                            return;
                        }
                    }
                }
                joined = tasks.join_next_with_id() => {
                    if let Some(joined) = joined {
                        record_worker_result(
                            joined,
                            &mut kinds,
                            &mut failures,
                            &fatal,
                        );
                        record_worker_completion(&active, &idle);
                    }
                }
            }
        }
    }
}

fn spawn_worker(
    tasks: &mut JoinSet<Result<()>>,
    kinds: &mut HashMap<Id, WorkerKind>,
    active: &AtomicUsize,
    kind: WorkerKind,
    task: WorkerFuture,
) {
    active.fetch_add(1, Ordering::AcqRel);
    let handle = tasks.spawn(task);
    kinds.insert(handle.id(), kind);
}

async fn finish_worker_shutdown(
    commands: &mut mpsc::Receiver<WorkerCommand>,
    tasks: &mut JoinSet<Result<()>>,
    kinds: &mut HashMap<Id, WorkerKind>,
    active: &Arc<AtomicUsize>,
    idle: &Arc<Notify>,
    failures: &mut Vec<String>,
    fatal: &broadcast::Sender<String>,
) -> Result<()> {
    commands.close();
    while let Some(command) = commands.recv().await {
        if let WorkerCommand::Spawn { kind, task } = command {
            spawn_worker(tasks, kinds, active, kind, task);
        }
    }
    let graceful = async {
        while let Some(joined) = tasks.join_next_with_id().await {
            record_worker_result(joined, kinds, failures, fatal);
            record_worker_completion(active, idle);
        }
    };
    if tokio::time::timeout(WORKER_SHUTDOWN_GRACE, graceful)
        .await
        .is_err()
    {
        abort_workers(tasks, kinds, active, idle, failures, fatal).await?;
    }
    Ok(())
}

async fn abort_workers(
    tasks: &mut JoinSet<Result<()>>,
    kinds: &mut HashMap<Id, WorkerKind>,
    active: &Arc<AtomicUsize>,
    idle: &Arc<Notify>,
    failures: &mut Vec<String>,
    fatal: &broadcast::Sender<String>,
) -> Result<()> {
    let mut aborted = std::mem::take(tasks);
    let mut aborted_kinds = std::mem::take(kinds);
    aborted.abort_all();
    let reaper_active = active.clone();
    let reaper_idle = idle.clone();
    let fatal = fatal.clone();
    let mut reaper = tokio::spawn(async move {
        let mut aborted_failures = Vec::new();
        while let Some(joined) = aborted.join_next_with_id().await {
            record_worker_result(joined, &mut aborted_kinds, &mut aborted_failures, &fatal);
            record_worker_completion(&reaper_active, &reaper_idle);
        }
        aborted_failures
    });
    match tokio::time::timeout(WORKER_ABORT_GRACE, &mut reaper).await {
        Ok(Ok(aborted_failures)) => {
            merge_worker_failures(failures, aborted_failures);
            Ok(())
        }
        Ok(Err(error)) => {
            Err(anyhow::Error::new(error)
                .context("unclean worker shutdown: abort reaper task failed"))
        }
        Err(error) => {
            let remaining = active.load(Ordering::Acquire);
            reaper.abort();
            Err(anyhow::Error::new(error).context(format!(
                "unclean worker shutdown: {remaining} worker(s) did not terminate within the abort deadline"
            )))
        }
    }
}

fn record_worker_completion(active: &AtomicUsize, idle: &Notify) {
    if active.fetch_sub(1, Ordering::AcqRel) == 1 {
        idle.notify_waiters();
    }
}

fn record_worker_result(
    joined: std::result::Result<(Id, Result<()>), JoinError>,
    kinds: &mut HashMap<Id, WorkerKind>,
    failures: &mut Vec<String>,
    fatal: &broadcast::Sender<String>,
) {
    let id = match &joined {
        Ok((id, _)) => *id,
        Err(error) => error.id(),
    };
    let kind = kinds.remove(&id).unwrap_or(WorkerKind::Recurring);
    match joined {
        Ok((_, Ok(()))) => {}
        Ok((_, Err(error))) if kind == WorkerKind::OneShot => {
            tracing::error!(?error, "one-shot worker failed");
        }
        Ok((_, Err(error))) => report_fatal_worker(failures, fatal, error.to_string()),
        Err(error) if error.is_cancelled() => {}
        Err(error) => {
            report_fatal_worker(failures, fatal, format!("worker task failed: {error}"));
        }
    }
}

fn report_fatal_worker(
    failures: &mut Vec<String>,
    fatal: &broadcast::Sender<String>,
    failure: String,
) {
    retain_worker_failure(failures, failure.clone());
    let _ = fatal.send(failure);
}

fn retain_worker_failure(failures: &mut Vec<String>, mut failure: String) {
    if failure.len() > MAX_WORKER_FAILURE_BYTES {
        let boundary = failure
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= MAX_WORKER_FAILURE_BYTES)
            .last()
            .unwrap_or_default();
        failure.truncate(boundary);
        failure.push('…');
    }
    tracing::error!(failure, "supervised worker failed");
    if failures.len() == MAX_RETAINED_WORKER_FAILURES {
        failures.remove(0);
    }
    failures.push(failure);
}

fn worker_failures(failures: Vec<String>) -> Result<()> {
    ensure!(
        failures.is_empty(),
        "worker task failure(s): {}",
        failures.join("; ")
    );
    Ok(())
}

fn merge_worker_failures(failures: &mut Vec<String>, mut additional: Vec<String>) {
    failures.append(&mut additional);
    if failures.len() > MAX_RETAINED_WORKER_FAILURES {
        failures.drain(..failures.len() - MAX_RETAINED_WORKER_FAILURES);
    }
}

fn combine_worker_results(shutdown: Result<()>, failures: Result<()>) -> Result<()> {
    match (shutdown, failures) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(shutdown), Err(failures)) => {
            Err(shutdown.context(format!("worker failures also occurred: {failures:#}")))
        }
    }
}

#[derive(Clone, Debug)]
pub struct RequestId(pub(crate) String);

pub fn build_router(state: AppState) -> Router {
    let security_state = state.clone();
    let dream_runner = Arc::new(
        StoreDreamRunner::new(state.pool.clone(), state.config.clone())
            .with_notifier(state.events.clone()),
    );
    let mcp = http::service(Arc::new(
        StoreMcpBackend::new(state.pool.clone(), state.config.clone())
            .with_dream_runner(dream_runner)
            .with_notifier(state.events.clone()),
    ));
    Router::new()
        .route("/", get(serve_index))
        .route("/admin", get(serve_index))
        .route("/config", get(serve_index))
        .route("/assets/{*path}", get(serve_assets))
        .route("/api/mcp/{operation}", post(proxy_operation))
        .nest("/api/providers", api::providers::routes())
        .nest("/api/settings", api::settings::routes())
        .nest("/api/admin", api::admin::routes())
        .route("/api/{*path}", any(api::placeholder))
        .route("/ws/admin", get(admin_ws))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/shutdown", post(shutdown))
        .nest_service("/mcp", mcp)
        .method_not_allowed_fallback(api::method_not_allowed)
        .fallback(get(serve_client_route).fallback(api::not_found))
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(assign_request_id))
                .layer(TraceLayer::new_for_http())
                .layer(middleware::from_fn(limit_request_body))
                .layer(middleware::from_fn_with_state(
                    security_state,
                    authorize_request,
                )),
        )
        .with_state(state)
}

pub async fn serve<S>(config: HieronymusConfig, port: u16, shutdown: S) -> Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    let listener = bind_listener(port).await?;
    let auth_token = load_or_create_auth_token(&config.auth_token_path())?;
    let pool = db::connect(&config)
        .await
        .context("failed to open daemon database")?;
    let (shutdown_sender, shutdown_receiver) = broadcast::channel(4);
    let (events, _) = admin_event_channel(64);
    let assets = AssetSource::from_environment().context("failed to configure frontend assets")?;
    let workers = WorkerSupervisor::default();
    let mut worker_fatal = workers.subscribe_fatal();
    start_recurring_workers(
        pool.clone(),
        Arc::new(config.clone()),
        shutdown_sender.clone(),
        events.clone(),
        &workers,
    )
    .await?;
    let router = build_router(AppState {
        pool,
        config: Arc::new(config),
        shutdown: shutdown_sender.clone(),
        auth_token,
        port,
        dream_running: Arc::new(AtomicBool::new(false)),
        provider_transport: None,
        provider_catalog_mutations: Arc::new(tokio::sync::Mutex::new(())),
        workers: workers.clone(),
        events,
        assets,
    });

    let shutdown_broadcast = shutdown_sender.clone();
    let shutdown_started = Arc::new(Notify::new());
    let graceful_started = shutdown_started.clone();
    let server = async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(await_shutdown_and_broadcast(
                shutdown,
                shutdown_receiver,
                shutdown_broadcast,
                graceful_started,
            ))
            .await
    };
    tokio::pin!(server);
    let result = tokio::select! {
        result = &mut server => result.context("daemon server failed"),
        () = shutdown_started.notified() => {
            match tokio::time::timeout(SERVER_SHUTDOWN_GRACE, &mut server).await {
                Ok(result) => result.context("daemon server failed"),
                Err(error) => Err(anyhow::Error::new(error)
                    .context("daemon graceful shutdown timed out")),
            }
        },
        failure = workers.receive_fatal(&mut worker_fatal) => {
            let _ = shutdown_sender.send(());
            let drained = tokio::time::timeout(SERVER_SHUTDOWN_GRACE, &mut server).await;
            match drained {
                Ok(Ok(())) => Err(anyhow::anyhow!(failure)),
                Ok(Err(server)) => Err(anyhow::anyhow!(
                    "{failure}; daemon server also failed: {server}"
                )),
                Err(error) => Err(anyhow::Error::new(error).context(format!(
                    "{failure}; daemon graceful shutdown timed out"
                ))),
            }
        }
    };
    let _ = shutdown_sender.send(());
    let worker_result = workers.shutdown().await;
    match (result, worker_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(server), Err(worker)) => {
            Err(server.context(format!("worker shutdown also failed: {worker:#}")))
        }
    }
}

async fn start_recurring_workers(
    pool: SqlitePool,
    config: Arc<HieronymusConfig>,
    shutdown: broadcast::Sender<()>,
    events: AdminNotifier,
    workers: &WorkerSupervisor,
) -> Result<()> {
    let load_config = config.clone();
    let dream_config = tokio::task::spawn_blocking(move || DreamConfig::load(&load_config))
        .await
        .context("background dream configuration task failed")??;
    if !dream_config.enabled {
        return Ok(());
    }
    let catalog_config = config.clone();
    let catalog = tokio::task::spawn_blocking(move || {
        ProviderCatalog::load(catalog_config.provider_config_path())
    })
    .await
    .context("background provider catalog task failed")??;

    let interval = Duration::from_secs(
        dream_config
            .schedule_interval_minutes
            .checked_mul(60)
            .context("background dream interval is out of range")?,
    );
    let registry = Arc::new(ProviderRegistry::production(
        ModelCache::new(128, 1_000_000, Duration::from_secs(24 * 60 * 60)),
        ReqwestTransportOptions::default(),
    ));
    let resolver_catalog = Arc::new(catalog.clone());
    let resolver = Arc::new(move |workflow: &WorkflowProfile| {
        registry
            .resolve(&resolver_catalog, &workflow.provider, &workflow.model)
            .map(Arc::from)
            .map_err(DreamPhaseError::Provider)
    });
    let shutdown_receiver = shutdown.subscribe();
    ensure!(
        workers
            .spawn_recurring(async move {
                events.notify_refresh();
                let service = DreamService::new_with_catalog(
                    &pool,
                    &config,
                    &dream_config,
                    resolver,
                    catalog,
                )
                .with_cleanup_deadline(DAEMON_DREAM_CLEANUP_DEADLINE);
                let result = run_background_loop(service, interval, shutdown_receiver)
                    .await
                    .map_err(anyhow::Error::from);
                events.notify_refresh();
                result
            })
            .await,
        "worker supervisor rejected the background dream scheduler"
    );
    Ok(())
}

pub async fn await_shutdown_and_broadcast<S>(
    shutdown: S,
    mut shutdown_receiver: broadcast::Receiver<()>,
    shutdown_sender: broadcast::Sender<()>,
    shutdown_started: Arc<Notify>,
) where
    S: Future<Output = ()> + Send,
{
    tokio::select! {
        () = shutdown => {}
        _ = shutdown_receiver.recv() => {}
    }
    let _ = shutdown_sender.send(());
    shutdown_started.notify_one();
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let terminate = signal(SignalKind::terminate());
        if let Ok(mut terminate) = terminate {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

pub async fn daemon_status(
    config: &HieronymusConfig,
    port: u16,
) -> Result<Option<serde_json::Value>> {
    let token = match read_auth_token(&config.auth_token_path()) {
        Ok(token) => token,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let response = match loopback_http_client()?
        .get(format!("http://127.0.0.1:{port}/status"))
        .header(AUTH_TOKEN_HEADER, token.as_ref())
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_connect() => return Ok(None),
        Err(error) => return Err(error).context("failed to query daemon status"),
    };
    ensure!(
        response.status().is_success(),
        "daemon status request failed with {}",
        response.status()
    );
    response
        .json()
        .await
        .map(Some)
        .context("daemon returned malformed status JSON")
}

pub async fn request_shutdown(config: &HieronymusConfig, port: u16) -> Result<serde_json::Value> {
    let token = read_auth_token(&config.auth_token_path())?;
    let response = loopback_http_client()?
        .post(format!("http://127.0.0.1:{port}/shutdown"))
        .header(AUTH_TOKEN_HEADER, token.as_ref())
        .send()
        .await
        .context("failed to contact the Hieronymus daemon")?;
    ensure!(
        response.status().is_success(),
        "daemon shutdown request failed with {}",
        response.status()
    );
    response
        .json()
        .await
        .context("daemon returned malformed shutdown JSON")
}

pub(crate) fn loopback_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(LOOPBACK_CONNECT_TIMEOUT)
        .timeout(LOOPBACK_REQUEST_TIMEOUT)
        .build()
        .context("failed to build hardened loopback HTTP client")
}

pub async fn bind_listener(port: u16) -> Result<tokio::net::TcpListener> {
    ensure!(port != 0, "daemon port 0 is not allowed");
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind daemon to {address}"))
}

pub fn load_or_create_auth_token(path: &Path) -> Result<Arc<str>> {
    let parent = path
        .parent()
        .context("auth token path must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create auth token directory `{}`",
            parent.display()
        )
    })?;

    let generated = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(generated.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.sync_all())
                .with_context(|| format!("failed to write auth token `{}`", path.display()))?;
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create auth token `{}`", path.display()));
        }
    }

    read_auth_token(path)
}

pub(crate) fn read_auth_token(path: &Path) -> Result<Arc<str>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect auth token `{}`", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "auth token must be a regular file"
    );
    #[cfg(unix)]
    ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "auth token file must have 0600 permissions"
    );

    let mut contents = String::new();
    OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open auth token `{}`", path.display()))?
        .read_to_string(&mut contents)
        .with_context(|| format!("failed to read auth token `{}`", path.display()))?;
    let token = contents
        .strip_suffix("\r\n")
        .or_else(|| contents.strip_suffix('\n'))
        .unwrap_or(&contents);
    ensure!(
        !token.is_empty()
            && !token.contains(['\r', '\n'])
            && token.bytes().all(|byte| byte.is_ascii_graphic()),
        "auth token file must contain one non-empty ASCII token"
    );
    Ok(Arc::from(token))
}

async fn assign_request_id(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map_or_else(|| Uuid::new_v4().to_string(), ToOwned::to_owned);
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        REQUEST_ID_HEADER,
        HeaderValue::from_str(&request_id).expect("validated or generated request ID is a header"),
    );
    response
}

async fn limit_request_body(request: Request, next: Next) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .expect("request ID middleware runs before the body limit");
    let (parts, body) = request.into_parts();
    match to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Err(_) => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "request body exceeds 1000000 bytes",
            request_id,
        )
        .into_response(),
    }
}

async fn authorize_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .expect("request ID middleware runs before security validation");
    let Some(host) = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Authority::from_str(value).ok())
    else {
        return forbidden(request_id);
    };
    if !is_expected_authority(&host, state.port) {
        return forbidden(request_id);
    }

    let has_origin = request.headers().contains_key(ORIGIN);
    if has_origin {
        let valid_origin = request
            .headers()
            .get(ORIGIN)
            .expect("Origin presence was checked")
            .to_str()
            .ok()
            .and_then(|value| value.parse::<axum::http::Uri>().ok())
            .filter(|uri| uri.scheme_str() == Some("http"))
            .and_then(|uri| uri.authority().cloned())
            .is_some_and(|origin| same_authority(&host, &origin, state.port));
        if !valid_origin {
            return forbidden(request_id);
        }
    }

    let token_valid = valid_token(request.headers(), &state.auth_token);
    let authorized = match route_policy(request.uri().path()) {
        RoutePolicy::Static => true,
        RoutePolicy::Browser => has_origin || token_valid,
        RoutePolicy::WebSocket => has_origin,
        RoutePolicy::Authenticated => token_valid,
    };
    if authorized {
        next.run(request).await
    } else {
        api::unauthorized(request_id)
    }
}

fn forbidden(request_id: RequestId) -> Response {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "request host or origin is not allowed",
        request_id,
    )
    .into_response()
}

fn is_expected_authority(authority: &Authority, port: u16) -> bool {
    (authority.host().eq_ignore_ascii_case("127.0.0.1")
        || authority.host().eq_ignore_ascii_case("localhost"))
        && authority.port_u16() == Some(port)
}

fn same_authority(host: &Authority, origin: &Authority, port: u16) -> bool {
    is_expected_authority(origin, port)
        && host.host().eq_ignore_ascii_case(origin.host())
        && host.port_u16() == origin.port_u16()
}

fn valid_token(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(AUTH_TOKEN_HEADER)
        .is_some_and(|provided| bool::from(provided.as_bytes().ct_eq(expected.as_bytes())))
}

#[derive(Clone, Copy)]
enum RoutePolicy {
    Static,
    Browser,
    WebSocket,
    Authenticated,
}

fn route_policy(path: &str) -> RoutePolicy {
    if matches!(path, "/" | "/admin" | "/config")
        || path.starts_with("/assets/")
        || is_client_route(path)
    {
        RoutePolicy::Static
    } else if path == "/ws/admin" {
        RoutePolicy::WebSocket
    } else if path.starts_with("/api/") && !path.starts_with("/api/mcp/") {
        RoutePolicy::Browser
    } else {
        RoutePolicy::Authenticated
    }
}

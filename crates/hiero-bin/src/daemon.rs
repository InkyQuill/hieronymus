use std::{
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
use hiero_core::provider::ProviderTransport;
use hiero_core::{config::HieronymusConfig, db};
use sqlx::SqlitePool;
use subtle::ConstantTimeEq;
use tokio::{
    sync::{Notify, broadcast, mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::api::{
    self,
    error::ApiError,
    events::{AdminEvent, admin_event_channel, admin_ws},
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
    pub workers: WorkerSupervisor,
    pub events: broadcast::Sender<AdminEvent>,
    pub assets: AssetSource,
}

type WorkerFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

enum WorkerCommand {
    Spawn(WorkerFuture),
    Shutdown(oneshot::Sender<Result<()>>),
}

struct WorkerSupervisorInner {
    commands: mpsc::Sender<WorkerCommand>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
    closed: AtomicBool,
    reaper: StdMutex<Option<JoinHandle<()>>>,
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
        let reaper = tokio::spawn(run_worker_supervisor(
            receiver,
            active.clone(),
            idle.clone(),
        ));
        Self {
            inner: Arc::new(WorkerSupervisorInner {
                commands,
                active,
                idle,
                closed: AtomicBool::new(false),
                reaper: StdMutex::new(Some(reaper)),
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
        if self.inner.closed.load(Ordering::Acquire) {
            return false;
        }
        self.inner
            .commands
            .send(WorkerCommand::Spawn(Box::pin(task)))
            .await
            .is_ok()
    }

    pub async fn shutdown(&self) -> Result<()> {
        match tokio::time::timeout(
            WORKER_SHUTDOWN_GRACE + WORKER_ABORT_GRACE,
            self.shutdown_inner(),
        )
        .await
        {
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
        self.wait_idle().await;
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
) {
    let mut tasks = JoinSet::new();
    let mut failures = Vec::new();
    loop {
        if tasks.is_empty() {
            match commands.recv().await {
                Some(WorkerCommand::Spawn(task)) => spawn_worker(&mut tasks, &active, task),
                Some(WorkerCommand::Shutdown(completed)) => {
                    finish_worker_shutdown(
                        &mut commands,
                        &mut tasks,
                        &active,
                        &idle,
                        &mut failures,
                    )
                    .await;
                    let _ = completed.send(worker_failures(failures));
                    return;
                }
                None => {
                    abort_workers(&mut tasks, &active, &idle);
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
                        Some(WorkerCommand::Spawn(task)) => {
                            spawn_worker(&mut tasks, &active, task);
                        }
                        Some(WorkerCommand::Shutdown(completed)) => {
                            finish_worker_shutdown(
                                &mut commands,
                                &mut tasks,
                                &active,
                                &idle,
                                &mut failures,
                            ).await;
                            let _ = completed.send(worker_failures(failures));
                            return;
                        }
                        None => {
                            abort_workers(&mut tasks, &active, &idle);
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
                joined = tasks.join_next() => {
                    if let Some(joined) = joined {
                        record_worker_result(joined, &mut failures);
                        record_worker_completion(&active, &idle);
                    }
                }
            }
        }
    }
}

fn spawn_worker(tasks: &mut JoinSet<Result<()>>, active: &AtomicUsize, task: WorkerFuture) {
    active.fetch_add(1, Ordering::AcqRel);
    tasks.spawn(task);
}

async fn finish_worker_shutdown(
    commands: &mut mpsc::Receiver<WorkerCommand>,
    tasks: &mut JoinSet<Result<()>>,
    active: &AtomicUsize,
    idle: &Notify,
    failures: &mut Vec<String>,
) {
    commands.close();
    while let Some(command) = commands.recv().await {
        if let WorkerCommand::Spawn(task) = command {
            spawn_worker(tasks, active, task);
        }
    }
    let graceful = async {
        while let Some(joined) = tasks.join_next().await {
            record_worker_result(joined, failures);
            record_worker_completion(active, idle);
        }
    };
    if tokio::time::timeout(WORKER_SHUTDOWN_GRACE, graceful)
        .await
        .is_err()
    {
        abort_workers(tasks, active, idle);
    }
}

fn abort_workers(tasks: &mut JoinSet<Result<()>>, active: &AtomicUsize, idle: &Notify) {
    let remaining = tasks.len();
    tasks.abort_all();
    if remaining > 0 {
        active.fetch_sub(remaining, Ordering::AcqRel);
        idle.notify_waiters();
        tracing::warn!(
            remaining,
            "workers exceeded the graceful shutdown window and were cancelled"
        );
    }
}

fn record_worker_completion(active: &AtomicUsize, idle: &Notify) {
    if active.fetch_sub(1, Ordering::AcqRel) == 1 {
        idle.notify_waiters();
    }
}

fn record_worker_result(
    joined: std::result::Result<Result<()>, tokio::task::JoinError>,
    failures: &mut Vec<String>,
) {
    match joined {
        Ok(Ok(())) => {}
        Ok(Err(error)) => retain_worker_failure(failures, error.to_string()),
        Err(error) if error.is_cancelled() => {}
        Err(error) => retain_worker_failure(failures, format!("worker task failed: {error}")),
    }
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

#[derive(Clone, Debug)]
pub struct RequestId(pub(crate) String);

pub fn build_router(state: AppState) -> Router {
    let security_state = state.clone();
    let dream_runner = Arc::new(StoreDreamRunner::new(
        state.pool.clone(),
        state.config.clone(),
    ));
    let mcp = http::service(Arc::new(
        StoreMcpBackend::new(state.pool.clone(), state.config.clone())
            .with_dream_runner(dream_runner),
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
    let router = build_router(AppState {
        pool,
        config: Arc::new(config),
        shutdown: shutdown_sender.clone(),
        auth_token,
        port,
        dream_running: Arc::new(AtomicBool::new(false)),
        provider_transport: None,
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
        }
    };
    let worker_result = workers.shutdown().await;
    match (result, worker_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(server), Err(worker)) => {
            Err(server.context(format!("worker shutdown also failed: {worker:#}")))
        }
    }
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

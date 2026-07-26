#![cfg(unix)]

use std::{process::Stdio, time::Duration};

use assert_cmd::cargo::cargo_bin;
use hiero_core::{
    config::HieronymusConfig,
    db,
    domain::{AddMemoryInput, TranslationContext, WorkspaceStore},
    dreaming::{DreamConfig, PhaseProfile, acquire_dream_cycle_lock},
    provider::{PassName, ProviderCatalog, ProviderProfile},
    registry::SeriesRegistry,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

async fn random_port() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    listener.local_addr().unwrap().port()
}

fn daemon_command(root: &TempDir, port: u16, explicit_start: bool) -> Command {
    let mut command = Command::new(cargo_bin!("hiero"));
    command
        .arg("--data-root")
        .arg(root.path().join("data"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("HIERONYMUS_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if explicit_start {
        command.arg("start").arg("--port").arg(port.to_string());
    }
    command
}

fn daemon_config(root: &TempDir) -> HieronymusConfig {
    HieronymusConfig::with_roots(
        root.path().join("data"),
        root.path().join("config").join("hieronymus"),
    )
}

async fn configure_pending_dream(root: &TempDir, provider_url: Option<&str>) -> HieronymusConfig {
    let config = daemon_config(root);
    config.ensure_directories().unwrap();
    let pool = db::connect(&config).await.unwrap();
    SeriesRegistry::new(&pool)
        .create("book", "Book", "en", "ru")
        .await
        .unwrap();
    let workspace = WorkspaceStore::new(&pool);
    let session = workspace
        .start_session(
            &TranslationContext::new("book", "en", "ru"),
            "translation",
            "1",
            "1",
        )
        .await
        .unwrap();
    workspace
        .add_short_term(
            session.id,
            AddMemoryInput {
                text: "The daemon must release its real dream lock.".into(),
                ..AddMemoryInput::default()
            },
        )
        .await
        .unwrap();
    workspace.complete_session(session.id).await.unwrap();
    pool.close().await;

    let mut dream = DreamConfig {
        enabled: true,
        schedule_interval_minutes: 1,
        min_pending_short_term_memories: 1,
        ..DreamConfig::default()
    };
    dream = dream.with_phase(
        PassName::KnowledgeCrystals,
        PhaseProfile {
            provider: "process-test".into(),
            model: "test-model".into(),
            enabled: true,
            max_records_per_pass: 10,
        },
    );
    dream.save(&config).unwrap();
    if let Some(provider_url) = provider_url {
        let mut catalog = ProviderCatalog::default();
        catalog
            .upsert(
                ProviderProfile::new("process-test", "Process Test", "openai", provider_url)
                    .with_inline_credential("process-test-key")
                    .with_timeout(Duration::from_secs(30)),
            )
            .unwrap();
        catalog.save(config.provider_config_path()).unwrap();
    }
    config
}

async fn spawn_daemon(root: &TempDir, port: u16) -> Child {
    let child = daemon_command(root, port, true).spawn().unwrap();
    wait_daemon_ready(root, port).await;
    child
}

async fn wait_daemon_ready(root: &TempDir, port: u16) {
    let token_path = root
        .path()
        .join("config")
        .join("hieronymus")
        .join("auth-token");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let (Ok(token), Ok(mut stream)) = (
                std::fs::read_to_string(&token_path),
                tokio::net::TcpStream::connect(("127.0.0.1", port)).await,
            ) {
                let request = format!(
                    "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                     X-Hieronymus-Token: {}\r\nConnection: close\r\n\r\n",
                    token.trim()
                );
                let mut response = [0_u8; 1_024];
                if stream.write_all(request.as_bytes()).await.is_ok()
                    && stream
                        .read(&mut response)
                        .await
                        .is_ok_and(|read| response[..read].starts_with(b"HTTP/1.1 200"))
                {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("daemon should start");
}

async fn assert_signal_stops_daemon(signal: &str) {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let mut child = spawn_daemon(&root, port).await;
    let pid = child.id().unwrap();
    let status = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let exit = tokio::time::timeout(Duration::from_secs(8), child.wait())
        .await
        .expect("signal should stop the daemon")
        .unwrap();
    assert!(exit.success(), "daemon exit: {exit}");
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ctrl_c_stops_listener_and_workers_cleanly() {
    assert_signal_stops_daemon("-INT").await;
}

#[tokio::test]
async fn sigterm_stops_listener_and_workers_cleanly() {
    assert_signal_stops_daemon("-TERM").await;
}

#[tokio::test]
async fn no_subcommand_prints_status_when_daemon_is_already_running() {
    let root = TempDir::new().unwrap();
    let port = random_port().await;
    let mut daemon = spawn_daemon(&root, port).await;
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        daemon_command(&root, port, false).output(),
    )
    .await
    .expect("no-subcommand status should not block")
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Hieronymus daemon is running\n"
    );
    assert!(daemon.try_wait().unwrap().is_none());
    Command::new("kill")
        .arg("-TERM")
        .arg(daemon.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    daemon.wait().await.unwrap();
}

#[tokio::test]
async fn fatal_background_dream_failure_stops_the_daemon_immediately() {
    let root = TempDir::new().unwrap();
    configure_pending_dream(&root, None).await;
    let port = random_port().await;
    let child = daemon_command(&root, port, true).spawn().unwrap();

    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("fatal recurring failure should stop the daemon")
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("fatal recurring worker failure"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn disabled_background_dreaming_does_not_load_provider_catalog() {
    let root = TempDir::new().unwrap();
    let config = daemon_config(&root);
    config.ensure_directories().unwrap();
    std::fs::write(config.provider_config_path(), "not valid = [toml").unwrap();
    let port = random_port().await;
    let mut daemon = spawn_daemon(&root, port).await;

    Command::new("kill")
        .arg("-TERM")
        .arg(daemon.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    assert!(daemon.wait().await.unwrap().success());
}

#[tokio::test]
async fn sigterm_releases_an_active_production_dream_lock_before_exit() {
    let provider = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let provider_url = format!("http://{}/v1", provider.local_addr().unwrap());
    let root = TempDir::new().unwrap();
    let config = configure_pending_dream(&root, Some(&provider_url)).await;
    let (request_started, request_observed) = tokio::sync::oneshot::channel();
    let provider_task = tokio::spawn(async move {
        let (mut stream, _) = provider.accept().await.unwrap();
        let mut request = [0_u8; 4_096];
        let read = stream.read(&mut request).await.unwrap();
        assert!(read > 0);
        let _ = request_started.send(());
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
    });
    let port = random_port().await;
    let mut daemon = daemon_command(&root, port, true).spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(5), request_observed)
        .await
        .expect("production dream provider request should start")
        .unwrap();
    wait_daemon_ready(&root, port).await;
    let lock_error = acquire_dream_cycle_lock(&config, "process-test-probe", false)
        .expect_err("daemon should own the real dream lock");
    assert!(lock_error.is_already_running());

    Command::new("kill")
        .arg("-TERM")
        .arg(daemon.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(8), daemon.wait())
        .await
        .expect("daemon should finish dream cancellation cleanup")
        .unwrap();
    assert!(exit.success(), "daemon exit: {exit}");
    tokio::time::timeout(Duration::from_secs(1), provider_task)
        .await
        .expect("provider connection should close before daemon exit completes")
        .unwrap();

    let cleanup_lock = acquire_dream_cycle_lock(&config, "process-test-after-exit", false)
        .expect("daemon must release the dream lock before exit");
    drop(cleanup_lock);
    let pool = db::connect(&config).await.unwrap();
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status = 'running'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(running, 0);
}

#[tokio::test]
async fn sigterm_bounds_persistent_dream_cleanup_failure_without_orphaning_lock() {
    let provider = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let provider_url = format!("http://{}/v1", provider.local_addr().unwrap());
    let root = TempDir::new().unwrap();
    let config = configure_pending_dream(&root, Some(&provider_url)).await;
    let pool = db::connect(&config).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER injected_persistent_cleanup_failure
         BEFORE UPDATE OF status ON dream_runs
         WHEN OLD.status='running' AND NEW.status='failed'
         BEGIN SELECT RAISE(ABORT,'injected persistent cleanup failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let (request_started, request_observed) = tokio::sync::oneshot::channel();
    let provider_task = tokio::spawn(async move {
        let (mut stream, _) = provider.accept().await.unwrap();
        let mut request = [0_u8; 4_096];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        let _ = request_started.send(());
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
    });
    let port = random_port().await;
    let daemon = daemon_command(&root, port, true).spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(5), request_observed)
        .await
        .expect("production dream provider request should start")
        .unwrap();
    wait_daemon_ready(&root, port).await;
    let pid = daemon.id().unwrap();
    Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .unwrap();

    let output = tokio::time::timeout(Duration::from_secs(5), daemon.wait_with_output())
        .await
        .expect("persistent cleanup failure must not prevent process exit")
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("audit cleanup"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    tokio::time::timeout(Duration::from_secs(1), provider_task)
        .await
        .expect("provider connection must not outlive daemon shutdown")
        .unwrap();
    let lock = acquire_dream_cycle_lock(&config, "after-persistent-cleanup", false)
        .expect("persistent cleanup failure must still release the OS lock");
    drop(lock);
    let pool = db::connect(&config).await.unwrap();
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status = 'running'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(running, 1);
}

#[tokio::test]
async fn sigterm_bounds_manual_admin_dream_cleanup_failure_without_orphaning_lock() {
    let provider = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let provider_url = format!("http://{}/v1", provider.local_addr().unwrap());
    let root = TempDir::new().unwrap();
    let config = configure_pending_dream(&root, Some(&provider_url)).await;
    let mut dream = DreamConfig::load(&config).unwrap();
    dream.enabled = false;
    dream.save(&config).unwrap();
    let pool = db::connect(&config).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER injected_manual_cleanup_failure
         BEFORE UPDATE OF status ON dream_runs
         WHEN OLD.status='running' AND NEW.status='failed'
         BEGIN SELECT RAISE(ABORT,'injected manual cleanup failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let (request_started, request_observed) = tokio::sync::oneshot::channel();
    let provider_task = tokio::spawn(async move {
        let (mut stream, _) = provider.accept().await.unwrap();
        let mut request = [0_u8; 4_096];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        let _ = request_started.send(());
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
    });
    let port = random_port().await;
    let daemon = daemon_command(&root, port, true).spawn().unwrap();
    wait_daemon_ready(&root, port).await;
    let token = std::fs::read_to_string(config.auth_token_path()).unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let body = "{}";
    client
        .write_all(
            format!(
                "POST /api/admin/actions/run_manual_dreaming HTTP/1.1\r\n\
                 Host: 127.0.0.1:{port}\r\n\
                 X-Hieronymus-Token: {}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                token.trim(),
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    tokio::time::timeout(Duration::from_secs(5), request_observed)
        .await
        .expect("manual admin dream provider request should start")
        .unwrap();

    Command::new("kill")
        .arg("-TERM")
        .arg(daemon.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(5), daemon.wait_with_output())
        .await
        .expect("manual cleanup failure must not prevent process exit")
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    tokio::time::timeout(Duration::from_secs(1), provider_task)
        .await
        .expect("provider connection must not outlive daemon shutdown")
        .unwrap();
    let lock = acquire_dream_cycle_lock(&config, "after-manual-cleanup", false)
        .expect("manual bounded cleanup must release the OS lock");
    drop(lock);
    let pool = db::connect(&config).await.unwrap();
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status = 'running'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(running, 1);
}

//! Lifecycle contract tests for `hiero daemon`: the non-secret discovery
//! record, the separately stored 0600 bearer token, occupied-port errors, and
//! fail-closed database classification per ADR 0009/0012 (as amended).

mod common;

use common::{send_request, start_daemon_on_ephemeral_port};
use hiero::daemon::{Daemon, DaemonError, DaemonOptions};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

fn write_python_sentinel_database(root: &Path) {
    let connection = rusqlite::Connection::open(root.join("hieronymus.sqlite")).unwrap();
    for table in [
        "series",
        "task_sessions",
        "short_term_memories",
        "strict_terms",
    ] {
        connection
            .execute(
                &format!("create table {table} (id integer primary key)"),
                [],
            )
            .unwrap();
    }
}

#[test]
fn discovery_record_is_atomic_non_secret_and_token_is_separate_with_0600() {
    let (root, daemon) = start_daemon_on_ephemeral_port();

    let discovery_path = root.path().join("daemon.json");
    let token_path = root.path().join("daemon.token");
    assert!(discovery_path.exists(), "discovery record must exist");
    assert!(token_path.exists(), "token file must exist");

    let discovery_text = std::fs::read_to_string(&discovery_path).unwrap();
    let record: Value = serde_json::from_str(&discovery_text).unwrap();
    assert_eq!(record["discovery_version"], json!(1));
    assert_eq!(record["protocol_version"], json!("2026-07-28"));
    assert_eq!(record["host"], json!("127.0.0.1"));
    assert_eq!(record["port"], json!(daemon.local_addr().port()));
    let pid = record["pid"].as_u64().unwrap();
    assert!(pid > 0);
    let instance_id = record["instance_id"].as_str().unwrap();
    assert_eq!(instance_id.len(), 32, "instance id is 16 random bytes hex");
    assert!(
        instance_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "instance id must be lowercase hex"
    );
    let started_at = record["started_at"].as_str().unwrap();
    chrono::DateTime::parse_from_rfc3339(started_at)
        .unwrap_or_else(|error| panic!("started_at must be RFC 3339: {error}"));

    let token = std::fs::read_to_string(&token_path).unwrap();
    assert!(!token.is_empty());
    assert!(
        !discovery_text.contains(token.trim()),
        "the discovery record must never contain the bearer token"
    );

    let mode = std::fs::metadata(&token_path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "the bearer token file must be user-only"
    );

    // A fresh data root gets its database created at the supported Rust
    // schema; classification must agree.
    let state = hieronymus::db::classify_database(&root.path().join("hieronymus.sqlite"));
    assert!(matches!(
        state,
        hieronymus::db::DatabaseState::RustSchema { .. }
    ));
}

#[test]
fn occupied_port_is_an_error_without_discovery_or_scan() {
    let holder = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = holder.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();

    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("cannot bind") && message.contains(&port.to_string()),
        "unexpected bind diagnostic: {message}"
    );
    assert!(
        !root.path().join("daemon.json").exists(),
        "a failed start must not publish discovery"
    );
    drop(holder);
}

#[test]
fn corrupt_database_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("hieronymus.sqlite"),
        b"definitely not sqlite",
    )
    .unwrap();

    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();

    assert!(error.to_string().contains("'corrupt'"), "{error}");
    assert!(!root.path().join("daemon.json").exists());
}

#[test]
fn python_schema_database_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    write_python_sentinel_database(root.path());

    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();

    assert!(
        error.to_string().contains("'python-schema'"),
        "unexpected diagnostic: {error}"
    );
    assert!(!root.path().join("daemon.json").exists());
}

#[test]
fn shutdown_route_authenticates_then_stops_and_cleans_discovery() {
    let (root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();

    // Frozen failure cases first: host and bearer are checked in order.
    let invalid_host = send_request(
        port,
        "POST",
        "/shutdown",
        &[
            ("Host".to_string(), "attacker.invalid".to_string()),
            (
                "Authorization".to_string(),
                format!("Bearer {}", daemon.bearer().expose_secret()),
            ),
        ],
        b"",
    );
    assert_eq!(invalid_host.status, 400);
    assert_eq!(invalid_host.body(), json!({"error": "invalid_host"}));

    let missing_bearer = send_request(port, "POST", "/shutdown", &[], b"");
    assert_eq!(missing_bearer.status, 401);
    assert_eq!(missing_bearer.body(), json!({"error": "unauthorized"}));

    // Wrong bearer must not stop the daemon either.
    let wrong = send_request(
        port,
        "POST",
        "/shutdown",
        &[(
            "Authorization".to_string(),
            "Bearer wrong-token".to_string(),
        )],
        b"",
    );
    assert_eq!(wrong.status, 401);

    // The authenticated success case stops the daemon.
    let success_headers = vec![(
        "Authorization".to_string(),
        format!("Bearer {}", daemon.bearer().expose_secret()),
    )];
    let success = send_request(port, "POST", "/shutdown", &success_headers, b"");
    assert_eq!(success.status, 200);
    assert_eq!(success.body(), json!({"ok": true, "stopping": true}));

    daemon.wait_for_shutdown().unwrap();
    assert!(
        !root.path().join("daemon.json").exists(),
        "graceful shutdown removes the matching discovery record"
    );
    assert!(
        root.path().join("daemon.token").exists(),
        "the per-installation token survives daemon shutdown"
    );
}

#[test]
fn daemon_binary_publishes_discovery_and_shuts_down_gracefully() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let discovery_path = root.path().join("daemon.json");
    let mut record = None;
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&discovery_path) {
            record = Some(serde_json::from_str::<Value>(&text).unwrap());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let record = record.expect("daemon binary must publish its discovery record");
    let port = record["port"].as_u64().unwrap() as u16;
    let health = send_request(port, "GET", "/health", &[], b"");
    assert_eq!(health.status, 200);
    assert_eq!(health.body(), json!({"ok": true}));

    // Wait for the ready line: it is printed only after the SIGINT handler is
    // installed, so an interrupt now is handled gracefully.
    let mut stdout = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut stdout,
    )
    .expect("daemon must print its ready line");
    assert!(
        stdout.contains("listening"),
        "unexpected ready line: {stdout}"
    );

    let pid = record["pid"].as_u64().unwrap().to_string();
    let signaled = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("kill must be available to send SIGINT");
    assert!(signaled.success());

    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "graceful shutdown must exit successfully"
    );
    assert!(
        !discovery_path.exists(),
        "graceful shutdown removes the matching discovery record"
    );
}

#[test]
fn unknown_daemon_database_state_prevents_ready_flag() {
    // Unknown table set: a database with tables that match no known schema.
    let root = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(root.path().join("hieronymus.sqlite")).unwrap();
    connection
        .execute("create table mystery_table (id integer primary key)", [])
        .unwrap();
    drop(connection);

    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();

    assert!(
        matches!(error, DaemonError::UnsupportedDatabase { .. }),
        "unexpected error: {error}"
    );
    assert!(!root.path().join("daemon.json").exists());
}

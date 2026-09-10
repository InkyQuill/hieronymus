//! Lifecycle contract tests for `hiero daemon`: the non-secret discovery
//! record, the separately stored 0600 bearer token, occupied-port errors, and
//! fail-closed database classification per ADR 0009/0012 (as amended).

mod common;

use common::{send_request, start_daemon_on_ephemeral_port};
use hiero::daemon::{Daemon, DaemonError, DaemonOptions};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::ownership::RootOwnership;
use serde_json::{Value, json};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

    match error {
        DaemonError::InvalidStartupState { code, .. } => assert_eq!(code, "invalid_startup_state"),
        other => panic!("unexpected error: {other}"),
    }
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

    match error {
        DaemonError::InvalidStartupState {
            code, remediation, ..
        } => {
            assert_eq!(code, "migration_required");
            assert_eq!(remediation, "hiero migrate");
        }
        other => panic!("unexpected error: {other}"),
    }
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

    match error {
        DaemonError::InvalidStartupState { code, .. } => assert_eq!(code, "invalid_startup_state"),
        other => panic!("unexpected error: {other}"),
    }
    assert!(!root.path().join("daemon.json").exists());
}

// ---------------------------------------------------------------------------
// Cutover journal gate (database-upgrade design: the daemon starts only when
// the journal is absent or complete)
// ---------------------------------------------------------------------------

#[test]
fn unfinished_cutover_journal_fails_closed_even_on_a_supported_database() {
    // A supported Rust database plus an unfinished cutover journal: the
    // database alone would start, but the middle state must not.
    // `prepared` is an unfinished upgrade (fail closed); a committed database
    // awaiting config promotion is the explicit `config_promotion_required`
    // diagnostic (ADR 0009).
    for (journal_state, expected_code) in [
        ("prepared", "upgrade_incomplete"),
        ("database_committed", "config_promotion_required"),
        ("config_promotion_required", "config_promotion_required"),
    ] {
        let root = tempfile::tempdir().unwrap();
        hieronymus::db::open_migrated(&root.path().join("hieronymus.sqlite")).unwrap();
        let journal = hieronymus::upgrade::CutoverJournal {
            journal_version: 1,
            state: journal_state.to_string(),
            source_state: "python-schema".to_string(),
            target_schema_version: hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION,
            staging_dir: ".migrate-staging".to_string(),
            backup_dir: "backups/pre-upgrade-test".to_string(),
            staged: Vec::new(),
            backup_database_sha256: "0".repeat(64),
            backup_configs: std::collections::BTreeMap::new(),
            updated_at: "2026-09-04T00:00:00+00:00".to_string(),
        };
        std::fs::write(
            root.path().join("cutover.json"),
            serde_json::to_string_pretty(&journal).unwrap(),
        )
        .unwrap();

        let error = Daemon::start(&DaemonOptions {
            data_root: Some(root.path().to_path_buf()),
            port: 0,
            assets: hiero::daemon::Assets::default(),
        })
        .unwrap_err();

        match error {
            DaemonError::InvalidStartupState {
                code, remediation, ..
            } => {
                assert_eq!(code, expected_code, "journal state {journal_state}");
                assert_eq!(remediation, "hiero migrate");
            }
            other => panic!("state {journal_state}: unexpected error: {other}"),
        }
        assert!(
            !root.path().join("daemon.json").exists(),
            "a refused start must not publish discovery"
        );
    }
}

#[test]
fn complete_cutover_journal_allows_daemon_start() {
    let root = tempfile::tempdir().unwrap();
    hieronymus::db::open_migrated(&root.path().join("hieronymus.sqlite")).unwrap();
    let journal = hieronymus::upgrade::CutoverJournal {
        journal_version: 1,
        state: "complete".to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-test".to_string(),
        staged: Vec::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: std::collections::BTreeMap::new(),
        updated_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        root.path().join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();

    let daemon = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_or_else(|error| panic!("complete journal must not block: {error}"));
    daemon.shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// Data-root ownership (ADR 0009 / Astra finding 9): one owner across the
// daemon, `hiero migrate`, and `hiero recover`.
// ---------------------------------------------------------------------------

#[test]
fn a_second_daemon_on_one_root_refuses_without_disturbing_the_first() {
    // Astra finding 9: the reviewer started two live daemons on one root with
    // different ephemeral ports; the second overwrote the token/discovery and
    // the first kept running. The ownership guard must make the second start
    // fail before it writes anything.
    let (root, first) = start_daemon_on_ephemeral_port();
    let first_port = first.local_addr().port();
    let token_before = std::fs::read_to_string(root.path().join("daemon.token")).unwrap();
    let discovery_before = std::fs::read_to_string(root.path().join("daemon.json")).unwrap();

    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();
    match error {
        DaemonError::OwnershipHeld { message } => {
            assert!(
                message.contains("daemon"),
                "diagnostic names the owner: {message}"
            );
        }
        other => panic!("expected an ownership refusal, got: {other}"),
    }

    // The second start touched neither the token nor the discovery record.
    assert_eq!(
        std::fs::read_to_string(root.path().join("daemon.token")).unwrap(),
        token_before
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("daemon.json")).unwrap(),
        discovery_before
    );

    // The first daemon is still serving on its original port.
    let health = send_request(first_port, "GET", "/health", &[], b"");
    assert_eq!(health.status, 200);
    assert_eq!(health.body(), json!({"ok": true}));

    first.shutdown().unwrap();
}

#[test]
fn a_graceful_stop_releases_ownership_for_the_next_daemon() {
    let (root, first) = start_daemon_on_ephemeral_port();
    first.shutdown().unwrap();

    // Ownership was released after discovery removal, so a fresh daemon on
    // the same root starts cleanly.
    let second = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_or_else(|error| panic!("ownership must be free after a graceful stop: {error}"));
    second.shutdown().unwrap();
}

#[test]
fn a_graceful_stop_releases_ownership_for_offline_maintenance() {
    // The maintenance direction: after a graceful stop `hiero migrate` and
    // `hiero recover` can take the root. Both get past step 1b (ownership
    // acquisition); they may then refuse for an unrelated reason (nothing to
    // do / no backup), which is fine — what matters is that neither fails
    // with `RootOwnership`.
    let (root, daemon) = start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());
    daemon.shutdown().unwrap();

    let migrate = hieronymus::upgrade::run_upgrade(
        &config,
        false,
        &hieronymus::upgrade::UpgradeOptions::default(),
    );
    assert!(
        !matches!(
            migrate,
            Err(hieronymus::migrate::MigrateError::RootOwnership(_))
        ),
        "migrate must be able to take the root after a stop: {migrate:?}"
    );

    let recover = hieronymus::upgrade::run_recovery(&config, false);
    assert!(
        !matches!(
            recover,
            Err(hieronymus::migrate::MigrateError::RootOwnership(_))
        ),
        "recover must be able to take the root after a stop: {recover:?}"
    );
}

#[test]
fn migrate_and_daemon_start_are_mutually_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());

    // A migration run that has taken ownership (here: the guard directly,
    // standing in for `run_upgrade`'s step 1b, which happens before the
    // journal is ever created) blocks the daemon.
    let migrate_guard = RootOwnership::acquire(&config, "migrate").unwrap();
    let error = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap_err();
    assert!(
        matches!(error, DaemonError::OwnershipHeld { .. }),
        "daemon must refuse while migrate owns the root: {error}"
    );
    assert!(!root.path().join("daemon.json").exists());
    drop(migrate_guard);

    // With the daemon running, a migration attempt refuses.
    let daemon = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap();
    let error = hieronymus::upgrade::run_upgrade(
        &config,
        false,
        &hieronymus::upgrade::UpgradeOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, hieronymus::migrate::MigrateError::RootOwnership(_)),
        "migrate must refuse while the daemon owns the root: {error}"
    );
    daemon.shutdown().unwrap();
}

#[test]
fn recover_refuses_while_the_daemon_owns_the_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let daemon = Daemon::start(&DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap();

    let error = hieronymus::upgrade::run_recovery(&config, false).unwrap_err();
    assert!(
        matches!(error, hieronymus::migrate::MigrateError::RootOwnership(_)),
        "recover must refuse while the daemon owns the root: {error}"
    );
    daemon.shutdown().unwrap();
}

#[test]
fn doctor_on_an_owned_root_stays_lock_free_and_non_mutating() {
    let (root, daemon) = start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());

    // Doctor reads an owned root without acquiring ownership itself and
    // without deadlocking.
    let report = hiero::doctor::run(&config);
    let _ = report.render_human();

    // The daemon still owns the root (nobody released it) and still serves.
    assert!(
        RootOwnership::acquire(&config, "probe").is_err(),
        "doctor must not have taken or released ownership"
    );
    let health = send_request(daemon.local_addr().port(), "GET", "/health", &[], b"");
    assert_eq!(health.status, 200);

    daemon.shutdown().unwrap();
}

#[test]
fn a_sigkilled_daemon_releases_data_root_ownership() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while !discovery_path.exists() {
        assert!(
            Instant::now() < deadline,
            "daemon never published discovery"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // While the daemon lives, the root is owned.
    assert!(RootOwnership::acquire(&config, "probe").is_err());

    let killed = Command::new("kill")
        .args(["-KILL", &child.id().to_string()])
        .status()
        .expect("kill must be available");
    assert!(killed.success());
    child.wait().unwrap();

    // The kernel dropped the daemon's flock; a maintenance run can take the
    // root now.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if RootOwnership::acquire(&config, "recover").is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ownership was not released after the daemon was SIGKILLed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

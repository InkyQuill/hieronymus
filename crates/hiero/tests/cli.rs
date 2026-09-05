//! Integration tests for the `hiero` binary skeleton: version output and the
//! classify command in human and JSON forms.

use std::process::Command;

fn hiero(arguments: &[&str]) -> (String, String, std::process::ExitStatus) {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(arguments)
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status,
    )
}

#[test]
fn version_prints_human_alpha_marker() {
    let (stdout, stderr, status) = hiero(&["version"]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert!(stdout.contains("hiero v"), "{stdout}");
    assert!(stdout.contains('\u{03b1}'), "{stdout}");
}

#[test]
fn version_prints_json_when_requested() {
    let (stdout, _stderr, status) = hiero(&["version", "--json"]);
    assert!(status.success());
    assert!(stdout.starts_with("{\"version\": \""), "{stdout}");
}

#[test]
fn classify_reports_empty_database_as_json() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, _stderr, status) = hiero(&[
        "classify",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("\"state\": \"empty\""), "{stdout}");
    assert!(stdout.contains("hieronymus.sqlite"), "{stdout}");
}

#[test]
fn classify_reports_python_schema_in_human_form() {
    let root = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(root.path().join("hieronymus.sqlite")).unwrap();
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
    drop(connection);

    let (stdout, _stderr, status) =
        hiero(&["classify", "--data-root", root.path().to_str().unwrap()]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("database state: python-schema"), "{stdout}");
}

#[test]
fn unknown_command_exits_two_with_usage_hint() {
    let (_stdout, stderr, status) = hiero(&["make-chaos"]);
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("unknown command: make-chaos"), "{stderr}");
    assert!(stderr.contains("usage: hiero"), "{stderr}");
}

// ---------------------------------------------------------------------------
// migrate subcommand (part 1: read-only preflight and dry-run)
// ---------------------------------------------------------------------------

fn write_legacy_fixture(root: &std::path::Path) {
    let path = root.join("hieronymus.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    let sql: &str = include_str!("../../../crates/hieronymus/migrations/global.sql");
    connection.execute_batch(sql).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
        .execute(
            "insert into series(slug, title, default_source_language,
                                default_target_language, created_at, updated_at)
             values ('demo', 'demo', 'ja', 'en', '2026-01-01T00:00:00+00:00',
                     '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into strict_terms(series_slug, source_language, target_language,
                                      category, source_text, canonical_translation,
                                      status, notes, created_at, updated_at)
             values ('demo', 'ja', 'en', 'name', 'センス', 'sense', 'approved', '',
                     '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
    connection.close().unwrap();
}

#[test]
fn migrate_requires_dry_run_flag() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) =
        hiero(&["migrate", "--data-root", root.path().to_str().unwrap()]);
    assert_eq!(status.code(), Some(2));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("--dry-run"), "{stderr}");
}

#[test]
fn migrate_dry_run_refuses_empty_root_in_json() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = hiero(&[
        "migrate",
        "--dry-run",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains("\"refused\": \"empty-database\""),
        "{stdout}"
    );
    assert!(!stderr.is_empty());
}

#[test]
fn migrate_dry_run_converts_legacy_fixture() {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    let (stdout, _stderr, status) = hiero(&[
        "migrate",
        "--dry-run",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("\"conversion_safe\": true"), "{stdout}");
    assert!(stdout.contains("\"refused\": null"), "{stdout}");
    assert!(stdout.contains("\"converted\": 1"), "{stdout}");

    let (stdout, _stderr, status) = hiero(&[
        "migrate",
        "--dry-run",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("dry-run"), "{stdout}");
    assert!(stdout.contains("python-schema"), "{stdout}");
    assert!(stdout.contains("temp artifacts removed: yes"), "{stdout}");
}

#[test]
fn migrate_dry_run_reports_an_active_daemon() {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    hiero::daemon::discovery::write_discovery(
        &config,
        &hiero::daemon::DiscoveryRecord {
            discovery_version: hiero::daemon::discovery::DISCOVERY_VERSION,
            protocol_version: "2026-07-28".to_string(),
            host: "127.0.0.1".to_string(),
            port,
            pid: std::process::id(),
            instance_id: "ab".repeat(16),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        },
    )
    .unwrap();
    let (stdout, _stderr, status) = hiero(&[
        "migrate",
        "--dry-run",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(status.code(), Some(2), "{stdout}");
    assert!(stdout.contains("\"daemon_active\": true"), "{stdout}");
    assert!(
        stdout.contains("\"refused\": \"daemon-active\""),
        "{stdout}"
    );
}

//! Task 17: read-only database fixture behavior.
//!
//! These tests pin the frozen fixture matrix, the CLI path boundaries, and
//! the neutral probe receipts against Task 3's verified projection. The
//! fixtures must remain byte-identical across every classify/import attempt,
//! and the probe may only write beneath the validated work root.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

#[path = "../src/classify.rs"]
mod classify;
#[path = "../src/probe_import.rs"]
mod probe_import;
#[path = "../src/report.rs"]
mod report;

use classify::{DatabaseContract, classify_read_only};
use probe_import::{ProbeRefusal, probe_import};

/// The frozen fixture matrix: (basename, classification, safe_to_convert).
fn expected_cases() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("minimal-python.sqlite", "supported-python", true),
        ("legacy-python.sqlite", "supported-legacy-python", true),
        ("empty.sqlite", "empty", false),
        ("partial-python.sqlite", "partial-python", false),
        ("corrupt.sqlite", "corrupt", false),
        ("unknown-schema.sqlite", "unknown-schema", false),
    ]
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("manifest dir is three levels below the repo root")
        .to_path_buf()
}

fn fixture_root() -> PathBuf {
    repo_root().join("compatibility/fixtures/database")
}

fn projection_contract() -> PathBuf {
    repo_root().join("qualification/compatibility/legacy-database-import.json")
}

fn fixture(name: &str) -> PathBuf {
    fixture_root().join(name)
}

fn allowed_work_root() -> PathBuf {
    let root = repo_root().join("qualification/.artifacts/work/legacy-database-import");
    std::fs::create_dir_all(&root).expect("create allowed work root");
    root
}

fn work_dir() -> tempfile::TempDir {
    tempfile::tempdir_in(allowed_work_root()).expect("create disposable work root")
}

fn sha256(path: impl AsRef<Path>) -> Result<String> {
    report::sha256_file(path.as_ref())
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("utf-8 repo path")
}

fn run_cli(args: &[&str]) -> std::process::ExitStatus {
    Command::new(env!("CARGO_BIN_EXE_legacy-database-import"))
        .args(args)
        .status()
        .expect("spawn legacy-database-import")
}

fn assert_rejected_source(source_name: &str) {
    let root = fixture_root();
    let contract = projection_contract();
    let status = run_cli(&[
        "classify",
        "--fixture-root",
        path_str(&root),
        "--source-name",
        source_name,
        "--contract",
        path_str(&contract),
    ]);
    assert_eq!(
        status.code(),
        Some(2),
        "source {source_name:?} must be rejected with exit code 2 before SQLite opens"
    );
}

fn assert_rejected_target(target_name: &str) {
    let root = fixture_root();
    let contract = projection_contract();
    let work = work_dir();
    let status = run_cli(&[
        "probe-import",
        "--fixture-root",
        path_str(&root),
        "--source-name",
        "minimal-python.sqlite",
        "--contract",
        path_str(&contract),
        "--work-root",
        path_str(work.path()),
        "--target-name",
        target_name,
    ]);
    assert_eq!(
        status.code(),
        Some(2),
        "target {target_name:?} must be rejected with exit code 2 before SQLite opens"
    );
    let created = std::fs::read_dir(work.path())
        .expect("list work root")
        .count();
    assert_eq!(created, 0, "rejected target must not be created");
}

fn assert_rejected_contract(relative: &str) {
    let root = fixture_root();
    let contract = repo_root().join(relative);
    let status = run_cli(&[
        "classify",
        "--fixture-root",
        path_str(&root),
        "--source-name",
        "minimal-python.sqlite",
        "--contract",
        path_str(&contract),
    ]);
    assert_eq!(
        status.code(),
        Some(2),
        "contract {relative:?} must be rejected with exit code 2 before SQLite opens"
    );
}

fn column_names(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
    let mut rows = stmt.query([])?;
    let mut names = Vec::new();
    while let Some(row) = rows.next()? {
        names.push(row.get::<_, String>(1)?);
    }
    Ok(names)
}

#[test]
fn frozen_fixture_matrix_is_read_only() -> Result<()> {
    for (name, classification, safe) in expected_cases() {
        let before = sha256(fixture(name))?;
        let actual = classify_read_only(&fixture(name), &fixture_root(), &projection_contract())?;
        assert_eq!(
            (actual.name.as_str(), actual.safe_to_convert),
            (classification, safe)
        );
        assert_eq!(sha256(fixture(name))?, before);
    }
    Ok(())
}

#[test]
fn cli_rejects_unowned_source_target_or_contract() {
    assert_rejected_source("../../user.sqlite");
    assert_rejected_target("../../sibling.sqlite");
    assert_rejected_contract("compatibility/snapshots/state.json");
}

#[test]
fn bundled_sqlite_reports_fts5_enabled() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory database");
    let enabled: i64 = conn
        .query_row(
            "select sqlite_compileoption_used('ENABLE_FTS5')",
            [],
            |row| row.get(0),
        )
        .expect("query compile options");
    assert_eq!(enabled, 1, "bundled SQLite must have FTS5 enabled");
}

#[test]
fn probe_domains_cover_every_content_table() -> Result<()> {
    let contract = DatabaseContract::load(&projection_contract())?;
    let mut content: BTreeSet<&str> = contract.tables.keys().map(String::as_str).collect();
    for (name, sql) in &contract.tables {
        if sql.contains("using fts5") {
            content.remove(name.as_str());
            for suffix in ["_config", "_data", "_docsize", "_idx"] {
                content.remove(format!("{name}{suffix}").as_str());
            }
        }
    }
    let domains: BTreeSet<&str> = probe_import::DOMAIN_TABLES.iter().copied().collect();
    assert_eq!(
        content, domains,
        "probe domains must cover every projected content table exactly"
    );
    Ok(())
}

#[test]
fn supported_probe_receipts_are_exact() -> Result<()> {
    let contract = DatabaseContract::load(&projection_contract())?;
    let cases = [
        ("minimal-python.sqlite", "supported-python", 57, 40, 5),
        ("legacy-python.sqlite", "supported-legacy-python", 1, 3, 0),
    ];
    for (name, classification, total_rows, table_count, fts_count) in cases {
        let before_directory = report::hash_directory(&fixture_root())?;
        let before_source = sha256(fixture(name))?;
        let work = work_dir();
        let target = work.path().join("probe-target.sqlite");

        let receipt = probe_import(
            &fixture(name),
            &target,
            &fixture_root(),
            work.path(),
            &contract,
        )?;

        assert_eq!(receipt.source_name, name);
        assert_eq!(receipt.classification, classification);
        assert!(receipt.ok);
        assert!(receipt.safe_to_convert);
        assert!(receipt.source_bytes_identical);
        assert!(receipt.fixture_directory_identical);
        assert_eq!(sha256(fixture(name))?, before_source);
        assert_eq!(report::hash_directory(&fixture_root())?, before_directory);
        assert_eq!(receipt.probe_row_count, total_rows);
        assert_eq!(
            (
                receipt.ledger.read,
                receipt.ledger.skipped,
                receipt.ledger.blocking
            ),
            (total_rows, 0, 0)
        );
        assert_eq!(receipt.rows_per_table.len(), table_count);
        assert_eq!(receipt.fts.len(), fts_count);

        // The work root holds exactly the disposable target.
        let mut siblings: Vec<String> = std::fs::read_dir(work.path())?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        siblings.sort();
        assert_eq!(siblings, vec!["probe-target.sqlite".to_string()]);

        // The neutral target content matches the receipt.
        let conn = rusqlite::Connection::open_with_flags(
            &target,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let probe_rows: i64 =
            conn.query_row("select count(*) from probe_rows", [], |row| row.get(0))?;
        let ledger_rows: i64 =
            conn.query_row("select count(*) from probe_ledger", [], |row| row.get(0))?;
        assert_eq!(probe_rows, total_rows as i64);
        assert_eq!(ledger_rows, total_rows as i64);
        assert_eq!(
            column_names(&conn, "probe_rows")?,
            ["source_table", "source_id", "payload_sha256", "field_count"]
        );
        assert_eq!(
            column_names(&conn, "probe_ledger")?,
            ["source_table", "source_id", "outcome", "reason_code"]
        );

        if name == "minimal-python.sqlite" {
            assert_eq!(
                receipt.row_counts, contract.row_counts,
                "row counts must match the verified projection"
            );
            assert_eq!(
                receipt.representative_row_digests.len(),
                contract.representative_rows.len()
            );
            for (table, result) in &receipt.fts {
                assert_eq!(result.ids, vec![1], "exact FTS ids for {table}");
            }
        }
        if name == "legacy-python.sqlite" {
            assert_eq!(receipt.rows_per_table.get("series"), Some(&1));
            assert!(receipt.representative_row_digests.is_empty());
        }
    }
    Ok(())
}

#[test]
fn legacy_probe_fingerprint_is_deterministic() -> Result<()> {
    let contract = DatabaseContract::load(&projection_contract())?;
    let mut fingerprints = Vec::new();
    for _ in 0..2 {
        let work = work_dir();
        let receipt = probe_import(
            &fixture("legacy-python.sqlite"),
            &work.path().join("probe-target.sqlite"),
            &fixture_root(),
            work.path(),
            &contract,
        )?;
        assert_eq!(receipt.classification, "supported-legacy-python");
        assert!(receipt.safe_to_convert);
        fingerprints.push((receipt.schema_digest, receipt.rows_per_table.clone()));
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    Ok(())
}

#[test]
fn nonconvertible_fixtures_fail_closed_before_target_creation() -> Result<()> {
    let contract = DatabaseContract::load(&projection_contract())?;
    let cases = [
        ("empty.sqlite", "empty"),
        ("partial-python.sqlite", "partial-python"),
        ("corrupt.sqlite", "corrupt"),
        ("unknown-schema.sqlite", "unknown-schema"),
    ];
    for (name, classification) in cases {
        let before_directory = report::hash_directory(&fixture_root())?;
        let before_source = sha256(fixture(name))?;
        let work = work_dir();
        let target = work.path().join("probe-target.sqlite");

        let error = probe_import(
            &fixture(name),
            &target,
            &fixture_root(),
            work.path(),
            &contract,
        )
        .expect_err("non-convertible fixtures must fail closed");
        let refusal = error
            .downcast_ref::<ProbeRefusal>()
            .unwrap_or_else(|| panic!("expected ProbeRefusal for {name}, got {error:#}"));
        assert_eq!(refusal.classification, classification);
        assert!(!refusal.error_code.is_empty());
        assert!(
            !target.exists(),
            "probe must fail before target creation for {name}"
        );
        assert_eq!(
            std::fs::read_dir(work.path())?.count(),
            0,
            "work root must stay empty for {name}"
        );
        assert_eq!(sha256(fixture(name))?, before_source);
        assert_eq!(report::hash_directory(&fixture_root())?, before_directory);
    }
    Ok(())
}

#[test]
fn cli_refuses_nonconvertible_fixture_with_exit_code_one() {
    let root = fixture_root();
    let contract = projection_contract();
    let work = work_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_legacy-database-import"))
        .args([
            "probe-import",
            "--fixture-root",
            path_str(&root),
            "--source-name",
            "corrupt.sqlite",
            "--contract",
            path_str(&contract),
            "--work-root",
            path_str(work.path()),
            "--target-name",
            "probe-target.sqlite",
        ])
        .output()
        .expect("spawn legacy-database-import");
    assert_eq!(
        output.status.code(),
        Some(1),
        "non-convertible fixtures must fail closed with exit code 1"
    );
    let refusal: report::ProbeRefusalReport =
        serde_json::from_slice(&output.stdout).expect("refusal report must be JSON");
    assert_eq!(
        (refusal.classification.as_str(), refusal.safe_to_convert),
        ("corrupt", false)
    );
    assert!(!refusal.error_code.is_empty());
    let created = std::fs::read_dir(work.path())
        .expect("list work root")
        .count();
    assert_eq!(created, 0, "refused probe must not create a target");
}

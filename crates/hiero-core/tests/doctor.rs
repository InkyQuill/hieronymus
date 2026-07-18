use std::fs;

use hiero_core::{
    config::HieronymusConfig,
    doctor::{CheckStatus, DoctorCheck, DoctorReport, run_doctor},
};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

fn check<'a>(report: &'a DoctorReport, name: &str) -> &'a DoctorCheck {
    report
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("missing doctor check: {name}"))
}

#[tokio::test]
async fn doctor_runs_each_foundation_check_independently() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let config = HieronymusConfig::load(Some(temporary.path().join("data")))
        .expect("configuration should resolve");

    let report = run_doctor(&config).await;

    for name in [
        "config:provider.conf",
        "config:dream.conf",
        "config:ingest.conf",
        "config:semantic.conf",
        "database",
        "agent:claude",
        "agent:codex",
        "agent:gemini",
        "agent:opencode",
        "agent:openclaw",
        "bind-port",
        "derived-index",
    ] {
        let _ = check(&report, name);
    }
}

#[tokio::test]
async fn doctor_reports_a_blocked_database_parent_without_stopping_other_checks() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let blocked_data_root = temporary.path().join("blocked-data-root");
    fs::write(&blocked_data_root, "not a directory").expect("blocking file should be written");
    let config = HieronymusConfig::load(Some(blocked_data_root))
        .expect("configuration resolution should not touch the filesystem");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "database").status, CheckStatus::Fail);
    let _ = check(&report, "bind-port");
    let _ = check(&report, "derived-index");
}

#[tokio::test]
async fn doctor_rejects_a_non_sqlite_database_file() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    fs::create_dir(&data_root).expect("data root should be created");
    fs::write(data_root.join("hieronymus.db"), "not a sqlite database")
        .expect("invalid database should be written");
    let config = HieronymusConfig::load(Some(data_root))
        .expect("configuration resolution should not inspect the database");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "database").status, CheckStatus::Fail);
    assert_eq!(
        check(&report, "derived-index").status,
        CheckStatus::Fail,
        "a corrupt authoritative source cannot rebuild derived data"
    );
}

#[tokio::test]
async fn doctor_rejects_an_orphaned_derived_index() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    fs::create_dir_all(data_root.join("lancedb")).expect("derived index should be created");
    let config = HieronymusConfig::load(Some(data_root))
        .expect("configuration resolution should not inspect derived data");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "derived-index").status, CheckStatus::Fail);
}

#[tokio::test]
async fn doctor_only_claims_rebuildability_from_a_verified_sqlite_source() {
    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    fs::create_dir(&data_root).expect("data root should be created");
    let database = data_root.join("hieronymus.db");
    let options = SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(true);
    let connection = SqliteConnection::connect_with(&options)
        .await
        .expect("valid SQLite fixture should be created");
    connection
        .close()
        .await
        .expect("SQLite fixture should close cleanly");
    let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "database").status, CheckStatus::Ok);
    let derived = check(&report, "derived-index");
    assert_eq!(derived.status, CheckStatus::Warn);
    assert!(derived.detail.contains("can be rebuilt"));
}

#[cfg(unix)]
#[tokio::test]
async fn doctor_rejects_a_dangling_database_symlink() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    fs::create_dir(&data_root).expect("data root should be created");
    symlink(
        temporary.path().join("missing.db"),
        data_root.join("hieronymus.db"),
    )
    .expect("dangling database symlink should be created");
    let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");

    let report = run_doctor(&config).await;
    let database = check(&report, "database");

    assert_eq!(database.status, CheckStatus::Fail);
    assert!(database.detail.contains("symlink"));
}

#[cfg(unix)]
#[tokio::test]
async fn doctor_rejects_a_dangling_derived_index_symlink() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    fs::create_dir(&data_root).expect("data root should be created");
    let database = data_root.join("hieronymus.db");
    let options = SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(true);
    SqliteConnection::connect_with(&options)
        .await
        .expect("valid SQLite fixture should be created")
        .close()
        .await
        .expect("SQLite fixture should close cleanly");
    symlink(
        temporary.path().join("missing-index"),
        data_root.join("lancedb"),
    )
    .expect("dangling index symlink should be created");
    let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");

    let report = run_doctor(&config).await;
    let derived = check(&report, "derived-index");

    assert_eq!(derived.status, CheckStatus::Fail);
    assert!(derived.detail.contains("symlink"));
}

#[cfg(unix)]
#[tokio::test]
async fn doctor_rejects_an_obstructing_symlink_component() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let obstruction = temporary.path().join("not-a-directory");
    fs::write(&obstruction, "file").expect("obstruction fixture should be written");
    let symlink_component = temporary.path().join("data-link");
    symlink(&obstruction, &symlink_component).expect("obstructing symlink should be created");
    let config = HieronymusConfig::load(Some(symlink_component.join("child")))
        .expect("configuration should resolve without traversing data paths");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "database").status, CheckStatus::Fail);
    assert!(check(&report, "database").detail.contains("symlink"));
    assert_eq!(check(&report, "derived-index").status, CheckStatus::Fail);
    assert!(check(&report, "derived-index").detail.contains("symlink"));
}

#[cfg(unix)]
#[tokio::test]
async fn doctor_accepts_valid_database_and_index_symlinks() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary root should be created");
    let data_root = temporary.path().join("data");
    let targets = temporary.path().join("targets");
    fs::create_dir(&data_root).expect("data root should be created");
    fs::create_dir(&targets).expect("target root should be created");
    let database_target = targets.join("authoritative.db");
    let options = SqliteConnectOptions::new()
        .filename(&database_target)
        .create_if_missing(true);
    SqliteConnection::connect_with(&options)
        .await
        .expect("valid SQLite target should be created")
        .close()
        .await
        .expect("SQLite target should close cleanly");
    let index_target = targets.join("lancedb");
    fs::create_dir(&index_target).expect("index target should be created");
    symlink(&database_target, data_root.join("hieronymus.db"))
        .expect("database symlink should be created");
    symlink(&index_target, data_root.join("lancedb")).expect("index symlink should be created");
    let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");

    let report = run_doctor(&config).await;

    assert_eq!(check(&report, "database").status, CheckStatus::Ok);
    assert_eq!(check(&report, "derived-index").status, CheckStatus::Ok);
}

#[test]
fn public_diagnostic_types_are_serializable_data() {
    let report = DoctorReport {
        checks: vec![DoctorCheck {
            name: "example".into(),
            status: CheckStatus::Warn,
            detail: "an observation".into(),
        }],
    };

    let payload = serde_json::to_value(report).expect("report should serialize");
    assert_eq!(payload["checks"][0]["status"], "warn");
    assert_eq!(payload["checks"][0]["detail"], "an observation");
}

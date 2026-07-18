use std::fs;

use hiero_core::{
    config::HieronymusConfig,
    doctor::{CheckStatus, DoctorCheck, DoctorReport, run_doctor},
};

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

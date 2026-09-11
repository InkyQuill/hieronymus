//! `hiero doctor`: non-mutating health checks with human and JSON reports and
//! healthy/degraded/unhealthy exit codes. The suite asserts every check's
//! verdicts and — via a whole-tree content digest — that a doctor run never
//! writes, downloads, or repairs anything.

use std::fs;
use std::process::Command;

use hiero::daemon::discovery::{self, DISCOVERY_VERSION, DiscoveryRecord};
use hiero::daemon::registry::PROTOCOL_REVISION;
use hiero::doctor::{self, Health, Level};
use hieronymus::data_root::HieronymusConfig;

fn run_doctor(config: &HieronymusConfig) -> doctor::DoctorReport {
    doctor::run(config)
}

fn seed_python_schema(config: &HieronymusConfig) {
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
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

fn seed_discovery(config: &HieronymusConfig, port: u16, protocol_version: &str) -> DiscoveryRecord {
    let record = DiscoveryRecord {
        discovery_version: DISCOVERY_VERSION,
        protocol_version: protocol_version.to_string(),
        host: "127.0.0.1".to_string(),
        port,
        pid: std::process::id(),
        instance_id: "ab".repeat(16),
        started_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    discovery::write_discovery(config, &record).unwrap();
    record
}

fn codes(report: &doctor::DoctorReport) -> Vec<String> {
    report
        .findings
        .iter()
        .map(|finding| finding.code.clone())
        .collect()
}

/// Relative path + SHA-256 of every file under `root`, sorted: the no-mutation
/// guarantee's tree digest.
fn tree_digest(root: &std::path::Path) -> String {
    use sha2::Digest;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(
        root: &std::path::Path,
        directory: &std::path::Path,
        entries: &mut Vec<(String, Vec<u8>)>,
    ) {
        for entry in fs::read_dir(directory).expect("readable directory") {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                walk(root, &path, entries);
            } else {
                entries.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    walk(root, root, &mut entries);
    entries.sort();
    let mut digest = sha2::Sha256::new();
    for (path, content) in &entries {
        digest.update(path.as_bytes());
        digest.update([0u8]);
        digest.update(content);
    }
    format!("{:x}", digest.finalize())
}

// ---------------------------------------------------------------------------
// Healthy baseline and per-check verdicts
// ---------------------------------------------------------------------------

#[test]
fn fresh_root_is_healthy() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let report = run_doctor(&config);
    assert_eq!(report.status, Health::Healthy);
    assert_eq!(report.exit_code(), 0);
    let codes = codes(&report);
    assert!(codes.contains(&"database".to_string()), "{codes:?}");
    assert!(codes.contains(&"provider-catalog".to_string()), "{codes:?}");
    assert!(codes.contains(&"semantic-model".to_string()), "{codes:?}");
}

#[test]
fn python_schema_database_is_degraded_with_upgrade_advice() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_python_schema(&config);
    let report = run_doctor(&config);
    assert_eq!(report.status, Health::Degraded);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "database-upgrade-required")
        .expect("upgrade advice");
    assert_eq!(finding.level, Level::Warning);
}

#[test]
fn unreadable_database_is_unhealthy() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    fs::write(config.database_path(), b"this is not a database").unwrap();
    let report = run_doctor(&config);
    assert_eq!(report.status, Health::Unhealthy);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "database-unreadable")
        .expect("unreadable database finding");
    assert_eq!(finding.level, Level::Error);
}

#[test]
fn invalid_provider_conf_is_unhealthy() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    fs::write(config.provider_config_path(), "not valid toml [[[").unwrap();
    let report = run_doctor(&config);
    assert_eq!(report.status, Health::Unhealthy);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "provider-conf-invalid" && finding.level == Level::Error)
    );
}

#[test]
fn invalid_dream_conf_is_degraded() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    fs::write(config.dream_config_path(), "[workflows\nbroken").unwrap();
    let report = run_doctor(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "dream-conf-invalid" && finding.level == Level::Warning),
        "{:?}",
        report.findings
    );
    assert_eq!(report.status, Health::Degraded);
}

#[cfg(unix)]
#[test]
fn loose_token_permissions_are_degraded() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    // Widen the just-written 0600 credential.
    let path = config.daemon_token_path();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o644);
    fs::set_permissions(&path, permissions).unwrap();

    let report = run_doctor(&config);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "token-permissions")
        .expect("token permission finding");
    assert_eq!(finding.level, Level::Warning);
    assert_eq!(report.status, Health::Degraded);
}

#[test]
fn strict_token_permissions_are_healthy() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    let report = run_doctor(&config);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "token-permissions")
        .expect("token permission finding");
    assert_eq!(finding.level, Level::Ok, "{finding:?}");
}

#[test]
fn stale_discovery_record_is_degraded() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    // A live daemon always has its credential next to the record, so the
    // authenticated probe gets as far as the transport. Port 1 on loopback:
    // nothing listens there.
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    seed_discovery(&config, 1, PROTOCOL_REVISION);
    let report = run_doctor(&config);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "daemon-unreachable")
        .expect("reachability finding");
    assert_eq!(finding.level, Level::Warning);
    assert_eq!(report.status, Health::Degraded);
}

#[test]
fn a_discovery_record_without_a_credential_cannot_be_verified() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_discovery(&config, 1, PROTOCOL_REVISION);
    let report = run_doctor(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "daemon-unverifiable" && finding.level == Level::Warning),
        "{:?}",
        report.findings
    );
    assert_eq!(report.status, Health::Degraded);
}

#[test]
fn protocol_version_mismatch_is_degraded() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_discovery(&config, 1, "2025-06-18");
    let report = run_doctor(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "protocol-mismatch" && finding.level == Level::Warning),
        "{:?}",
        report.findings
    );
}

#[test]
fn non_loopback_discovery_is_unhealthy() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let record = DiscoveryRecord {
        discovery_version: DISCOVERY_VERSION,
        protocol_version: PROTOCOL_REVISION.to_string(),
        host: "10.9.9.9".to_string(),
        port: 9768,
        pid: 1,
        instance_id: "ab".repeat(16),
        started_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    discovery::write_discovery(&config, &record).unwrap();
    let report = run_doctor(&config);
    assert!(
        report.findings.iter().any(
            |finding| finding.code == "discovery-not-loopback" && finding.level == Level::Error
        ),
        "{:?}",
        report.findings
    );
    assert_eq!(report.status, Health::Unhealthy);
}

#[test]
fn reachable_daemon_is_healthy() {
    // ADR 0009: reachability is decided by an *authenticated* probe and a
    // process-instance comparison, so this needs a real daemon — a bare
    // listener on the recorded port is no longer evidence of anything.
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        ..Default::default()
    })
    .unwrap();
    let report = run_doctor(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "daemon-reachable" && finding.level == Level::Ok),
        "{:?}",
        report.findings
    );
    assert_eq!(report.status, Health::Healthy);
    daemon.shutdown().unwrap();
}

#[test]
fn a_foreign_listener_on_the_recorded_port_is_not_a_reachable_daemon() {
    // Stale-port reuse: the record survives, an unrelated process now owns
    // the port. A bare TCP connect would call this healthy.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(stream);
        }
    });
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    seed_discovery(&config, port, PROTOCOL_REVISION);
    let report = run_doctor(&config);
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "daemon-reachable"),
        "{:?}",
        report.findings
    );
    assert_ne!(report.status, Health::Healthy);
    // Read-only: doctor never repairs the record it just disproved.
    assert!(config.daemon_discovery_path().exists());
}

#[test]
fn invalid_semantic_model_is_degraded_without_downloading() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let store = hieronymus::semantic_store::SemanticStore::open(&config).unwrap();
    let path = store.model_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"short").unwrap();

    let report = run_doctor(&config);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "semantic-model")
        .expect("semantic model finding");
    assert_eq!(finding.level, Level::Warning);
    assert_eq!(report.status, Health::Degraded);
}

// ---------------------------------------------------------------------------
// No-mutation guarantee
// ---------------------------------------------------------------------------

#[test]
fn doctor_never_mutates_the_data_root() {
    use hieronymus::provider_config::{
        ProviderCatalog, ProviderDefaults, ProviderProfile, save_provider_catalog,
    };

    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_python_schema(&config);
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    seed_discovery(&config, 1, "2025-06-18");
    fs::write(
        config.dream_config_path(),
        "[workflows.knowledge_crystals]\nprovider = \"openai\"\nmodel = \"m\"\nenabled = true\n",
    )
    .unwrap();
    save_provider_catalog(
        &config,
        &ProviderCatalog {
            providers: [(
                "openai".to_string(),
                ProviderProfile::new(
                    "OpenAI",
                    "openai",
                    "https://api.openai.com/v1",
                    "existing-secret",
                    30.0,
                ),
            )]
            .into(),
            defaults: ProviderDefaults::default(),
        },
    )
    .unwrap();

    let before = tree_digest(root.path());
    let report = run_doctor(&config);
    let after = tree_digest(root.path());
    assert_eq!(before, after, "doctor must never write anything");
    // The report is still useful: python schema + protocol mismatch + a
    // closed daemon port all fire on this root.
    assert_eq!(report.status, Health::Degraded);
}

// ---------------------------------------------------------------------------
// Rendering and the binary surface
// ---------------------------------------------------------------------------

#[test]
fn json_report_carries_status_and_findings() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_python_schema(&config);
    let report = run_doctor(&config);
    let json = report.to_json();
    assert_eq!(json["status"], "degraded");
    let findings = json["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    assert!(findings[0].get("level").is_some());
    assert!(findings[0].get("code").is_some());
    assert!(findings[0].get("message").is_some());
}

#[test]
fn human_report_names_every_finding() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_python_schema(&config);
    let report = run_doctor(&config);
    let rendered = report.render_human();
    assert!(rendered.contains("degraded"), "{rendered}");
    assert!(rendered.contains("database-upgrade-required"), "{rendered}");
}

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
fn binary_doctor_exits_zero_on_a_fresh_root() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = hiero(&["doctor", "--data-root", root.path().to_str().unwrap()]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("healthy"), "{stdout}");
    // Reading never writes: the root stays empty.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn binary_doctor_exit_codes_distinguish_health() {
    let degraded_root = tempfile::tempdir().unwrap();
    let degraded_config = HieronymusConfig::new(degraded_root.path());
    seed_python_schema(&degraded_config);
    let (_, _, degraded_status) = hiero(&[
        "doctor",
        "--data-root",
        degraded_config.data_root().to_str().unwrap(),
    ]);
    assert_eq!(degraded_status.code(), Some(1));

    let unhealthy_root = tempfile::tempdir().unwrap();
    let unhealthy_config = HieronymusConfig::new(unhealthy_root.path());
    fs::write(unhealthy_config.database_path(), b"not a database").unwrap();
    let (_, _, unhealthy_status) = hiero(&[
        "doctor",
        "--data-root",
        unhealthy_config.data_root().to_str().unwrap(),
    ]);
    assert_eq!(unhealthy_status.code(), Some(2));
}

#[test]
fn binary_doctor_json_is_parseable() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, _, status) = hiero(&[
        "doctor",
        "--json",
        "--data-root",
        root.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["status"], "healthy");
    assert!(parsed["findings"].is_array());
}

#[test]
fn independent_candidate_check_retains_parent_lock_and_reports_its_scope() {
    let temp = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(temp.path());
    let _parent = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["doctor", "--skip-registration", "--json", "--data-root"])
        .arg(temp.path())
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "payload-config-without-registration");
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| !finding["code"]
                .as_str()
                .unwrap()
                .starts_with("service-unit"))
    );
    assert!(hiero::lifecycle::operation::LifecycleOperation::acquire(&config).is_err());
    assert_eq!(doctor::run(&config).to_json()["scope"], "full");
}

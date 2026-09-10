//! `hiero update` end-to-end over local fixture releases: the release-build
//! archive layout (`scripts/release-build.sh`), checksum verification, the
//! protocol/schema gates, the link switch, the health check with automatic
//! rollback, and idempotent reruns. No network: every "release" is a local
//! directory; the systemd user manager is never contacted (`--unit-dir`
//! override disables manager integration by design).

mod common;

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The real binary's identity values, so fixtures stay correct when they
/// change upstream.
fn real_identity() -> (String, i64) {
    let output = Command::new(common::installed::binary_or_development())
        .args(["version", "--json"])
        .output()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    (
        payload["protocol_revision"].as_str().unwrap().to_string(),
        payload["supported_schema_version"].as_i64().unwrap(),
    )
}

fn protocol_revision() -> String {
    real_identity().0
}

fn supported_schema() -> i64 {
    real_identity().1
}

struct FakeRelease {
    release_dir: PathBuf,
}

/// Write a fake `hiero` shell binary: `version --json` reports the given
/// identity, `doctor` exits with `doctor_exit`, everything else succeeds.
fn write_fake_binary(path: &Path, version: &str, protocol: &str, schema: i64, doctor_exit: i32) {
    let json = format!(
        "{{\"version\": \"{version}\", \"protocol_revision\": \"{protocol}\", \
         \"supported_schema_version\": {schema}}}"
    );
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\n  version) printf '%s\\n' '{json}' ;;\n  release-assets) printf '%s\\n' '{{}}' ;;\n  doctor) exit {doctor_exit} ;;\n  *) exit 0 ;;\nesac\n"
    );
    std::fs::write(path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

/// Assemble the release-build payload: the binary plus its relative argv[0]
/// links.
fn stage_payload(payload: &Path, binary: &Path) {
    std::fs::create_dir_all(payload).unwrap();
    std::fs::copy(binary, payload.join("hiero")).unwrap();
    // Match the fake candidate asset probe while preserving the production gate.
    std::fs::write(payload.join("assets.json"), "{}\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(payload.join("hiero"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(payload.join("hiero"), permissions).unwrap();
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        std::os::unix::fs::symlink("hiero", payload.join(name)).unwrap();
    }
}

/// Package a payload directory into the release-build archive layout and
/// return the release directory.
fn package(payload: &Path, version: &str) -> FakeRelease {
    let release_dir = payload.parent().unwrap().to_path_buf();
    let name = format!("hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz");
    let archive = release_dir.join(&name);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(payload)
        .args([
            "assets.json",
            "hiero",
            "hieronymus",
            "hieronymus-agent-hook",
            "hieronymus-mcp",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "tar failed");
    let digest = sha256_hex(&archive);
    std::fs::write(
        release_dir.join(format!("{name}.sha256")),
        format!("{digest}  {name}\n"),
    )
    .unwrap();
    FakeRelease { release_dir }
}

fn sha256_hex(path: &Path) -> String {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).unwrap();
    let mut digest = sha2::Sha256::new();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    digest.update(&buffer);
    format!("{:x}", digest.finalize())
}

/// An installed prior release: a fake old binary in the managed layout with
/// stable links.
fn seed_install(temp: &Path, version: &str) -> PathBuf {
    let app = temp.join("app");
    let version_dir = app.join("versions").join(version);
    std::fs::create_dir_all(&version_dir).unwrap();
    write_fake_binary(
        &version_dir.join("hiero"),
        version,
        &protocol_revision(),
        1,
        0,
    );
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        std::os::unix::fs::symlink("hiero", version_dir.join(name)).unwrap();
    }
    let layout = hiero::app::AppLayout::new(&app);
    layout.switch_stable_links(version).unwrap();
    app
}

fn run_update(
    release_dir: &Path,
    app: &Path,
    data_root: &Path,
    unit_dir: &Path,
    extra: &[&str],
) -> (String, String, std::process::ExitStatus) {
    let output = Command::new(common::installed::binary_or_development())
        .args([
            "update",
            "--release-dir",
            release_dir.to_str().unwrap(),
            "--app-dir",
            app.to_str().unwrap(),
            "--data-root",
            data_root.to_str().unwrap(),
            "--unit-dir",
            unit_dir.to_str().unwrap(),
        ])
        .args(extra)
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status,
    )
}

fn stable_target(app: &Path, name: &str) -> PathBuf {
    std::fs::read_link(app.join("bin").join(name)).unwrap()
}

fn seed_python_schema(data_root: &Path) {
    let connection = rusqlite::Connection::open(data_root.join("hieronymus.sqlite")).unwrap();
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

// ---------------------------------------------------------------------------
// Happy path and its guarantees
// ---------------------------------------------------------------------------

#[test]
fn update_installs_a_verified_release_and_switches_every_link() {
    let real_protocol = protocol_revision();
    let real_schema = supported_schema();
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    // Replace the copied real binary with a fake one at a different version
    // so the update is observable.
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &real_protocol,
        real_schema,
        0,
    );
    let release = package(&payload, "9.9.0");

    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    let unit_dir = temp.path().join("units");
    std::fs::create_dir_all(&data_root).unwrap();

    let (stdout, stderr, status) = run_update(
        &release.release_dir,
        &app,
        &data_root,
        &unit_dir,
        &["--json"],
    );
    assert!(status.success(), "{stdout}{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["outcome"], "updated");
    assert_eq!(report["previous_version"], "0.9.0");
    assert_eq!(report["daemon_started"], false);

    // Stable links now serve the new version; the old version stays alongside.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/9.9.0/hiero")
    );
    assert_eq!(
        stable_target(&app, "hieronymus-mcp"),
        PathBuf::from("../versions/9.9.0/hieronymus-mcp")
    );
    assert!(app.join("versions/0.9.0/hiero").exists());
    // No staging leftovers.
    let leftovers: Vec<_> = std::fs::read_dir(app.join("versions"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers.len(), 2, "{leftovers:?}");
    // The health check ran against the data root via the candidate binary.
    assert!(stdout.contains("healthy"), "{stdout}");
}

#[test]
fn update_refreshes_the_service_unit_to_the_new_absolute_binary() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");

    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    let unit_dir = temp.path().join("units");
    std::fs::create_dir_all(&data_root).unwrap();
    std::fs::create_dir_all(&unit_dir).unwrap();
    // A unit left behind by the bootstrap installer points at the old binary.
    std::fs::write(
        unit_dir.join("hieronymus.service"),
        format!(
            "[Service]\nExecStart=\"{}\" daemon --data-root \"{}\"\n",
            app.join("versions/0.9.0/hiero").display(),
            data_root.display()
        ),
    )
    .unwrap();

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert!(status.success(), "{stdout}{stderr}");
    let unit = std::fs::read_to_string(unit_dir.join("hieronymus.service")).unwrap();
    assert!(
        unit.contains(&app.join("versions/9.9.0/hiero").display().to_string()),
        "{unit}"
    );
    assert!(stdout.contains("service unit updated"), "{stdout}");
}

#[test]
fn update_is_idempotent_when_already_up_to_date() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "9.9.0");
    let data_root = temp.path().join("data");
    let unit_dir = temp.path().join("units");
    std::fs::create_dir_all(&data_root).unwrap();

    let before = stable_target(&app, "hiero");
    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("already up to date"), "{stdout}");
    assert_eq!(stable_target(&app, "hiero"), before);
}

// ---------------------------------------------------------------------------
// Refusals: nothing changes, exit code 2
// ---------------------------------------------------------------------------

#[test]
fn update_refuses_a_checksum_mismatch_without_touching_links() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");
    // Tamper with the recorded checksum.
    let checksum = release
        .release_dir
        .join("hieronymus-9.9.0-x86_64-unknown-linux-gnu.tar.gz.sha256");
    std::fs::write(&checksum, format!("{:0>64}  tampered\n", "beef")).unwrap();

    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert!(!app.join("versions/9.9.0").exists());
    assert!(!app.join("versions/.staging-9.9.0").exists());
}

#[test]
fn update_refuses_a_protocol_change() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(&payload.join("hiero"), "9.9.0", "1999-01-01", 1, 0);
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    let unit_dir = temp.path().join("units");

    let (_, stderr, status) = run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("protocol"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert!(!app.join("versions/.staging-9.9.0").exists());
}

#[test]
fn update_refuses_launching_an_older_binary_against_a_newer_schema() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    // A newer release that only supports schema 1, while the database was
    // already upgraded to schema 2 by a newer binary: launching the candidate
    // against it is forbidden, so the whole update is refused.
    write_fake_binary(&payload.join("hiero"), "9.9.0", &protocol_revision(), 1, 0);
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let connection = rusqlite::Connection::open(data_root.join("hieronymus.sqlite")).unwrap();
    connection
        .execute_batch(
            "create table hieronymus_meta (schema_version integer not null unique);
             insert into hieronymus_meta values (2);",
        )
        .unwrap();
    drop(connection);
    let unit_dir = temp.path().join("units");

    let (_, stderr, status) = run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("newer schema"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
}

#[test]
fn update_refuses_an_older_release_version() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-0.1.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "0.1.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "0.1.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");

    let (_, stderr, status) = run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("downgrade is unsupported"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
}

#[test]
fn update_refuses_running_while_a_daemon_is_active_without_a_stoppable_service() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");

    // An authenticated, matching daemon is active, but there is no service
    // definition through which the updater may stop it.
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(data_root.clone()),
        port: 0,
        ..Default::default()
    })
    .unwrap();

    let config = hieronymus::data_root::HieronymusConfig::new(&data_root);
    assert!(hiero::lifecycle::probe(&config).is_live());
    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(2), "{stdout}\n{stderr}");
    assert!(stderr.contains("daemon is currently running"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    daemon.shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// I/O failure in the stop→switch window: rollback with step trace
// ---------------------------------------------------------------------------

#[test]
fn update_rolls_back_when_the_install_or_link_switch_fails() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");
    std::fs::create_dir_all(&unit_dir).unwrap();
    let unit_before = format!(
        "[Service]\nExecStart=\"{}\" daemon --data-root \"{}\"\n",
        app.join("versions/0.9.0/hiero").display(),
        data_root.display()
    );
    std::fs::write(unit_dir.join("hieronymus.service"), &unit_before).unwrap();

    // Inject an I/O failure into the stop→switch window:
    // `switch_stable_links` stages each link at `bin/.<name>.switch`, so a
    // directory at that path makes the symlink creation fail — after the
    // rename promoted the staged copy and after the updater was committed to
    // the daemon being stopped.
    std::fs::create_dir_all(app.join("bin/.hiero.switch")).unwrap();

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("install/link switch failed"), "{stderr}");
    assert!(stderr.contains("rolled back"), "{stderr}");
    // The failure is reported with the step trace, not a bare i/o error.
    assert!(stderr.contains("archive checksum verified"), "{stderr}");
    assert!(stderr.contains("candidate compatibility"), "{stderr}");
    // The prior links, version directory, and unit survive untouched.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert_eq!(
        stable_target(&app, "hieronymus-mcp"),
        PathBuf::from("../versions/0.9.0/hieronymus-mcp")
    );
    assert!(app.join("versions/0.9.0/hiero").exists());
    assert!(!app.join("versions/9.9.0").exists());
    assert!(!app.join("versions/.staging-9.9.0").exists());
    assert_eq!(
        std::fs::read_to_string(unit_dir.join("hieronymus.service")).unwrap(),
        unit_before
    );
    // A follow-up update recovers cleanly once the obstruction is gone.
    std::fs::remove_dir_all(app.join("bin/.hiero.switch")).unwrap();
    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert!(status.success(), "{stdout}{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/9.9.0/hiero")
    );
}

// ---------------------------------------------------------------------------
// Schema upgrade required: install completes, daemon stays stopped
// ---------------------------------------------------------------------------

#[test]
fn update_over_a_python_schema_completes_but_keeps_the_daemon_stopped() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        0,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    seed_python_schema(&data_root);
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("migration-pending"), "{stdout}");
    assert!(stdout.contains("migrate"), "{stdout}");
    assert!(stdout.contains("stays stopped"), "{stdout}");
    // The install completed: links serve the new version.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/9.9.0/hiero")
    );
}

// ---------------------------------------------------------------------------
// Failed health before a schema change: automatic rollback
// ---------------------------------------------------------------------------

#[test]
fn update_rolls_back_when_the_candidate_doctor_returns_an_unexpected_exit() {
    // Astra finding 10: `unwrap_or(2)` used to let a code like 42 fall through
    // every branch to the healthy path. An unexpected exit must roll back.
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        42,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("health check failed"), "{stderr}");
    assert!(stderr.contains("Some(42)"), "{stderr}");
    assert!(stderr.contains("rolled back"), "{stderr}");
    // The prior binary is back, the failed candidate is gone.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert!(!app.join("versions/9.9.0").exists());
    assert!(app.join("versions/0.9.0/hiero").exists());
}

#[test]
fn update_rolls_back_a_degraded_candidate_when_no_daemon_confirms_it() {
    // doctor exit 1 with no started daemon: authenticated readiness cannot
    // confirm the intended version, so a degraded candidate is not activated.
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        1,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("degraded"), "{stderr}");
    assert!(stderr.contains("rolled back"), "{stderr}");
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert!(!app.join("versions/9.9.0").exists());
}

#[test]
fn update_over_an_older_rust_schema_reports_migration_pending_and_never_rolls_back() {
    // Sonnet 3.3: a v1-on-disk / v2-candidate update completes the install and
    // reports `migration-pending`; it must NOT roll back for that reason
    // (R3 behaviour preserved by R4).
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    // Candidate supports a schema newer than the disk.
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema() + 1,
        // doctor would say "unhealthy" here, but the daemon stays stopped for
        // the migration so the health gate is never reached.
        2,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let connection = rusqlite::Connection::open(data_root.join("hieronymus.sqlite")).unwrap();
    connection
        .execute_batch(&format!(
            "create table hieronymus_meta (schema_version integer not null unique);
             insert into hieronymus_meta values ({});",
            supported_schema()
        ))
        .unwrap();
    drop(connection);
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("migration-pending"), "{stdout}");
    // The install completed and links serve the new version — no rollback.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/9.9.0/hiero")
    );
    assert!(app.join("versions/0.9.0/hiero").exists());
}

#[test]
fn update_rolls_back_when_the_new_binary_fails_its_health_check() {
    let temp = tempfile::tempdir().unwrap();
    let payload = temp.path().join("payload-9.9.0");
    stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
    write_fake_binary(
        &payload.join("hiero"),
        "9.9.0",
        &protocol_revision(),
        supported_schema(),
        2,
    );
    let release = package(&payload, "9.9.0");
    let app = seed_install(temp.path(), "0.9.0");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let unit_dir = temp.path().join("units");

    let (stdout, stderr, status) =
        run_update(&release.release_dir, &app, &data_root, &unit_dir, &[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("health check failed"), "{stderr}");
    assert!(stderr.contains("restored 0.9.0"), "{stderr}");
    // The prior binary is back and the failed version is gone.
    assert_eq!(
        stable_target(&app, "hiero"),
        PathBuf::from("../versions/0.9.0/hiero")
    );
    assert!(!app.join("versions/9.9.0").exists());
    assert!(app.join("versions/0.9.0/hiero").exists());
}

#[test]
fn update_refuses_an_owned_root_even_when_authenticated_discovery_fails() {
    for failure in ["credential", "missing-discovery"] {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("payload-9.9.0");
        stage_payload(&payload, Path::new(env!("CARGO_BIN_EXE_hiero")));
        // Without the ownership gate this path falsely reports MigrationPending
        // and switches the installation while the old daemon still owns the root.
        write_fake_binary(
            &payload.join("hiero"),
            "9.9.0",
            &protocol_revision(),
            supported_schema() + 1,
            0,
        );
        let release = package(&payload, "9.9.0");
        let app = seed_install(temp.path(), "0.9.0");
        let data_root = temp.path().join("data");
        let config = hieronymus::data_root::HieronymusConfig::new(&data_root);
        let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
            data_root: Some(data_root.clone()),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        if failure == "credential" {
            std::fs::write(config.daemon_token_path(), "rejected-token").unwrap();
        } else {
            std::fs::remove_file(config.daemon_discovery_path()).unwrap();
        }
        assert!(!hiero::lifecycle::probe(&config).is_live());
        assert!(hieronymus::ownership::RootOwnership::acquire(&config, "test").is_err());
        let (stdout, stderr, status) = run_update(
            &release.release_dir,
            &app,
            &data_root,
            &temp.path().join("units"),
            &[],
        );
        assert_eq!(status.code(), Some(2), "{failure}: {stdout}\n{stderr}");
        assert!(stderr.contains("owns this data root"), "{stderr}");
        assert_eq!(
            stable_target(&app, "hiero"),
            PathBuf::from("../versions/0.9.0/hiero")
        );
        assert!(!app.join("versions/9.9.0").exists());
        assert!(hieronymus::ownership::RootOwnership::acquire(&config, "test").is_err());
        daemon.shutdown().unwrap();
    }
}

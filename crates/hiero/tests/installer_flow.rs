//! The bootstrap installer (`scripts/install.sh`) executed end-to-end against
//! local fixture releases built from the real binary: the 8 installer steps
//! with the release-build archive layout. Tests override `HOME` so nothing
//! touches the developer's real home, pass `--unit-dir`/`--no-activate` so
//! the systemd user manager is never contacted, and only use local
//! directories (no network).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn real_version() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args(["version", "--json"])
        .output()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    payload["version"].as_str().unwrap().to_string()
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

/// Build a local fixture release in the exact layout
/// `scripts/release-build.sh` produces: archive with the binary plus relative
/// argv[0] links, and the `.sha256` sibling.
fn build_release(root: &Path, binary: &str) -> PathBuf {
    let release_dir = root.join("release");
    let payload = root.join("payload");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::copy(binary, payload.join("hiero")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(payload.join("hiero"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(payload.join("hiero"), permissions).unwrap();
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        std::os::unix::fs::symlink("hiero", payload.join(name)).unwrap();
    }
    std::fs::create_dir_all(&release_dir).unwrap();
    let version = real_version();
    let name = format!("hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz");
    let archive = release_dir.join(&name);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&payload)
        .args([
            "hiero",
            "hieronymus",
            "hieronymus-agent-hook",
            "hieronymus-mcp",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "tar failed");
    std::fs::write(
        release_dir.join(format!("{name}.sha256")),
        format!("{}  {name}\n", sha256_hex(&archive)),
    )
    .unwrap();
    release_dir
}

fn installer_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/install.sh")
        .canonicalize()
        .unwrap()
}

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn app(&self) -> PathBuf {
        self.root.path().join("app")
    }

    fn data_root(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn unit_dir(&self) -> PathBuf {
        self.root.path().join("units")
    }

    fn run(&self, extra: &[&str]) -> (String, String, std::process::ExitStatus) {
        let output = Command::new(installer_path())
            .arg("--release-dir")
            .arg(self.root.path().join("release"))
            .arg("--app-dir")
            .arg(self.app())
            .arg("--data-root")
            .arg(self.data_root())
            .arg("--unit-dir")
            .arg(self.unit_dir())
            .arg("--no-activate")
            .args(extra)
            .env("HOME", self.home())
            .output()
            .expect("installer could not be executed");
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
            output.status,
        )
    }
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
// The 8 steps, happy path
// ---------------------------------------------------------------------------

#[test]
fn install_runs_all_eight_steps_and_lays_out_the_managed_tree() {
    let sandbox = Sandbox::new();
    build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    std::fs::create_dir_all(sandbox.data_root()).unwrap();
    let version = real_version();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(status.success(), "{stdout}{stderr}");

    // Step 1 prints the resolved platform without executing anything.
    assert!(stdout.contains("x86_64-unknown-linux-gnu"), "{stdout}");
    // Step 3 verified the checksum.
    assert!(stdout.contains("sha256 verified"), "{stdout}");
    // Step 4: versioned application directory.
    let version_dir = sandbox.app().join("versions").join(&version);
    assert!(version_dir.join("hiero").is_file(), "{stdout}");
    // Step 5: stable command links (relative into the versioned directory).
    assert_eq!(
        std::fs::read_link(sandbox.app().join("bin/hieronymus-mcp")).unwrap(),
        PathBuf::from(format!("../versions/{version}/hieronymus-mcp"))
    );
    // The installed binary works through the stable link.
    let probe = Command::new(sandbox.app().join("bin/hiero"))
        .arg("version")
        .output()
        .unwrap();
    assert!(probe.status.success());
    assert!(String::from_utf8(probe.stdout).unwrap().contains("hiero v"));
    // The PATH links went into the sandboxed HOME only.
    assert_eq!(
        std::fs::read_link(sandbox.home().join(".local/bin/hiero")).unwrap(),
        sandbox.app().join("bin/hiero")
    );
    // Step 6: the unit file is written, pointing at the installed binary and
    // data root; the daemon was not started.
    let unit = std::fs::read_to_string(sandbox.unit_dir().join("hieronymus.service")).unwrap();
    assert!(
        unit.contains(&version_dir.join("hiero").display().to_string()),
        "{unit}"
    );
    assert!(
        unit.contains(&sandbox.data_root().display().to_string()),
        "{unit}"
    );
    assert!(unit.contains("Restart=on-failure"));
    // Steps 7 and 8: doctor ran, the daemon stayed stopped (--no-activate).
    assert!(stdout.contains("doctor: healthy"), "{stdout}");
    assert!(stdout.contains("daemon not started"), "{stdout}");
    // No staging leftovers.
    let versions: Vec<_> = std::fs::read_dir(sandbox.app().join("versions"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(versions, vec![version.clone()], "{versions:?}");
}

#[test]
fn install_is_idempotent_on_rerun() {
    let sandbox = Sandbox::new();
    build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    std::fs::create_dir_all(sandbox.data_root()).unwrap();

    let (first, first_stderr, first_status) = sandbox.run(&[]);
    assert!(first_status.success(), "{first}{first_stderr}");
    let (second, second_stderr, second_status) = sandbox.run(&[]);
    assert!(second_status.success(), "{second}{second_stderr}");
    let version = real_version();
    let versions: Vec<_> = std::fs::read_dir(sandbox.app().join("versions"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(versions, vec![version]);
    assert!(sandbox.app().join("bin/hiero").is_file());
}

// ---------------------------------------------------------------------------
// Degrade and refusal paths
// ---------------------------------------------------------------------------

#[test]
fn install_refuses_a_checksum_mismatch_and_installs_nothing() {
    let sandbox = Sandbox::new();
    let release_dir = build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    let version = real_version();
    let checksum = release_dir.join(format!(
        "hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz.sha256"
    ));
    std::fs::write(&checksum, format!("{:0>64}  tampered\n", "dead")).unwrap();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    // Nothing was installed: the app directory was never created.
    assert!(!sandbox.app().exists());
    assert!(!sandbox.unit_dir().join("hieronymus.service").exists());
}

#[test]
fn install_completes_over_a_python_schema_but_keeps_the_daemon_stopped() {
    let sandbox = Sandbox::new();
    build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    std::fs::create_dir_all(sandbox.data_root()).unwrap();
    seed_python_schema(&sandbox.data_root());

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(status.success(), "{stdout}{stderr}");
    // The upgrade is reported, never performed, and the daemon stays stopped.
    assert!(stdout.contains("DATABASE UPGRADE REQUIRED"), "{stdout}");
    assert!(stdout.contains("stays stopped"), "{stdout}");
    assert!(stdout.contains("migrate"), "{stdout}");
    assert!(stdout.contains("doctor: degraded"), "{stdout}");
    // The managed install itself completed.
    assert!(sandbox.app().join("bin/hiero").is_file());
    assert!(sandbox.unit_dir().join("hieronymus.service").exists());
}

#[test]
fn install_reports_an_unhealthy_doctor_and_still_completes() {
    let sandbox = Sandbox::new();
    build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    std::fs::create_dir_all(sandbox.data_root()).unwrap();
    // A corrupt database: doctor exits 2, the installer reports it and leaves
    // the start decision to the user.
    std::fs::write(
        sandbox.data_root().join("hieronymus.sqlite"),
        b"not a database",
    )
    .unwrap();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("UNHEALTHY"), "{stdout}");
    assert!(stdout.contains("daemon NOT started"), "{stdout}");
    assert!(sandbox.app().join("bin/hiero").is_file());
}

#[test]
fn install_refuses_a_signed_release_in_the_waived_first_line() {
    let sandbox = Sandbox::new();
    let release_dir = build_release(sandbox.root.path(), env!("CARGO_BIN_EXE_hiero"));
    let version = real_version();
    std::fs::write(
        release_dir.join("release.json"),
        format!(
            "{{\"version\": \"{version}\", \
             \"archive\": \"hieronymus-{version}-x86_64-unknown-linux-gnu.tar.gz\", \
             \"sha256\": \"{}\", \
             \"signature\": \"MEUCIQ==\"}}",
            "ab".repeat(32)
        ),
    )
    .unwrap();

    let (stdout, stderr, status) = sandbox.run(&[]);
    assert_eq!(status.code(), Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("signature"), "{stderr}");
    assert!(!sandbox.app().join("bin/hiero").exists());
}

#[test]
fn install_requires_a_release_source() {
    let sandbox = Sandbox::new();
    let output = Command::new(installer_path())
        .env("HOME", sandbox.home())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--release-dir"), "{stderr}");
}

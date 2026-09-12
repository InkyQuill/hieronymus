#![cfg(target_os = "linux")]
//! `hiero service` CLI surface: install (idempotent unit render), status,
//! uninstall, and the lifecycle commands' refusal to touch the systemd user
//! manager for an overridden `--unit-dir` (which is how these tests — and any
//! non-systemd setup — stay safe).

use std::path::Path;
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

struct Environment {
    root: tempfile::TempDir,
    unit_dir: std::path::PathBuf,
    data_root: std::path::PathBuf,
    binary: std::path::PathBuf,
}

impl Environment {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("app/versions/1.0.0/hiero");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fake binary").unwrap();
        let environment = Self {
            unit_dir: root.path().join("units"),
            data_root: root.path().join("data"),
            binary,
            root,
        };
        std::fs::create_dir_all(&environment.data_root).unwrap();
        environment
    }

    fn unit_path(&self) -> std::path::PathBuf {
        self.root.path().join("units").join("hieronymus.service")
    }

    fn base_arguments(&self) -> Vec<String> {
        vec![
            "--data-root".to_string(),
            self.data_root.to_string_lossy().into_owned(),
            "--unit-dir".to_string(),
            self.unit_dir.to_string_lossy().into_owned(),
        ]
    }

    fn run(&self, arguments: &[&str]) -> (String, String, std::process::ExitStatus) {
        let mut all = vec!["service".to_string()];
        all.extend(arguments.iter().map(|argument| argument.to_string()));
        all.extend(self.base_arguments());
        let references: Vec<&str> = all.iter().map(|argument| argument.as_str()).collect();
        hiero(&references)
    }
}

fn unit_content(binary: &Path, data_root: &Path) -> String {
    format!(
        "ExecStart=\"{}\" daemon --data-root \"{}\"",
        binary.display(),
        data_root.display()
    )
}

#[test]
fn installed_cli_keeps_stable_launcher_in_registration() {
    let environment = Environment::new();
    std::fs::remove_file(&environment.binary).unwrap();
    std::fs::hard_link(env!("CARGO_BIN_EXE_hiero"), &environment.binary)
        .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_hiero"), &environment.binary).map(|_| ()))
        .unwrap();
    let stable = environment.root.path().join("app/bin/hiero");
    std::fs::create_dir_all(stable.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&environment.binary, &stable).unwrap();
    let output = Command::new(&stable)
        .args(["service", "install", "--no-activate"])
        .args(environment.base_arguments())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let unit = std::fs::read_to_string(environment.unit_path()).unwrap();
    assert!(
        unit.contains(&unit_content(&stable, &environment.data_root)),
        "{unit}"
    );
}

#[test]
fn install_writes_a_unit_pointing_at_the_absolute_binary_and_data_root() {
    let environment = Environment::new();
    let (stdout, stderr, status) = environment.run(&[
        "install",
        "--no-activate",
        "--binary",
        environment.binary.to_str().unwrap(),
    ]);
    assert!(status.success(), "{stdout}{stderr}");
    let unit = std::fs::read_to_string(environment.unit_path()).unwrap();
    assert!(
        unit.contains(&unit_content(&environment.binary, &environment.data_root)),
        "{unit}"
    );
    assert!(unit.contains("Restart=on-failure"), "{unit}");
    assert!(stdout.contains("manager integration skipped"), "{stdout}");
}

#[test]
fn install_is_idempotent_and_status_tracks_consistency() {
    let environment = Environment::new();
    let arguments = [
        "install",
        "--no-activate",
        "--binary",
        environment.binary.to_str().unwrap(),
    ];
    let (first, _, first_status) = environment.run(&arguments);
    assert!(first_status.success(), "{first}");

    // Rerun: same content, still success.
    let (_, _, second_status) = environment.run(&arguments);
    assert!(second_status.success());

    // Status: consistent → exit 0, JSON parseable.
    let (stdout, _, status) = environment.run(&["status", "--json"]);
    assert!(status.success(), "{stdout}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["installed"], true);
    assert_eq!(report["consistent"], true);

    // Break the unit's binary: status degrades to exit 1.
    std::fs::remove_file(&environment.binary).unwrap();
    let (_, _, status) = environment.run(&["status"]);
    assert_eq!(status.code(), Some(1));
}

#[test]
fn uninstall_removes_only_the_unit_and_is_idempotent() {
    let environment = Environment::new();
    environment.run(&[
        "install",
        "--no-activate",
        "--binary",
        environment.binary.to_str().unwrap(),
    ]);
    let (stdout, _, status) = environment.run(&["uninstall"]);
    assert!(status.success(), "{stdout}");
    assert!(!environment.unit_path().exists());

    // Rerun: already absent, still success.
    let (stdout, _, status) = environment.run(&["uninstall"]);
    assert!(status.success(), "{stdout}");
    assert!(stdout.contains("already absent"), "{stdout}");
}

#[test]
fn lifecycle_commands_refuse_without_a_unit_or_with_a_custom_unit_dir() {
    let environment = Environment::new();
    // No unit at all.
    let (_, stderr, status) = environment.run(&["start"]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("no service unit"), "{stderr}");

    // Unit present, but the overridden unit dir means no manager contact.
    std::fs::create_dir_all(&environment.unit_dir).unwrap();
    std::fs::write(environment.unit_path(), "[Unit]\n").unwrap();
    let (_, stderr, status) = environment.run(&["stop"]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unit file has no ExecStart"), "{stderr}");
}

#[test]
fn unknown_service_subcommands_fail_with_usage() {
    let environment = Environment::new();
    let (_, stderr, status) = environment.run(&["reinstall"]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unknown service subcommand"), "{stderr}");
}

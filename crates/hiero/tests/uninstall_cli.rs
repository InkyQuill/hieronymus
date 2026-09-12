//! `hiero uninstall` CLI surface: mandatory confirmation, software removal
//! with data preservation, the separate explicit `--delete-data` action, and
//! the refusal to remove a foreign application directory.

use std::path::Path;
use std::process::Command;

use hiero::app::AppLayout;
use hiero::service::{self, ServiceOptions};
use hieronymus::data_root::HieronymusConfig;

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

/// A full managed install inside a temp root: application directory with one
/// versioned release, a service unit, generated agent plugins, and user data.
struct Fixture {
    root: tempfile::TempDir,
    app: std::path::PathBuf,
    data_root: std::path::PathBuf,
    unit_dir: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let fixture = Self {
            app: root.path().join("app"),
            data_root: root.path().join("data"),
            unit_dir: root.path().join("units"),
            root,
        };
        let layout = AppLayout::new(&fixture.app);
        std::fs::create_dir_all(layout.version_dir("1.0.0")).unwrap();
        let binary = layout
            .version_dir("1.0.0")
            .join(format!("hiero{}", std::env::consts::EXE_SUFFIX));
        #[cfg(not(windows))]
        std::fs::write(&binary, b"binary").unwrap();
        #[cfg(windows)]
        {
            std::fs::copy(env!("CARGO_BIN_EXE_hiero"), &binary).unwrap();
            std::fs::copy(
                env!("CARGO_BIN_EXE_hiero-launcher"),
                layout.version_dir("1.0.0").join("hiero-launcher.exe"),
            )
            .unwrap();
        }
        layout.switch_stable_links("1.0.0").unwrap();

        std::fs::create_dir_all(&fixture.unit_dir).unwrap();
        std::fs::create_dir_all(&fixture.data_root).unwrap();
        let service_options = ServiceOptions {
            data_root: fixture.data_root.clone(),
            unit_dir: fixture.unit_dir.clone(),
            binary: if cfg!(windows) {
                layout.stable_link("hiero")
            } else {
                binary
            },
            use_manager: cfg!(windows),
        };
        service::install(&service_options).unwrap();

        let config = HieronymusConfig::new(&fixture.data_root);
        std::fs::create_dir_all(config.agent_plugins_root()).unwrap();
        std::fs::write(config.database_path(), b"database bytes").unwrap();
        std::fs::create_dir_all(config.backups_root().join("pre-upgrade")).unwrap();
        std::fs::write(config.dream_config_path(), "[workflows]\n").unwrap();
        fixture
    }

    fn arguments(&self, extra: &[&str]) -> Vec<String> {
        let mut all = vec!["uninstall".to_string()];
        all.extend(extra.iter().map(|argument| argument.to_string()));
        all.push("--app-dir".to_string());
        all.push(self.app.to_string_lossy().into_owned());
        all.push("--data-root".to_string());
        all.push(self.data_root.to_string_lossy().into_owned());
        all.push("--unit-dir".to_string());
        all.push(self.unit_dir.to_string_lossy().into_owned());
        all
    }

    fn run(&self, extra: &[&str]) -> (String, String, std::process::ExitStatus) {
        let all = self.arguments(extra);
        let references: Vec<&str> = all.iter().map(|argument| argument.as_str()).collect();
        hiero(&references)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Refusal tests leave an installed fixture; remove its native task
        // while the executable and registration identity still exist.
        #[cfg(windows)]
        if self.app.exists() {
            let _ = self.run(&["--yes"]);
        }
    }
}

#[test]
fn uninstall_without_confirmation_refuses_and_changes_nothing() {
    let fixture = Fixture::new();
    let (_, stderr, status) = fixture.run(&[]);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("confirmation"), "{stderr}");
    // Nothing was removed by the refusal.
    assert!(fixture.app.exists());
    assert!(fixture.unit_dir.join(service::SERVICE_UNIT_NAME).exists());
    assert!(fixture.data_root.exists());
}

#[test]
fn uninstall_removes_software_and_generated_entries_and_preserves_data() {
    let fixture = Fixture::new();
    let (stdout, stderr, status) = fixture.run(&["--yes", "--json"]);
    assert!(status.success(), "{stdout}{stderr}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["data_deleted"], false);

    // Software and generated entries are gone.
    assert!(!fixture.app.exists());
    assert!(!fixture.unit_dir.join(service::SERVICE_UNIT_NAME).exists());
    assert!(
        !HieronymusConfig::new(&fixture.data_root)
            .agent_plugins_root()
            .exists()
    );

    // The data root and everything the spec protects survive.
    let config = HieronymusConfig::new(&fixture.data_root);
    assert!(config.database_path().exists());
    assert!(config.backups_root().exists());
    assert!(config.dream_config_path().exists());
    assert!(
        report["preserved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.as_str().unwrap().contains("database"))
    );
}

#[test]
fn delete_data_removes_the_exact_named_root_only() {
    let fixture = Fixture::new();
    for name in [
        "foo.lock",
        ".tray-unknown.lock",
        ".tray-abc.lock",
        ".tray-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.lock",
        ".tray-000000000000000000000000000000000000000000000000000000000000000g.lock",
    ] {
        std::fs::write(fixture.data_root.join(name), b"user data").unwrap();
    }
    std::fs::create_dir(fixture.data_root.join("notes.lock")).unwrap();
    std::fs::write(fixture.data_root.join("notes.lock/contents"), b"user data").unwrap();
    let (stdout, stderr, status) = fixture.run(&["--yes", "--delete-data"]);
    assert!(status.success(), "{stdout}{stderr}");
    assert!(
        stdout.contains(&fixture.data_root.display().to_string()),
        "{stdout}"
    );
    let mut retained: Vec<_> = std::fs::read_dir(&fixture.data_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    retained.sort();
    let mut expected = vec![".desktop-launch.lock", ".lifecycle.lock", ".owner.lock"];
    if cfg!(windows) {
        expected.push(".windows-browser.lock");
        expected.push(".windows-native.lock");
    }
    expected.sort();
    assert_eq!(retained, expected);
    // The software side is still removed exactly once.
    assert!(!fixture.app.exists());
}

#[test]
fn uninstall_refuses_a_foreign_application_directory() {
    let fixture = Fixture::new();
    let foreign = fixture.root.path().join("precious");
    std::fs::create_dir_all(&foreign).unwrap();
    let mut arguments = fixture.arguments(&["--yes"]);
    // Replace the --app-dir value with the foreign directory.
    let position = arguments
        .iter()
        .position(|argument| argument == "--app-dir")
        .unwrap();
    arguments[position + 1] = foreign.to_string_lossy().into_owned();
    let references: Vec<&str> = arguments.iter().map(|argument| argument.as_str()).collect();
    let (_, stderr, status) = hiero(&references);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("does not look like"), "{stderr}");
    assert!(foreign.exists());
    assert!(Path::new(&fixture.app).exists());
}

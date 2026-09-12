//! Disposable contract tests; these are not native desktop/Task Scheduler acceptance.
use hiero::{
    app::AppLayout,
    desktop::control,
    update::{self, UpdateOptions},
};
use hieronymus::data_root::HieronymusConfig;
#[test]
fn pending_lifecycle_operation_refuses_update_before_acquisition_or_selection() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let _operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    let app = AppLayout::new(root.path().join("app"));
    let result = update::run_update(&UpdateOptions {
        release_dir: root.path().join("missing"),
        app_dir: Some(app.root().into()),
        data_root: Some(config.data_root().into()),
        unit_dir: Some(root.path().join("units")),
    });
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("lifecycle operation")
    );
    assert!(!app.root().exists());
}
#[test]
fn explicit_quit_prevents_replacement_start_even_when_executable_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    control::record_quit(root.path()).unwrap();
    control::restart(
        &config,
        &root.path().join("missing/hiero"),
        &root.path().join("units"),
    )
    .unwrap();
    assert!(control::quit_requested(root.path()).unwrap());
}

#[test]
fn replacement_start_failure_does_not_create_a_quit_intent_or_claim_success() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    assert!(
        control::restart(
            &config,
            &root.path().join("missing/hiero"),
            &root.path().join("units")
        )
        .is_err()
    );
    assert!(!control::quit_requested(root.path()).unwrap());
}
#[test]
fn a_contended_explicit_start_cannot_clear_durable_quit() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    control::record_quit(root.path()).unwrap();
    let _operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    let options = hiero::service::ServiceOptions {
        data_root: root.path().into(),
        unit_dir: root.path().join("units"),
        binary: root.path().join("missing/hiero"),
        use_manager: false,
    };
    assert!(hiero::lifecycle::start(&options).is_err());
    assert!(control::quit_requested(root.path()).unwrap());
}

/// Real binaries, pinned assets, isolated registration paths, and no display/manager substitution.
/// This qualifies the offline Linux transaction; native helper-host acceptance is separate.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires HIERO_DESKTOP_RELEASE_DIR and HIERO_DESKTOP_BASELINE_DIR (actual complete 0.8.0 version)"]
fn real_linux_bootstrap_upgrade_rollback_and_uninstall() {
    use std::{
        path::{Path, PathBuf},
        process::{Command, Output},
    };
    fn copy_tree(source: &Path, destination: &Path) {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                copy_tree(&entry.path(), &target);
            } else if kind.is_symlink() {
                std::os::unix::fs::symlink(std::fs::read_link(entry.path()).unwrap(), target)
                    .unwrap();
            } else {
                assert!(kind.is_file());
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    fn command(root: &Path, program: &Path, args: &[&str]) -> Output {
        Command::new(program)
            .args(args)
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("XDG_DATA_HOME", root.join("xdg-data"))
            .env_remove("HIERO_SEMANTIC_MODEL_DIR")
            .env_remove("HIERO_ONNX_RUNTIME")
            .env_remove("ORT_DYLIB_PATH")
            .env_remove("LD_LIBRARY_PATH")
            .output()
            .unwrap()
    }
    fn success(output: Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let release = PathBuf::from(
        std::env::var_os("HIERO_DESKTOP_RELEASE_DIR").expect("HIERO_DESKTOP_RELEASE_DIR required"),
    )
    .canonicalize()
    .unwrap();
    let baseline = PathBuf::from(
        std::env::var_os("HIERO_DESKTOP_BASELINE_DIR")
            .expect("HIERO_DESKTOP_BASELINE_DIR required"),
    )
    .canonicalize()
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("hiero-real-desktop-")
        .tempdir()
        .unwrap()
        .keep();
    eprintln!(
        "retained disposable desktop qualification: {}",
        root.display()
    );
    let fresh = root.join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    let app = fresh.join("app");
    let data = fresh.join("data");
    let units = fresh.join("units");
    success(command(
        &fresh,
        Path::new("bash"),
        &[
            release
                .join("install-desktop-x86_64-unknown-linux-gnu.sh")
                .to_str()
                .unwrap(),
            "--release-dir",
            release.to_str().unwrap(),
            "--app-dir",
            app.to_str().unwrap(),
            "--data-root",
            data.to_str().unwrap(),
            "--unit-dir",
            units.to_str().unwrap(),
            "--no-activate",
        ],
    ));
    let cli = app.join("bin/hiero");
    assert_eq!(
        AppLayout::new(&app).current_version().as_deref(),
        Some("0.9.0")
    );
    assert!(!hiero::lifecycle::probe(&HieronymusConfig::new(&data)).is_live());
    // Directly own a real disposable daemon; custom unit directories intentionally cannot start a manager.
    struct OwnedDaemon(std::process::Child);
    impl Drop for OwnedDaemon {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut daemon = OwnedDaemon(
        Command::new(&cli)
            .args([
                "daemon",
                "--port",
                "0",
                "--data-root",
                data.to_str().unwrap(),
            ])
            .env("XDG_CONFIG_HOME", fresh.join("xdg-config"))
            .env("XDG_DATA_HOME", fresh.join("xdg-data"))
            .env_remove("HIERO_SEMANTIC_MODEL_DIR")
            .env_remove("HIERO_ONNX_RUNTIME")
            .env_remove("ORT_DYLIB_PATH")
            .env_remove("LD_LIBRARY_PATH")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(fresh.join("daemon.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if hiero::update::require_release_ready(&HieronymusConfig::new(&data)).is_ok() {
            break;
        }
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "disposable daemon exited; inspect daemon.log"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "semantic readiness timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let ready = command(
        &fresh,
        &cli,
        &["release-ready", "--data-root", data.to_str().unwrap()],
    );
    let stopped = command(
        &fresh,
        &cli,
        &[
            "stop",
            "--data-root",
            data.to_str().unwrap(),
            "--unit-dir",
            units.to_str().unwrap(),
            "--binary",
            cli.to_str().unwrap(),
        ],
    );
    success(stopped);
    success(ready);
    for rollback in [false, true] {
        let fixture = root.join(if rollback { "rollback" } else { "upgrade" });
        std::fs::create_dir_all(&fixture).unwrap();
        let app = AppLayout::new(fixture.join("app"));
        copy_tree(&baseline, &app.version_dir("0.8.0"));
        app.switch_stable_links("0.8.0").unwrap();
        let baseline_identity =
            command(&fixture, &app.stable_link("hiero"), &["version", "--json"]);
        let identity: serde_json::Value =
            serde_json::from_slice(&baseline_identity.stdout).unwrap();
        assert_eq!(identity["version"], "0.8.0");
        let data = fixture.join("data");
        let units = fixture.join("units");
        let foreign = fixture.join("xdg-config/autostart/hieronymus.desktop");
        if rollback {
            std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
            std::fs::write(&foreign, "foreign desktop entry").unwrap();
        }
        let output = command(
            &fixture,
            &cli,
            &[
                "desktop-bootstrap",
                "--release-dir",
                release.to_str().unwrap(),
                "--app-dir",
                app.root().to_str().unwrap(),
                "--data-root",
                data.to_str().unwrap(),
                "--unit-dir",
                units.to_str().unwrap(),
                "--no-activate",
            ],
        );
        assert!(
            !hiero::lifecycle::probe(&HieronymusConfig::new(&data)).is_live(),
            "previously stopped daemon must remain stopped"
        );
        if rollback {
            assert!(!output.status.success(), "foreign registration must fail");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("rolled back"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(app.current_version().as_deref(), Some("0.8.0"));
            assert_eq!(
                std::fs::read_to_string(foreign).unwrap(),
                "foreign desktop entry"
            );
            assert_eq!(
                std::fs::read(app.version_dir("0.8.0").join("assets.json")).unwrap(),
                std::fs::read(baseline.join("assets.json")).unwrap()
            );
        } else {
            success(output);
            assert_eq!(app.current_version().as_deref(), Some("0.9.0"));
            assert!(app.version_dir("0.8.0").join("assets.json").exists());
            assert!(!hiero::lifecycle::probe(&HieronymusConfig::new(&data)).is_live());
            std::fs::write(data.join("user-settings.txt"), "preserve user data").unwrap();
            success(command(
                &fixture,
                &cli,
                &[
                    "uninstall",
                    "--yes",
                    "--app-dir",
                    app.root().to_str().unwrap(),
                    "--data-root",
                    data.to_str().unwrap(),
                    "--unit-dir",
                    units.to_str().unwrap(),
                ],
            ));
            assert!(!app.root().exists());
            assert_eq!(
                std::fs::read_to_string(data.join("user-settings.txt")).unwrap(),
                "preserve user data"
            );
        }
    }
}

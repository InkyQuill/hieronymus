//! Cross-process lifecycle serialization. Every fixture and manager is disposable.
use hiero::service::{self, ServiceOptions};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::ownership::RootOwnership;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn lock_holder_child() {
    let Ok(root) = std::env::var("HIERO_TEST_LOCK_ROOT") else {
        return;
    };
    let config = HieronymusConfig::new(root);
    let _operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    println!("lock held");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}

#[test]
fn a_child_held_lifecycle_lock_blocks_stop_until_process_exit() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_holder_child", "--nocapture"])
        .env("HIERO_TEST_LOCK_ROOT", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        if line.contains("lock held") {
            break;
        }
    }
    let stop = || {
        Command::new(env!("CARGO_BIN_EXE_hiero"))
            .arg("stop")
            .arg("--data-root")
            .arg(root.path())
            .arg("--unit-dir")
            .arg(root.path().join("units"))
            .output()
            .unwrap()
    };
    assert_eq!(
        hiero::lifecycle::operation::LifecycleOperation::acquire(&HieronymusConfig::new(
            root.path()
        ))
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::WouldBlock
    );
    let blocked = stop();
    child.stdin.take().unwrap().write_all(b"release\n").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        !blocked.status.success(),
        "a competing lifecycle operation must be refused"
    );
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("lifecycle"));
    assert!(
        stop().status.success(),
        "process exit must release the operation lock"
    );
    assert!(
        root.path().join(".lifecycle.lock").exists(),
        "never unlink coordination locks"
    );
}

fn options(root: &std::path::Path) -> ServiceOptions {
    ServiceOptions {
        data_root: root.join("data"),
        unit_dir: root.join("units"),
        binary: std::env::current_exe().unwrap(),
        use_manager: false,
    }
}

#[test]
fn service_install_refuses_another_roots_unit_without_overwriting_it() {
    let root = tempfile::tempdir().unwrap();
    let first = options(root.path());
    service::install(&first).unwrap();
    let original = std::fs::read(first.unit_path()).unwrap();
    let mut second = first.clone();
    second.data_root = root.path().join("other-data");
    let result = service::install(&second);
    assert!(
        result.is_err(),
        "install must not overwrite another root's registration"
    );
    assert_eq!(std::fs::read(first.unit_path()).unwrap(), original);
}

#[test]
fn stop_does_not_claim_resource_release_when_a_daemon_owner_has_no_record() {
    let root = tempfile::tempdir().unwrap();
    let options = options(root.path());
    let config = HieronymusConfig::new(&options.data_root);
    let _owner = RootOwnership::acquire(&config, "daemon").unwrap();
    assert!(
        hiero::lifecycle::stop(&config, &options).is_err(),
        "missing discovery is not resource release"
    );
}

#[test]
fn lifecycle_lock_does_not_replace_daemon_ownership() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let owner = RootOwnership::acquire(&config, "daemon").unwrap();
    let operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    assert_eq!(
        hiero::lifecycle::operation::LifecycleOperation::acquire(&config)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(RootOwnership::acquire(&config, "second-daemon").is_err());
    drop(operation);
    drop(owner);
}

#[test]
fn uninstall_refuses_before_removing_software_while_root_is_owned() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let _owner = RootOwnership::acquire(&config, "daemon").unwrap();
    let app = root.path().join("app");
    std::fs::create_dir_all(app.join("versions")).unwrap();
    std::fs::write(config.database_path(), b"precious data").unwrap();
    let result = hiero::uninstall::run_uninstall(&hiero::uninstall::UninstallOptions {
        app_dir: Some(app.clone()),
        data_root: Some(config.data_root().to_path_buf()),
        unit_dir: Some(root.path().join("units")),
        confirmed: true,
        delete_data: true,
    });
    assert!(
        result.is_err(),
        "uninstall must prove resource release before removal"
    );
    assert!(app.exists());
    assert_eq!(
        std::fs::read(config.database_path()).unwrap(),
        b"precious data"
    );
}

#[test]
fn update_contention_is_refused_before_release_resolution_or_staging() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let _operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .arg("update")
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--app-dir")
        .arg(root.path().join("app"))
        .arg("--release-dir")
        .arg(root.path().join("missing-release"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("lifecycle"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.path().join("app").exists());
}

#[cfg(unix)]
mod managed {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    struct Fixture {
        root: tempfile::TempDir,
        config: HieronymusConfig,
        options: ServiceOptions,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let config = HieronymusConfig::new(root.path().join("data"));
            let options = ServiceOptions {
                data_root: config.data_root().to_path_buf(),
                unit_dir: root.path().join("home/.config/systemd/user"),
                binary: env!("CARGO_BIN_EXE_hiero").into(),
                use_manager: false,
            };
            service::install(&options).unwrap();
            let bin = root.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let script = bin.join("systemctl");
            std::fs::write(&script, r##"#!/bin/sh
printf '%s\n' "$*" >> "$HIERO_TEST_ROOT/manager.log"
case "$2" in
  enable)
    if test -n "$HIERO_TEST_PAUSE_INSTALL"; then
      touch "$HIERO_TEST_ROOT/install-entered"
      n=0
      while test ! -e "$HIERO_TEST_ROOT/continue"; do
        n=$((n+1))
        test "$n" -lt 1000 || exit 24
        sleep 0.02
      done
    fi
    ;;
  start)
    test -z "$HIERO_TEST_START_FAIL" || exit 23
    if test -n "$HIERO_TEST_PAUSE_START"; then
      touch "$HIERO_TEST_ROOT/start-entered"
      n=0
      while test ! -e "$HIERO_TEST_ROOT/continue"; do
        n=$((n+1))
        test "$n" -lt 1000 || exit 24
        sleep 0.02
      done
    fi
    "$HIERO_TEST_BINARY" daemon --data-root "$HIERO_TEST_DATA" --port 0 > "$HIERO_TEST_ROOT/daemon.log" 2>&1 &
    ;;
esac
"##).unwrap();
            std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                root,
                config,
                options,
            }
        }
        fn command(&self, action: &str) -> Command {
            let mut command = Command::new(env!("CARGO_BIN_EXE_hiero"));
            command
                .arg(action)
                .arg("--data-root")
                .arg(self.config.data_root())
                .env("HOME", self.root.path().join("home"))
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.root.path().join("bin").display()),
                )
                .env("HIERO_TEST_ROOT", self.root.path())
                .env("HIERO_TEST_DATA", self.config.data_root())
                .env("HIERO_TEST_BINARY", env!("CARGO_BIN_EXE_hiero"));
            command
        }
        fn manager_calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.root.path().join("manager.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
        fn wait_for(&self, predicate: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !predicate() {
                assert!(Instant::now() < deadline, "fixture deadline expired");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::write(self.root.path().join("continue"), b"");
            if hiero::lifecycle::probe(&self.config).is_live() {
                let _ = hiero::lifecycle::stop(&self.config, &self.options);
            }
        }
    }

    #[test]
    fn restart_holds_operation_across_shutdown_and_start_and_rejects_competing_stop() {
        let fixture = Fixture::new();
        let mut daemon = fixture
            .command("daemon")
            .args(["--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        fixture.wait_for(|| hiero::lifecycle::probe(&fixture.config).is_live());
        let old = hiero::daemon::discovery::read_discovery(&fixture.config).unwrap();
        let restart = fixture
            .command("restart")
            .env("HIERO_TEST_PAUSE_START", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        fixture.wait_for(|| fixture.root.path().join("start-entered").exists());
        let competing = fixture.command("stop").output().unwrap();
        let mut other_root = fixture.options.clone();
        other_root.data_root = fixture.root.path().join("other-data");
        let foreign_install = service::install(&other_root);
        std::fs::write(fixture.root.path().join("continue"), b"").unwrap();
        let output = restart.wait_with_output().unwrap();
        assert!(!competing.status.success());
        assert!(
            foreign_install
                .unwrap_err()
                .to_string()
                .contains("registration")
        );
        assert!(String::from_utf8_lossy(&competing.stderr).contains("lifecycle"));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            daemon.wait().unwrap().code(),
            Some(0),
            "explicit shutdown must suppress Restart=on-failure recovery"
        );
        let current = hiero::daemon::discovery::read_discovery(&fixture.config).unwrap();
        assert_ne!(current.instance_id, old.instance_id);
        assert_eq!(fixture.manager_calls(), ["--user start hieronymus.service"]);
        let stopped = fixture.command("stop").output().unwrap();
        assert!(
            stopped.status.success(),
            "{}",
            String::from_utf8_lossy(&stopped.stderr)
        );
        assert!(!fixture.config.data_root().join("daemon.json").exists());
        assert!(RootOwnership::acquire(&fixture.config, "assert-released").is_ok());
        assert!(
            fixture.options.unit_path().exists(),
            "quit preserves future login registration"
        );
    }

    fn assert_install_refuses_unverified_owner(unreachable: bool) {
        use hiero::daemon::discovery;
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.options.unit_path()).unwrap();
        if unreachable {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            discovery::write_token(
                &fixture.config,
                &discovery::generate_bearer_token().unwrap(),
            )
            .unwrap();
            discovery::write_discovery(
                &fixture.config,
                &discovery::DiscoveryRecord {
                    discovery_version: discovery::DISCOVERY_VERSION,
                    protocol_version: hiero::daemon::registry::PROTOCOL_REVISION.into(),
                    host: "127.0.0.1".into(),
                    port,
                    pid: std::process::id(),
                    instance_id: "ab".repeat(16),
                    started_at: "2026-09-11T00:00:00+00:00".into(),
                },
            )
            .unwrap();
            assert!(matches!(
                hiero::lifecycle::probe(&fixture.config),
                hiero::lifecycle::DiscoveryHealth::Unreachable { .. }
            ));
        }
        let owner = RootOwnership::acquire(&fixture.config, "unverifiable-daemon").unwrap();
        let install = || {
            fixture
                .command("service")
                .arg("install")
                .arg("--binary")
                .arg(&fixture.options.binary)
                .output()
                .unwrap()
        };
        let blocked = install();
        assert!(
            !blocked.status.success(),
            "installation must refuse an unverifiable owner"
        );
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("owns this data root"));
        assert!(
            !fixture.options.unit_path().exists(),
            "refusal must precede unit writes"
        );
        assert!(
            fixture.manager_calls().is_empty(),
            "refusal must precede reload/enable"
        );
        drop(owner);
        let permitted = install();
        assert!(
            permitted.status.success(),
            "{}",
            String::from_utf8_lossy(&permitted.stderr)
        );
        assert!(fixture.options.unit_path().exists());
        assert_eq!(
            fixture.manager_calls(),
            ["--user daemon-reload", "--user enable hieronymus.service"]
        );
    }

    #[test]
    fn install_refuses_owner_with_missing_discovery_then_succeeds_after_release() {
        assert_install_refuses_unverified_owner(false);
    }

    #[test]
    fn install_refuses_owner_with_unreachable_discovery_then_succeeds_after_release() {
        assert_install_refuses_unverified_owner(true);
    }

    fn assert_installation_retains_owner(start: bool) {
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.options.unit_path()).unwrap();
        let mut command = fixture.command(if start { "start" } else { "service" });
        if !start {
            command
                .arg("install")
                .arg("--binary")
                .arg(&fixture.options.binary);
        }
        let child = command
            .env("HIERO_TEST_PAUSE_INSTALL", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        fixture.wait_for(|| fixture.root.path().join("install-entered").exists());
        let competing = RootOwnership::acquire(&fixture.config, "competing-daemon");
        let blocked =
            matches!(&competing, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock);
        drop(competing);
        std::fs::write(fixture.root.path().join("continue"), b"").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            blocked,
            "offline ownership must remain held through manager enable"
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(hiero::lifecycle::probe(&fixture.config).is_live(), start);
    }

    #[test]
    fn service_installation_holds_owner_through_manager_enable() {
        assert_installation_retains_owner(false);
    }

    #[test]
    fn startup_installation_holds_owner_then_releases_it_before_start() {
        assert_installation_retains_owner(true);
    }

    #[test]
    fn install_accepts_authenticated_live_owner_without_reacquiring_it() {
        let fixture = Fixture::new();
        let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
            data_root: Some(fixture.config.data_root().to_path_buf()),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        let output = fixture
            .command("service")
            .arg("install")
            .arg("--binary")
            .arg(&fixture.options.binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(hiero::lifecycle::probe(&fixture.config).is_live());
        assert!(!hiero::lifecycle::root_is_released(&fixture.config).unwrap());
        assert_eq!(
            fixture.manager_calls(),
            ["--user daemon-reload", "--user enable hieronymus.service"]
        );
        daemon.shutdown().unwrap();
    }

    #[test]
    fn install_and_uninstall_issue_only_the_verified_manager_commands() {
        let fixture = Fixture::new();
        std::fs::remove_file(fixture.options.unit_path()).unwrap();
        let installed = fixture
            .command("service")
            .arg("install")
            .arg("--binary")
            .arg(&fixture.options.binary)
            .output()
            .unwrap();
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );
        assert_eq!(
            fixture.manager_calls(),
            ["--user daemon-reload", "--user enable hieronymus.service"]
        );
        let removed = fixture
            .command("service")
            .arg("uninstall")
            .output()
            .unwrap();
        assert!(
            removed.status.success(),
            "{}",
            String::from_utf8_lossy(&removed.stderr)
        );
        assert_eq!(
            fixture.manager_calls(),
            [
                "--user daemon-reload",
                "--user enable hieronymus.service",
                "--user stop hieronymus.service",
                "--user daemon-reload"
            ]
        );
        assert!(!fixture.options.unit_path().exists());
        assert!(
            fixture
                .options
                .unit_dir
                .join(".hieronymus.service.lock")
                .exists()
        );
    }

    #[test]
    fn systemd_manager_child() {
        let Ok(root) = std::env::var("HIERO_TEST_MANAGER_DATA") else {
            return;
        };
        use hiero::service::ServiceManager;
        let config = HieronymusConfig::new(root);
        let operation = hiero::lifecycle::operation::LifecycleOperation::acquire(&config).unwrap();
        let options = hiero::lifecycle::default_service_options(&config).unwrap();
        let manager = service::SystemdManager::new(options, &operation);
        manager.stop().unwrap();
        manager.reload().unwrap();
        manager.start().unwrap();
    }

    #[test]
    fn systemd_primitives_use_the_existing_guard_without_recursive_acquisition() {
        let fixture = Fixture::new();
        let template = fixture.command("status");
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (key, value) in template.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            }
        }
        let output = command
            .args(["--exact", "managed::systemd_manager_child", "--nocapture"])
            .env("HIERO_TEST_MANAGER_DATA", fixture.config.data_root())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        fixture.wait_for(|| hiero::lifecycle::probe(&fixture.config).is_live());
        assert_eq!(
            fixture.manager_calls(),
            [
                "--user stop hieronymus.service",
                "--user daemon-reload",
                "--user start hieronymus.service"
            ]
        );
    }

    #[test]
    fn failed_start_is_reported_once_and_status_never_starts_a_daemon() {
        let fixture = Fixture::new();
        let start = fixture
            .command("start")
            .env("HIERO_TEST_START_FAIL", "1")
            .output()
            .unwrap();
        assert!(!start.status.success());
        for _ in 0..3 {
            let status = fixture.command("status").arg("--json").output().unwrap();
            let body: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
            assert_eq!(body["running"], false);
        }
        assert_eq!(fixture.manager_calls(), ["--user start hieronymus.service"]);
        assert!(!fixture.config.data_root().join("daemon.json").exists());
    }

    #[test]
    fn authentication_failure_never_falls_back_to_manager_stop_or_discovery_repair() {
        let fixture = Fixture::new();
        let mut daemon = fixture
            .command("daemon")
            .args(["--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        fixture.wait_for(|| hiero::lifecycle::probe(&fixture.config).is_live());
        let token_path = fixture.config.data_root().join("daemon.token");
        let token = std::fs::read(&token_path).unwrap();
        let discovery_path = fixture.config.data_root().join("daemon.json");
        let record = std::fs::read(&discovery_path).unwrap();
        std::fs::write(&token_path, "wrong-installation-token").unwrap();
        let stop = fixture.command("stop").output().unwrap();
        let start = fixture.command("start").output().unwrap();
        std::fs::write(&token_path, token).unwrap();
        assert!(!stop.status.success());
        assert!(!start.status.success());
        assert!(fixture.manager_calls().is_empty());
        assert_eq!(std::fs::read(discovery_path).unwrap(), record);
        assert!(hiero::lifecycle::probe(&fixture.config).is_live());
        assert!(fixture.command("stop").output().unwrap().status.success());
        assert_eq!(daemon.wait().unwrap().code(), Some(0));
    }

    #[test]
    fn restart_stop_timeout_never_starts_or_forces_the_daemon() {
        let fixture = Fixture::new();
        // Keep the actual daemon handle alive without driving its shutdown
        // join: it accepts POST /shutdown, but ownership remains held. This
        // exercises the production 30-second timeout with real admission.
        let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
            data_root: Some(fixture.config.data_root().to_path_buf()),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        let started = Instant::now();
        let output = fixture.command("restart").output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("did not stop within"));
        assert!(started.elapsed() >= Duration::from_secs(30));
        assert!(started.elapsed() < Duration::from_secs(40));
        assert!(
            fixture.manager_calls().is_empty(),
            "failed stop aborts restart without force or start"
        );
        assert!(fixture.options.unit_path().exists());
        assert!(!hiero::lifecycle::root_is_released(&fixture.config).unwrap());
        daemon.shutdown().unwrap();
        assert!(hiero::lifecycle::root_is_released(&fixture.config).unwrap());
    }

    #[test]
    fn mismatched_unit_refuses_start_stop_restart_and_uninstall_before_manager_action() {
        let fixture = Fixture::new();
        let foreign = service::render_unit(
            &fixture.options.binary,
            &fixture.root.path().join("foreign-data"),
        )
        .unwrap();
        std::fs::write(fixture.options.unit_path(), &foreign).unwrap();
        for action in ["start", "stop", "restart"] {
            let output = fixture.command(action).output().unwrap();
            assert!(!output.status.success(), "{action}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("unit serves data root"));
        }
        let output = fixture
            .command("service")
            .arg("uninstall")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            std::fs::read_to_string(fixture.options.unit_path()).unwrap(),
            foreign
        );
        assert!(fixture.manager_calls().is_empty());
    }
}

#[test]
fn uninstall_preserves_the_lock_inodes_when_data_root_is_inside_application_directory() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    std::fs::create_dir_all(app.join("versions")).unwrap();
    let config = HieronymusConfig::new(app.join("data"));
    std::fs::create_dir_all(config.data_root()).unwrap();
    std::fs::write(config.database_path(), b"user contents").unwrap();
    let result = hiero::uninstall::run_uninstall(&hiero::uninstall::UninstallOptions {
        app_dir: Some(app.clone()),
        data_root: Some(config.data_root().to_path_buf()),
        unit_dir: Some(root.path().join("units")),
        confirmed: true,
        delete_data: true,
    });
    assert!(
        result.is_err(),
        "removing the enclosing application would unlink held locks"
    );
    assert_eq!(
        std::fs::read(config.database_path()).unwrap(),
        b"user contents"
    );
    assert!(app.exists());
}

#[test]
fn two_roots_cannot_claim_one_absent_unit_while_registration_is_locked() {
    let root = tempfile::tempdir().unwrap();
    let units = root.path().join("units");
    std::fs::create_dir_all(&units).unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(units.join(".hieronymus.service.lock"))
        .unwrap();
    file.try_lock().unwrap();
    let install = |data: &str| {
        Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args(["service", "install", "--no-activate"])
            .arg("--data-root")
            .arg(root.path().join(data))
            .arg("--unit-dir")
            .arg(&units)
            .arg("--binary")
            .arg(env!("CARGO_BIN_EXE_hiero"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = install("first");
    let second = install("second");
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    file.unlock().unwrap();
    assert!(
        !first.status.success() && !second.status.success(),
        "both roots must respect the shared unit registration lock"
    );
    assert!(String::from_utf8_lossy(&first.stderr).contains("registration"));
    assert!(String::from_utf8_lossy(&second.stderr).contains("registration"));
    assert!(!units.join(service::SERVICE_UNIT_NAME).exists());
    assert!(install("first").wait().unwrap().success());
    assert!(!install("second").wait().unwrap().success());
}

#[test]
fn delete_data_refuses_to_unlink_registration_inside_the_root() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("user-data"), b"preserve on refusal").unwrap();
    let result = hiero::uninstall::run_uninstall(&hiero::uninstall::UninstallOptions {
        app_dir: Some(root.path().join("app")),
        data_root: Some(data.clone()),
        unit_dir: Some(data.join("units")),
        confirmed: true,
        delete_data: true,
    });
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("registration directory")
    );
    assert_eq!(
        std::fs::read(data.join("user-data")).unwrap(),
        b"preserve on refusal"
    );
}

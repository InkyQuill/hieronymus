//! Execute the actual macOS dispatcher on Linux with only launchctl/session and
//! unreachable outer lifecycle/broker boundaries substituted. No native calls run.
#![cfg(target_os = "linux")]
#![allow(dead_code)]
use hiero::service::*;
use hieronymus::data_root::HieronymusConfig;
struct LifecycleOperation;
impl LifecycleOperation {
    fn acquire(_: &HieronymusConfig) -> std::io::Result<Self> {
        panic!("dispatcher test must not enter outer lifecycle guards")
    }
    fn register_unit(&self, _: &ServiceOptions) -> std::io::Result<()> {
        Ok(())
    }
}
use std::{
    cell::RefCell,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
};

#[path = "../src/service/macos.rs"]
mod backend;
#[path = "../src/service/package_preflight.rs"]
mod package_preflight;
mod lifecycle {
    use super::*;
    pub fn checked_probe(_: &HieronymusConfig) -> Result<(), String> {
        panic!("dispatcher test must not probe a real daemon")
    }
    pub fn stop_guarded(
        _: &HieronymusConfig,
        _: &ServiceOptions,
        _: &LifecycleOperation,
    ) -> Result<(), String> {
        panic!("dispatcher test must not stop a real daemon")
    }
}
mod platform {
    pub mod macos_identity {
        pub fn domain() -> Result<String, String> {
            Ok("gui/501".into())
        }
    }
    pub mod macos_broker {
        use super::super::*;
        #[derive(Clone, Copy)]
        pub enum TaskAction {
            PackageCapture,
            PackageRestore,
            PackageCommit,
            Inspect,
            Install,
            InstallDesktop,
            DesktopMode,
            FinishStop,
            Reconcile,
            Remove,
            Start,
            Rearm,
            Suppress,
            EnableLogin,
            DisableLogin,
        }
        pub fn task(
            _: &ServiceOptions,
            _: TaskAction,
            _: bool,
        ) -> Result<serde_json::Value, String> {
            panic!("dispatcher test must not spawn a native broker")
        }
        pub fn task_guarded(
            options: &ServiceOptions,
            action: TaskAction,
            tray: bool,
            _: &LifecycleOperation,
        ) -> Result<serde_json::Value, String> {
            task(options, action, tray)
        }
        pub fn inspect(_: &ServiceOptions, _: bool) -> Result<serde_json::Value, String> {
            panic!("dispatcher test must not spawn a native broker")
        }
    }
}
use platform::macos_broker::TaskAction;

struct FakeLaunchd {
    options: ServiceOptions,
    loaded: bool,
    disabled: bool,
    bootstrapped_modes: Vec<macos_agent::DaemonMode>,
    kickstarts: usize,
    force_idle: bool,
}
thread_local! { static NATIVE: RefCell<Option<FakeLaunchd>> = const { RefCell::new(None) }; }
struct CommandFixture;
impl Drop for CommandFixture {
    fn drop(&mut self) {
        backend::COMMAND_OVERRIDE.set(None);
        NATIVE.with_borrow_mut(|native| *native = None);
    }
}
fn fake_command(args: &[&str]) -> Result<(std::process::ExitStatus, String), String> {
    NATIVE.with_borrow_mut(|native| {
        let native = native.as_mut().expect("fixture installed before dispatcher");
        let label = macos_agent::label(&native.options.data_root, false);
        let target = format!("gui/501/{label}");
        let path = native.options.unit_dir.join(format!("{label}.plist"));
        let success = std::process::ExitStatus::from_raw(0);
        match args {
            ["print", "gui/501"] => Ok((success, String::new())),
            ["print", requested] if *requested == format!("gui/501/{}", macos_agent::label(&native.options.data_root, true)) => Ok((std::process::ExitStatus::from_raw(113 << 8), String::new())),
            ["enable" | "disable", requested] if *requested == format!("gui/501/{}", macos_agent::label(&native.options.data_root, true)) => Ok((success, String::new())),
            ["bootout", requested] if *requested == target => {
                native.loaded = false;
                native.kickstarts = 0;
                Ok((success, String::new()))
            }
            ["print", requested] if *requested == target => {
                if !native.loaded {
                    return Ok((std::process::ExitStatus::from_raw(113 << 8), String::new()));
                }
                let mode = *native.bootstrapped_modes.last().unwrap();
                let state = if !native.force_idle && (mode == macos_agent::DaemonMode::Headless || native.kickstarts > 0) {
                    "state = running\n\tpid = 1234"
                } else {
                    "state = not running"
                };
                let arguments = macos_agent::arguments(&native.options.binary, &native.options.data_root, &native.options.unit_dir, false).unwrap().join("\n\t\t");
                Ok((success, format!("{target} = {{\n\tpath = {}\n\tprogram = {}\n\targuments = {{\n\t\t{arguments}\n\t}}\n\t{state}\n\tproperties = {}\n}}", path.display(), native.options.binary.display(), if mode == macos_agent::DaemonMode::Headless { "runatload" } else { "inferred program" })))
            }
            ["print-disabled", "gui/501"] => Ok((success, format!("disabled services = {{\n\t\"{label}\" => {}\n}}", native.disabled))),
            [verb @ ("enable" | "disable"), requested] if *requested == target => {
                native.disabled = *verb == "disable";
                Ok((success, String::new()))
            }
            ["bootstrap", "gui/501", requested] if Path::new(requested) == path => {
                let definition = std::fs::read_to_string(&path).unwrap();
                native.bootstrapped_modes.push(macos_agent::owned_mode(&definition, &native.options.binary, &native.options.data_root, &native.options.unit_dir).unwrap());
                native.loaded = true;
                Ok((success, String::new()))
            }
            ["kickstart", requested] if *requested == target => {
                native.kickstarts += 1;
                Ok((success, String::new()))
            }
            _ => panic!("unexpected native command: {args:?}"),
        }
    })
}
fn recover_then(action: TaskAction, desired: macos_agent::DaemonMode) {
    let temp = tempfile::tempdir().unwrap();
    let options = ServiceOptions {
        data_root: temp.path().join("root"),
        unit_dir: temp.path().join("agents"),
        binary: std::env::current_exe().unwrap(),
        use_manager: true,
    };
    std::fs::create_dir_all(&options.data_root).unwrap();
    std::fs::create_dir_all(&options.unit_dir).unwrap();
    let path = options.unit_dir.join(format!(
        "{}.plist",
        macos_agent::label(&options.data_root, false)
    ));
    let headless = macos_agent::render_mode(
        &options.binary,
        &options.data_root,
        &options.unit_dir,
        false,
        macos_agent::DaemonMode::Headless,
    )
    .unwrap();
    let desktop = macos_agent::render_mode(
        &options.binary,
        &options.data_root,
        &options.unit_dir,
        false,
        macos_agent::DaemonMode::Desktop,
    )
    .unwrap();
    // Real on-disk interrupted standalone DesktopMode: candidate already
    // published, before snapshot still Headless and unloaded.
    std::fs::write(&path, &desktop).unwrap();
    let pending = path.with_extension("pending.json");
    std::fs::write(
        &pending,
        serde_json::to_vec(&serde_json::json!({
            "before": {"definition": headless, "disabled": false, "loaded": false},
            "after": desktop,
        }))
        .unwrap(),
    )
    .unwrap();
    NATIVE.with_borrow_mut(|native| {
        *native = Some(FakeLaunchd {
            options: options.clone(),
            loaded: false,
            disabled: false,
            bootstrapped_modes: Vec::new(),
            kickstarts: 0,
            force_idle: false,
        })
    });
    backend::COMMAND_OVERRIDE.set(Some(fake_command));
    let _fixture = CommandFixture;
    let result = backend::execute(&options, action, false).unwrap();
    let actual = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        macos_agent::owned_mode(
            &actual,
            &options.binary,
            &options.data_root,
            &options.unit_dir
        )
        .unwrap(),
        desired
    );
    assert_eq!(result["mode"], serde_json::to_value(desired).unwrap());
    assert_eq!(
        result["login_enabled"],
        desired == macos_agent::DaemonMode::Headless
    );
    assert!(!pending.exists());
    NATIVE.with_borrow(|native| {
        let native = native.as_ref().unwrap();
        assert!(!native.disabled);
        assert_eq!(native.kickstarts, 0);
        assert_eq!(
            native.bootstrapped_modes,
            if matches!(action, TaskAction::Start | TaskAction::DesktopMode) {
                vec![desired]
            } else {
                vec![]
            }
        );
    });
}
#[test]
fn interrupted_desktop_conversion_then_install_preserves_recovered_headless() {
    recover_then(TaskAction::Install, macos_agent::DaemonMode::Headless);
}
#[test]
fn interrupted_desktop_conversion_then_start_preserves_recovered_headless() {
    recover_then(TaskAction::Start, macos_agent::DaemonMode::Headless);
}
#[test]
fn interrupted_desktop_conversion_then_explicit_conversion_selects_desktop() {
    recover_then(TaskAction::DesktopMode, macos_agent::DaemonMode::Desktop);
}

fn package_rollback(mode: macos_agent::DaemonMode, running: bool) {
    let temp = tempfile::tempdir().unwrap();
    let options = ServiceOptions {
        data_root: temp.path().join("root"),
        unit_dir: temp.path().join("agents"),
        binary: std::env::current_exe().unwrap(),
        use_manager: true,
    };
    std::fs::create_dir_all(&options.data_root).unwrap();
    std::fs::create_dir_all(&options.unit_dir).unwrap();
    let path = options.unit_dir.join(format!(
        "{}.plist",
        macos_agent::label(&options.data_root, false)
    ));
    let original = macos_agent::render_mode(
        &options.binary,
        &options.data_root,
        &options.unit_dir,
        false,
        mode,
    )
    .unwrap();
    std::fs::write(&path, &original).unwrap();
    NATIVE.with_borrow_mut(|native| {
        *native = Some(FakeLaunchd {
            options: options.clone(),
            loaded: true,
            disabled: false,
            bootstrapped_modes: vec![mode],
            kickstarts: 0,
            force_idle: !running,
        })
    });
    backend::COMMAND_OVERRIDE.set(Some(fake_command));
    let _fixture = CommandFixture;
    if mode == macos_agent::DaemonMode::Headless && !running {
        assert!(
            backend::execute(&options, TaskAction::PackageCapture, true)
                .unwrap_err()
                .contains("Loaded idle Headless")
        );
        assert!(!path.with_extension("package-pending.json").exists());
        NATIVE.with_borrow(|native| assert!(native.as_ref().unwrap().loaded));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        return;
    }
    backend::execute(&options, TaskAction::PackageCapture, true).unwrap();
    // Simulate the common authenticated daemon stop completing before FinishStop.
    NATIVE.with_borrow_mut(|native| native.as_mut().unwrap().force_idle = true);
    backend::execute(&options, TaskAction::FinishStop, false).unwrap();
    NATIVE.with_borrow(|native| assert!(!native.as_ref().unwrap().loaded));
    // Fault boundary: activation failed after daemon stop. Real package dispatcher restores.
    backend::execute(&options, TaskAction::PackageRestore, true).unwrap();
    NATIVE.with_borrow(|native| {
        let native = native.as_ref().unwrap();
        assert_eq!(native.loaded, mode == macos_agent::DaemonMode::Desktop);
        assert!(!native.disabled);
        assert_eq!(native.kickstarts, 0);
        if mode == macos_agent::DaemonMode::Headless {
            assert_eq!(
                native.bootstrapped_modes.len(),
                1,
                "rollback must defer RunAtLoad to explicit Start"
            );
        }
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(path.with_extension("package-pending.json").exists());
    backend::execute(&options, TaskAction::PackageCommit, true).unwrap();
    assert!(!path.with_extension("package-pending.json").exists());
}

#[test]
fn native_registration_read_uses_existing_parent_authority() {
    let temp = tempfile::tempdir().unwrap();
    let options = ServiceOptions {
        data_root: temp.path().join("root"),
        unit_dir: temp.path().join("agents"),
        binary: std::env::current_exe().unwrap(),
        use_manager: false,
    };
    std::fs::create_dir_all(&options.data_root).unwrap();
    std::fs::create_dir_all(&options.unit_dir).unwrap();
    let text = macos_agent::render_mode(
        &options.binary,
        &options.data_root,
        &options.unit_dir,
        false,
        macos_agent::DaemonMode::Headless,
    )
    .unwrap();
    std::fs::write(options.unit_path(), text).unwrap();
    let parent = LifecycleOperation;
    // Public read_unit would enter acquire() above and panic. Guarded read must not.
    let definition = backend::read_unit_guarded(&options, &parent)
        .unwrap()
        .unwrap();
    assert_eq!(definition.binary, options.binary);
    assert_eq!(definition.data_root, options.data_root);
}

#[test]
fn package_failure_restores_loaded_idle_desktop_without_starting_daemon() {
    package_rollback(macos_agent::DaemonMode::Desktop, false);
}
#[test]
fn package_preflight_refuses_loaded_idle_headless_before_mutation() {
    package_rollback(macos_agent::DaemonMode::Headless, false);
}
#[test]
fn package_failure_defers_active_headless_reload_to_explicit_restart() {
    package_rollback(macos_agent::DaemonMode::Headless, true);
}

#[test]
fn package_capture_rejects_inner_and_composite_journals_even_when_definitions_absent() {
    for (composite, present) in [(false, false), (false, true), (true, false)] {
        let temp = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: temp.path().join("root"),
            unit_dir: temp.path().join("units"),
            binary: temp.path().join("hiero"),
            use_manager: true,
        };
        std::fs::create_dir_all(&options.data_root).unwrap();
        std::fs::create_dir_all(&options.unit_dir).unwrap();
        let path = options.unit_dir.join(format!(
            "{}.plist",
            macos_agent::label(&options.data_root, false)
        ));
        let before = macos_agent::render_mode(
            &options.binary,
            &options.data_root,
            &options.unit_dir,
            false,
            macos_agent::DaemonMode::Desktop,
        )
        .unwrap();
        if present {
            std::fs::write(&path, &before).unwrap();
        }
        let snapshot = serde_json::json!({"definition":before,"disabled":false,"loaded":false});
        let (pending, journal) = if composite {
            (
                path.with_extension("desktop-pending.json"),
                serde_json::json!({"daemon":{"definition":null,"disabled":false,"loaded":false},"tray":{"definition":null,"disabled":false,"loaded":false}}),
            )
        } else {
            (
                path.with_extension("pending.json"),
                serde_json::json!({"before":snapshot,"after":null}),
            )
        };
        let bytes = serde_json::to_vec(&journal).unwrap();
        std::fs::write(&pending, &bytes).unwrap();
        NATIVE.with_borrow_mut(|native| {
            *native = Some(FakeLaunchd {
                options: options.clone(),
                loaded: false,
                disabled: false,
                bootstrapped_modes: Vec::new(),
                kickstarts: 0,
                force_idle: false,
            })
        });
        backend::COMMAND_OVERRIDE.set(Some(fake_command));
        let _fixture = CommandFixture;
        // Passive inspection may return null, but must leave recovery untouched.
        backend::execute(&options, TaskAction::Inspect, false).unwrap();
        assert_eq!(std::fs::read(&pending).unwrap(), bytes);
        assert_eq!(path.exists(), present);
        assert!(
            backend::execute(&options, TaskAction::PackageCapture, true)
                .unwrap_err()
                .contains("recovery is pending")
        );
        assert!(!path.with_extension("package-pending.json").exists());
        assert_eq!(std::fs::read(&pending).unwrap(), bytes);
        assert_eq!(path.exists(), present);
        NATIVE.with_borrow(|native| {
            let native = native.as_ref().unwrap();
            assert!(!native.loaded && !native.disabled);
            assert!(native.bootstrapped_modes.is_empty());
            assert_eq!(native.kickstarts, 0);
        });
        // Normal explicit recovery settles first, then capture binds its exact state.
        backend::execute(&options, TaskAction::Reconcile, false).unwrap();
        assert!(!pending.exists());
        let recovered = if composite {
            None
        } else {
            Some(before.as_str())
        };
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), recovered);
        backend::execute(&options, TaskAction::PackageCapture, true).unwrap();
        let captured: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path.with_extension("package-pending.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            captured["daemon"]["definition"],
            serde_json::to_value(recovered).unwrap()
        );
        assert_eq!(captured["daemon"]["loaded"], false);
        backend::execute(&options, TaskAction::PackageCommit, true).unwrap();
    }
}

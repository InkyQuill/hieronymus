#![cfg(target_os = "linux")]
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    binary: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("app/hiero");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"fixture CLI").unwrap();
        fs::write(temp.path().join("app/hiero-desktop"), b"fixture helper").unwrap();
        Self { temp, binary }
    }
    fn command(&self, action: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hiero"));
        c.env("HOME", self.temp.path())
            .env("XDG_CONFIG_HOME", self.temp.path().join(".config"))
            .env("XDG_DATA_HOME", self.temp.path().join(".local/share"))
            .args(["desktop"])
            .args(action)
            .arg("--data-root")
            .arg(self.temp.path().join("data"))
            .arg("--binary")
            .arg(&self.binary)
            .arg("--unit-dir")
            .arg(self.temp.path().join("units"))
            .arg("--no-activate");
        c
    }
    fn run(&self, action: &[&str]) -> Output {
        self.command(action).output().unwrap()
    }
    fn entry(&self, login: bool) -> PathBuf {
        self.temp.path().join(if login {
            ".config/autostart/hieronymus.desktop"
        } else {
            ".local/share/applications/hieronymus.desktop"
        })
    }
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn install_toggle_reinstall_and_unregister_preserve_on_demand_unit_and_data() {
    let f = Fixture::new();
    success(f.run(&["install"]));
    let entry = fs::read_to_string(f.entry(true)).unwrap();
    assert!(entry.contains("Type=Application\nName=Hieronymus\nTerminal=false\nIcon=hieronymus\n"));
    assert!(entry.contains(&format!(
        "Exec=\"{}\" tray --data-root \"{}\"",
        f.binary.display(),
        f.temp.path().join("data").display()
    )));
    assert!(success(f.run(&["autostart", "status"])).contains("enabled"));
    success(f.run(&["autostart", "off"]));
    assert!(!f.entry(true).exists());
    assert!(success(f.run(&["autostart", "status"])).contains("disabled"));
    success(f.run(&["install"]));
    assert!(!f.entry(true).exists(), "reinstall preserves opt-out");
    success(f.run(&["autostart", "on"]));
    success(f.run(&["uninstall"]));
    success(f.run(&["uninstall"]));
    assert!(!f.entry(true).exists());
    assert!(!f.entry(false).exists());
    assert!(f.temp.path().join("units/hieronymus.service").exists());
    assert!(f.temp.path().join("data/desktop-settings.json").exists());
}

#[test]
fn foreign_or_modified_desktop_files_and_mismatched_units_are_preserved() {
    let f = Fixture::new();
    fs::create_dir_all(f.entry(false).parent().unwrap()).unwrap();
    fs::write(f.entry(false), "foreign entry").unwrap();
    assert!(!f.run(&["install"]).status.success());
    assert_eq!(fs::read_to_string(f.entry(false)).unwrap(), "foreign entry");
    assert!(!f.temp.path().join("units/hieronymus.service").exists());
    fs::remove_file(f.entry(false)).unwrap();
    success(f.run(&["install"]));
    let record = fs::read(f.temp.path().join("units/.hieronymus-desktop.json")).unwrap();
    let output = f
        .command(&["install"])
        .arg("--data-root")
        .arg(f.temp.path().join("foreign-root"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = f
        .command(&["install"])
        .arg("--binary")
        .arg(f.temp.path().join("foreign-cli"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read(f.temp.path().join("units/.hieronymus-desktop.json")).unwrap(),
        record
    );
    fs::write(f.entry(true), "modified entry").unwrap();
    assert!(!f.run(&["uninstall"]).status.success());
    assert!(f.entry(false).exists());
    assert_eq!(fs::read_to_string(f.entry(true)).unwrap(), "modified entry");
}

#[test]
fn special_paths_encode_systemd_and_desktop_arguments_and_reject_controls() {
    use hiero::{desktop::linux_entry, service};
    use std::path::Path;
    let binary = Path::new("/app space/Книга'\"\\$`%f/hiero");
    let root = Path::new("/root space/本'\"\\$`%U");
    let entry = linux_entry::render(binary, root).unwrap();
    assert!(entry.contains("%%f"));
    assert!(entry.contains("\\\\\""));
    assert!(entry.contains("\\\\\\\\"));
    assert!(entry.contains("\\\\$"));
    let unit = service::render_unit(binary, root).unwrap();
    let parsed = service::parse_unit(&unit).unwrap();
    assert_eq!(parsed.binary, binary);
    assert_eq!(parsed.data_root, root);
    for bad in ["relative", "/a\nb", "/a\rb", "/a\tb", "/a\0b"] {
        assert!(linux_entry::render(Path::new(bad), root).is_err());
        assert!(service::render_unit(Path::new(bad), root).is_err());
    }
    assert!(linux_entry::render(Path::new("/a=b/hiero"), root).is_err());
    use std::os::unix::ffi::OsStringExt;
    let invalid = PathBuf::from(std::ffi::OsString::from_vec(b"/bad\xff".to_vec()));
    assert!(linux_entry::render(&invalid, root).is_err());
}

#[test]
fn no_activate_removes_only_owned_old_login_link_without_manager_or_stop() {
    use hiero::service;
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let unit = f.temp.path().join("units/hieronymus.service");
    let link = f
        .temp
        .path()
        .join("units/default.target.wants/hieronymus.service");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(
        &unit,
        service::render_unit(&f.binary, &f.temp.path().join("data")).unwrap(),
    )
    .unwrap();
    symlink("../hieronymus.service", &link).unwrap();
    success(f.run(&["install"]));
    assert!(!link.exists());
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(f.temp.path().join("units/.hieronymus-desktop.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["prior_login_link"], "../hieronymus.service");
    assert!(
        record["prior_unit"]
            .as_str()
            .unwrap()
            .contains("ExecStart=")
    );
    success(f.run(&["autostart", "off"]));
    assert!(unit.exists());
}

#[test]
fn manager_failure_does_not_claim_toggle_success_and_never_stops_daemon() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let fake_bin = f.temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    let systemctl = fake_bin.join("systemctl");
    fs::write(&systemctl, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/manager.log\"\n[ ! -e \"$HOME/fail-manager\" ]\n").unwrap();
    fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o700)).unwrap();
    let invoke = |action: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hiero"));
        c.env("HOME", f.temp.path())
            .env("XDG_CONFIG_HOME", f.temp.path().join(".config"))
            .env("XDG_DATA_HOME", f.temp.path().join(".local/share"))
            .env("PATH", &fake_bin)
            .arg("desktop")
            .args(action)
            .arg("--data-root")
            .arg(f.temp.path().join("data"))
            .arg("--binary")
            .arg(&f.binary);
        c.output().unwrap()
    };
    success(invoke(&["install"]));
    let record_path = f
        .temp
        .path()
        .join(".config/systemd/user/.hieronymus-desktop.json");
    assert!(record_path.exists());
    fs::write(f.temp.path().join("fail-manager"), "").unwrap();
    assert!(!invoke(&["autostart", "off"]).status.success());
    assert!(success(invoke(&["autostart", "status"])).contains("enabled"));
    fs::remove_file(f.temp.path().join("fail-manager")).unwrap();
    success(invoke(&["autostart", "off"]));
    fs::write(f.temp.path().join("fail-manager"), "").unwrap();
    assert!(!invoke(&["autostart", "on"]).status.success());
    assert!(success(invoke(&["autostart", "status"])).contains("disabled"));
    let log = fs::read_to_string(f.temp.path().join("manager.log")).unwrap();
    assert!(log.contains("--user daemon-reload\n"));
    assert!(log.contains("--user disable hieronymus.service\n"));
    for bad in [" enable ", " stop ", " start ", "--now"] {
        assert!(!log.contains(bad), "{log}");
    }
}

#[test]
fn mismatched_old_daemon_binary_and_foreign_login_link_are_never_reconciled() {
    use hiero::service;
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let unit = f.temp.path().join("units/hieronymus.service");
    fs::create_dir_all(unit.parent().unwrap()).unwrap();
    let foreign = service::render_unit(
        &f.temp.path().join("other-binary"),
        &f.temp.path().join("data"),
    )
    .unwrap();
    fs::write(&unit, &foreign).unwrap();
    assert!(!f.run(&["install"]).status.success());
    assert_eq!(fs::read_to_string(&unit).unwrap(), foreign);
    fs::write(
        &unit,
        service::render_unit(&f.binary, &f.temp.path().join("data")).unwrap(),
    )
    .unwrap();
    let link = f
        .temp
        .path()
        .join("units/default.target.wants/hieronymus.service");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    let other = f.temp.path().join("foreign.service");
    fs::write(&other, "foreign").unwrap();
    symlink(&other, &link).unwrap();
    assert!(!f.run(&["install"]).status.success());
    assert_eq!(fs::read_link(&link).unwrap(), other);
    assert!(!f.entry(true).exists());
}

#[test]
fn full_uninstall_removes_recorded_desktop_files_and_preserves_preferences() {
    let f = Fixture::new();
    fs::create_dir_all(f.temp.path().join("app/versions")).unwrap();
    success(f.run(&["install"]));
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .env("HOME", f.temp.path())
        .args(["uninstall", "--yes"])
        .arg("--app-dir")
        .arg(f.temp.path().join("app"))
        .arg("--data-root")
        .arg(f.temp.path().join("data"))
        .arg("--unit-dir")
        .arg(f.temp.path().join("units"))
        .output()
        .unwrap();
    success(output);
    assert!(!f.entry(true).exists());
    assert!(!f.entry(false).exists());
    assert!(!f.temp.path().join("units/hieronymus.service").exists());
    assert!(f.temp.path().join("data/desktop-settings.json").exists());
}

#[test]
fn failed_initial_manager_install_can_retry_without_losing_default_autostart() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let fake_bin = f.temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    let systemctl = fake_bin.join("systemctl");
    fs::write(&systemctl, "#!/bin/sh\n[ ! -e \"$HOME/fail-manager\" ]\n").unwrap();
    fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o700)).unwrap();
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_hiero"))
            .env("HOME", f.temp.path())
            .env("XDG_CONFIG_HOME", f.temp.path().join(".config"))
            .env("XDG_DATA_HOME", f.temp.path().join(".local/share"))
            .env("PATH", &fake_bin)
            .args(["desktop", "install"])
            .arg("--data-root")
            .arg(f.temp.path().join("data"))
            .arg("--binary")
            .arg(&f.binary)
            .output()
            .unwrap()
    };
    fs::write(f.temp.path().join("fail-manager"), "").unwrap();
    assert!(!invoke().status.success());
    fs::remove_file(f.temp.path().join("fail-manager")).unwrap();
    success(invoke());
    assert!(
        f.entry(true).exists(),
        "retry must finish default login registration"
    );
}

/// Exercises GLib's real desktop parser and launches only an inert argv recorder.
#[test]
#[ignore = "requires Linux gio and desktop-file-validate; uses disposable HOME and inert executable"]
fn gio_desktop_entry_roundtrips_literal_special_path_argv() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("app space/Книга'\"\\$`%f/hiero");
    let root = temp.path().join("root space/本'\"\\$`%U");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$ARGV_FILE\"\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let entry = temp.path().join("hieronymus.desktop");
    fs::write(
        &entry,
        hiero::desktop::linux_entry::render(&binary, &root).unwrap(),
    )
    .unwrap();
    success(
        Command::new("desktop-file-validate")
            .arg(&entry)
            .output()
            .expect("desktop-file-validate required"),
    );
    let output = temp.path().join("argv");
    success(
        Command::new("gio")
            .args(["launch"])
            .arg(&entry)
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_DATA_HOME", temp.path().join("share"))
            .env("ARGV_FILE", &output)
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .output()
            .expect("gio required"),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !output.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let actual = fs::read(&output).unwrap();
    let expected = format!("tray\0--data-root\0{}\0", root.display());
    assert_eq!(actual, expected.as_bytes());
}

#[test]
fn unit_parser_unwraps_only_the_fixed_env_dispatcher_and_absolute_owned_paths() {
    use hiero::service::parse_unit;
    let valid = "ExecStart=\"/usr/bin/env\" -- \"/app/hiero\" daemon --data-root \"/root\"\n";
    assert_eq!(
        parse_unit(valid).unwrap().binary,
        PathBuf::from("/app/hiero")
    );
    for bad in [
        "ExecStart=/usr/bin/env PATH=/other /app/hiero daemon --data-root /root",
        "ExecStart=/usr/bin/env -i /app/hiero daemon --data-root /root",
        "ExecStart=/bin/env -- /app/hiero daemon --data-root /root",
        "ExecStart=/usr/bin/env -- relative daemon --data-root /root",
        "ExecStart=/usr/bin/env -- /app/hiero daemon --data-root relative",
        "ExecStart=/app/hiero daemon --data-root relative",
    ] {
        assert!(parse_unit(bad).is_err(), "{bad}");
    }
}

#[test]
#[ignore = "requires systemd-analyze; read-only verification, generators disabled, disposable paths"]
fn systemd_parser_accepts_special_executable_and_root_paths() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("app space/Книга'\"\\$`%f/hiero");
    let root = temp.path().join("root space/本'\"\\$`%U");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let unit = temp.path().join("hieronymus.service");
    fs::write(&unit, hiero::service::render_unit(&binary, &root).unwrap()).unwrap();
    success(
        Command::new("systemd-analyze")
            .args(["verify", "--man=no", "--generators=no"])
            .arg(unit)
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_DATA_HOME", temp.path().join("share"))
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .output()
            .expect("systemd-analyze required"),
    );
}

#[test]
fn installs_and_uninstalls_special_paths_through_real_cli() {
    let f = Fixture::new();
    let special_binary = f.temp.path().join("app/Книга '\"\\$`%f/hiero");
    let special_root = f.temp.path().join("data/本 '\"\\$`%U");
    fs::create_dir_all(special_binary.parent().unwrap()).unwrap();
    fs::write(&special_binary, "fixture").unwrap();
    fs::write(
        special_binary.parent().unwrap().join("hiero-desktop"),
        "fixture",
    )
    .unwrap();
    for action in ["install", "uninstall"] {
        success(
            f.command(&[action])
                .arg("--binary")
                .arg(&special_binary)
                .arg("--data-root")
                .arg(&special_root)
                .output()
                .unwrap(),
        );
    }
    assert!(!f.entry(false).exists());
    assert!(special_root.join("desktop-settings.json").exists());
}

#[test]
fn registration_respects_lifecycle_and_shared_unit_locks() {
    use hiero::lifecycle::operation::LifecycleOperation;
    let f = Fixture::new();
    let config = hieronymus::data_root::HieronymusConfig::new(f.temp.path().join("data"));
    let operation = LifecycleOperation::acquire(&config).unwrap();
    assert!(!f.run(&["install"]).status.success());
    assert!(!f.entry(true).exists());
    drop(operation);
    fs::create_dir_all(f.temp.path().join("units")).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(f.temp.path().join("units/.hieronymus.service.lock"))
        .unwrap();
    file.lock().unwrap();
    assert!(!f.run(&["install"]).status.success());
    assert!(!f.entry(true).exists());
    file.unlock().unwrap();
    success(f.run(&["install"]));
}

#[test]
fn interrupted_install_preserves_explicit_opt_out_and_no_activate_skips_default_manager() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let bin = f.temp.path().join("manager-bin");
    fs::create_dir_all(&bin).unwrap();
    symlink("/bin/false", bin.join("systemctl")).unwrap();
    let unit_dir = f.temp.path().join(".config/systemd/user");
    let install = || {
        Command::new(env!("CARGO_BIN_EXE_hiero"))
            .env("HOME", f.temp.path())
            .env("XDG_CONFIG_HOME", f.temp.path().join(".config"))
            .env("XDG_DATA_HOME", f.temp.path().join(".local/share"))
            .env("PATH", &bin)
            .args(["desktop", "install"])
            .arg("--data-root")
            .arg(f.temp.path().join("data"))
            .arg("--binary")
            .arg(&f.binary)
            .output()
            .unwrap()
    };
    assert!(!install().status.success());
    // Fixture command includes --no-activate. Even with default unit directory
    // and a failing available manager, this explicit preference change succeeds.
    success(
        f.command(&["autostart", "off"])
            .env("PATH", &bin)
            .arg("--unit-dir")
            .arg(&unit_dir)
            .output()
            .unwrap(),
    );
    fs::remove_file(bin.join("systemctl")).unwrap();
    symlink("/bin/true", bin.join("systemctl")).unwrap();
    success(install());
    assert!(
        !f.entry(true).exists(),
        "repair must preserve explicit opt-out"
    );
}

#[test]
fn env_dispatcher_rejects_absolute_assignment_operands() {
    use hiero::service::parse_unit;
    assert!(
        parse_unit("ExecStart=\"/usr/bin/env\" -- \"/a=b\" daemon --data-root \"/root\"").is_err()
    );
}

#[test]
fn env_dispatcher_renderer_rejects_assignments_without_changing_direct_units() {
    use hiero::service::{parse_unit, render_unit};
    use std::path::Path;
    for binary in ["/a=$b/hiero", "/a='b/hiero", "/a=\"b/hiero", "/a=\\b/hiero"] {
        assert!(
            render_unit(Path::new(binary), Path::new("/root")).is_err(),
            "{binary}"
        );
    }
    let binary = Path::new("/a=b/hiero");
    let root = Path::new("/root=x");
    let unit = render_unit(binary, root).unwrap();
    assert!(unit.contains("ExecStart=\"/a=b/hiero\" daemon"));
    let parsed = parse_unit(&unit).unwrap();
    assert_eq!(parsed.binary, binary);
    assert_eq!(parsed.data_root, root);
}

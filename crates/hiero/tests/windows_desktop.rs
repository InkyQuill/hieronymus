//! Portable definition checks; native registration tests below require Windows.
use hiero::service::windows_task;
#[test]
fn daemon_and_login_tasks_have_separate_triggers_and_safe_settings() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("app & space/hiero.exe");
    let root = temp.path().join("root & space");
    let daemon =
        windows_task::render(&binary, &root, "S-1-5-21-1-2-3-1001", false, true, true).unwrap();
    let tray =
        windows_task::render(&binary, &root, "S-1-5-21-1-2-3-1001", true, true, false).unwrap();
    assert!(!daemon.contains("LogonTrigger"));
    assert!(tray.contains("LogonTrigger"));
    for xml in [&daemon, &tray] {
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<RunOnlyIfIdle>false</RunOnlyIfIdle>"));
        assert!(xml.contains("<AllowHardTerminate>false</AllowHardTerminate>"));
        assert!(xml.contains("app &amp; space"));
    }
    assert!(daemon.contains("<RestartOnFailure>"));
    let stopped =
        windows_task::render(&binary, &root, "S-1-5-21-1-2-3-1001", false, false, false).unwrap();
    assert!(!stopped.contains("RestartOnFailure"));
    assert!(stopped.contains("<Enabled>false</Enabled>"));
}

#[test]
fn task_readback_refuses_foreign_actions_principals_triggers_and_settings() {
    let temp = tempfile::tempdir().unwrap();
    let text = windows_task::render(
        &temp.path().join("hiero.exe"),
        temp.path(),
        "S-1-5-21-1-2-3-1001",
        false,
        true,
        true,
    )
    .unwrap();
    windows_task::equivalent(&text.replace("><", ">\n  <"), &text).unwrap();
    for changed in [
        text.replace("<RunLevel>LeastPrivilege", "<RunLevel>HighestAvailable"),
        text.replace("<ExecutionTimeLimit>PT0S", "<ExecutionTimeLimit>PT72H"),
        text.replace("<AllowHardTerminate>false", "<AllowHardTerminate>true"),
        text.replace(
            "<Triggers></Triggers>",
            "<Triggers><BootTrigger /></Triggers>",
        ),
        text.replace(
            "</Exec>",
            "</Exec><ComHandler><ClassId>other</ClassId></ComHandler>",
        ),
        text.replace("S-1-5-21-1-2-3-1001", "S-1-5-21-4-5-6-1001"),
        text.replace("<Arguments>daemon", "<Arguments>foreign"),
        text.replace("</Command>", " </Command>"),
        text.replace(
            "</Settings>",
            "<UnknownConstraint>true</UnknownConstraint></Settings>",
        ),
    ] {
        assert!(windows_task::equivalent(&changed, &text).is_err());
    }
    assert_ne!(
        windows_task::name(temp.path(), "S-1-5-21-1", false),
        windows_task::name(temp.path(), "S-1-5-21-2", false)
    );
}

#[test]
fn eof_or_incomplete_commit_never_mutates_and_releases_persistent_gates() {
    use hiero::platform::{native_gate, native_protocol};
    use std::sync::atomic::{AtomicBool, Ordering};
    for input in ["", "COMMIT", "COMMIT\r\n", "NO\n", "COMMIT\nextra"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(native_gate::MANAGER_GATE);
        let gate = native_gate::acquire(directory.path(), native_gate::MANAGER_GATE).unwrap();
        let called = AtomicBool::new(false);
        let result = native_protocol::committed(
            &mut std::io::Cursor::new(input),
            &mut Vec::new(),
            gate,
            || {
                called.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        // Exact first-line COMMIT is the only authorization; later input is irrelevant.
        assert_eq!(called.load(Ordering::SeqCst), input == "COMMIT\nextra");
        assert_eq!(result.is_ok(), input == "COMMIT\nextra");
        native_gate::check(directory.path()).unwrap();
        assert!(path.is_file());
    }
}

#[test]
fn committed_work_retains_both_gates_through_rollback() {
    use hiero::platform::{native_gate, native_protocol};
    use std::sync::mpsc;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    let units = directory.path().join("units");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&units).unwrap();
    let (began_tx, began_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let thread_root = root.clone();
    let thread_units = units.clone();
    let worker = std::thread::spawn(move || {
        let guards = (
            native_gate::acquire(&thread_root, native_gate::MANAGER_GATE).unwrap(),
            native_gate::acquire(&thread_units, native_gate::MANAGER_GATE).unwrap(),
        );
        native_protocol::committed(
            &mut std::io::Cursor::new("COMMIT\n"),
            &mut Vec::new(),
            guards,
            || {
                std::fs::write(thread_root.join("transaction"), "pending").unwrap();
                began_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                // A failed native call restores its earlier state while retaining both gates.
                std::fs::remove_file(thread_root.join("transaction")).unwrap();
                Err::<(), _>("registration rolled back".into())
            },
        )
    });
    began_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        native_gate::check(&root).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        native_gate::check(&units).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    #[cfg(windows)]
    assert_eq!(
        hiero::lifecycle::operation::LifecycleOperation::acquire(
            &hieronymus::data_root::HieronymusConfig::new(&root)
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(root.join("transaction").exists());
    finish_tx.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    native_gate::check(&root).unwrap();
    native_gate::check(&units).unwrap();
    assert!(!root.join("transaction").exists());
}

#[cfg(windows)]
#[test]
fn native_disposable_tasks_register_inspect_toggle_and_remove_through_cli_broker() {
    use hiero::{
        desktop::{AutostartRegistration, windows_registration::WindowsRegistration},
        service::{self, ServiceOptions},
    };
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("hiero.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_hiero"), &binary).unwrap();
    let options = ServiceOptions {
        data_root: temp.path().join("root"),
        unit_dir: temp.path().join("tasks"),
        binary,
        use_manager: true,
    };
    struct Cleanup(ServiceOptions);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let mut registration = WindowsRegistration::new(self.0.clone());
            let _ = registration.uninstall();
        }
    }
    let _cleanup = Cleanup(options.clone());
    let mut registration = WindowsRegistration::new(options.clone());
    registration.install().unwrap();
    assert!(service::status(&options).unwrap().consistent());
    assert!(registration.is_enabled().unwrap());
    registration.set_enabled(false).unwrap();
    assert!(!registration.is_enabled().unwrap());
    registration.set_enabled(true).unwrap();
    assert!(registration.is_enabled().unwrap());
    // Stop suppresses recovery while preserving login; these tasks were never run.
    service::stop(&options).unwrap();
    assert!(registration.is_enabled().unwrap());
    let daemon = hiero::platform::windows_broker::inspect(&options, false).unwrap();
    assert_eq!(daemon["enabled"], false);
    assert_eq!(daemon["recovery"], false);
    registration.uninstall().unwrap();
    assert!(!registration.is_enabled().unwrap());
    assert!(service::read_unit(&options).unwrap().is_none());
}

//! Behavior ported from `tests/test_dream_locks.py`: one nonblocking OS lock
//! per dream cycle, a readable state file with the recorded owner, and
//! conservative stale-state cleanup. The Python `wait` mode is intentionally
//! not ported (the Rust dreaming spec forbids wait/retry).

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_locks::{
    DreamCycleState, DreamLockError, dream_cycle_lock, dream_cycle_paths, read_dream_cycle_state,
    remove_state_if_unchanged,
};

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

#[test]
fn dream_cycle_lock_acquires_and_releases() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);

    {
        let lock = dream_cycle_lock(&config, "manual").unwrap();
        assert_eq!(lock.state().owner, "manual");
        assert_eq!(lock.state().pid, std::process::id() as i32);
        let observed = read_dream_cycle_state(&config).unwrap();
        assert_eq!(observed.owner, "manual");
    }

    assert!(read_dream_cycle_state(&config).is_none());
}

#[test]
fn second_dream_cycle_lock_fails_while_active() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);

    let _outer = dream_cycle_lock(&config, "manual").unwrap();
    let error = dream_cycle_lock(&config, "autostart").unwrap_err();

    match &error {
        DreamLockError::AlreadyRunning { state } => {
            let state = state.as_ref().expect("conflict records the owner state");
            assert_eq!(state.owner, "manual");
        }
        other => panic!("expected AlreadyRunning, got {other:?}"),
    }
    assert!(
        error.to_string().contains("dream cycle already running"),
        "{error}"
    );
}

#[test]
fn dream_cycle_lock_releases_after_exception() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);

    let outcome: Result<(), DreamLockError> = (|| {
        let _lock = dream_cycle_lock(&config, "manual")?;
        Err(DreamLockError::Io(std::io::Error::other("provider failed")))
    })();
    assert!(outcome.is_err());

    let lock = dream_cycle_lock(&config, "manual").unwrap();
    assert_eq!(lock.state().owner, "manual");
}

#[test]
fn lock_file_try_lock_is_exclusive_across_handles() {
    // The dream lock must rely on kernel file locking (which is what keeps
    // two processes from overlapping), not on the state file or in-process
    // state. A second open file description must be unable to lock.
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    std::fs::create_dir_all(&paths.config_root).unwrap();

    {
        let _lock = dream_cycle_lock(&config, "manual").unwrap();
        let second = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&paths.lock_file)
            .unwrap();
        match second.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => {
                // The kernel refused the second exclusive lock: exactly the
                // cross-process semantics the dream cycle relies on.
            }
            Err(std::fs::TryLockError::Error(error)) => {
                panic!("unexpected lock error: {error}");
            }
            Ok(()) => panic!("second handle acquired the dream lock while it was held"),
        }
    }

    let after = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.lock_file)
        .unwrap();
    assert!(after.try_lock().is_ok(), "lock must be released on drop");
}

#[test]
fn stale_state_with_dead_pid_is_cleaned_conservatively() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    std::fs::create_dir_all(&paths.config_root).unwrap();
    std::fs::write(
        &paths.state_json,
        r#"{"owner":"manual","pid":-1,"started_at":"2026-06-09T00:00:00+00:00","token":"stale"}"#,
    )
    .unwrap();

    assert!(read_dream_cycle_state(&config).is_none());
    assert!(!paths.state_json.exists());
}

#[test]
fn stale_cleanup_does_not_remove_replaced_state() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    std::fs::create_dir_all(&paths.config_root).unwrap();
    let fresh = DreamCycleState {
        owner: "autostart".to_string(),
        pid: std::process::id() as i32,
        started_at: "2026-06-10T00:00:00+00:00".to_string(),
        token: "fresh".to_string(),
    };
    serde_json::to_writer(std::fs::File::create(&paths.state_json).unwrap(), &fresh).unwrap();

    // A cleanup that only knows the stale token must leave the live state.
    let stale = DreamCycleState {
        owner: "manual".to_string(),
        pid: -1,
        started_at: "2026-06-09T00:00:00+00:00".to_string(),
        token: "stale".to_string(),
    };
    remove_state_if_unchanged(&config, &stale);
    let on_disk: DreamCycleState =
        serde_json::from_str(&std::fs::read_to_string(&paths.state_json).unwrap()).unwrap();
    assert_eq!(on_disk.token, "fresh");

    // Knowing the exact live state removes it (used by the lock guard on drop).
    remove_state_if_unchanged(&config, &fresh);
    assert!(!paths.state_json.exists());
}

#[test]
fn reused_live_pid_does_not_keep_an_unlocked_crashed_run_alive() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    std::fs::create_dir_all(&paths.config_root).unwrap();
    let state = DreamCycleState {
        owner: "crashed".into(),
        pid: std::process::id() as i32,
        started_at: "old-run".into(),
        token: "old-instance".into(),
    };
    std::fs::write(&paths.state_json, serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(read_dream_cycle_state(&config).is_none());
    assert!(!paths.state_json.exists());
}

#[test]
fn cleanup_cannot_remove_state_while_the_kernel_lock_is_held() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let lock = dream_cycle_lock(&config, "manual").unwrap();
    remove_state_if_unchanged(&config, lock.state());
    assert_eq!(read_dream_cycle_state(&config).as_ref(), Some(lock.state()));
}

#[test]
fn crashed_process_releases_kernel_ownership_without_running_drop() {
    const FIXTURE: &str = "HIERO_TEST_DREAM_CRASH_ROOT";
    if let Some(path) = std::env::var_os(FIXTURE) {
        let config = HieronymusConfig::new(path);
        let _guard = dream_cycle_lock(&config, "crash-fixture").unwrap();
        std::process::exit(0); // Deliberately bypass destructors, like a crashed owner.
    }
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crashed_process_releases_kernel_ownership_without_running_drop",
        ])
        .env(FIXTURE, config.data_root())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(dream_cycle_paths(&config).state_json.exists());
    assert!(read_dream_cycle_state(&config).is_none());
    let _successor = dream_cycle_lock(&config, "successor").unwrap();
}

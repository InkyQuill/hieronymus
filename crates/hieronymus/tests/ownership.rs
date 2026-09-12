//! The one shared data-root ownership guard (ADR 0009): a nonblocking
//! OS advisory lock on `data-root/.owner.lock`, held for the whole critical
//! section and released on drop. The daemon, `hiero migrate`, and
//! `hiero recover` all take it, so a root has exactly one owner at a time.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hieronymus::{data_root::HieronymusConfig, ownership::RootOwnership};

#[test]
fn different_handles_cannot_own_the_same_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let first = RootOwnership::acquire(&config, "daemon").unwrap();
    assert!(RootOwnership::acquire(&config, "migrate").is_err());
    drop(first);
    assert!(RootOwnership::acquire(&config, "recover").is_ok());
}

#[test]
fn independent_data_roots_are_owned_independently() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let first =
        RootOwnership::acquire(&HieronymusConfig::new(first_root.path()), "daemon").unwrap();
    let second =
        RootOwnership::acquire(&HieronymusConfig::new(second_root.path()), "daemon").unwrap();
    drop((first, second));
}

#[test]
fn a_contended_acquire_names_the_current_owner() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let _held = RootOwnership::acquire(&config, "daemon").unwrap();
    let error = RootOwnership::acquire(&config, "migrate").unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("owns this data root"),
        "diagnostic must name the current owner: {message}"
    );
    // Windows denies reads through another handle while the range is locked.
    #[cfg(not(windows))]
    assert!(message.contains("daemon"), "{message}");
}

#[test]
fn the_lock_inode_is_never_unlinked() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let lock_path = root.path().join(hieronymus::ownership::OWNER_LOCK_FILE);
    {
        let _held = RootOwnership::acquire(&config, "daemon").unwrap();
        assert!(lock_path.exists());
    }
    // Drop releases the OS lock but keeps the inode: unlinking it would break
    // other holders' flock.
    assert!(lock_path.exists());
    assert!(RootOwnership::acquire(&config, "migrate").is_ok());
}

// ---------------------------------------------------------------------------
// Subprocess crash: a SIGKILLed holder's OS lock is released by the kernel,
// so the next acquire succeeds. This is the guarantee the upgrade protocol
// leans on after a crashed run (there is no PID file to steal any more).
// ---------------------------------------------------------------------------

/// Re-exec hook: when `HIERO_OWNER_HOLD` names a data root, this "test" is a
/// subprocess that acquires ownership, signals readiness, and blocks until it
/// is killed. With the variable unset it is an immediate no-op.
#[test]
fn subprocess_ownership_holder() {
    let Ok(root) = std::env::var("HIERO_OWNER_HOLD") else {
        return;
    };
    let config = HieronymusConfig::new(&root);
    let _owner =
        RootOwnership::acquire(&config, "subprocess").expect("subprocess acquires the lock");
    std::fs::write(std::path::Path::new(&root).join(".owner-held"), b"1").unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn os_lock_is_released_when_the_holder_is_killed() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "subprocess_ownership_holder", "--nocapture"])
        .env("HIERO_OWNER_HOLD", root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let ready = root.path().join(".owner-held");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "subprocess never took the lock");
        std::thread::sleep(Duration::from_millis(20));
    }

    // While the subprocess lives, the root is owned.
    assert!(RootOwnership::acquire(&config, "contender").is_err());

    child.kill().unwrap();
    child.wait().unwrap();

    // The kernel dropped the flock with the process; reacquire now succeeds.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if RootOwnership::acquire(&config, "after-crash").is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "OS lock was not released after SIGKILL"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

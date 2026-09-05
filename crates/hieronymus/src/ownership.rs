//! One OS-backed data-root ownership guard, shared by the daemon, the
//! write-side upgrade protocol, and Rust recovery (ADR 0009: "Daemon startup
//! acquires data-root ownership… Every caller performs one nonblocking OS
//! `try_lock_exclusive`… The guard owns the open file handle for the entire
//! critical section and releases it on drop"). Offline database maintenance
//! (`hiero migrate` / `hiero recover`) must refuse to run while the daemon
//! owns the root, and two daemons must never serve one root.
//!
//! The lock is the OS advisory lock on an open handle to `.owner.lock` inside
//! the data root — nothing else. The file's bytes (`"<pid> <role>"`) are a
//! best-effort diagnostic for a human error message; a PID read from them
//! NEVER grants or denies ownership. The lock inode is never unlinked:
//! removing it would drop the `flock` every other holder depends on.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Seek, Write};
use std::path::Path;

use crate::data_root::HieronymusConfig;

/// The ownership lock file inside the data root.
pub const OWNER_LOCK_FILE: &str = ".owner.lock";

/// An acquired exclusive claim on a data root. Held for the entire critical
/// section — the daemon's whole lifetime, or a complete `run_upgrade` /
/// `run_recovery` call — and released when dropped.
#[derive(Debug)]
pub struct RootOwnership {
    file: File,
}

impl RootOwnership {
    /// Take the data root's ownership lock for `role` (`"daemon"`,
    /// `"migrate"`, `"recover"`, …). Exactly one nonblocking `try_lock`: a
    /// contended lock is an error naming the current owner, never a wait or a
    /// retry.
    pub fn acquire(config: &HieronymusConfig, role: &str) -> io::Result<Self> {
        let root = config.data_root();
        std::fs::create_dir_all(root)?;
        let path = root.join(OWNER_LOCK_FILE);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(contended_error(&path)),
            Err(TryLockError::Error(error)) => return Err(error),
        }
        // We hold the lock: refresh the advisory owner metadata.
        file.set_len(0)?;
        file.rewind()?;
        writeln!(file, "{} {}", std::process::id(), role)?;
        file.sync_all()?;
        Ok(Self { file })
    }
}

impl Drop for RootOwnership {
    fn drop(&mut self) {
        // Best-effort: the OS also releases the lock when the handle closes.
        let _ = self.file.unlock();
    }
}

/// A best-effort description of the current lock holder, for a
/// contended-lock diagnostic only. The advisory `"<pid> <role>"` line is
/// data, not authority.
fn contended_error(path: &Path) -> io::Error {
    let holder = std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let message = match holder {
        Some(holder) => {
            format!("another process owns this data root ({holder}); stop it before retrying")
        }
        None => "another process owns this data root".to_string(),
    };
    io::Error::new(io::ErrorKind::WouldBlock, message)
}

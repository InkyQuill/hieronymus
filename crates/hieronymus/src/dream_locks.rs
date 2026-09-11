//! Dream-cycle locking (port of `dream_locks.py`): one nonblocking OS-level
//! exclusive lock per data root. There is no wait/retry mode — the dreaming
//! design spec requires exactly one `try_lock` per acquisition; scheduler
//! contention records `locked` and skips, manual requests get a conflict, and
//! exclusive CLI maintenance exits with the recorded owner.
//!
//! The lock guard holds the file handle for the complete cycle and releases
//! it on drop; the recorded state file is removed only when it still belongs
//! to the same run (token + pid match), never a replacement owner's.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::data_root::HieronymusConfig;

/// Resolved dream-cycle file paths under the config root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamCyclePaths {
    pub config_root: PathBuf,
    pub lock_file: PathBuf,
    pub state_json: PathBuf,
}

/// The recorded owner of a running (or crashed) dream cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DreamCycleState {
    pub owner: String,
    pub pid: i32,
    pub started_at: String,
    pub token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DreamLockError {
    #[error("dream cycle already running{}", Self::owner_detail(state))]
    AlreadyRunning { state: Option<DreamCycleState> },
    #[error("dream cycle lock failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("dream cycle state could not be serialized: {0}")]
    State(String),
}

impl DreamLockError {
    fn owner_detail(state: &Option<DreamCycleState>) -> String {
        match state {
            Some(state) => format!(" by {} pid {}", state.owner, state.pid),
            None => String::new(),
        }
    }
}

pub fn dream_cycle_paths(config: &HieronymusConfig) -> DreamCyclePaths {
    let root = config.config_root();
    DreamCyclePaths {
        config_root: root.to_path_buf(),
        lock_file: root.join("dream-cycle.lock"),
        state_json: root.join("dream-cycle.json"),
    }
}

fn read_state_file(path: &Path) -> Option<DreamCycleState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Probe the kernel lock before deciding whether a recorded run is stale.
/// An inaccessible/contended lock is conservative evidence: keep the record.
pub fn read_dream_cycle_state(config: &HieronymusConfig) -> Option<DreamCycleState> {
    let paths = dream_cycle_paths(config);
    let state = read_state_file(&paths.state_json)?;
    let Ok(lock) = open_lock(&paths.lock_file) else {
        return Some(state);
    };
    if lock.try_lock().is_err() {
        return Some(state);
    }
    remove_record(&paths.state_json, &state);
    None
}

/// Cleanup holds kernel ownership across comparing the complete run identity
/// and deleting its record. PIDs are diagnostic and never authorize cleanup.
pub fn remove_state_if_unchanged(config: &HieronymusConfig, expected: &DreamCycleState) {
    let paths = dream_cycle_paths(config);
    let Ok(lock) = open_lock(&paths.lock_file) else {
        return;
    };
    if lock.try_lock().is_ok() {
        remove_record(&paths.state_json, expected);
    }
}

fn open_lock(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn remove_record(path: &Path, expected: &DreamCycleState) {
    if read_state_file(path).as_ref() == Some(expected) {
        let _ = std::fs::remove_file(path);
    }
}

fn new_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = format!(
        "{}:{}:{}",
        std::process::id(),
        Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_millis()),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let digest = sha2::Sha256::digest(unique.as_bytes());
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Acquire the dream-cycle lock with exactly one nonblocking exclusive
/// `try_lock` on the lock file. On conflict the error carries the recorded
/// owner state when one is readable.
pub fn dream_cycle_lock(
    config: &HieronymusConfig,
    owner: &str,
) -> Result<DreamCycleLock, DreamLockError> {
    let paths = dream_cycle_paths(config);
    std::fs::create_dir_all(&paths.config_root)?;
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.lock_file)?;
    match lock_file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(DreamLockError::AlreadyRunning {
                state: read_state_file(&paths.state_json),
            });
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }

    let state = DreamCycleState {
        owner: owner.to_string(),
        pid: std::process::id() as i32,
        started_at: Utc::now().to_rfc3339(),
        token: new_token(),
    };
    let payload =
        serde_json::to_string(&state).map_err(|error| DreamLockError::State(error.to_string()))?;
    crate::atomic::atomic_write_text(&paths.state_json, &payload)?;
    Ok(DreamCycleLock {
        lock_file,
        state,
        state_json: paths.state_json,
    })
}

/// Held for the whole dream cycle; releases the OS lock and removes the
/// recorded state (when still ours) on drop.
#[derive(Debug)]
pub struct DreamCycleLock {
    lock_file: std::fs::File,
    state: DreamCycleState,
    state_json: PathBuf,
}

impl DreamCycleLock {
    pub fn state(&self) -> &DreamCycleState {
        &self.state
    }
}

impl Drop for DreamCycleLock {
    fn drop(&mut self) {
        remove_record(&self.state_json, &self.state);
        let _ = self.lock_file.unlock();
    }
}

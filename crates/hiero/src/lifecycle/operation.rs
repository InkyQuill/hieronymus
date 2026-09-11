//! Serializes complete lifecycle orchestration, independently of daemon ownership.
//!
//! Lock order is lifecycle -> unit registration -> stop/wait -> root ownership -> release root ->
//! start/readiness. Daemons never take this lock. No coordination file may
//! be unlinked, including during explicit user-data deletion.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::service::ServiceOptions;

use hieronymus::data_root::HieronymusConfig;

/// Persistent coordination file; its contents confer no authority.
pub const LIFECYCLE_LOCK_FILE: &str = ".lifecycle.lock";

/// One nonblocking, cross-process claim for a complete lifecycle operation.
#[derive(Debug)]
pub struct LifecycleOperation {
    file: File,
    root: PathBuf,
    registration: Mutex<Option<UnitRegistration>>,
}

impl LifecycleOperation {
    /// Acquire once, before any service-manager action or offline ownership.
    pub fn acquire(config: &HieronymusConfig) -> io::Result<Self> {
        std::fs::create_dir_all(config.data_root())?;
        let root = config.data_root().canonicalize()?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LIFECYCLE_LOCK_FILE))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another lifecycle operation is in progress for this data root; retry after it finishes",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
        #[cfg(windows)]
        crate::platform::native_gate::check(&root)?;
        file.set_len(0)?;
        writeln!(file, "{} lifecycle", std::process::id())?;
        Ok(Self {
            file,
            root,
            registration: Mutex::new(None),
        })
    }

    /// Claim the globally shared Linux unit before probing/mutating it or
    /// taking root ownership. Retain the claim for this whole operation,
    /// including restart's gap between stop and start. Repeated guarded
    /// primitives check the same claim without recursively locking its file.
    pub(crate) fn register_unit(&self, options: &ServiceOptions) -> io::Result<()> {
        self.check(&HieronymusConfig::new(&options.data_root))?;
        let mut registration = self
            .registration
            .lock()
            .map_err(|_| io::Error::other("unit registration guard was poisoned"))?;
        std::fs::create_dir_all(&options.unit_dir)?;
        let unit_dir = options.unit_dir.canonicalize()?;
        #[cfg(windows)]
        crate::platform::native_gate::check(&unit_dir)?;
        if let Some(held) = registration.as_ref() {
            if held.unit_dir != unit_dir {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "lifecycle operation already holds a different unit registration",
                ));
            }
            return Ok(());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(unit_dir.join(".hieronymus.service.lock"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another operation holds this service registration; retry after it finishes",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
        *registration = Some(UnitRegistration { file, unit_dir });
        Ok(())
    }

    /// Prevent accidentally using a guard acquired for a different root.
    pub(crate) fn check(&self, config: &HieronymusConfig) -> io::Result<()> {
        if config.data_root().canonicalize()? != self.root {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lifecycle operation guard belongs to a different data root",
            ));
        }
        Ok(())
    }
}

impl Drop for LifecycleOperation {
    fn drop(&mut self) {
        drop(
            self.registration
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        let _ = self.file.unlock();
    }
}

/// A persistent inode beside the systemd unit. Its scope is registration,
/// so different data-root operations cannot race to claim one absent unit.
#[derive(Debug)]
struct UnitRegistration {
    file: File,
    unit_dir: PathBuf,
}

impl Drop for UnitRegistration {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

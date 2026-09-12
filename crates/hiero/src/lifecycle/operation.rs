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
    #[cfg(any(windows, target_os = "macos", test))]
    broker: Mutex<Option<(PathBuf, PathBuf)>>,
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
        #[cfg(any(windows, target_os = "macos"))]
        crate::platform::native_gate::check(&root)?;
        file.set_len(0)?;
        writeln!(file, "{} lifecycle", std::process::id())?;
        Ok(Self {
            file,
            root,
            registration: Mutex::new(None),
            #[cfg(any(windows, target_os = "macos", test))]
            broker: Mutex::new(None),
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
        #[cfg(any(windows, target_os = "macos"))]
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

    /// Pin the executing CLI for a verified install transaction. Registration
    /// identity remains the exact stable endpoint, which may not exist yet.
    #[cfg(any(windows, target_os = "macos", test))]
    pub(crate) fn bind_native_broker(&self, options: &ServiceOptions) -> io::Result<()> {
        self.register_unit(options)?;
        if !options.binary.is_absolute() {
            return Err(io::Error::other(
                "broker registration endpoint must be absolute",
            ));
        }
        let mut binding = self
            .broker
            .lock()
            .map_err(|_| io::Error::other("broker binding poisoned"))?;
        if binding.is_some() {
            return Err(io::Error::other(
                "broker execution is already bound for this operation",
            ));
        }
        *binding = Some((
            options.binary.clone(),
            std::env::current_exe()?.canonicalize()?,
        ));
        Ok(())
    }
    #[cfg(any(windows, target_os = "macos", test))]
    pub(crate) fn native_broker_executable(
        &self,
        options: &ServiceOptions,
    ) -> io::Result<Option<PathBuf>> {
        self.register_unit(options)?;
        let binding = self
            .broker
            .lock()
            .map_err(|_| io::Error::other("broker binding poisoned"))?;
        match binding.as_ref() {
            Some((stable, executable)) if *stable == options.binary => Ok(Some(executable.clone())),
            Some(_) => Err(io::Error::other(
                "broker binding belongs to a different stable endpoint",
            )),
            None => Ok(None),
        }
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

#[cfg(test)]
mod broker_tests {
    use super::*;
    #[test]
    fn broker_binding_survives_absent_selection_and_refuses_other_identity() {
        let root = tempfile::tempdir().unwrap();
        let options = ServiceOptions {
            data_root: root.path().join("data"),
            unit_dir: root.path().join("units"),
            binary: root.path().join("app/bin/hiero"),
            use_manager: true,
        };
        let operation =
            LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
        assert!(
            operation
                .native_broker_executable(&options)
                .unwrap()
                .is_none()
        );
        operation.bind_native_broker(&options).unwrap();
        assert!(!options.binary.exists());
        assert_eq!(
            operation
                .native_broker_executable(&options)
                .unwrap()
                .unwrap(),
            std::env::current_exe().unwrap().canonicalize().unwrap()
        );
        assert!(operation.bind_native_broker(&options).is_err());
        for altered in [
            ServiceOptions {
                binary: root.path().join("other"),
                ..options.clone()
            },
            ServiceOptions {
                data_root: root.path().to_path_buf(),
                ..options.clone()
            },
            ServiceOptions {
                unit_dir: root.path().join("other-units"),
                ..options.clone()
            },
        ] {
            assert!(operation.native_broker_executable(&altered).is_err());
        }
        drop(operation);
        let next = LifecycleOperation::acquire(&HieronymusConfig::new(&options.data_root)).unwrap();
        assert!(next.native_broker_executable(&options).unwrap().is_none());
    }
}

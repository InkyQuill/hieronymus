//! Compile-time native service backend; public lifecycle APIs remain stable.
#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;
#[cfg(not(windows))]
pub(crate) use linux::{
    disable_login_guarded, install_guarded, owned_login_link, start_guarded, stop_guarded,
    uninstall_guarded, validate_unit_root,
};
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(windows)]
pub(crate) use windows::{
    disable_login_guarded, install_guarded, owned_login_link, start_guarded, stop_guarded,
    uninstall_guarded, validate_unit_root,
};
pub mod windows_task;

use crate::lifecycle::operation::LifecycleOperation;
use hieronymus::{data_root::HieronymusConfig, ownership::RootOwnership};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Manager(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Explicit root, registration location and executable identity for one native service.
/// `use_manager=false` permits only local definition work.
pub struct ServiceOptions {
    pub data_root: PathBuf,
    pub unit_dir: PathBuf,
    pub binary: PathBuf,
    pub use_manager: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitDefinition {
    pub binary: PathBuf,
    pub data_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub unit_path: PathBuf,
    pub definition: Option<UnitDefinition>,
    /// Why the definition is not consistent with these options, if it is not.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitVerdict {
    Absent,
    Consistent,
    /// The unit is unreadable, is not one of ours, or its binary is gone.
    Broken(String),
    /// The unit serves a different data root than the one being checked.
    DataRootMismatch {
        unit_root: PathBuf,
    },
    /// The unit still execs a different binary than the running one (typical
    /// after an update that did not reinstall the unit).
    StaleBinary {
        unit_binary: PathBuf,
    },
}

impl ServiceOptions {
    pub fn unit_path(&self) -> PathBuf {
        self.unit_dir.join(SERVICE_UNIT_NAME)
    }
}

impl ServiceStatus {
    pub fn consistent(&self) -> bool {
        self.definition.is_some() && self.problems.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "unit": self.unit_path,
            "installed": self.definition.is_some(),
            "consistent": self.consistent(),
            "binary": self.definition.as_ref().map(|definition| definition.binary.clone()),
            "data_root": self.definition.as_ref().map(|definition| definition.data_root.clone()),
            "problems": self.problems,
        })
    }

    pub fn render_human(&self) -> String {
        match &self.definition {
            None => format!(
                "service unit: not installed ({} does not exist)",
                self.unit_path.display()
            ),
            Some(definition) => {
                let mut text = format!(
                    "service unit: {}\n  binary: {}\n  data root: {}",
                    self.unit_path.display(),
                    definition.binary.display(),
                    definition.data_root.display()
                );
                for problem in &self.problems {
                    text.push_str(&format!("\n  problem: {problem}"));
                }
                text
            }
        }
    }
}

/// Install idempotently without starting a daemon, under shared ownership guards.
pub fn install(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    operation.register_unit(options)?;
    let health = crate::lifecycle::checked_probe(&config)
        .map_err(|error| ServiceError::Invalid(error.to_string()))?;
    // Missing/unreachable discovery is not proof that an owner is absent.
    // Keep the offline claim through both unit publication and reload/enable;
    // an authenticated live owner already supplies the required identity.
    let _ownership = if health.is_live() {
        None
    } else {
        Some(RootOwnership::acquire(&config, "service-install")?)
    };
    install_guarded(options, &operation)
}

/// Confirm graceful daemon exit before removing the owned registration.
pub fn uninstall(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    crate::lifecycle::stop_guarded(&config, options, &operation)
        .map_err(|error| ServiceError::Invalid(error.to_string()))?;
    uninstall_guarded(options, &operation)
}

/// Read the native definition and report missing binaries or root mismatch.
pub fn status(options: &ServiceOptions) -> Result<ServiceStatus, ServiceError> {
    let definition = read_unit(options).map_err(ServiceError::Invalid)?;
    let mut problems = Vec::new();
    if let Some(definition) = &definition {
        if !definition.binary.is_file() {
            problems.push(format!(
                "unit binary does not exist: {}",
                definition.binary.display()
            ));
        }
        if !same_path(&definition.data_root, &options.data_root) {
            problems.push(format!(
                "unit serves data root {} but this root is {}",
                definition.data_root.display(),
                options.data_root.display()
            ));
        }
    }
    Ok(ServiceStatus {
        unit_path: options.unit_path(),
        definition,
        problems,
    })
}

pub fn start(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    let config = HieronymusConfig::new(&options.data_root);
    let operation = LifecycleOperation::acquire(&config)?;
    operation.register_unit(options)?;
    if !options.unit_path().exists() {
        return Err(ServiceError::Invalid(
            "no service unit; install one with `hiero service install`".into(),
        ));
    }
    crate::lifecycle::start_guarded(&config, options, &operation)
        .map_err(|error| ServiceError::Manager(error.to_string()))
}

pub fn stop(options: &ServiceOptions) -> Result<Vec<String>, ServiceError> {
    crate::lifecycle::stop(&HieronymusConfig::new(&options.data_root), options)
        .map_err(|error| ServiceError::Manager(error.to_string()))
}

pub fn check_unit(options: &ServiceOptions, current_binary: &Path) -> UnitVerdict {
    let definition = match read_unit(options) {
        Ok(Some(definition)) => definition,
        Ok(None) => return UnitVerdict::Absent,
        Err(reason) => return UnitVerdict::Broken(reason),
    };
    if !definition.binary.is_file() {
        return UnitVerdict::Broken(format!(
            "unit binary does not exist: {}",
            definition.binary.display()
        ));
    }
    if !same_path(&definition.data_root, &options.data_root) {
        return UnitVerdict::DataRootMismatch {
            unit_root: definition.data_root.clone(),
        };
    }
    #[cfg(windows)]
    let selected = crate::desktop::launch::selected_cli(&definition.binary).ok();
    #[cfg(windows)]
    let registered_binary = selected.as_deref().unwrap_or(&definition.binary);
    #[cfg(not(windows))]
    let registered_binary = &definition.binary;
    let same_binary = match (
        registered_binary.canonicalize(),
        current_binary.canonicalize(),
    ) {
        (Ok(unit), Ok(current)) => unit == current,
        _ => registered_binary == current_binary,
    };
    if !same_binary {
        return UnitVerdict::StaleBinary {
            unit_binary: definition.binary.clone(),
        };
    }
    UnitVerdict::Consistent
}

pub trait ServiceManager {
    /// Stop the managed unit (the candidate the failed activation may have
    /// started). A unit that is already stopped is still `Ok`.
    fn stop(&self) -> Result<(), ServiceError>;
    /// Re-read unit files after the on-disk unit was restored.
    fn reload(&self) -> Result<(), ServiceError>;
    /// Start the managed unit (the restored previous version).
    fn start(&self) -> Result<(), ServiceError>;
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

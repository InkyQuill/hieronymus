//! OS-specific filesystem contracts shared by the runtime entry points.
pub mod credentials;
pub(crate) mod export;
pub mod install;
pub(crate) mod managed_files;

pub mod browser;
pub mod native_gate;
pub mod native_protocol;
#[cfg(windows)]
pub mod windows_broker;
#[cfg(windows)]
pub mod windows_identity;

#[cfg(target_os = "macos")]
pub mod macos_broker;
#[cfg(target_os = "macos")]
pub mod macos_identity;

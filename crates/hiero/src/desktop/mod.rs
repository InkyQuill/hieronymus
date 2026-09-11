mod controller;
pub mod launch;
pub mod linux_cli;
pub mod linux_entry;
pub mod linux_registration;
mod settings;
mod singleton;
mod state;

pub use controller::{Controller, DesktopBackend, LifecycleBackend, PollSchedule};
pub use settings::{
    AutostartRegistration, DesktopSettings, ForegroundMode, SettingsStore, UnsupportedAutostart,
};
pub use singleton::{SingletonOutcome, TraySingleton};
pub use state::{Accent, Action, DesktopState, Event, View};

#[cfg(windows)]
pub mod windows_registration;

#[cfg(target_os = "macos")]
pub mod macos_registration;

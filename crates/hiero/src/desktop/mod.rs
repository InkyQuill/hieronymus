mod controller;
pub mod launch;
mod settings;
mod singleton;
mod state;

pub use controller::{Controller, DesktopBackend, LifecycleBackend, PollSchedule};
pub use settings::{
    AutostartRegistration, DesktopSettings, ForegroundMode, SettingsStore, UnsupportedAutostart,
};
pub use singleton::{SingletonOutcome, TraySingleton};
pub use state::{Accent, Action, DesktopState, Event, View};

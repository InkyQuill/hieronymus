//! Preferences follow actual OS registration, including repair after interruption.
use hieronymus::{atomic::atomic_write_text, data_root::HieronymusConfig};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Light/Dark select light/dark icon ink, not the panel theme. Auto lets the
/// native adapter choose contrasting ink from the panel appearance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundMode {
    #[default]
    Auto,
    Light,
    Dark,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSettings {
    pub autostart: bool,
    pub foreground: ForegroundMode,
}

/// Native adapters must perform bounded registration operations and return only
/// sanitized errors. Readback must inspect actual registration, not cached intent.
pub trait AutostartRegistration: Send + 'static {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String>;
    fn is_enabled(&mut self) -> Result<bool, String>;
}
/// An honest boundary for platforms whose registration adapter is unavailable.
pub struct UnsupportedAutostart;
impl AutostartRegistration for UnsupportedAutostart {
    fn set_enabled(&mut self, _: bool) -> Result<(), String> {
        Err("Start at login is not supported by this desktop installation".into())
    }
    fn is_enabled(&mut self) -> Result<bool, String> {
        Err("Start at login is not supported by this desktop installation".into())
    }
}

pub struct SettingsStore {
    path: PathBuf,
}
impl SettingsStore {
    pub fn new(config: &HieronymusConfig) -> Self {
        Self {
            path: config.data_root().join("desktop-settings.json"),
        }
    }
    pub fn load(&self) -> Result<DesktopSettings, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| "Desktop preferences are invalid".into())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(DesktopSettings::default())
            }
            Err(_) => Err("Could not read desktop preferences".into()),
        }
    }
    fn save(&self, settings: &DesktopSettings) -> Result<(), String> {
        let text = serde_json::to_string_pretty(settings)
            .map_err(|_| "Could not encode desktop preferences")?;
        atomic_write_text(&self.path, &text)
            .map_err(|_| "Could not save desktop preferences".into())
    }
    pub fn set_foreground(&self, foreground: ForegroundMode) -> Result<DesktopSettings, String> {
        let mut settings = self.load()?;
        settings.foreground = foreground;
        self.save(&settings)?;
        Ok(settings)
    }
    /// Readback repairs a crash between OS registration and atomic persistence.
    pub fn reconcile(
        &self,
        registration: &mut impl AutostartRegistration,
    ) -> Result<DesktopSettings, String> {
        let actual = registration.is_enabled()?;
        let mut settings = self.load()?;
        if settings.autostart != actual {
            settings.autostart = actual;
            self.save(&settings)?;
        }
        Ok(settings)
    }
    pub fn set_autostart(
        &self,
        registration: &mut impl AutostartRegistration,
        enabled: bool,
    ) -> Result<DesktopSettings, String> {
        let mut settings = self.load()?;
        let changed = registration.set_enabled(enabled);
        let actual = registration.is_enabled()?;
        changed?;
        if actual != enabled {
            return Err(
                "Start at login registration did not match the requested preference".into(),
            );
        }
        settings.autostart = actual;
        self.save(&settings)?;
        Ok(settings)
    }
}

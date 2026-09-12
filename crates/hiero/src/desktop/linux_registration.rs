//! Owned XDG registration serialized with the daemon's lifecycle and unit lock.
use super::{AutostartRegistration, SettingsStore, linux_entry};
use crate::{
    lifecycle::{self, operation::LifecycleOperation},
    service::{self, ServiceOptions},
};
use hieronymus::{
    atomic::atomic_write_text, data_root::HieronymusConfig, ownership::RootOwnership,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RECORD_NAME: &str = ".hieronymus-desktop.json";
const ENTRY_NAME: &str = "hieronymus.desktop";
const ICON: &str = include_str!("../../../../assets/icons/icon-color.svg");

#[derive(Debug, Clone)]
pub struct LinuxRegistration {
    pub service: ServiceOptions,
    pub autostart_dir: PathBuf,
    pub applications_dir: PathBuf,
    pub icons_dir: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    binary: PathBuf,
    data_root: PathBuf,
    autostart_dir: PathBuf,
    applications_dir: PathBuf,
    icons_dir: PathBuf,
    installation_complete: bool,
    initial_autostart: bool,
    prior_unit: Option<String>,
    prior_login_link: Option<PathBuf>,
}
impl LinuxRegistration {
    pub fn new(service: ServiceOptions) -> Self {
        let home = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        Self {
            service,
            autostart_dir: config.join("autostart"),
            applications_dir: data.join("applications"),
            icons_dir: data.join("icons/hicolor/scalable/apps"),
        }
    }
    fn record_path(&self) -> PathBuf {
        self.service.unit_dir.join(RECORD_NAME)
    }
    fn entry(&self, login: bool) -> PathBuf {
        if login {
            &self.autostart_dir
        } else {
            &self.applications_dir
        }
        .join(ENTRY_NAME)
    }
    fn icon(&self) -> PathBuf {
        self.icons_dir.join("hieronymus.svg")
    }
    fn text(&self) -> Result<String, String> {
        linux_entry::render(&self.service.binary, &self.service.data_root)
    }
    fn validate(&self) -> Result<(), String> {
        self.text()?;
        for path in [
            &self.service.unit_dir,
            &self.autostart_dir,
            &self.applications_dir,
            &self.icons_dir,
        ] {
            linux_entry::argument(path)?;
        }
        Ok(())
    }
    fn record(&self) -> Result<Option<Record>, String> {
        match read_record(&self.record_path())? {
            Some(record) => {
                if record.version != 1
                    || record.binary != self.service.binary
                    || record.data_root != self.service.data_root
                    || record.autostart_dir != self.autostart_dir
                    || record.applications_dir != self.applications_dir
                    || record.icons_dir != self.icons_dir
                {
                    return Err("Desktop registration belongs to another binary, root, or directory; use that installation to unregister it first".into());
                }
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }
    fn check_files(&self, owned: bool) -> Result<(), String> {
        for (path, expected) in [
            (self.entry(true), self.text()?),
            (self.entry(false), self.text()?),
            (self.icon(), ICON.into()),
        ] {
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if owned && meta.is_file() && meta.len() == expected.len() as u64 && std::fs::read_to_string(&path).ok().as_deref() == Some(&expected) => {},
                Ok(_) => return Err("Desktop registration file is foreign or modified; move it aside before retrying".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err("Could not inspect desktop registration files".into()),
            }
        }
        Ok(())
    }
    fn check_unit(&self) -> Result<(), String> {
        if let Ok(meta) = std::fs::symlink_metadata(self.service.unit_path())
            && (!meta.is_file() || meta.len() > 65536)
        {
            return Err(
                "Daemon registration is not a bounded regular file owned by this installer".into(),
            );
        }
        if let Some(unit) = service::read_unit(&self.service)? {
            if unit.binary != self.service.binary || unit.data_root != self.service.data_root {
                return Err("Daemon registration belongs to another binary or data root; unregister that installation first".into());
            }
            let text = std::fs::read_to_string(self.service.unit_path())
                .map_err(|_| "Could not read daemon registration")?;
            if text
                != service::render_unit(&self.service.binary, &self.service.data_root)
                    .map_err(|e| e.to_string())?
            {
                return Err("Daemon registration is not owned by this installer".into());
            }
        }
        Ok(())
    }
    fn operation(&self) -> Result<LifecycleOperation, String> {
        self.validate()?;
        let op = LifecycleOperation::acquire(&HieronymusConfig::new(&self.service.data_root))
            .map_err(|e| e.to_string())?;
        op.register_unit(&self.service).map_err(|e| e.to_string())?;
        Ok(op)
    }
    fn ownership(&self) -> Result<Option<RootOwnership>, String> {
        let config = HieronymusConfig::new(&self.service.data_root);
        if lifecycle::checked_probe(&config)
            .map_err(|_| "Could not verify daemon ownership")?
            .is_live()
        {
            Ok(None)
        } else {
            RootOwnership::acquire(&config, "desktop-registration")
                .map(Some)
                .map_err(|_| "Data root is owned by another process; retry after it stops".into())
        }
    }
    /// Installs future-login startup by default, preserving an existing opt-out.
    /// This operation never launches the helper or stops/starts the daemon.
    pub fn install(&mut self) -> Result<(), String> {
        let op = self.operation()?;
        let _owner = self.ownership()?;
        self.install_guarded(&op)?;
        drop(_owner);
        drop(op);
        SettingsStore::new(&HieronymusConfig::new(&self.service.data_root))
            .reconcile(self)
            .map(|_| ())
    }
    pub(crate) fn install_guarded(&mut self, op: &LifecycleOperation) -> Result<(), String> {
        self.validate()?;
        op.register_unit(&self.service).map_err(|e| e.to_string())?;
        let existing = self.record()?;
        self.check_files(existing.is_some())?;
        self.check_unit()?;
        super::launch::sibling_binary(&self.service.binary, "hiero-desktop")?;
        if !self.service.binary.is_file() {
            return Err(
                "Desktop CLI binary is missing; reinstall the matching desktop package".into(),
            );
        }
        // Validate both renderers before publishing an ownership record.
        service::render_unit(&self.service.binary, &self.service.data_root)
            .map_err(|e| e.to_string())?;
        let enabled = existing.as_ref().is_none_or(|record| {
            if record.installation_complete {
                self.entry(true).is_file()
            } else {
                record.initial_autostart
            }
        });
        let mut record = match existing {
            Some(record) => record,
            None => Record {
                version: 1,
                installation_complete: false,
                initial_autostart: true,
                binary: self.service.binary.clone(),
                data_root: self.service.data_root.clone(),
                autostart_dir: self.autostart_dir.clone(),
                applications_dir: self.applications_dir.clone(),
                icons_dir: self.icons_dir.clone(),
                prior_unit: std::fs::read_to_string(self.service.unit_path()).ok(),
                prior_login_link: service::owned_login_link(&self.service)
                    .map_err(|e| e.to_string())?,
            },
        };
        if !record.installation_complete {
            atomic_write_text(
                &self.record_path(),
                &serde_json::to_string_pretty(&record)
                    .map_err(|_| "Could not encode desktop ownership")?,
            )
            .map_err(|_| "Could not record desktop ownership")?;
        }
        // The record makes install_guarded retain on-demand desktop mode.
        service::install_guarded(&self.service, op).map_err(|e| e.to_string())?;
        service::disable_login_guarded(&self.service, op).map_err(|e| e.to_string())?;
        atomic_write_text(&self.icon(), ICON).map_err(|_| "Could not install desktop icon")?;
        atomic_write_text(&self.entry(false), &self.text()?)
            .map_err(|_| "Could not install application launcher")?;
        if enabled {
            atomic_write_text(&self.entry(true), &self.text()?)
                .map_err(|_| "Could not enable desktop login startup")?;
        }
        record.installation_complete = true;
        atomic_write_text(
            &self.record_path(),
            &serde_json::to_string_pretty(&record)
                .map_err(|_| "Could not encode desktop ownership")?,
        )
        .map_err(|_| "Could not complete desktop ownership record")?;
        Ok(())
    }
    pub fn uninstall(&mut self) -> Result<(), String> {
        let op = self.operation()?;
        let _owner = self.ownership()?;
        self.uninstall_guarded(&op)?;
        drop(_owner);
        drop(op);
        SettingsStore::new(&HieronymusConfig::new(&self.service.data_root))
            .reconcile(self)
            .map(|_| ())
    }
    pub(crate) fn uninstall_guarded(&self, op: &LifecycleOperation) -> Result<(), String> {
        op.register_unit(&self.service).map_err(|e| e.to_string())?;
        if self.record()?.is_none() {
            return Ok(());
        }
        self.check_files(true)?;
        self.check_unit()?;
        for path in [
            self.entry(true),
            self.entry(false),
            self.icon(),
            self.record_path(),
        ] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Could not remove owned desktop registration".into()),
            }
        }
        Ok(())
    }
}
impl AutostartRegistration for LinuxRegistration {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        let op = self.operation()?;
        let _owner = self.ownership()?;
        let Some(mut record) = self.record()? else {
            return Err("Desktop registration is missing; run hiero desktop install first".into());
        };
        self.check_files(true)?;
        self.check_unit()?;
        service::disable_login_guarded(&self.service, &op).map_err(|e| e.to_string())?;
        if enabled {
            atomic_write_text(&self.entry(true), &self.text()?)
                .map_err(|_| "Could not enable desktop login startup")?;
        } else if self.entry(true).exists() {
            std::fs::remove_file(self.entry(true))
                .map_err(|_| "Could not disable desktop login startup")?;
        }
        if !record.installation_complete {
            record.initial_autostart = enabled;
            atomic_write_text(
                &self.record_path(),
                &serde_json::to_string_pretty(&record)
                    .map_err(|_| "Could not encode desktop ownership")?,
            )
            .map_err(|_| "Could not save the preference for interrupted installation recovery")?;
        }
        Ok(())
    }
    fn is_enabled(&mut self) -> Result<bool, String> {
        let _op = self.operation()?;
        let owned = self.record()?.is_some();
        self.check_files(owned)?;
        if owned {
            self.check_unit()?;
        }
        // A remaining standalone daemon login link would bypass the checkbox.
        if owned
            && service::owned_login_link(&self.service)
                .map_err(|e| e.to_string())?
                .is_some()
        {
            return Err("Standalone daemon login is still enabled; rerun hiero desktop install to reconcile it".into());
        }
        Ok(owned && self.entry(true).is_file())
    }
}

/// Full uninstall discovers only its unit-local record; it never probes unrelated
/// default XDG locations when a disposable/custom installation has no record.
#[cfg(target_os = "linux")]
pub(crate) fn for_uninstall(
    service: &ServiceOptions,
    app: &Path,
) -> Result<Option<LinuxRegistration>, String> {
    let path = service.unit_dir.join(RECORD_NAME);
    let Some(record) = read_record(&path)? else {
        return Ok(None);
    };
    let contained = match (record.binary.canonicalize(), app.canonicalize()) {
        (Ok(binary), Ok(app)) => binary.starts_with(app),
        _ => {
            record.binary.starts_with(app)
                && !record
                    .binary
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
        }
    };
    if !contained {
        return Err("Desktop binary belongs to another application installation".into());
    }
    let mut options = service.clone();
    options.binary = record.binary.clone();
    let registration = LinuxRegistration {
        service: options,
        autostart_dir: record.autostart_dir,
        applications_dir: record.applications_dir,
        icons_dir: record.icons_dir,
    };
    registration.validate()?;
    registration.record()?;
    registration.check_files(true)?;
    registration.check_unit()?;
    Ok(Some(registration))
}

/// Unit repair/update must retain on-demand mode and refuse an invalid marker.
/// The selected-version binary reconciliation itself is owned by the updater.
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn desktop_mode(options: &ServiceOptions) -> Result<bool, String> {
    let path = options.unit_dir.join(RECORD_NAME);
    let Some(record) = read_record(&path)? else {
        return Ok(false);
    };
    if record.version != 1 || record.data_root != options.data_root {
        return Err("Desktop registration belongs to another data root".into());
    }
    Ok(true)
}

fn read_record(path: &Path) -> Result<Option<Record>, String> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.len() <= 65536 => {}
        _ => return Err("Desktop ownership record is not a readable bounded regular file".into()),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(65537).read_to_end(&mut bytes))
        .map_err(|_| "Could not read desktop ownership record")?;
    if bytes.len() > 65536 {
        return Err("Desktop ownership record exceeds its size limit".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Desktop ownership record is invalid; restore it before retrying".into())
}

//! Separate native login registration; quitting never changes this preference.
use super::{AutostartRegistration, SettingsStore};
use crate::{
    lifecycle::{self, operation::LifecycleOperation},
    platform::macos_broker::{self, TaskAction},
    service::{self, ServiceOptions},
};
use hieronymus::data_root::HieronymusConfig;
#[derive(Clone)]
pub struct MacosRegistration {
    pub service: ServiceOptions,
}
impl MacosRegistration {
    pub fn new(service: ServiceOptions) -> Self {
        Self { service }
    }
    fn guarded(&self, action: TaskAction) -> Result<serde_json::Value, String> {
        let config = HieronymusConfig::new(&self.service.data_root);
        let operation = LifecycleOperation::acquire(&config).map_err(|e| e.to_string())?;
        operation
            .register_unit(&self.service)
            .map_err(|e| e.to_string())?;
        if !self.service.use_manager {
            return Err("macOS login registration requires native LaunchAgents".into());
        }
        let _owner = if matches!(action, TaskAction::Inspect) {
            None
        } else {
            let health = lifecycle::checked_probe(&config).map_err(|e| e.to_string())?;
            if health.is_live() {
                None
            } else {
                Some(
                    hieronymus::ownership::RootOwnership::acquire(
                        &config,
                        "macos-login-registration",
                    )
                    .map_err(|e| e.to_string())?,
                )
            }
        };
        if !matches!(action, TaskAction::Inspect) {
            let daemon = macos_broker::task(&self.service, TaskAction::Inspect, false)?;
            let tray = macos_broker::task(&self.service, TaskAction::Inspect, true)?;
            if daemon.get("mode").and_then(|value| value.as_str()) != Some("desktop")
                || tray.is_null()
            {
                return Err(
                    "Desktop registration is missing; run hiero desktop install first".into(),
                );
            }
        }
        macos_broker::task(&self.service, action, true)
    }
    pub fn install(&mut self) -> Result<(), String> {
        let config = HieronymusConfig::new(&self.service.data_root);
        let operation = LifecycleOperation::acquire(&config).map_err(|e| e.to_string())?;
        operation
            .register_unit(&self.service)
            .map_err(|e| e.to_string())?;
        if !self.service.use_manager {
            return Err("macOS desktop installation requires native LaunchAgents".into());
        }
        let health = lifecycle::checked_probe(&config).map_err(|e| e.to_string())?;
        let _owner = if health.is_live() {
            None
        } else {
            Some(
                hieronymus::ownership::RootOwnership::acquire(&config, "macos-desktop-install")
                    .map_err(|e| e.to_string())?,
            )
        };
        macos_broker::task(&self.service, TaskAction::InstallDesktop, true)?;
        Ok(())
    }
    pub fn uninstall(&mut self) -> Result<(), String> {
        let config = HieronymusConfig::new(&self.service.data_root);
        let operation = LifecycleOperation::acquire(&config).map_err(|e| e.to_string())?;
        operation
            .register_unit(&self.service)
            .map_err(|e| e.to_string())?;
        // Refuse a foreign tray before stopping or removing any daemon.
        macos_broker::task(&self.service, TaskAction::Inspect, true)?;
        lifecycle::stop_guarded(&config, &self.service, &operation).map_err(|e| e.to_string())?;
        macos_broker::task(&self.service, TaskAction::Remove, true)?;
        service::uninstall_guarded(&self.service, &operation)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
impl AutostartRegistration for MacosRegistration {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.guarded(if enabled {
            TaskAction::EnableLogin
        } else {
            TaskAction::DisableLogin
        })
        .map(|_| ())
    }
    fn is_enabled(&mut self) -> Result<bool, String> {
        let value = self.guarded(TaskAction::Inspect)?;
        if value.is_null() {
            return Ok(false);
        }
        value
            .get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or("Invalid native login readback".into())
    }
}
pub fn run(arguments: &[String]) -> Result<(), String> {
    let mut root = None;
    let mut binary =
        super::launch::stable_cli(&std::env::current_exe().map_err(|_| "Could not locate CLI")?)?;
    let mut directory = service::default_unit_dir();
    let mut json = false;
    let mut positional = Vec::new();
    let mut args = arguments.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--data-root" | "--binary" | "--unit-dir" => {
                let path = std::path::absolute(
                    args.next().ok_or("Native desktop option requires a path")?,
                )
                .map_err(|_| "Could not resolve path")?;
                match arg.as_str() {
                    "--data-root" => root = Some(path),
                    "--binary" => binary = path,
                    _ => directory = path,
                }
            }
            s if s.starts_with('-') => return Err("Unknown native desktop option".into()),
            s => positional.push(s),
        }
    }
    let config = hieronymus::data_root::load_config(root.as_deref());
    let data_root =
        std::path::absolute(config.data_root()).map_err(|_| "Could not resolve data root")?;
    let mut registration = MacosRegistration::new(ServiceOptions {
        data_root,
        unit_dir: directory,
        binary,
        use_manager: true,
    });
    let store = SettingsStore::new(&config);
    match positional.as_slice() {
        ["install"] => registration.install()?, ["uninstall"] => registration.uninstall()?,
        ["autostart", "on"] => { store.set_autostart(&mut registration, true)?; },
        ["autostart", "off"] => { store.set_autostart(&mut registration, false)?; },
        ["autostart", "status"] => {},
        _ => return Err("usage: hiero desktop <install|uninstall|autostart on|off|status> [--data-root PATH] [--binary PATH] [--unit-dir DIR] [--json]".into()),
    }
    let actual = registration.is_enabled()?;
    if json {
        println!("{}", serde_json::json!({"autostart": actual}));
    } else {
        println!(
            "desktop start at login: {}",
            if actual { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}

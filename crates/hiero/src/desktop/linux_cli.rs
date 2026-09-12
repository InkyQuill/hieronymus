//! Non-GUI desktop registration commands with explicit disposable path seams.
use super::{AutostartRegistration, SettingsStore, linux_registration::LinuxRegistration};
use crate::service::{self, ServiceOptions};
use hieronymus::data_root::load_config;
const USAGE: &str = "hiero desktop <install|uninstall|autostart on|off|status> [--data-root PATH] [--binary PATH] [--unit-dir DIR] [--autostart-dir DIR] [--applications-dir DIR] [--icons-dir DIR] [--no-activate] [--json]";
pub fn run(arguments: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Desktop registration commands currently require Linux".into());
    }
    let mut root = None;
    let mut binary = std::env::current_exe().map_err(|_| "Could not locate the installed CLI")?;
    let mut unit = service::default_unit_dir();
    let mut autostart = None;
    let mut applications = None;
    let mut icons = None;
    let mut activate = true;
    let mut json = false;
    let mut positionals = Vec::new();
    let mut args = arguments.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-activate" => activate = false,
            "--json" => json = true,
            "--data-root" | "--binary" | "--unit-dir" | "--autostart-dir"
            | "--applications-dir" | "--icons-dir" => {
                let path = std::path::absolute(
                    args.next()
                        .ok_or_else(|| format!("{arg} requires a path; {USAGE}"))?,
                )
                .map_err(|_| "Could not resolve desktop path")?;
                match arg.as_str() {
                    "--data-root" => root = Some(path),
                    "--binary" => binary = path,
                    "--unit-dir" => unit = path,
                    "--autostart-dir" => autostart = Some(path),
                    "--applications-dir" => applications = Some(path),
                    _ => icons = Some(path),
                }
            }
            s if s.starts_with('-') => return Err(format!("Unknown desktop option; {USAGE}")),
            s => positionals.push(s),
        }
    }
    let config = load_config(root.as_deref());
    let data_root =
        std::path::absolute(config.data_root()).map_err(|_| "Could not resolve data root")?;
    let mut registration = LinuxRegistration::new(ServiceOptions {
        data_root,
        unit_dir: unit,
        binary,
        use_manager: activate,
    });
    if let Some(path) = autostart {
        registration.autostart_dir = path;
    }
    if let Some(path) = applications {
        registration.applications_dir = path;
    }
    if let Some(path) = icons {
        registration.icons_dir = path;
    }
    let settings = SettingsStore::new(&config);
    match positionals.as_slice() {
        ["install"] => registration.install()?,
        ["uninstall"] => registration.uninstall()?,
        ["autostart", "on"] => {
            settings.set_autostart(&mut registration, true)?;
        }
        ["autostart", "off"] => {
            settings.set_autostart(&mut registration, false)?;
        }
        ["autostart", "status"] => {}
        _ => return Err(format!("Invalid desktop command; {USAGE}")),
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

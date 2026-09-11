//! Absolute sibling dispatch keeps GUI libraries out of the CLI/server graph.
use hieronymus::data_root::HieronymusConfig;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn sibling_binary(executable: &Path, name: &str) -> Result<PathBuf, String> {
    if !executable.is_absolute() {
        return Err("Desktop executable location must be absolute".into());
    }
    let parent = executable
        .parent()
        .ok_or("Could not locate the desktop installation")?;
    let path = parent.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file() {
        return Err(format!(
            "Missing {}; install the matching Hieronymus desktop package",
            path.display()
        ));
    }
    Ok(path)
}

pub fn launch(config: &HieronymusConfig) -> Result<(), String> {
    #[cfg(windows)]
    {
        let options = crate::lifecycle::default_service_options(config)
            .map_err(|_| "Could not resolve native service options")?;
        launch_with_options(config, &options)
    }
    #[cfg(not(windows))]
    launch_impl(config)
}
#[cfg(windows)]
pub fn launch_with_options(
    config: &HieronymusConfig,
    options: &crate::service::ServiceOptions,
) -> Result<(), String> {
    let cli = selected_cli(&options.binary)?;
    let helper = sibling_binary(&cli, "hiero-desktop")?;
    let mut command = Command::new(helper);
    use std::os::windows::process::CommandExt;
    let status = command
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--unit-dir")
        .arg(&options.unit_dir)
        .arg("--binary")
        .arg(&options.binary)
        .status()
        .map_err(|_| "Could not start native helper")?;
    if status.success() {
        Ok(())
    } else {
        Err("Native helper failed".into())
    }
}
#[cfg(not(windows))]
fn launch_impl(config: &HieronymusConfig) -> Result<(), String> {
    let executable =
        std::env::current_exe().map_err(|_| "Could not locate the Hieronymus installation")?;
    let helper = sibling_binary(&executable, "hiero-desktop")?;
    let root = std::path::absolute(config.data_root())
        .map_err(|_| "Could not resolve the desktop data root")?;
    let mut command = Command::new(helper);
    command.arg("--data-root").arg(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(format!(
            "Could not run hiero-desktop: {}; reinstall the desktop package and check its native library prerequisites",
            command.exec()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|error| {
            format!("Could not run hiero-desktop: {error}; reinstall the desktop package")
        })?;
        if status.success() {
            Ok(())
        } else {
            Err("hiero-desktop failed; check its diagnostic output".into())
        }
    }
}

/// A packaged Windows helper retains the immutable launcher endpoint across updates.
#[cfg(windows)]
pub fn stable_cli(executable: &Path) -> Result<PathBuf, String> {
    let parent = executable.parent().ok_or("Missing executable directory")?;
    if let Some(versions) = parent
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "versions"))
    {
        let launcher = versions
            .parent()
            .ok_or("Missing app directory")?
            .join("bin/hiero.exe");
        selected_cli(&launcher)?;
        return Ok(launcher);
    }
    sibling_binary(executable, "hiero")
}
/// Actual execution identity is distinct from the registered stable launcher.
#[cfg(windows)]
pub fn selected_cli(cli: &Path) -> Result<PathBuf, String> {
    if cli
        .parent()
        .is_some_and(|p| p.file_name().is_some_and(|n| n == "bin"))
        && cli
            .parent()
            .and_then(Path::parent)
            .is_some_and(|p| p.join("selected-version.json").exists())
    {
        return crate::platform::install::selected_executable(cli)
            .map_err(|_| "Installed launcher selection failed validation".into());
    }
    if !cli.is_absolute() || !cli.is_file() {
        return Err("Installed CLI is missing or not absolute".into());
    }
    Ok(cli.to_path_buf())
}

//! Absolute sibling dispatch keeps GUI libraries out of the CLI/server graph.
use hieronymus::data_root::HieronymusConfig;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn sibling_binary(executable: &Path, name: &str) -> Result<PathBuf, String> {
    if !executable.is_absolute() {
        return Err("Desktop executable location must be absolute".into());
    }
    let resolved = executable.canonicalize().map_err(|e| e.to_string())?;
    let executable = resolved.as_path();
    let parent = executable
        .parent()
        .ok_or("Could not locate the desktop installation")?;
    let path = if cfg!(target_os = "macos") && name == "hiero-desktop" {
        parent.join("Hieronymus.app/Contents/MacOS/hiero-desktop")
    } else if cfg!(target_os = "macos")
        && name == "hiero"
        && parent.ends_with("Hieronymus.app/Contents/MacOS")
    {
        parent
            .ancestors()
            .nth(3)
            .ok_or("Invalid application bundle")?
            .join("hiero")
    } else {
        parent.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    };
    if !path.is_file() {
        return Err(format!(
            "Missing {}; install the matching Hieronymus desktop package",
            path.display()
        ));
    }
    Ok(path)
}

pub fn launch(config: &HieronymusConfig) -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let options = crate::lifecycle::default_service_options(config)
            .map_err(|_| "Could not resolve native service options")?;
        launch_with_options(config, &options)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    launch_impl(config)
}
#[cfg(any(windows, target_os = "macos"))]
pub fn launch_with_options(
    config: &HieronymusConfig,
    options: &crate::service::ServiceOptions,
) -> Result<(), String> {
    let helper = selected_helper(&options.binary)?;
    let mut command = Command::new(helper);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let status = command
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
#[cfg(not(any(windows, target_os = "macos")))]
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

/// Resolve the stable Unix selection endpoint; direct CLI fixtures are explicit.
#[cfg(unix)]
pub fn stable_cli(executable: &Path) -> Result<PathBuf, String> {
    if executable
        .parent()
        .is_some_and(|p| p.ends_with("Hieronymus.app/Contents/MacOS"))
    {
        return stable_cli(&sibling_binary(executable, "hiero")?);
    }
    let parent = executable.parent().ok_or("Missing executable directory")?;
    if let Some(versions) = parent
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "versions"))
    {
        let stable = versions
            .parent()
            .ok_or("Missing app directory")?
            .join("bin/hiero");
        selected_cli(&stable)?;
        return Ok(stable);
    }
    sibling_binary(executable, "hiero")
}
#[cfg(unix)]
pub fn selected_cli(cli: &Path) -> Result<PathBuf, String> {
    if !cli.is_absolute() || !cli.is_file() {
        return Err("Installed CLI is missing or not absolute".into());
    }
    cli.canonicalize()
        .map_err(|_| "Installed CLI selection is unavailable".into())
}

/// Resolve version selection before sibling lookup; callers retain `cli` as --binary.
pub fn selected_helper(cli: &Path) -> Result<PathBuf, String> {
    sibling_binary(&selected_cli(cli)?, "hiero-desktop")
}

#[cfg(all(test, windows))]
mod selection_tests {
    use super::*;
    #[test]
    fn regular_windows_launcher_selects_versioned_helper() {
        let root = tempfile::tempdir().unwrap();
        let layout = crate::app::AppLayout::new(root.path());
        let version = layout.version_dir("0.9.0");
        std::fs::create_dir_all(&version).unwrap();
        for name in ["hiero.exe", "hiero-launcher.exe", "hiero-desktop.exe"] {
            std::fs::write(version.join(name), name).unwrap();
        }
        layout.switch_stable_links("0.9.0").unwrap();
        let stable = layout.stable_link("hiero");
        assert!(
            !std::fs::symlink_metadata(&stable)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(root.path().join("selected-version.json").is_file());
        assert_eq!(
            selected_helper(&stable).unwrap().canonicalize().unwrap(),
            version.join("hiero-desktop.exe").canonicalize().unwrap()
        );
        assert_eq!(stable, root.path().join("bin/hiero.exe"));
        assert!(!root.path().join("bin/hiero-desktop.exe").exists());
    }
}

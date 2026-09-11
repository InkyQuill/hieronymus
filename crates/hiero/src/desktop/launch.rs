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
    let executable =
        std::env::current_exe().map_err(|_| "Could not locate the Hieronymus installation")?;
    let helper = sibling_binary(&executable, "hiero-desktop")?;
    let root = std::path::absolute(config.data_root())
        .map_err(|_| "Could not resolve the desktop data root")?;
    let mut command = Command::new(helper);
    command.arg("--data-root").arg(root);
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

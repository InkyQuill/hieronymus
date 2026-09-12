//! Installed command endpoints and one native version-selection authority.
use crate::app::{AppLayout, LINK_NAMES};
use std::{
    io,
    path::{Path, PathBuf},
};

pub fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
pub fn validate_version(version: &str) -> io::Result<()> {
    if version.ends_with('.')
        || version.is_empty()
        || version == "."
        || version == ".."
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "version must be one normal directory name",
        ));
    }
    Ok(())
}
pub fn current_version(layout: &AppLayout) -> Option<String> {
    #[cfg(unix)]
    {
        let target = std::fs::read_link(layout.stable_link("hiero")).ok()?;
        let text = target.to_str()?;
        let version = text.strip_prefix("../versions/")?.strip_suffix("/hiero")?;
        validate_version(version).ok()?;
        Some(version.into())
    }
    #[cfg(windows)]
    {
        read_selection(layout)
            .ok()
            .map(|selection| selection.version)
    }
}
pub fn switch(layout: &AppLayout, version: &str) -> io::Result<()> {
    validate_version(version)?;
    std::fs::create_dir_all(layout.bin_dir())?;
    #[cfg(unix)]
    {
        for name in LINK_NAMES {
            let link = layout.stable_link(name);
            let temporary = layout.bin_dir().join(format!(".{name}.switch"));
            match std::fs::remove_file(&temporary) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            std::os::unix::fs::symlink(format!("../versions/{version}/{name}"), &temporary)?;
            std::fs::rename(&temporary, &link)?;
        }
        hieronymus::atomic::sync_directory(&layout.bin_dir())
    }
    #[cfg(windows)]
    {
        switch_windows(layout, version)
    }
}
pub fn verify_selection(layout: &AppLayout, version: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        for name in LINK_NAMES {
            let want = PathBuf::from(format!("../versions/{version}/{name}"));
            match std::fs::read_link(layout.stable_link(name)) {
                Ok(target) if target == want => {}
                Ok(target) => {
                    return Err(format!(
                        "link {name} points at {} instead of {}",
                        target.display(),
                        want.display()
                    ));
                }
                Err(error) => return Err(format!("link {name} is unreadable: {error}")),
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let selection = read_selection(layout).map_err(|e| e.to_string())?;
        if selection.version != version {
            return Err("selected version differs".into());
        }
        verify_launchers(layout, &selection.launcher_sha256).map_err(|e| e.to_string())
    }
}
/// Clear selection during rollback of an initially unselected installation.
pub fn clear_selection(layout: &AppLayout) -> io::Result<()> {
    #[cfg(windows)]
    {
        remove_if_present(&layout.root().join("selected-version.json"))?;
    }
    for name in LINK_NAMES {
        remove_if_present(&layout.stable_link(name))?;
    }
    Ok(())
}
fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(windows)]
#[derive(serde::Serialize, serde::Deserialize)]
struct Selection {
    version: String,
    launcher_sha256: String,
}
#[cfg(windows)]
fn read_selection(layout: &AppLayout) -> io::Result<Selection> {
    let selection: Selection =
        serde_json::from_slice(&std::fs::read(layout.root().join("selected-version.json"))?)
            .map_err(io::Error::other)?;
    validate_version(&selection.version)?;
    Ok(selection)
}
#[cfg(windows)]
fn digest(path: &Path) -> io::Result<String> {
    use sha2::Digest;
    let metadata = std::fs::symlink_metadata(path)?;
    use std::os::windows::fs::MetadataExt;
    if !metadata.is_file()
        || metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    {
        return Err(io::Error::other("launcher must be a regular file"));
    }
    Ok(format!("{:x}", sha2::Sha256::digest(std::fs::read(path)?)))
}
#[cfg(windows)]
fn verify_launchers(layout: &AppLayout, expected: &str) -> io::Result<()> {
    for name in LINK_NAMES {
        if digest(&layout.stable_link(name))? != expected {
            return Err(io::Error::other(
                "stable launcher differs from selection identity",
            ));
        }
    }
    Ok(())
}
#[cfg(windows)]
fn switch_windows(layout: &AppLayout, version: &str) -> io::Result<()> {
    let payload = layout.version_dir(version).join("hiero.exe");
    if !payload.is_file() {
        return Err(io::Error::other("selected version lacks hiero.exe"));
    }
    let launcher_hash = match read_selection(layout) {
        Ok(previous) => {
            verify_launchers(layout, &previous.launcher_sha256)?;
            previous.launcher_sha256
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let launcher = layout.version_dir(version).join("hiero-launcher.exe");
            let hash = digest(&launcher)?;
            for name in LINK_NAMES {
                let target = layout.stable_link(name);
                if target.exists() {
                    if digest(&target)? != hash {
                        return Err(io::Error::other("refusing to replace a foreign launcher"));
                    }
                } else {
                    let mut source = std::fs::File::open(&launcher)?;
                    let mut target = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(target)?;
                    std::io::copy(&mut source, &mut target)?;
                    target.sync_all()?;
                }
            }
            hash
        }
        Err(error) => return Err(error),
    };
    let bytes = serde_json::to_vec(&Selection {
        version: version.into(),
        launcher_sha256: launcher_hash,
    })
    .map_err(io::Error::other)?;
    hieronymus::atomic::atomic_write(&layout.root().join("selected-version.json"), &bytes)
}

/// Resolve the executable from the single selection record. The launcher stays
/// immutable; later update coordination owns launcher replacement when needed.
#[cfg(windows)]
pub fn selected_executable(launcher: &Path) -> io::Result<PathBuf> {
    let bin = launcher
        .parent()
        .ok_or_else(|| io::Error::other("missing launcher directory"))?;
    if bin.file_name() != Some(std::ffi::OsStr::new("bin")) {
        return Err(io::Error::other(
            "launcher must live in the managed bin directory",
        ));
    }
    let layout = AppLayout::new(
        bin.parent()
            .ok_or_else(|| io::Error::other("missing application directory"))?,
    );
    let selection = read_selection(&layout)?;
    verify_launchers(&layout, &selection.launcher_sha256)?;
    let executable = layout.version_dir(&selection.version).join("hiero.exe");
    if !executable.is_file() {
        return Err(io::Error::other("selected executable is missing"));
    }
    Ok(executable)
}

/// Native library filename. Exact hashes/acquisition belong to runtime qualification.
#[cfg(target_os = "linux")]
pub const RUNTIME_LIBRARY: &str = "lib/libonnxruntime.so";
#[cfg(target_os = "macos")]
pub const RUNTIME_LIBRARY: &str = "lib/libonnxruntime.dylib";
#[cfg(windows)]
pub const RUNTIME_LIBRARY: &str = "lib/onnxruntime.dll";

//! Persistent native-call coordination survives a caller timing out. Never unlink.
use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
};
#[cfg(not(target_os = "macos"))]
pub const MANAGER_GATE: &str = ".windows-native.lock";
#[cfg(not(target_os = "macos"))]
pub const BROWSER_GATE: &str = ".windows-browser.lock";
pub fn acquire(directory: &Path, name: &str) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(name))?;
    file.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "A native operation is still in progress; wait for completion before retrying",
        ),
        TryLockError::Error(e) => e,
    })?;
    Ok(file)
}
pub fn check(directory: &Path) -> io::Result<()> {
    drop(acquire(directory, MANAGER_GATE)?);
    Ok(())
}

#[cfg(target_os = "macos")]
pub const MANAGER_GATE: &str = ".macos-native.lock";
#[cfg(target_os = "macos")]
pub const BROWSER_GATE: &str = ".macos-browser.lock";

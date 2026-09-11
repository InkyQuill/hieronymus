//! Secret-bearing files: secure creation, atomic publication, same-handle reads.
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};
#[cfg(unix)]
#[path = "private_file/unix.rs"]
mod unix;
#[cfg(windows)]
#[path = "private_file/windows.rs"]
mod windows;
#[cfg(unix)]
use unix as native;
#[cfg(windows)]
use windows as native;

/// Validate permissions and read bytes from the very same opened object.
pub fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = native::open_private(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Publish a completely written owner-only file, refusing any existing name.
pub fn create_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish(path, bytes, false)
}

/// Deliberately replace a secret without exposing a permissive intermediate.
pub fn replace_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish(path, bytes, true)
}

fn publish(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    // tempfile owns cleanup of the name. The secret itself is created by our
    // native implementation with restrictive mode/DACL before publication.
    let staging = tempfile::Builder::new()
        .prefix(".hiero-private-")
        .tempdir_in(parent)?;
    let temporary = staging.path().join("secret");
    let mut file = native::create_new(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if replace {
        crate::atomic::replace_file(&temporary, path)?;
    } else {
        native::publish_new(&temporary, path)?;
    }
    crate::atomic::sync_directory(parent)
}

fn validate_regular(file: &File) -> io::Result<()> {
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "credential must be a regular owner-only file",
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub use windows::{SecurityDescriptor, owner_only_security};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validated_handle_keeps_its_secret_when_the_path_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret");
        create_private_new(&path, b"original").unwrap();
        let mut opened = native::open_private(&path).unwrap();
        replace_private(&path, b"replacement").unwrap();
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original");
        assert_eq!(read_private(&path).unwrap(), b"replacement");
    }
    #[test]
    fn native_creation_is_already_owner_protected_before_any_secret_write() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret");
        drop(native::create_new(&path).unwrap());
        assert!(read_private(&path).unwrap().is_empty());
        let mut created = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        created.write_all(b"secret").unwrap();
        created.sync_all().unwrap();
        drop(created);
        assert_eq!(read_private(&path).unwrap(), b"secret");
    }
}

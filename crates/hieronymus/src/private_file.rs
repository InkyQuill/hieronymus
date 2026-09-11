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

/// Open a non-secret coordination file for OS locking, validating the same handle.
/// Windows permits concurrent writable lock handles; credential reads retain their
/// stricter sharing policy. Callers must never use this handle to read secrets.
pub fn open_coordination(path: &Path) -> io::Result<File> {
    native::open_coordination(path)
}

/// Read one regular, current-user-owned, non-alias file and report whether
/// that same handle has owner-only mode/DACL protection. Nonprivate snapshots
/// are for callers that prove the exact returned bytes contain no credentials;
/// object identity, ownership and I/O errors are never downgraded to a flag.
pub fn read_owned_snapshot(path: &Path) -> io::Result<(Vec<u8>, bool)> {
    let (mut file, private) = native::open_owned(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok((bytes, private))
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
    fn coordination_handles_validate_live_and_released_persistent_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("coordination.lock");
        create_private_new(&path, b"").unwrap();
        let held = open_coordination(&path).unwrap();
        held.try_lock().unwrap();
        let probe = open_coordination(&path).unwrap();
        assert!(matches!(
            probe.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        probe.try_lock().unwrap();
        assert!(path.is_file());
        drop(probe);
        assert!(read_private(&path).unwrap().is_empty());
    }
    #[cfg(windows)]
    #[test]
    fn coordination_sharing_does_not_relax_secret_readers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("coordination.lock");
        create_private_new(&path, b"").unwrap();
        let _held = open_coordination(&path).unwrap();
        assert!(
            read_private(&path).is_err(),
            "secret reads still deny live writers"
        );
    }
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

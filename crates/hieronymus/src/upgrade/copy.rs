//! Ordinary upgrade copies retain their write handle through metadata and flush.
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

pub(super) fn copy_and_sync(source: &Path, destination: &Path) -> io::Result<()> {
    let mut source = File::open(source)?;
    let permissions = source.metadata()?.permissions();
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(permissions.mode());
    }
    let mut destination = options.open(destination)?;
    io::copy(&mut source, &mut destination)?;
    // Setting a readonly mode/attribute does not revoke the already granted
    // write handle needed by Windows FlushFileBuffers. Never reopen readonly.
    destination.set_permissions(permissions)?;
    destination.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_copy_flushes_and_preserves_readonly_source_permissions() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        std::fs::write(&source, b"ordinary fixture").unwrap();
        let original = std::fs::metadata(&source).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&source, readonly).unwrap();
        copy_and_sync(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"ordinary fixture");
        assert!(std::fs::metadata(&source).unwrap().permissions().readonly());
        assert!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .readonly()
        );
        std::fs::set_permissions(&source, original.clone()).unwrap();
        std::fs::set_permissions(&destination, original).unwrap();
    }
}

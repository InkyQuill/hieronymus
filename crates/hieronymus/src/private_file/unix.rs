use super::*;
use rustix::fs::{Mode, OFlags, open};
use std::os::unix::fs::MetadataExt;

pub(super) fn create_new(path: &Path) -> io::Result<File> {
    Ok(File::from(open(
        path,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?))
}
pub(super) fn open_private(path: &Path) -> io::Result<File> {
    let file = File::from(open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    validate_regular(&file)?;
    let metadata = file.metadata()?;
    if metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "credential is readable beyond its owner or has aliases; run chmod 600 or recreate it",
        ));
    }
    Ok(file)
}
pub(super) fn publish_new(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

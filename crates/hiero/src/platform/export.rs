//! Handle identity and retained-parent publication boundaries.
#[cfg(unix)]
#[path = "export_unix.rs"]
mod native;
#[cfg(windows)]
#[path = "export_windows.rs"]
mod native;
pub(crate) use native::Destination;
use std::{io, path::Path};

pub(crate) fn is_link(path: &Path) -> bool {
    #[cfg(unix)]
    {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }
}

pub(crate) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags, open};
        use std::os::unix::fs::MetadataExt;
        let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let left = std::fs::File::from(open(left, flags, Mode::empty())?).metadata()?;
        let right = std::fs::File::from(open(right, flags, Mode::empty())?).metadata()?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(windows)]
    {
        native::same_file(left, right)
    }
}

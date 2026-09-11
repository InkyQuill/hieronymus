use crate::export::Publication;
use std::{
    io::Write,
    path::{Component, Path},
};
/// An opened destination directory. Publication writes and fsyncs a sibling
/// temporary file, then renames it relative to this retained descriptor.
/// Returned errors remove the temporary file; interruption may leave debris
/// but never a partially written destination.
pub(crate) struct Destination {
    directory: std::os::fd::OwnedFd,
    name: std::ffi::OsString,
}

impl Destination {
    /// Walk each component relative to an already opened directory. NOFOLLOW
    /// prevents an ancestor swap from redirecting the export through a link;
    /// retaining the final descriptor anchors all subsequent publication I/O.
    pub(crate) fn open(output: &Path) -> std::io::Result<Self> {
        use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = open("/", flags, Mode::empty())?;
        let parent = output
            .parent()
            .ok_or_else(|| std::io::Error::other("missing parent"))?;
        for component in parent.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            directory = match openat(&directory, name, flags, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) => {
                    match mkdirat(&directory, name, Mode::from_raw_mode(0o755)) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(error) => return Err(error.into()),
                    }
                    openat(&directory, name, flags, Mode::empty())?
                }
                Err(error) => return Err(error.into()),
            };
        }
        Ok(Self {
            directory,
            name: output
                .file_name()
                .ok_or_else(|| std::io::Error::other("missing filename"))?
                .to_os_string(),
        })
    }

    pub(crate) fn publish(&self, text: &str, publication: Publication) -> std::io::Result<()> {
        use rustix::fs::{
            AtFlags, Mode, OFlags, RenameFlags, openat, renameat, renameat_with, unlinkat,
        };
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(std::io::Error::other)?;
        let name = format!(
            ".hiero-export-{}.tmp",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let fd = openat(
            &self.directory,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?;
        let result = (|| {
            let mut file = std::fs::File::from(fd);
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            match publication {
                Publication::NewFileOnly => renameat_with(
                    &self.directory,
                    &name,
                    &self.directory,
                    &self.name,
                    RenameFlags::NOREPLACE,
                ),
                Publication::Overwrite => {
                    renameat(&self.directory, &name, &self.directory, &self.name)
                }
            }
            .map_err(|error| {
                if error == rustix::io::Errno::EXIST {
                    std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "destination already exists; pass --force or choose another path",
                    )
                } else {
                    std::io::Error::from(error)
                }
            })?;
            rustix::fs::fsync(&self.directory)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = unlinkat(&self.directory, &name, AtFlags::empty());
        }
        result
    }
}

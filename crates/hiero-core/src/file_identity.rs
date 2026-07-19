use std::{fs::File, io};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    pub(crate) first: u64,
    pub(crate) second: u64,
}

#[cfg(unix)]
pub(crate) fn identity_and_link_count(file: &File) -> io::Result<(u64, FileIdentity)> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handle is not a regular file",
        ));
    }
    Ok((
        metadata.nlink(),
        FileIdentity {
            first: metadata.dev(),
            second: metadata.ino(),
        },
    ))
}

#[cfg(windows)]
pub(crate) fn identity_and_link_count(file: &File) -> io::Result<(u64, FileIdentity)> {
    let information = winapi_util::file::information(file)?;
    validate_windows_regular_file(&information)?;
    Ok((
        information.number_of_links(),
        FileIdentity {
            first: information.volume_serial_number(),
            second: information.file_index(),
        },
    ))
}

pub(crate) fn identity_for_handle(file: &File) -> io::Result<FileIdentity> {
    identity_and_link_count(file).map(|(_, identity)| identity)
}

#[cfg(windows)]
fn validate_windows_regular_file(information: &winapi_util::file::Information) -> io::Result<()> {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    let attributes = information.file_attributes();
    if attributes & u64::from(FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handle is not a regular non-reparse file",
        ))
    }
}

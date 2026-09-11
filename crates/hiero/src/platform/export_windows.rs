use crate::export::Publication;
use hieronymus::windows_file::{file_from_handle, information, wide};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::Win32::System::SystemServices::MAXIMUM_ALLOWED;
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
            RtlNtStatusToDosErrorNoTeb,
        },
    },
    Win32::{Foundation::*, Storage::FileSystem::*, System::IO::IO_STATUS_BLOCK},
};

/// Every operation after traversal is relative to this retained parent handle.
pub(crate) struct Destination {
    directory: File,
    name: OsString,
}
impl Destination {
    pub(crate) fn open(output: &Path) -> io::Result<Self> {
        let parent = output.parent().ok_or_else(invalid_path)?;
        let mut components = parent.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(invalid_path());
        };
        if !matches!(
            prefix.kind(),
            Prefix::Disk(_)
                | Prefix::VerbatimDisk(_)
                | Prefix::UNC(_, _)
                | Prefix::VerbatimUNC(_, _)
        ) {
            return Err(invalid_path());
        }
        let mut root = PathBuf::from(prefix.as_os_str());
        root.push("\\");
        let root = wide(root.as_os_str())?;
        // SAFETY: the root is an absolute volume/share root, not an attacker-chosen component.
        let mut directory = unsafe {
            file_from_handle({
                CreateFileW(
                    root.as_ptr(),
                    MAXIMUM_ALLOWED,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            })
        }?;
        information(&directory)?;
        for component in components {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = relative_open(&directory, name, true)?;
                }
                _ => return Err(invalid_path()),
            }
        }
        let name = output.file_name().ok_or_else(invalid_path)?.to_os_string();
        validate_name(&name)?;
        Ok(Self { directory, name })
    }
    pub(crate) fn publish(&self, text: &str, publication: Publication) -> io::Result<()> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let temporary = format!(
            ".hiero-export-{}.tmp",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let mut file = relative_open(&self.directory, OsStr::new(&temporary), false)?;
        let mut published = false;
        let result = (|| {
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            rename(
                &file,
                &self.directory,
                &self.name,
                publication == Publication::Overwrite,
            )?;
            published = true;
            file.sync_all()?;
            Ok(())
        })();
        if result.is_err() && !published {
            // Remove exactly our opened temporary, never resolve a substituted name.
            let deletion = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: live owned handle and correctly sized structure.
            unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    (&deletion as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                );
            }
        }
        result
    }
}
fn invalid_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "export requires an absolute normal filesystem path",
    )
}
fn validate_name(name: &OsStr) -> io::Result<()> {
    let text = name.to_string_lossy();
    if text.is_empty() || text == "." || text == ".." || text.contains([':', '/', '\\', '\0']) {
        return Err(invalid_path());
    }
    Ok(())
}
fn relative_open(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
    validate_name(name)?;
    let mut name: Vec<u16> = name.encode_wide().collect();
    let bytes: u16 = (name.len() * 2).try_into().map_err(|_| invalid_path())?;
    let mut unicode = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let security = if directory {
        None
    } else {
        Some(hieronymus::private_file::owner_only_security()?)
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: security
            .as_ref()
            .map_or(ptr::null_mut(), |security| security.as_raw().cast()),
        SecurityQualityOfService: ptr::null_mut(),
    };
    let mut handle = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    let access = if directory {
        MAXIMUM_ALLOWED
    } else {
        FILE_GENERIC_WRITE | DELETE
    };
    let options = FILE_OPEN_REPARSE_POINT
        | FILE_SYNCHRONOUS_IO_NONALERT
        | if directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        };
    // SAFETY: all structures live through this synchronous call; RootDirectory is retained.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access,
            &attributes,
            &mut status,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            if directory { FILE_OPEN_IF } else { FILE_CREATE },
            options,
            ptr::null(),
            0,
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosErrorNoTeb(result) } as i32,
        ));
    }
    let file = unsafe { file_from_handle(handle) }?;
    information(&file)?;
    Ok(file)
}
fn rename(file: &File, parent: &File, name: &OsStr, replace: bool) -> io::Result<()> {
    let name: Vec<u16> = name.encode_wide().collect();
    // Reserve the complete fixed structure as well as the variable UTF-16
    // name and a trailing zero, including for one-character destinations.
    let size = size_of::<FILE_RENAME_INFO>()
        .max(std::mem::offset_of!(FILE_RENAME_INFO, FileName) + (name.len() + 1) * 2);
    let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
    // SAFETY: allocation is aligned for FILE_RENAME_INFO, includes the variable name,
    // and stays live through the syscall. Rename resolves the name relative to parent.
    unsafe {
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        (*info).Anonymous.ReplaceIfExists = replace;
        (*info).RootDirectory = parent.as_raw_handle();
        (*info).FileNameLength = (name.len() * 2) as u32;
        ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
        if SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            size as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
pub(super) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    fn open(path: &Path) -> io::Result<File> {
        let path = wide(path.as_os_str())?;
        // SAFETY: valid path; OPEN_REPARSE_POINT ensures identity never follows final links.
        unsafe {
            file_from_handle({
                CreateFileW(
                    path.as_ptr(),
                    FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                    ptr::null_mut(),
                )
            })
        }
    }
    let left = open(left)?;
    let right = open(right)?;
    let left = information(&left)?;
    let right = information(&right)?;
    Ok(left.dwVolumeSerialNumber == right.dwVolumeSerialNumber
        && left.nFileIndexHigh == right.nFileIndexHigh
        && left.nFileIndexLow == right.nFileIndexLow)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_stays_in_opened_directory_after_parent_replacement() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("exports");
        std::fs::create_dir(&original).unwrap();
        let destination = Destination::open(&original.join("memory.json")).unwrap();
        let moved = root.path().join("moved");
        std::fs::rename(&original, &moved).unwrap();
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("memory.json"), "protected").unwrap();
        destination
            .publish("export", Publication::Overwrite)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(moved.join("memory.json")).unwrap(),
            "export"
        );
        assert_eq!(
            std::fs::read_to_string(original.join("memory.json")).unwrap(),
            "protected"
        );
    }
    #[test]
    fn no_clobber_and_overwrite_are_native_atomic_publications() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("x");
        let destination = Destination::open(&path).unwrap();
        destination
            .publish("first", Publication::NewFileOnly)
            .unwrap();
        assert!(
            destination
                .publish("second", Publication::NewFileOnly)
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        destination
            .publish("second", Publication::Overwrite)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

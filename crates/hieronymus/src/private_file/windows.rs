use super::*;
use crate::windows_file::{file_from_handle, information, wide};
use std::{os::windows::io::AsRawHandle, ptr};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Threading::*,
};

pub struct SecurityDescriptor(*mut std::ffi::c_void);
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: these allocations are returned by the security APIs using LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn current_user() -> io::Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo handle is valid; token is an output handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { file_from_handle(token) }?;
    let mut size = 0;
    // SAFETY: null buffer queries required size only.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut size,
        );
    }
    if (size as usize) < size_of::<TOKEN_USER>() {
        return Err(io::Error::other("token user information is truncated"));
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    // SAFETY: buffer is aligned and has at least the queried size.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}
fn sid(user: &[usize]) -> PSID {
    // SAFETY: current_user constructs this aligned TOKEN_USER buffer; it stays alive.
    unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid }
}
pub fn owner_only_security() -> io::Result<SecurityDescriptor> {
    let user = current_user()?;
    let mut text = ptr::null_mut();
    // SAFETY: token user owns a valid SID and text is the output allocation.
    if unsafe { ConvertSidToStringSidW(sid(&user), &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let allocation = SecurityDescriptor(text.cast());
    let mut length = 0;
    // SAFETY: API returns a NUL-terminated UTF-16 SID string.
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    let sid_text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    drop(allocation);
    // Protected DACL, one allow ACE for the exact process-user SID. No inherited ACEs.
    let sddl = wide(std::ffi::OsStr::new(&format!(
        "O:{sid_text}D:P(A;;FA;;;{sid_text})"
    )))?;
    let mut descriptor = ptr::null_mut();
    // SAFETY: live SDDL input and valid output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(SecurityDescriptor(descriptor))
}
pub(super) fn create_new(path: &Path) -> io::Result<File> {
    let descriptor = owner_only_security()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path = wide(path.as_os_str())?;
    // SAFETY: descriptor lives through CreateFile; CREATE_NEW never opens an existing object.
    unsafe {
        file_from_handle({
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        })
    }
}
pub(super) fn open_private(path: &Path) -> io::Result<File> {
    let (file, private) = open_owned(path)?;
    if !private {
        return Err(unsafe_credential());
    }
    Ok(file)
}
pub(super) fn open_owned(path: &Path) -> io::Result<(File, bool)> {
    let path = wide(path.as_os_str())?;
    // Deny concurrent writes and ACL/path mutation via a conflicting write handle;
    // replacement may happen, but validation and bytes always use this same handle.
    let file = unsafe {
        file_from_handle({
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        })
    }?;
    validate_owned(file)
}
fn validate_owned(file: File) -> io::Result<(File, bool)> {
    validate_regular(&file)?;
    if information(&file)?.nNumberOfLinks != 1 {
        return Err(unsafe_credential());
    }
    let user = current_user()?;
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: File owns a readable handle; output pointers are live and aligned.
    let error = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let _allocation = SecurityDescriptor(descriptor);
    let mut control = 0;
    let mut revision = 0;
    let mut private;
    // SAFETY: descriptor and subordinate owner/ACL pointers live in allocation.
    unsafe {
        if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
            return Err(io::Error::last_os_error());
        }
        if owner.is_null() || EqualSid(owner, sid(&user)) == 0 {
            return Err(unsafe_credential());
        }
        if dacl.is_null() {
            return Ok((file, false));
        }
        private = control & SE_DACL_PROTECTED != 0;
        for index in 0..(*dacl).AceCount {
            let mut ace = ptr::null_mut();
            if GetAce(dacl, u32::from(index), &mut ace) == 0 {
                return Err(io::Error::last_os_error());
            }
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
                private = false;
                continue;
            }
            if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(unsafe_credential());
            }
            if header.AceFlags & INHERITED_ACE as u8 != 0 {
                private = false;
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let ace_sid = (&allowed.SidStart as *const u32).cast::<u8>();
            let sid_offset = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
            // A SID has an eight-byte header followed by SubAuthorityCount
            // u32 values. Bound the variable body before asking EqualSid.
            if usize::from(header.AceSize) < sid_offset + 8 {
                return Err(unsafe_credential());
            }
            let sid_length = 8 + usize::from(*ace_sid.add(1)) * 4;
            if sid_offset + sid_length > usize::from(header.AceSize)
                || IsValidSid(ace_sid.cast_mut().cast()) == 0
            {
                return Err(unsafe_credential());
            }
            if EqualSid(ace_sid.cast_mut().cast(), sid(&user)) == 0 {
                private = false;
            }
        }
    }
    Ok((file, private))
}
fn unsafe_credential() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "credential requires a protected owner-only DACL and no aliases",
    )
}
pub(super) fn publish_new(source: &Path, destination: &Path) -> io::Result<()> {
    crate::windows_file::move_file(source, destination, false)
}

impl SecurityDescriptor {
    /// Borrow the descriptor for a synchronous native create operation.
    pub fn as_raw(&self) -> *mut std::ffi::c_void {
        self.0
    }
}

/// Unlike credential snapshots, coordination permits other writable lock handles.
/// Validate the very handle that will be locked; deny aliases and inherited ACLs.
pub(super) fn open_coordination(path: &Path) -> io::Result<File> {
    let path = wide(path.as_os_str())?;
    // SAFETY: live NUL-terminated path; returned handle gets RAII ownership.
    let file = unsafe {
        file_from_handle(CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        ))
    }?;
    let (file, private) = validate_owned(file)?;
    if !private {
        return Err(unsafe_credential());
    }
    Ok(file)
}

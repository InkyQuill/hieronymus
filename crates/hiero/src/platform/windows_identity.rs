//! Authority comes from the current process token and kernel session, never env.
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{Authorization::ConvertSidToStringSidW, *},
    System::Threading::*,
};
pub fn sid() -> io::Result<String> {
    unsafe {
        let mut handle = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(handle);
        let mut size = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut size,
        );
        if size < size_of::<TOKEN_USER>() as u32 || size > 65536 {
            return Err(io::Error::other("Invalid token user size"));
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut text = ptr::null_mut();
        if ConvertSidToStringSidW(
            (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            &mut text,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let sid = String::from_utf16(std::slice::from_raw_parts(text, length))
            .map_err(|_| io::Error::other("Invalid SID encoding"));
        LocalFree(text.cast());
        sid
    }
}
pub fn session() -> io::Result<String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 || session == 0 {
        return Err(io::Error::other(
            "A native interactive user session is required",
        ));
    }
    Ok(format!("{}-session-{session}", sid()?))
}

/// Resolve a native account spelling to its SID without trusting environment data.
pub(crate) fn account_sid(account: &str) -> io::Result<String> {
    let account = hieronymus::windows_file::wide(std::ffi::OsStr::new(account))?;
    let mut sid_bytes = 0;
    let mut domain_chars = 0;
    let mut kind = 0;
    // SAFETY: the first call queries sizes only; all output pointers are valid.
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut sid_bytes,
            ptr::null_mut(),
            &mut domain_chars,
            &mut kind,
        );
    }
    if sid_bytes == 0 || sid_bytes > 65536 || domain_chars > 65536 {
        return Err(io::Error::other("Could not resolve native task account"));
    }
    let mut buffer = vec![0usize; (sid_bytes as usize).div_ceil(size_of::<usize>())];
    let mut domain = vec![0u16; domain_chars as usize];
    // SAFETY: both buffers have the queried capacity and stay live through lookup
    // and SID serialization. ConvertSidToStringSidW returns a LocalFree allocation.
    unsafe {
        if LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut sid_bytes,
            domain.as_mut_ptr(),
            &mut domain_chars,
            &mut kind,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut text = ptr::null_mut();
        if ConvertSidToStringSidW(buffer.as_mut_ptr().cast(), &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16(std::slice::from_raw_parts(text, length))
            .map_err(|_| io::Error::other("Invalid SID encoding"));
        LocalFree(text.cast());
        result
    }
}

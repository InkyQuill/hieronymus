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

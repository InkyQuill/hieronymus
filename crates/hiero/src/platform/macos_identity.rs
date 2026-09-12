//! Public Security session identity: inherited across fork/exec, never selected
//! by caller environment or process group. Graphic access is mandatory.
use std::io;
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SessionGetInfo(session: u32, actual: *mut u32, attributes: *mut u32) -> i32;
}
pub fn session() -> io::Result<String> {
    let mut id = 0;
    let mut attributes = 0;
    // AuthSession.h: SecuritySessionId/SessionAttributeBits UInt32, OSStatus SInt32.
    let result = unsafe { SessionGetInfo(u32::MAX, &mut id, &mut attributes) };
    if result != 0 || id == 0 || id >= u32::MAX - 1 || attributes & 0x10 == 0 || attributes & 1 != 0
    {
        return Err(io::Error::other(
            "A trusted macOS graphical login session is required",
        ));
    }
    Ok(format!("security-{}-{id}", unsafe { libc::geteuid() }))
}
pub fn domain() -> io::Result<String> {
    session()?;
    Ok(super::super::service::macos_agent::domain(unsafe {
        libc::geteuid()
    }))
}

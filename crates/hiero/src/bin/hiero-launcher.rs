//! Immutable Windows command launcher; all endpoints consult one selection record.
#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
fn main() {
    let result = (|| -> std::io::Result<i32> {
        let launcher = std::env::current_exe()?;
        let executable = hiero::platform::install::selected_executable(&launcher)?;
        let mut command = std::process::Command::new(executable);
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        match launcher.file_stem().and_then(|name| name.to_str()) {
            Some("hieronymus-mcp") => {
                command.arg("mcp");
            }
            Some("hieronymus-agent-hook") => {
                command.arg("agent-hook");
            }
            _ => {}
        }
        Ok(command
            .args(std::env::args_os().skip(1))
            .status()?
            .code()
            .unwrap_or(1))
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("hiero launcher: {error}");
            std::process::exit(2);
        }
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("the immutable native launcher is only used on Windows");
    std::process::exit(2);
}

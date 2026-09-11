//! Opens only the already-minted grant URL, with fixed secret-free errors.
#[cfg(not(any(windows, target_os = "macos")))]
use std::process::{Command, Stdio};
#[cfg(not(any(windows, target_os = "macos")))]
use std::time::{Duration, Instant};
#[cfg(not(any(windows, target_os = "macos")))]
const OPENER_ENV: &str = "HIERO_CONSOLE_BROWSER";
#[cfg(not(any(windows, target_os = "macos")))]
const DEFAULT_OPENER: &str = "xdg-open";
/// Invoke the platform opener with `url`, discarding its streams so the URL
/// (which carries the grant) cannot be echoed anywhere. Returns `Err` when the
/// opener cannot be spawned, exits non-zero, or exceeds its ten-second budget.
pub fn open(options: &crate::service::ServiceOptions, url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos_broker::browser(options, url)
    }
    #[cfg(windows)]
    {
        crate::platform::windows_broker::browser(options, url)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = options;
        let opener = std::env::var(OPENER_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENER.to_string());
        open_with_command(&opener, url, Duration::from_secs(10))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn open_with_command(opener: &str, url: &str, timeout: Duration) -> Result<(), String> {
    let mut command = Command::new(opener);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "browser opener could not be started".to_owned())?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("browser opener timed out or could not be observed".into());
            }
        }
    };

    if status.success() {
        Ok(())
    } else {
        Err("browser opener failed".into())
    }
}

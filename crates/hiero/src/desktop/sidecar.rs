//! The foreground server maintains its native tray companion without GUI dependencies.
use hieronymus::data_root::HieronymusConfig;
use std::{
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn graphical_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        // User services need not inherit display variables from the login.
        // logind validates a local graphical session owned by this user.
        super::singleton::trusted_session().is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        cfg!(any(windows, target_os = "macos"))
    }
}

/// The helper's session singleton handles login races and duplicate launches.
/// `--resume` keeps this companion passive: it never starts a second server.
pub(crate) fn supervise(
    config: HieronymusConfig,
    service: Option<crate::service::ServiceOptions>,
    stop: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    if std::env::var_os("HIERONYMUS_HEADLESS").is_some_and(|value| value == "1") {
        return None;
    }
    Some(thread::spawn(move || {
        let mut child: Option<Child> = None;
        let mut next_start = Instant::now();
        let mut reported_failure = false;
        while !stop.load(Ordering::Acquire) {
            if let Some(running) = child.as_mut() {
                match running.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        if !status.success() {
                            eprintln!(
                                "Tray companion exited with {status}; retrying in five seconds"
                            );
                        }
                        child = None;
                        next_start = Instant::now() + Duration::from_secs(5);
                    }
                    Err(error) => {
                        if !reported_failure {
                            eprintln!("Tray companion status unavailable: {error}");
                            reported_failure = true;
                        }
                        // Keep the owned handle; an uncertain wait is not permission to duplicate it.
                    }
                }
            } else if Instant::now() >= next_start {
                if !graphical_session() {
                    next_start = Instant::now() + Duration::from_secs(5);
                    continue;
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
                match spawn(&config, service.as_ref(), &stop) {
                    Ok(Some(started)) => {
                        child = Some(started);
                        reported_failure = false;
                    }
                    Ok(None) => next_start = Instant::now() + Duration::from_secs(5),
                    Err(error) => {
                        if !reported_failure {
                            eprintln!("Tray companion unavailable: {error}");
                            reported_failure = true;
                        }
                        next_start = Instant::now() + Duration::from_secs(5);
                    }
                }
            }
            thread::sleep(Duration::from_millis(200));
        }
        // The helper observes server/root release and exits itself. During a
        // controlled restart it can attach to the replacement without losing
        // an in-flight native action; never kill that action from this thread.
    }))
}

fn spawn(
    config: &HieronymusConfig,
    service: Option<&crate::service::ServiceOptions>,
    stop: &AtomicBool,
) -> Result<Option<Child>, String> {
    // Avoid repeatedly launching short-lived duplicates when login startup or
    // a previous server already owns the helper. The helper still arbitrates races.
    match super::TraySingleton::acquire_or_existing(config, "supervisor")
        .map_err(|error| error.to_string())?
    {
        super::SingletonOutcome::AlreadyRunning => return Ok(None),
        super::SingletonOutcome::Acquired(guard) => drop(guard),
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let helper = super::launch::sibling_binary(&executable, "hiero-desktop")?;
    let binary = match service {
        Some(options) => options.binary.clone(),
        None => super::launch::stable_cli(&executable)?,
    };
    let mut command = Command::new(helper);
    #[cfg(target_os = "linux")]
    if !["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
    {
        command.envs(super::singleton::graphical_environment().map_err(|e| e.to_string())?);
    }
    command
        .arg("--resume")
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--binary")
        .arg(binary);
    detach_stdio(&mut command);
    crate::diagnostics::redirect(&mut command, config.data_root(), "desktop-helper.log")
        .map_err(|error| format!("Could not record helper diagnostics: {error}"))?;
    if let Some(options) = service {
        command.arg("--unit-dir").arg(&options.unit_dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    if stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    command.spawn().map(Some).map_err(|error| error.to_string())
}

fn detach_stdio(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
}

#[cfg(all(test, unix))]
mod pipe_tests {
    use super::*;
    #[test]
    fn running_helper_does_not_keep_invocation_stderr_pipe_open() {
        use std::io::Read;
        use std::os::fd::OwnedFd;
        let (mut reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let fd: OwnedFd = writer.into();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "exec sleep 30"])
            .stderr(Stdio::from(fd));
        detach_stdio(&mut command);
        let mut child = command.spawn().unwrap();
        drop(command);
        let result = reader.read(&mut [0; 1]);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            result.unwrap(),
            0,
            "helper must not keep pipe writers alive"
        );
    }
}

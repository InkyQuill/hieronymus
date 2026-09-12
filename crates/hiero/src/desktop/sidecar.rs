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
    if cfg!(target_os = "linux") {
        ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
    } else {
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
    if !graphical_session() {
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
                match spawn(&config, service.as_ref()) {
                    Ok(started) => {
                        child = Some(started);
                        reported_failure = false;
                    }
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
) -> Result<Child, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let helper = super::launch::sibling_binary(&executable, "hiero-desktop")?;
    let binary = match service {
        Some(options) => options.binary.clone(),
        None => super::launch::stable_cli(&executable)?,
    };
    let mut command = Command::new(helper);
    command
        .arg("--resume")
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--binary")
        .arg(binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(options) = service {
        command.arg("--unit-dir").arg(&options.unit_dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|error| error.to_string())
}

//! Bounded native COM/shell isolation. Parent owns lifecycle/registration guards.
//! Child holds both continuation gates BEFORE READY; only COMMIT permits native
//! work. A committed timeout is indeterminate and leaves the child/gates alive.
//! Every later common operation checks these gates before shutdown or mutation.
use super::native_gate::{self, BROWSER_GATE, MANAGER_GATE};
use crate::service::ServiceOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{self, BufRead, Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::{Pipes::PeekNamedPipe, Threading::CREATE_NO_WINDOW},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};
#[cfg(test)]
thread_local! {
    pub(crate) static SPAWN_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
const MAX_REQUEST: usize = 2048;
const MAX_RESPONSE: usize = 65536;
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum TaskAction {
    PackageCapture,
    PackageRestore,
    PackageCommit,
    Inspect,
    Install,
    Remove,
    Start,
    Rearm,
    Suppress,
    EnableLogin,
    DisableLogin,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    root: PathBuf,
    directory: PathBuf,
    binary: PathBuf,
    action: Option<TaskAction>,
    tray: bool,
    url: Option<String>,
}
pub(crate) fn task(
    options: &ServiceOptions,
    action: TaskAction,
    tray: bool,
) -> Result<Value, String> {
    call(
        options,
        Request {
            root: options.data_root.clone(),
            directory: options.unit_dir.clone(),
            binary: options.binary.clone(),
            action: Some(action),
            tray,
            url: None,
        },
        Duration::from_secs(30),
    )
}
/// Read-only native task inspection with common lifecycle/registration authority.
pub fn inspect(options: &ServiceOptions, tray: bool) -> Result<Value, String> {
    let operation = crate::lifecycle::operation::LifecycleOperation::acquire(
        &hieronymus::data_root::HieronymusConfig::new(&options.data_root),
    )
    .map_err(|e| e.to_string())?;
    operation
        .register_unit(options)
        .map_err(|e| e.to_string())?;
    task(options, TaskAction::Inspect, tray)
}
pub fn browser(options: &ServiceOptions, url: &str) -> Result<(), String> {
    call(
        options,
        Request {
            root: options.data_root.clone(),
            directory: options.unit_dir.clone(),
            binary: options.binary.clone(),
            action: None,
            tray: false,
            url: Some(url.into()),
        },
        Duration::from_secs(10),
    )
    .map(|_| ())
}
fn call(
    options: &ServiceOptions,
    mut request: Request,
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    request.root = request
        .root
        .canonicalize()
        .map_err(|_| "Native operation root is unavailable")?;
    std::fs::create_dir_all(&request.directory)
        .map_err(|_| "Native registration directory is unavailable")?;
    request.directory = request
        .directory
        .canonicalize()
        .map_err(|_| "Native registration directory is unavailable")?;
    let gate = if request.url.is_some() {
        BROWSER_GATE
    } else {
        MANAGER_GATE
    };
    drop(
        native_gate::acquire(&request.root, gate)
            .map_err(|_| "A native operation remains in progress")?,
    );
    if request.url.is_none() {
        native_gate::check(&request.directory)
            .map_err(|_| "A native registration operation remains in progress")?;
    }
    let mut bytes = serde_json::to_vec(&request).map_err(|_| "Invalid native request")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_REQUEST {
        return Err("Native operation request exceeds its bound".into());
    }
    let executable = crate::desktop::launch::selected_cli(&options.binary)?;
    #[cfg(test)]
    SPAWN_ATTEMPTS.set(SPAWN_ATTEMPTS.get() + 1);
    let mut child = Command::new(executable)
        .arg("__windows-native-broker")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Native broker could not start")?;
    let mut input = child
        .stdin
        .take()
        .ok_or("Native broker input is unavailable")?;
    // Below the anonymous pipe's 4KiB capacity; child reads this before READY.
    if input.write_all(&bytes).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Native broker request failed".into());
    }
    let mut output = child
        .stdout
        .take()
        .ok_or("Native broker output is unavailable")?;
    let mut pending = Vec::new();
    match line_until(&mut output, &mut pending, deadline) {
        Ok(line) if line == b"READY" && Instant::now() < deadline => {}
        _ => {
            drop(input);
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "Native broker did not become ready; no native operation was committed".into(),
            );
        }
    }
    // Once writing COMMIT is attempted the outcome may be indeterminate. Never kill.
    let committed = input.write_all(b"COMMIT\n").is_ok();
    drop(input);
    if !committed {
        return Err("Native operation outcome is indeterminate; wait for its gate to clear".into());
    }
    let line = line_until(&mut output, &mut pending, deadline).map_err(|_| "Native operation timed out; outcome is indeterminate and later operations remain blocked until it completes")?;
    let result: Result<Value, String> =
        serde_json::from_slice(&line).map_err(|_| "Native broker returned an invalid result")?;
    // The response is sent only after rollback/readback and all gate handles drop.
    result
}
fn line_until(
    pipe: &mut std::process::ChildStdout,
    pending: &mut Vec<u8>,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    loop {
        if let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line = pending.drain(..=end).collect::<Vec<_>>();
            return Ok(line[..end].to_vec());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Native deadline"));
        }
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if available > 0 {
            let mut buffer = [0u8; 4096];
            let count = pipe.read(&mut buffer[..(available as usize).min(4096)])?;
            if count == 0 || pending.len() + count > MAX_RESPONSE {
                return Err(io::Error::other("Native response bound"));
            }
            pending.extend_from_slice(&buffer[..count]);
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
/// Hidden CLI dispatch. Only bounded private pipe input is accepted; no argv URL.
pub fn run() -> Result<(), String> {
    let input = io::stdin();
    let mut input = input.lock();
    let mut line = String::new();
    (&mut input)
        .take(MAX_REQUEST as u64 + 1)
        .read_line(&mut line)
        .map_err(|_| "Native input failed")?;
    if line.len() > MAX_REQUEST || !line.ends_with('\n') {
        return Err("Invalid native request size".into());
    }
    let request: Request = serde_json::from_str(&line).map_err(|_| "Invalid native request")?;
    if request.root.canonicalize().ok().as_ref() != Some(&request.root)
        || request.directory.canonicalize().ok().as_ref() != Some(&request.directory)
    {
        return Err("Native root/directory must be canonical".into());
    }
    let result = committed(&request, &mut input);
    let bytes = serde_json::to_vec(&result).map_err(|_| "Native result encoding failed")?;
    if bytes.len() >= MAX_RESPONSE {
        return Err("Native result exceeds its bound".into());
    }
    let mut output = io::stdout().lock();
    output
        .write_all(&bytes)
        .and_then(|_| output.write_all(b"\n"))
        .map_err(|_| "Native result delivery failed".into())
}
fn committed(request: &Request, input: &mut impl BufRead) -> Result<Value, String> {
    let gate = if request.url.is_some() {
        BROWSER_GATE
    } else {
        MANAGER_GATE
    };
    let _root =
        native_gate::acquire(&request.root, gate).map_err(|_| "Native root gate is busy")?;
    let _registration = if request.url.is_none() && request.directory != request.root {
        Some(
            native_gate::acquire(&request.directory, MANAGER_GATE)
                .map_err(|_| "Native registration gate is busy")?,
        )
    } else {
        None
    };
    super::native_protocol::committed(input, &mut io::stdout(), (_root, _registration), || {
        match (&request.url, request.action) {
            (Some(url), None) => {
                // Restrict the private broker to the exact existing local grant URL shape.
                let parsed = url::Url::parse(url).map_err(|_| "Invalid browser request")?;
                if parsed.scheme() != "http"
                    || !matches!(parsed.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                        && !matches!(parsed.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
                    || !matches!(parsed.path(), "/admin" | "/config")
                    || parsed.query().is_some()
                    || !parsed
                        .fragment()
                        .is_some_and(|s| s.starts_with("launch_grant="))
                {
                    return Err("Invalid browser request".into());
                }
                let text: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
                let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
                let result = unsafe {
                    ShellExecuteW(
                        std::ptr::null_mut(),
                        verb.as_ptr(),
                        text.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    )
                };
                if result as isize <= 32 {
                    return Err("Native browser opener failed".into());
                }
                Ok(Value::Null)
            }
            (None, Some(action)) => crate::service::windows::execute(
                &ServiceOptions {
                    data_root: request.root.clone(),
                    unit_dir: request.directory.clone(),
                    binary: request.binary.clone(),
                    use_manager: true,
                },
                action,
                request.tray,
            ),
            _ => Err("Invalid native operation".into()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_transport_timeout_before_commit_aborts_and_after_commit_retains_authority() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let units = temp.path().join("units");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&units).unwrap();
        let fixture = temp.path().join("after.exe");
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/windows-native-broker.rs");
        assert!(
            Command::new("rustc")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["--edition=2024", "-Adead_code"])
                .arg(source)
                .arg("-o")
                .arg(&fixture)
                .status()
                .unwrap()
                .success()
        );
        let before = temp.path().join("before.exe");
        std::fs::copy(&fixture, &before).unwrap();
        let mut options = ServiceOptions {
            data_root: root.clone(),
            unit_dir: units.clone(),
            binary: before,
            use_manager: true,
        };
        let request = |options: &ServiceOptions| Request {
            root: options.data_root.clone(),
            directory: options.unit_dir.clone(),
            binary: options.binary.clone(),
            action: Some(TaskAction::Install),
            tray: false,
            url: None,
        };
        let error = call(&options, request(&options), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("no native operation was committed"));
        assert!(!temp.path().join("committed").exists());
        native_gate::check(&root).unwrap();
        native_gate::check(&units).unwrap();
        options.binary = fixture;
        let began = Instant::now();
        let error = call(&options, request(&options), Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("indeterminate"));
        assert!(began.elapsed() < Duration::from_secs(3));
        assert!(temp.path().join("committed").exists());
        assert_eq!(
            native_gate::check(&root).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            native_gate::check(&units).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            crate::lifecycle::operation::LifecycleOperation::acquire(
                &hieronymus::data_root::HieronymusConfig::new(&root)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::WouldBlock
        );
        let began = Instant::now();
        assert!(task(&options, TaskAction::Inspect, false).is_err());
        assert!(began.elapsed() < Duration::from_millis(250));
        let deadline = Instant::now() + Duration::from_secs(5);
        while native_gate::check(&root).is_err() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        native_gate::check(&root).unwrap();
        native_gate::check(&units).unwrap();
    }
}

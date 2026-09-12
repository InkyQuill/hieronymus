//! Authenticated root/session/instance-bound helper retirement. PIDs confer no authority.
//! Lock order: lifecycle -> registration -> helper launch gate -> session locks.
use super::controller::RetirementHandle;
use hieronymus::{data_root::HieronymusConfig, private_file};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, TryLockError},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
pub const LAUNCH_GATE: &str = ".desktop-launch.lock";
const QUIT: &str = ".desktop-quit.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u8,
    root: PathBuf,
    session_lock: String,
    instance: String,
    token: String,
    address: SocketAddr,
    executable: PathBuf,
}
fn error(s: impl std::fmt::Display) -> io::Error {
    io::Error::other(s.to_string())
}
fn nonce() -> io::Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|_| error("Could not create helper control secret"))?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}
/// Persistent files are never removed: an opened inode remains coordination authority.
pub fn lock(path: &Path) -> io::Result<File> {
    if !path.exists() {
        match private_file::create_private_new(path, b"") {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let f = private_file::open_coordination(path)?;
    f.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "Desktop operation/session is still active; retry after completion",
        ),
        TryLockError::Error(e) => e,
    })?;
    Ok(f)
}
pub fn launch_gate(config: &HieronymusConfig) -> io::Result<File> {
    std::fs::create_dir_all(config.data_root())?;
    lock(&config.data_root().join(LAUNCH_GATE))
}
pub fn record_quit(root: &Path) -> io::Result<()> {
    private_file::replace_private(&root.join(QUIT), nonce()?.as_bytes())
}
pub fn clear_quit(root: &Path) -> io::Result<()> {
    match std::fs::remove_file(root.join(QUIT)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}
pub fn quit_requested(root: &Path) -> io::Result<bool> {
    match private_file::read_private(&root.join(QUIT)) {
        Ok(bytes) if bytes.len() == 64 => Ok(true),
        Ok(_) => Err(error("Invalid durable desktop Quit intent")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
pub struct Server {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
}
impl Server {
    pub(crate) fn start(
        root: &Path,
        session_lock: &Path,
        quit_intent: Arc<AtomicBool>,
        handle: RetirementHandle,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let record = Record {
            format: 1,
            root: root.canonicalize()?,
            session_lock: session_lock
                .file_name()
                .ok_or_else(|| error("Missing session lock"))?
                .to_string_lossy()
                .into(),
            instance: nonce()?,
            token: nonce()?,
            address: listener.local_addr()?,
            executable: std::env::current_exe()?.canonicalize()?,
        };
        let path = session_lock.with_extension("json");
        private_file::replace_private(&path, &serde_json::to_vec(&record).map_err(error)?)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                        let bytes =
                            read_frame(&mut stream, Duration::from_secs(1)).unwrap_or_default();
                        let text = std::str::from_utf8(&bytes).unwrap_or("");
                        let ping = format!("{} {} ping\n", record.token, record.instance);
                        let retire = format!("{} {} retire\n", record.token, record.instance);
                        let response = if text == ping {
                            "ready"
                        } else if text == retire {
                            let flushed = !quit_intent.load(Ordering::Acquire)
                                || record_quit(&record.root).is_ok();
                            if flushed && handle.retire().is_ok() {
                                "retiring"
                            } else {
                                "busy"
                            }
                        } else {
                            "denied"
                        };
                        let _ = stream
                            .write_all(format!("{} {response}\n", record.instance).as_bytes());
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    Err(_) => break,
                }
            }
            if quit_intent.load(Ordering::Acquire) {
                record_quit(&record.root)?;
            }
            // Record removal is a completion receipt; a failure leaves a stale record
            // and replacement refuses even if the process later releases its OS lock.
            std::fs::remove_file(path)
        });
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| error("Helper control worker failed"))??;
        }
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
fn read_record(path: &Path, root: &Path, lock_name: &str) -> io::Result<Record> {
    if std::fs::symlink_metadata(path)?.len() > 8192 {
        return Err(error("Helper record exceeds bound"));
    }
    let bytes = private_file::read_private(path)?;
    if bytes.len() > 8192 {
        return Err(error("Helper record exceeds bound"));
    }
    let r: Record = serde_json::from_slice(&bytes).map_err(error)?;
    if r.format != 1
        || r.root != root
        || r.session_lock != lock_name
        || !r.address.ip().is_loopback()
        || r.token.len() != 64
        || r.instance.len() != 64
        || !r.executable.is_absolute()
    {
        return Err(error("Helper control identity mismatch"));
    }
    Ok(r)
}
/// One absolute deadline covers the whole frame, including a peer trickling bytes.
fn read_frame(stream: &mut TcpStream, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::with_capacity(256);
    while bytes.len() < 256 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Helper frame deadline exceeded",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(bytes);
        }
    }
    Err(error("Helper frame exceeds bound"))
}
fn request(r: &Record, action: &str) -> io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&r.address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{} {} {action}\n", r.token, r.instance).as_bytes())?;
    let bytes = read_frame(&mut stream, Duration::from_secs(2))?;
    let text = String::from_utf8(bytes).map_err(error)?;
    text.strip_prefix(&format!("{} ", r.instance))
        .map(|s| s.trim().to_owned())
        .ok_or_else(|| error("Helper acknowledgement identity mismatch"))
}
/// Holds the launch gate before enumeration; accepts at most the caller's native session.
/// Unknown/malformed live ownership refuses. OS lock release, never an ack/PID, proves retirement.
pub struct Retirement {
    gate: Option<File>,
    sessions: Vec<File>,
    pub was_running: bool,
}
impl Retirement {
    pub fn begin(config: &HieronymusConfig) -> io::Result<Self> {
        Self::begin_mode(config, true)
    }
    pub fn begin_mode(config: &HieronymusConfig, retire_active: bool) -> io::Result<Self> {
        Self::begin_owned(config, retire_active, None)
    }
    pub fn begin_owned(
        config: &HieronymusConfig,
        retire_active: bool,
        expected_cli: Option<&Path>,
    ) -> io::Result<Self> {
        let gate = launch_gate(config)?;
        let root = config.data_root().canonicalize()?;
        #[cfg(any(windows, target_os = "macos"))]
        {
            crate::platform::native_gate::check(&root)?;
            drop(crate::platform::native_gate::acquire(
                &root,
                crate::platform::native_gate::BROWSER_GATE,
            )?);
        }
        let mut sessions = Vec::new();
        let mut active = Vec::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !super::singleton::is_session_lock_name(&name) {
                continue;
            }
            match lock(&entry.path()) {
                Ok(f) => {
                    if entry.path().with_extension("json").try_exists()? {
                        return Err(error(
                            "Stale helper control record; launch and cleanly quit the selected helper before retrying",
                        ));
                    }
                    sessions.push(f);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let r = read_record(&entry.path().with_extension("json"), &root, &name)?;
                    active.push((entry.path(), r));
                }
                Err(e) => return Err(e),
            }
        }
        if !active.is_empty() {
            if !retire_active {
                return Err(error(
                    "--no-activate cannot take over an active helper; quit it first",
                ));
            }
            if let Some(cli) = expected_cli {
                let expected = super::launch::selected_helper(cli)
                    .map_err(error)?
                    .canonicalize()?;
                if active.iter().any(|(_, r)| r.executable != expected) {
                    return Err(error(
                        "Active helper belongs to a different installed binary; quit that instance before replacement",
                    ));
                }
            }
            let own = super::singleton::TraySingleton::session_path(config)?;
            if active.len() != 1 || active[0].0 != own {
                return Err(error(
                    "Other desktop sessions own this installation; quit them before updating",
                ));
            }
            if request(&active[0].1, "retire")? != "retiring" {
                return Err(error(
                    "A desktop action is pending or Quit failed; resolve it in the helper before retrying",
                ));
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match lock(&active[0].0) {
                    Ok(f) => {
                        if active[0].0.with_extension("json").try_exists()? {
                            return Err(error(
                                "Helper retirement did not complete durable intent cleanup; replacement refused",
                            ));
                        }
                        sessions.push(f);
                        break;
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    Err(e) => {
                        return Err(error(format!(
                            "Helper has not released session ownership; nothing may be replaced: {e}"
                        )));
                    }
                }
            }
        }
        Ok(Self {
            gate: Some(gate),
            sessions,
            was_running: !active.is_empty(),
        })
    }
    pub fn allow_launch(&mut self) {
        self.sessions.clear();
        self.gate.take();
    }
}
/// Start only an explicitly previously-active helper, without auto-starting the daemon.
/// Authentication and session ownership are required; a spawned PID alone is not success.
pub fn restart(config: &HieronymusConfig, cli: &Path, unit_dir: &Path) -> io::Result<()> {
    if quit_requested(config.data_root())? {
        return Ok(());
    }
    let helper = super::launch::selected_helper(cli).map_err(error)?;
    let mut command = std::process::Command::new(&helper);
    command
        .arg("--data-root")
        .arg(config.data_root())
        .arg("--resume")
        .arg("--unit-dir")
        .arg(unit_dir)
        .arg("--binary")
        .arg(cli);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let path = super::singleton::TraySingleton::session_path(config)?;
    let root = config.data_root().canonicalize()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return Err(error(
                "Replacement desktop helper exited before authenticated startup",
            ));
        }
        if let Ok(r) = read_record(
            &path.with_extension("json"),
            &root,
            &path.file_name().unwrap().to_string_lossy(),
        ) && r.executable == helper.canonicalize()?
            && request(&r, "ping").is_ok_and(|s| s == "ready")
            && matches!(lock(&path),Err(e) if e.kind()==io::ErrorKind::WouldBlock)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(error(
        "Replacement helper startup is indeterminate; ownership must be retired before rollback",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Action, Controller, DesktopBackend, Event, PollSchedule, TraySingleton};
    struct Backend {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl DesktopBackend for Backend {
        fn probe(&mut self) -> Event {
            Event::Stopped
        }
        fn perform(&mut self, _: &Action) -> Result<(), String> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(())
        }
    }
    fn fixture() -> (
        tempfile::TempDir,
        HieronymusConfig,
        TraySingleton,
        Controller,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(temp.path().canonicalize().unwrap());
        let mut singleton =
            TraySingleton::acquire_trusted(&config, "disposable-test-session").unwrap();
        let (entered, rx) = std::sync::mpsc::channel();
        let (tx, release) = std::sync::mpsc::channel();
        let mut controller = Controller::spawn_with_schedule(
            Backend { entered, release },
            PollSchedule {
                poll_interval: Duration::from_millis(20),
                startup_timeout: Duration::from_millis(50),
            },
        );
        controller.attach_control(&config, &mut singleton).unwrap();
        (temp, config, singleton, controller, rx, tx)
    }
    #[test]
    fn authenticated_retirement_refuses_pending_then_acknowledges_but_lock_is_release_proof() {
        let (_temp, config, singleton, controller, entered, release) = fixture();
        let record = read_record(
            &singleton.path.with_extension("json"),
            config.data_root(),
            &singleton.path.file_name().unwrap().to_string_lossy(),
        )
        .unwrap();
        controller.submit(Action::Restart).unwrap();
        entered.recv().unwrap();
        assert_eq!(request(&record, "retire").unwrap(), "busy");
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if matches!(controller.try_event(), Some(Event::Finished { .. })) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(request(&record, "retire").unwrap(), "retiring");
        assert_eq!(
            lock(&singleton.path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        controller.shutdown().unwrap();
        let path = singleton.path.clone();
        drop(singleton);
        assert!(lock(&path).is_ok());
    }
    #[test]
    fn forged_control_and_wrong_root_are_refused_and_quit_survives_retirement_race() {
        let (_temp, config, singleton, controller, _entered, _release) = fixture();
        let path = singleton.path.with_extension("json");
        let name = singleton.path.file_name().unwrap().to_string_lossy();
        assert!(read_record(&path, Path::new("/wrong-root"), &name).is_err());
        let mut record = read_record(&path, config.data_root(), &name).unwrap();
        let token = record.token.clone();
        record.token = "0".repeat(64);
        assert_eq!(request(&record, "retire").unwrap(), "denied");
        record.token = token;
        assert_eq!(request(&record, "retire").unwrap(), "retiring");
        assert!(controller.submit(Action::Quit).is_err());
        controller.shutdown().unwrap();
        assert!(quit_requested(config.data_root()).unwrap());
    }
    #[test]
    fn launch_gate_precedes_session_enumeration_and_never_disappears() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let mut retirement = Retirement::begin(&config).unwrap();
        assert!(TraySingleton::acquire_trusted(&config, "session").is_err());
        retirement.allow_launch();
        assert!(root.path().join(LAUNCH_GATE).exists());
        assert!(TraySingleton::acquire_trusted(&config, "session").is_ok());
    }
    #[test]
    fn unknown_live_session_without_authenticated_record_refuses_before_replacement() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let held =
            lock(&root.path().join(
                ".tray-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.lock",
            ))
            .unwrap();
        assert!(Retirement::begin(&config).is_err());
        drop(held);
    }
    #[test]
    fn failed_quit_keeps_the_helper_owned_and_refuses_retirement() {
        struct FailsQuit;
        impl DesktopBackend for FailsQuit {
            fn probe(&mut self) -> Event {
                Event::Stopped
            }
            fn perform(&mut self, _: &Action) -> Result<(), String> {
                Err("stop refused".into())
            }
        }
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let mut singleton = TraySingleton::acquire_trusted(&config, "failed-quit-test").unwrap();
        let mut controller = Controller::spawn(FailsQuit);
        controller.attach_control(&config, &mut singleton).unwrap();
        controller.submit(Action::Quit).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if matches!(
                controller.try_event(),
                Some(Event::Finished {
                    action: Action::Quit,
                    error: Some(_)
                })
            ) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let record = read_record(
            &singleton.path.with_extension("json"),
            &root.path().canonicalize().unwrap(),
            &singleton.path.file_name().unwrap().to_string_lossy(),
        )
        .unwrap();
        assert_eq!(request(&record, "retire").unwrap(), "busy");
        assert_eq!(
            lock(&singleton.path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(quit_requested(root.path()).unwrap());
        controller.shutdown().unwrap();
    }
    #[test]
    fn trickled_control_frame_cannot_extend_absolute_deadline() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..8 {
                if client.write_all(b"a").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let started = Instant::now();
        assert!(read_frame(&mut server, Duration::from_millis(75)).is_err());
        assert!(started.elapsed() < Duration::from_millis(160));
        drop(server);
        writer.join().unwrap();
    }
    #[test]
    fn pending_native_quit_only_sets_a_flag_then_control_worker_persists_before_release() {
        let (_temp, config, singleton, controller, entered, release) = fixture();
        controller.submit(Action::Restart).unwrap();
        entered.recv().unwrap();
        assert!(controller.submit(Action::Quit).is_err());
        assert!(
            !quit_requested(config.data_root()).unwrap(),
            "UI submission must not perform file I/O"
        );
        release.send(()).unwrap();
        controller.shutdown().unwrap();
        assert!(quit_requested(config.data_root()).unwrap());
        assert!(!singleton.path.with_extension("json").exists());
    }
    #[test]
    fn unlocked_stale_control_record_is_not_retirement_completion() {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        drop(
            lock(&root.path().join(
                ".tray-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.lock",
            ))
            .unwrap(),
        );
        private_file::create_private_new(
            &root.path().join(
                ".tray-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.json",
            ),
            b"malformed",
        )
        .unwrap();
        assert!(Retirement::begin(&config).is_err());
    }
    #[test]
    fn late_quit_flush_failure_keeps_record_and_refuses_replacement_after_exit() {
        let (_temp, config, singleton, controller, _entered, _release) = fixture();
        std::fs::create_dir(config.data_root().join(QUIT)).unwrap();
        controller
            .quit_intent_handle()
            .store(true, Ordering::Release);
        assert!(controller.shutdown().is_err());
        assert!(singleton.path.with_extension("json").exists());
        drop(singleton);
        assert!(Retirement::begin(&config).is_err());
    }
}

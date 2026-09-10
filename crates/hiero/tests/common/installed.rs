//! Explicit installed release support, shared by the live product and registry tests.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hiero::daemon_client::DaemonClient;
use hieronymus::data_root::HieronymusConfig;

pub fn binary() -> PathBuf {
    let app = std::env::var_os("HIERO_TEST_INSTALLED_APP")
        .expect("HIERO_TEST_INSTALLED_APP must name a disposable installed application");
    std::fs::canonicalize(PathBuf::from(app).join("bin/hiero"))
        .expect("installed bin/hiero must exist; no development fallback")
}

/// Ordinary regression tests keep their development default; any explicit
/// installed selection is validated strictly by the same live-test helper.
pub fn binary_or_development() -> PathBuf {
    if std::env::var_os("HIERO_TEST_INSTALLED_APP").is_some() {
        binary()
    } else {
        PathBuf::from(env!("CARGO_BIN_EXE_hiero"))
    }
}

pub fn command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("HOME", root)
        .env("PATH", "")
        .arg("--data-root")
        .arg(root);
    command
}

pub struct InstalledDaemon {
    child: Child,
    pub client: DaemonClient,
}

impl InstalledDaemon {
    pub fn start(binary: &Path, root: &Path) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let mut child = command(binary, root)
            .args(["daemon", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(root.join("rehearsal-daemon.log")).unwrap())
            .spawn()
            .expect("installed daemon must spawn");
        let config = HieronymusConfig::new(root);
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut last_state = serde_json::Value::Null;
        loop {
            if let Ok(client) = DaemonClient::connect(&config)
                && client
                    .get("/status")
                    .is_ok_and(|s| s["semantic"]["state"] == "ready")
            {
                return Self { child, client };
            }
            if let Ok(client) = DaemonClient::connect(&config)
                && let Ok(status) = client.get("/status")
            {
                last_state = status["semantic"].clone();
            }
            if child.try_wait().unwrap().is_some()
                || last_state["state"] == "failed"
                || Instant::now() >= deadline
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "installed daemon failed to reach authenticated semantic ready: {last_state}"
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn ready(&self) -> bool {
        self.client
            .get("/status")
            .is_ok_and(|s| s["semantic"]["state"] == "ready")
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn wait_stopped(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "daemon shutdown must succeed: {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "daemon failed to stop within 30s"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for InstalledDaemon {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

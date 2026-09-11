//! Explicit native final-payload qualification, independent of desktop-manager acceptance.
use hiero::daemon_client::DaemonClient;
use hieronymus::data_root::HieronymusConfig;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires HIERO_DESKTOP_INSTALLED_CLI naming a final verified disposable payload"]
fn final_native_payload_runs_semantic_inference_and_authenticated_mcp() {
    let binary = PathBuf::from(
        std::env::var_os("HIERO_DESKTOP_INSTALLED_CLI")
            .expect("HIERO_DESKTOP_INSTALLED_CLI required"),
    )
    .canonicalize()
    .expect("final installed executable must exist");
    let root = tempfile::tempdir().unwrap();
    let command = || {
        let mut c = Command::new(&binary);
        for key in [
            "HIERO_SEMANTIC_MODEL_DIR",
            "HIERO_ONNX_RUNTIME",
            "ORT_DYLIB_PATH",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
        ] {
            c.env_remove(key);
        }
        c.env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"));
        c
    };
    let assets = command()
        .args(["release-assets", "--output"])
        .arg(binary.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        assets.status.success(),
        "native inference: {}",
        String::from_utf8_lossy(&assets.stderr)
    );
    let expected = std::fs::read(binary.parent().unwrap().join("assets.json")).unwrap();
    let expected: serde_json::Value = serde_json::from_slice(&expected).unwrap();
    let actual: serde_json::Value = serde_json::from_slice(&assets.stdout).unwrap();
    assert_eq!(actual, expected);
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.0.try_wait().unwrap().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut child = Owned(
        command()
            .args(["daemon", "--port", "0", "--data-root"])
            .arg(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let config = HieronymusConfig::new(root.path());
    let deadline = Instant::now() + Duration::from_secs(120);
    let client = loop {
        if let Ok(client) = DaemonClient::connect(&config)
            && client
                .get("/status")
                .is_ok_and(|s| s["semantic"]["state"] == "ready")
        {
            break client;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "native daemon exited"
        );
        assert!(
            Instant::now() < deadline,
            "native semantic readiness timeout"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let status = client
        .call_tool("hieronymus_status", &serde_json::json!({}))
        .unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["service"]["mode"],
        "local-http"
    );
    client.post("/shutdown", &serde_json::json!({})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "authenticated shutdown timeout");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!config.daemon_discovery_path().exists());
}

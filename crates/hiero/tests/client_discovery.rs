//! Real entry points must authenticate discovery before sending tool traffic.
use hiero::daemon::discovery::{self, DiscoveryRecord};
use hiero::daemon::{Daemon, DaemonOptions};
use hieronymus::data_root::HieronymusConfig;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

fn command(root: &std::path::Path, stdio: bool) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hiero"));
    if stdio {
        cmd.arg("mcp");
    } else {
        cmd.args(["tool-call", "hieronymus_series_list"]);
    }
    cmd.arg("--data-root").arg(root).stdin(Stdio::null());
    cmd
}

#[test]
fn cli_and_stdio_reject_missing_stale_instance_and_protocol_discovery() {
    for case in ["missing", "stale", "instance", "protocol", "non-loopback"] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path());
        let daemon = Daemon::start(&DaemonOptions {
            data_root: Some(root.path().into()),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        let token = std::fs::read_to_string(config.daemon_token_path()).unwrap();
        let mut record = discovery::read_discovery(&config).unwrap();
        let daemon = if case == "stale" {
            daemon.shutdown().unwrap();
            None
        } else {
            Some(daemon)
        };
        if case == "instance" {
            record.instance_id = "wrong-instance".into();
        }
        if case == "protocol" {
            record.protocol_version = "wrong-protocol".into();
        }
        if case == "non-loopback" {
            record.host = "10.1.2.3".into();
        }
        discovery::write_discovery(&config, &record).unwrap();
        if case == "missing" {
            std::fs::remove_file(config.daemon_discovery_path()).unwrap();
        }
        for stdio in [false, true] {
            let output = command(root.path(), stdio).output().unwrap();
            assert!(!output.status.success(), "{case}, stdio={stdio}");
            assert!(output.stdout.is_empty(), "{case}: {:?}", output.stdout);
            assert!(!String::from_utf8_lossy(&output.stderr).contains(token.trim()));
        }
        drop(daemon);
    }
}

#[test]
fn reused_port_receives_only_status_probes_from_both_entry_points() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    discovery::write_discovery(
        &config,
        &DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: hiero::daemon::registry::PROTOCOL_REVISION.into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            pid: 1,
            instance_id: "old-instance".into(),
            started_at: "2026-09-06T00:00:00Z".into(),
        },
    )
    .unwrap();
    let responder = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 8192];
            let count = stream.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..count]).starts_with("GET /status "));
            stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
        }
    });
    for stdio in [false, true] {
        let output = command(root.path(), stdio).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    responder.join().unwrap();
}

#[test]
fn only_explicit_opt_in_calls_the_managed_service() {
    for (stdio, stale) in [(false, false), (true, false), (false, true), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("data"));
        let daemon = Daemon::start(&DaemonOptions {
            data_root: Some(config.data_root().into()),
            port: 0,
            ..Default::default()
        })
        .unwrap();
        let saved = root.path().join("saved.json");
        std::fs::rename(config.daemon_discovery_path(), &saved).unwrap();
        if stale {
            let mut record: DiscoveryRecord =
                serde_json::from_slice(&std::fs::read(&saved).unwrap()).unwrap();
            record.instance_id = "previous-instance".into();
            discovery::write_discovery(&config, &record).unwrap();
        }
        let shim = root.path().join("systemctl");
        std::fs::write(&shim, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TEST_MANAGER_LOG\"\nif [ \"$2\" = start ]; then /bin/cp \"$TEST_DISCOVERY_SAVED\" \"$TEST_DISCOVERY_TARGET\"; fi\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = root.path().join("manager.log");
        let run = |opt_in| {
            let mut cmd = command(config.data_root(), stdio);
            cmd.env("HOME", root.path())
                .env("PATH", root.path())
                .env("TEST_MANAGER_LOG", &log)
                .env("TEST_DISCOVERY_SAVED", &saved)
                .env("TEST_DISCOVERY_TARGET", config.daemon_discovery_path());
            if opt_in {
                cmd.arg("--start-daemon");
            }
            cmd.output().unwrap()
        };
        assert!(!run(false).status.success());
        assert!(!log.exists());
        let output = run(true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = std::fs::read_to_string(log).unwrap();
        assert!(calls.contains("--user start hieronymus.service"), "{calls}");
        if stdio {
            assert!(output.stdout.is_empty());
        }
        daemon.shutdown().unwrap();
    }
}

#[test]
fn agreeing_remote_protocol_still_must_be_supported_by_this_client() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    discovery::write_discovery(
        &config,
        &DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: "future-protocol".into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            pid: 1,
            instance_id: "matching-instance".into(),
            started_at: "2026-09-06T00:00:00Z".into(),
        },
    )
    .unwrap();
    let responder = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 8192];
            let count = stream.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..count]).starts_with("GET /status "));
            let body =
                r#"{"instance_id":"matching-instance","protocol_revision":"future-protocol"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    for stdio in [false, true] {
        let output = command(root.path(), stdio).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("protocol"));
    }
    responder.join().unwrap();
}

#[test]
fn stdio_wraps_route_errors_after_successful_authenticated_startup() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let token = discovery::generate_bearer_token().unwrap();
    discovery::write_token(&config, &token).unwrap();
    discovery::write_discovery(
        &config,
        &DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: hiero::daemon::registry::PROTOCOL_REVISION.into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            pid: 1,
            instance_id: "matching-instance".into(),
            started_at: "2026-09-06T00:00:00Z".into(),
        },
    )
    .unwrap();
    let responder = std::thread::spawn(move || {
        for probe in [true, false] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 8192];
            let count = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            assert!(request.contains(token.expose_secret().as_str()));
            let (status, body) = if probe {
                assert!(request.starts_with("GET /status "));
                (200, serde_json::json!({"instance_id":"matching-instance", "protocol_revision":hiero::daemon::registry::PROTOCOL_REVISION}).to_string())
            } else {
                assert!(request.starts_with("POST /mcp "));
                (401, r#"{"error":"unauthorized"}"#.into())
            };
            write!(stream, "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let mut child = command(root.path(), true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["id"], 7);
    assert_eq!(body["error"]["code"], -32603);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unauthorized")
    );
    responder.join().unwrap();
}

//! Real entry points must authenticate discovery before sending tool traffic.
use hiero::daemon::discovery::{self, DiscoveryRecord};
use hiero::daemon::{Daemon, DaemonOptions};
use hieronymus::data_root::HieronymusConfig;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
#[cfg(target_os = "linux")]
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

// A mock must consume the entire request before closing its response socket.
// The real client writes headers and body separately; closing after one read
// can reset that connection while the POST body is still in flight.
fn read_complete_request(reader: &mut impl Read) -> std::io::Result<String> {
    const MAX_REQUEST: usize = 64 * 1024;
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid or oversized mock request",
        )
    };
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        if request.len() == MAX_REQUEST {
            return Err(invalid());
        }
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        request.push(byte[0]);
    }
    let headers = std::str::from_utf8(&request).map_err(|_| invalid())?;
    let body_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, length)| length.trim().parse::<usize>())
        .transpose()
        .map_err(|_| invalid())?
        .unwrap_or(0);
    let header_length = request.len();
    let total = header_length
        .checked_add(body_length)
        .filter(|total| *total <= MAX_REQUEST)
        .ok_or_else(invalid)?;
    request.resize(total, 0);
    reader.read_exact(&mut request[header_length..])?;
    String::from_utf8(request).map_err(|_| invalid())
}

fn read_mock_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    read_complete_request(stream).expect("the mock must receive complete request headers and body")
}

#[test]
fn mock_requests_require_complete_bounded_headers_and_bodies() {
    let request = b"POST /mcp HTTP/1.1\r\nContent-Length: 8\r\n\r\n{\"id\":7}";
    let split = request.len() - 6;
    let mut fragmented =
        std::io::Cursor::new(&request[..split]).chain(std::io::Cursor::new(&request[split..]));
    assert_eq!(
        read_complete_request(&mut fragmented).unwrap().as_bytes(),
        request
    );
    for truncated in [&request[..10], &request[..request.len() - 1]] {
        assert_eq!(
            read_complete_request(&mut std::io::Cursor::new(truncated))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }
    let oversized = b"POST /mcp HTTP/1.1\r\nContent-Length: 65536\r\n\r\n";
    assert_eq!(
        read_complete_request(&mut std::io::Cursor::new(oversized))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
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
            let request = read_mock_request(&mut stream);
            assert!(request.starts_with("GET /status "));
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
fn opt_in_cannot_bypass_owned_root_or_identity_mismatch() {
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
        #[cfg(target_os = "linux")]
        {
            let shim = root.path().join("systemctl");
            std::fs::write(&shim, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TEST_MANAGER_LOG\"\nif [ \"$2\" = start ]; then /bin/cp \"$TEST_DISCOVERY_SAVED\" \"$TEST_DISCOVERY_TARGET\"; fi\n").unwrap();
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
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
        assert!(!output.status.success());
        assert!(!log.exists());
        assert!(output.stdout.is_empty());
        daemon.shutdown().unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
#[cfg(target_os = "linux")]
fn only_explicit_opt_in_starts_a_truly_stopped_root() {
    use std::time::{Duration, Instant};
    for stdio in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("data"));
        let log = root.path().join("manager.log");
        let shim = root.path().join("systemctl");
        std::fs::write(
            &shim,
            "#!/bin/sh\nprintf '%s\n' \"$*\" >> \"$TEST_MANAGER_LOG\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |opt_in| {
            let mut cmd = command(config.data_root(), stdio);
            cmd.env("HOME", root.path())
                .env("PATH", root.path())
                .env("TEST_MANAGER_LOG", &log);
            if opt_in {
                cmd.arg("--start-daemon");
            }
            cmd.output().unwrap()
        };
        assert!(!run(false).status.success());
        assert!(!log.exists());
        // Publish a real daemon only when the disposable manager receives
        // start. The root is actually unowned before the manager action.
        let daemon_root = config.data_root().to_owned();
        let manager_log = log.clone();
        let (done, stop) = std::sync::mpsc::channel();
        let manager = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if std::fs::read_to_string(&manager_log)
                    .unwrap_or_default()
                    .contains("--user start hieronymus.service")
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "manager was never asked to start"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let daemon = Daemon::start(&DaemonOptions {
                data_root: Some(daemon_root),
                port: 0,
                ..Default::default()
            })
            .unwrap();
            let _ = stop.recv_timeout(Duration::from_secs(30));
            daemon.shutdown().unwrap();
        });
        let output = run(true);
        let _ = done.send(());
        manager.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            std::fs::read_to_string(log)
                .unwrap()
                .contains("--user start hieronymus.service")
        );
        if stdio {
            assert!(output.stdout.is_empty());
        }
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
            let request = read_mock_request(&mut stream);
            assert!(request.starts_with("GET /status "));
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
            let request = read_mock_request(&mut stream);
            assert!(request.contains(token.expose_secret().as_str()));
            let (status, body) = if probe {
                assert!(request.starts_with("GET /status "));
                (200, serde_json::json!({"instance_id":"matching-instance", "protocol_revision":hiero::daemon::registry::PROTOCOL_REVISION}).to_string())
            } else {
                assert!(request.starts_with("POST /mcp "));
                let (_, payload) = request.split_once("\r\n\r\n").unwrap();
                let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
                assert_eq!(payload["id"], 7);
                assert_eq!(payload["method"], "tools/list");
                assert_eq!(
                    payload["params"]["_meta"]["fixture_padding"]
                        .as_str()
                        .unwrap()
                        .len(),
                    32 * 1024
                );
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
    // Exceed the old mock's one-read buffer so its request-body handling is
    // exercised regardless of how TCP happens to packetize this request.
    let request = serde_json::json!({"jsonrpc":"2.0", "id":7, "method":"tools/list",
        "params":{"_meta":{"fixture_padding":"x".repeat(32 * 1024)}}});
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "{request}").unwrap();
    input.flush().unwrap();
    // Keep the host session alive until this transaction completes. EOF has a
    // separate bounded cancellation contract and must not race this mock.
    let mut response = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut response)
        .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(body["id"], 7);
    assert_eq!(body["error"]["code"], -32603);
    assert_eq!(
        body["error"]["message"],
        "daemon returned an invalid or mismatched response"
    );
    assert!(
        !body.to_string().contains("unauthorized"),
        "route body must not leak: {body}"
    );
    responder.join().unwrap();
}

#[test]
fn tool_call_waits_past_the_control_request_deadline() {
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
            started_at: "2026-09-10T00:00:00Z".into(),
        },
    )
    .unwrap();
    let responder = std::thread::spawn(move || {
        let (mut probe, _) = listener.accept().unwrap();
        assert!(read_mock_request(&mut probe).starts_with("GET /status "));
        let body = serde_json::json!({
            "instance_id": "matching-instance",
            "protocol_revision": hiero::daemon::registry::PROTOCOL_REVISION,
        })
        .to_string();
        write!(
            probe,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();

        let (mut tool, _) = listener.accept().unwrap();
        let request = read_mock_request(&mut tool);
        assert!(request.starts_with("POST /mcp "));
        let payload: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["method"], "tools/call");
        assert_eq!(payload["params"]["name"], "hieronymus_dream");
        std::thread::sleep(std::time::Duration::from_millis(10_500));
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"resultType": "complete"},
        })
        .to_string();
        let _ = write!(
            tool,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });

    let client = hiero::lifecycle::connect(&config, false).unwrap();
    let reply = client.call_tool("hieronymus_dream", &serde_json::json!({}));
    responder.join().unwrap();
    assert_eq!(reply.unwrap()["result"]["resultType"], "complete");
}

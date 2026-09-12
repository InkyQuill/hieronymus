//! Runtime shutdown, credential stability, and authenticated lifecycle
//! contract tests (ADR 0009 §Decision and §Lifecycle And Discovery, ADR 0012
//! as amended 2026-09-03; astra findings 9 and 11).
//!
//! Three properties are under test here:
//!
//! - the installation bearer token is *stable*: a plain restart reuses it and
//!   never rotates it;
//! - a graceful stop drains every supervised worker before it removes its
//!   discovery record and releases data-root ownership, on every path
//!   (RPC, SIGINT, SIGTERM, `Drop`);
//! - discovery health is decided by an authenticated probe plus a
//!   process-instance comparison — never by a PID or a bare TCP connect.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hiero::daemon::discovery::{self, DiscoveryRecord};
use hiero::daemon::registry::PROTOCOL_REVISION;
use hiero::daemon::workers::WorkerGroup;
use hiero::daemon::{Daemon, DaemonOptions};
use hiero::lifecycle::{self, DiscoveryHealth};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::ownership::RootOwnership;
use hieronymus::secret::Secret;

fn options(root: &Path) -> DaemonOptions {
    DaemonOptions {
        data_root: Some(root.into()),
        port: 0,
        ..Default::default()
    }
}

fn hiero(arguments: &[&str]) -> (String, String, std::process::ExitStatus) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiero"));
    command.args(arguments);
    #[cfg(windows)]
    if matches!(
        arguments.first(),
        Some(&("start" | "stop" | "restart" | "service"))
    ) {
        command.arg("--no-activate");
    }
    let output = command.output().expect("the hiero binary must run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status,
    )
}

/// A listener that accepts connections and never speaks HTTP: the "someone
/// else inherited the port" case ADR 0009 names.
fn foreign_listener() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            std::thread::sleep(Duration::from_millis(300));
            drop(stream);
        }
    });
    port
}

fn seed_record(config: &HieronymusConfig, port: u16, instance_id: &str, protocol: &str) {
    discovery::write_discovery(
        config,
        &DiscoveryRecord {
            discovery_version: discovery::DISCOVERY_VERSION,
            protocol_version: protocol.to_string(),
            host: "127.0.0.1".to_string(),
            port,
            // Deliberately this live process: a PID check would call every
            // record below "alive".
            pid: std::process::id(),
            instance_id: instance_id.to_string(),
            started_at: "2026-09-04T00:00:00+00:00".to_string(),
        },
    )
    .unwrap();
}

// ---------------------------------------------------------------- credentials

#[test]
fn restart_reuses_installation_token() {
    let root = tempfile::tempdir().unwrap();
    let options = options(root.path());
    let first = Daemon::start(&options).unwrap();
    let token = first.bearer().expose_secret().clone();
    first.shutdown().unwrap();
    let second = Daemon::start(&options).unwrap();
    assert_eq!(second.bearer().expose_secret(), &token);
    second.shutdown().unwrap();

    // And the file on disk is the same one, still user-only.
    let on_disk = std::fs::read_to_string(root.path().join("daemon.token")).unwrap();
    assert_eq!(on_disk.trim(), token);
    assert_private_token(&root.path().join("daemon.token"));
}

#[test]
fn a_freshly_minted_token_is_user_only() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    assert_private_token(&root.path().join("daemon.token"));
    daemon.shutdown().unwrap();
}

fn assert_private_token(path: &Path) {
    assert!(hieronymus::private_file::read_private(path).is_ok());
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn an_empty_token_file_is_refused_with_repair_guidance() {
    let root = tempfile::tempdir().unwrap();
    let token_path = root.path().join("daemon.token");
    hieronymus::private_file::create_private_new(&token_path, b"\n").unwrap();

    let error = Daemon::start(&options(root.path())).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("is empty"), "{text}");
    assert!(
        text.contains(&token_path.display().to_string()),
        "the error must name the token path: {text}"
    );
    assert!(
        text.contains("delete the file"),
        "the error must carry repair guidance: {text}"
    );
    // Fail closed: nothing was published and the root was not taken over.
    assert!(!root.path().join("daemon.json").exists());
    let config = HieronymusConfig::new(root.path());
    assert!(RootOwnership::acquire(&config, "probe").is_ok());
}

#[test]
#[cfg(unix)]
fn a_world_readable_token_file_is_refused_with_repair_guidance() {
    let root = tempfile::tempdir().unwrap();
    let token_path = root.path().join("daemon.token");
    std::fs::write(&token_path, "ab".repeat(32)).unwrap();
    std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let error = Daemon::start(&options(root.path())).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("readable beyond its owner"), "{text}");
    assert!(text.contains("chmod 600"), "{text}");
    // Sentinel secret: the refusal never echoes the credential.
    assert!(!text.contains(&"ab".repeat(32)), "{text}");
}

// ------------------------------------------------------------------- shutdown

#[test]
fn two_successive_stops_are_a_clean_no_op() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(&options(root.path())).unwrap();
    let config = HieronymusConfig::new(root.path());

    daemon.stop().unwrap();
    assert!(!root.path().join("daemon.json").exists());
    assert!(
        RootOwnership::acquire(&config, "probe").is_ok(),
        "the first stop released ownership"
    );

    // The second stop (and the `Drop` guard right after it) must not fail,
    // double-remove, or touch a record a later daemon published.
    daemon.stop().unwrap();
    daemon.stop().unwrap();
    drop(daemon);
}

#[test]
fn a_stalled_socket_does_not_wedge_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(&options(root.path())).unwrap();
    let port = daemon.local_addr().port();

    // Connect and send nothing: the worker is blocked reading request
    // headers. Shutdown must wake that read rather than wait out the socket's
    // 5s read timeout.
    let stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
    // Give the accept loop a moment to admit the connection.
    std::thread::sleep(Duration::from_millis(150));

    let started = Instant::now();
    daemon.stop().unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(4),
        "shutdown must wake the stalled read, not wait out the 5s timeout (took {elapsed:?})"
    );
    assert!(!root.path().join("daemon.json").exists());
    drop(stalled);
}

#[test]
fn shutdown_waits_for_a_worker_that_is_mid_request() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(&options(root.path())).unwrap();
    let port = daemon.local_addr().port();

    // A complete request head with a Content-Length whose body never arrives:
    // the head is parsed (so the socket is no longer parked) and the worker
    // then blocks in the body read. Shutdown must join it, bounded by the
    // socket's own 5s read timeout.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(
            format!(
                "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                 Content-Type: application/json\r\nContent-Length: 4096\r\n\r\n{{"
            )
            .as_bytes(),
        )
        .unwrap();
    stream.flush().unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let started = Instant::now();
    daemon.stop().unwrap();
    let elapsed = started.elapsed();
    // Joined, not abandoned: the wait is real, and it is bounded.
    assert!(
        elapsed < Duration::from_secs(15),
        "shutdown must stay bounded (took {elapsed:?})"
    );
    // Ownership is released only after the worker joined.
    let config = HieronymusConfig::new(root.path());
    assert!(RootOwnership::acquire(&config, "probe").is_ok());
    assert!(!root.path().join("daemon.json").exists());
    drop(stream);
}

#[test]
fn a_request_woken_during_shutdown_is_never_dispatched() {
    // The read→release window: `read_request` returns a complete head, but
    // `wake_all` drains the ticket and shuts the socket down before `release`
    // takes the lock. Dispatching there would run the handler (committing any
    // mutation) while only the response write failed silently, so a client
    // retrying against the next daemon instance would double-apply it.
    //
    // The window is a few instructions wide, so this hammers it: many
    // connections send a complete request head and shutdown races them. The
    // invariant asserted is the one that matters and holds either way — a
    // request is answered in full or not applied at all, never applied with a
    // dropped response. The deterministic half of this fix is covered by
    // `workers::tests::release_reports_whether_the_connection_won_the_race_against_wake_all`.
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(&options(root.path())).unwrap();
    let port = daemon.local_addr().port();

    let mut clients = Vec::new();
    for _ in 0..64 {
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
            break;
        };
        // A complete head, so the worker reaches the release/dispatch gate.
        let _ = stream.write_all(
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
        );
        let _ = stream.flush();
        clients.push(stream);
    }

    // Race the in-flight heads.
    daemon.stop().unwrap();

    for mut stream in clients {
        let mut body = Vec::new();
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.read_to_end(&mut body);
        let text = String::from_utf8_lossy(&body);
        // Either a complete response, or nothing at all. A truncated
        // response would mean a handler ran and its write was cut off.
        if text.is_empty() {
            continue;
        }
        assert!(
            text.starts_with("HTTP/1.1 ") && text.contains("\r\n\r\n"),
            "a dispatched request must produce a complete response, got: {text:?}"
        );
    }

    // Shutdown still completed in full.
    assert!(!root.path().join("daemon.json").exists());
    let config = HieronymusConfig::new(root.path());
    assert!(RootOwnership::acquire(&config, "probe").is_ok());
}

#[test]
fn the_worker_group_joins_before_the_daemon_releases_ownership() {
    // The R2 carry-over: a detached handler could still hold the database
    // when another owner took the root. Nothing may run after `stop()`.
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(&options(root.path())).unwrap();
    let port = daemon.local_addr().port();
    for _ in 0..8 {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = stream.write_all(
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
        );
        let mut sink = Vec::new();
        let _ = stream.read_to_end(&mut sink);
    }
    daemon.stop().unwrap();

    let config = HieronymusConfig::new(root.path());
    let guard = RootOwnership::acquire(&config, "maintenance")
        .expect("ownership must be free the moment stop() returned");
    // Exclusive: the daemon's own connection is gone too.
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute_batch("begin immediate; rollback")
        .unwrap();
    drop(guard);
}

#[test]
#[cfg(unix)]
fn a_sigterm_shuts_the_daemon_binary_down_gracefully() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // The ready line is printed only after the signal handler is installed.
    let mut ready = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut ready,
    )
    .expect("the daemon must print its ready line");
    assert!(ready.contains("listening"), "{ready}");
    let discovery_path = root.path().join("daemon.json");
    assert!(discovery_path.exists());

    // SIGTERM, not SIGINT: `ctrlc`'s `termination` feature routes it to the
    // same graceful path, so a service manager stopping the unit drains
    // exactly like ctrl-c.
    let signaled = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill must be available");
    assert!(signaled.success());

    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "SIGTERM must exit through the graceful path"
    );
    assert!(
        !discovery_path.exists(),
        "SIGTERM removes the matching discovery record"
    );
    assert!(
        RootOwnership::acquire(&config, "probe").is_ok(),
        "SIGTERM releases data-root ownership"
    );
    // The credential survives: stopping is not rotating.
    assert!(root.path().join("daemon.token").exists());
}

#[test]
fn a_refused_worker_is_never_lost_by_a_concurrent_join() {
    // Admission and the handle list share one lock, so a handle can never be
    // pushed after `stop_and_join` drained the list.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let stop = Arc::new(AtomicBool::new(false));
    let group = Arc::new(WorkerGroup::new(Arc::clone(&stop)));
    let ran = Arc::new(AtomicUsize::new(0));
    let admitted = Arc::new(AtomicUsize::new(0));

    let spawner = {
        let group = Arc::clone(&group);
        let ran = Arc::clone(&ran);
        let admitted = Arc::clone(&admitted);
        std::thread::spawn(move || {
            for _ in 0..200 {
                let ran = Arc::clone(&ran);
                let accepted = group.spawn(Box::new(move |_| {
                    std::thread::sleep(Duration::from_millis(1));
                    ran.fetch_add(1, Ordering::SeqCst);
                }));
                if accepted.is_ok() {
                    admitted.fetch_add(1, Ordering::SeqCst);
                } else {
                    break;
                }
            }
        })
    };
    std::thread::sleep(Duration::from_millis(20));
    group.stop_and_join().unwrap();
    spawner.join().unwrap();
    // Anything admitted before the stop edge ran to completion.
    assert_eq!(
        ran.load(Ordering::SeqCst),
        admitted.load(Ordering::SeqCst),
        "every admitted worker must have been joined"
    );
}

// ------------------------------------------------------------ discovery health

#[test]
fn a_live_daemon_passes_the_authenticated_probe() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let config = HieronymusConfig::new(root.path());
    let health = lifecycle::probe(&config);
    assert!(health.is_live(), "{health:?}");
    assert!(!health.is_stale_record());
    let DiscoveryHealth::Live { status, .. } = &health else {
        panic!("expected a live verdict: {health:?}");
    };
    assert_eq!(
        status["instance_id"].as_str().unwrap(),
        daemon.discovery_record().instance_id
    );
    assert_eq!(
        status["protocol_revision"].as_str().unwrap(),
        PROTOCOL_REVISION
    );
    // Sentinel secret: the authenticated status never carries the credential.
    let rendered = status.to_string();
    assert!(
        !rendered.contains(daemon.bearer().expose_secret().as_str()),
        "the status payload must never contain the bearer token"
    );
    daemon.shutdown().unwrap();
}

#[test]
fn a_foreign_listener_on_the_recorded_port_never_passes_as_the_daemon() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    seed_record(
        &config,
        foreign_listener(),
        &"ab".repeat(16),
        PROTOCOL_REVISION,
    );

    let health = lifecycle::probe(&config);
    assert!(!health.is_live(), "{health:?}");
    assert!(
        health.is_stale_record(),
        "an authenticated probe proved the record stale: {health:?}"
    );

    // Doctor reports it and repairs nothing.
    let report = hiero::doctor::run(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code.starts_with("daemon-") || finding.code == "discovery-stale"),
        "{:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "daemon-reachable"),
        "a foreign listener must never be reported as the reachable daemon: {:?}",
        report.findings
    );
    assert!(
        config.daemon_discovery_path().exists(),
        "doctor never deletes state"
    );

    // The agent hook agrees, and also repairs nothing.
    let output = hiero::agent_hook::session_end(&config);
    assert!(
        output.json.contains("\"available\": false"),
        "{}",
        output.json
    );
    assert!(config.daemon_discovery_path().exists());
}

#[test]
fn an_instance_mismatch_is_detected_by_the_authenticated_comparison() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let config = HieronymusConfig::new(root.path());
    // Same live endpoint and credential, but the record claims another
    // process instance: only the authenticated comparison can see this.
    seed_record(
        &config,
        daemon.local_addr().port(),
        &"ff".repeat(16),
        PROTOCOL_REVISION,
    );
    let health = lifecycle::probe(&config);
    assert!(
        matches!(health, DiscoveryHealth::InstanceMismatch { .. }),
        "{health:?}"
    );
    assert!(health.is_stale_record());
    daemon.shutdown().unwrap();
}

#[test]
fn a_protocol_mismatch_is_detected_by_the_authenticated_comparison() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_record(
        &config,
        daemon.local_addr().port(),
        &daemon.discovery_record().instance_id,
        "1999-01-01",
    );
    let health = lifecycle::probe(&config);
    assert!(
        matches!(health, DiscoveryHealth::ProtocolMismatch { .. }),
        "{health:?}"
    );
    daemon.shutdown().unwrap();
}

#[test]
fn an_authenticated_but_malformed_status_is_not_a_repairable_record() {
    // Something that answers 200 to our credential is our daemon having a bad
    // moment, not a stale record. It must never become repairable: an
    // autostarting `connect` would otherwise delete a live daemon's discovery
    // record over one malformed response.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut scratch = [0_u8; 2048];
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.read(&mut scratch);
            // 200, JSON content type, but a body with no identity at all.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\n\
                  Content-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
            let _ = stream.flush();
        }
    });

    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    discovery::write_token(&config, &discovery::generate_bearer_token().unwrap()).unwrap();
    seed_record(&config, port, &"ab".repeat(16), PROTOCOL_REVISION);

    let health = lifecycle::probe(&config);
    assert!(
        matches!(health, DiscoveryHealth::Unparseable { .. }),
        "{health:?}"
    );
    assert!(!health.is_live());
    assert!(
        !health.is_stale_record(),
        "a malformed answer must never authorize deleting the record"
    );

    // Doctor reports it as unverifiable, not stale, and repairs nothing.
    let report = hiero::doctor::run(&config);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "daemon-unverifiable"),
        "{:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "discovery-stale"),
        "{:?}",
        report.findings
    );
    assert!(config.daemon_discovery_path().exists());
}

#[test]
fn a_wrong_credential_is_not_a_live_daemon() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let config = HieronymusConfig::new(root.path());
    // Overwrite the stored credential: whatever is listening no longer
    // authenticates us, so it is not this installation's daemon.
    discovery::write_token(&config, &Secret::new("cd".repeat(32))).unwrap();
    let health = lifecycle::probe(&config);
    assert!(
        matches!(health, DiscoveryHealth::Unauthenticated { status: 401, .. }),
        "{health:?}"
    );
    assert!(health.is_stale_record());
    // Restore so the daemon's own shutdown path is unaffected.
    discovery::write_token(&config, daemon.bearer()).unwrap();
    daemon.shutdown().unwrap();
}

#[test]
fn discovery_consumers_read_the_published_port_and_never_assume_9768() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let port = daemon.local_addr().port();
    assert_ne!(port, hiero::daemon::DEFAULT_PORT);
    let config = HieronymusConfig::new(root.path());

    // The record carries the actual bound port ...
    assert_eq!(discovery::read_discovery(&config).unwrap().port, port);
    // ... the shared probe reaches it there ...
    assert!(lifecycle::probe(&config).is_live());
    // ... and the generated agent-hook integration payload advertises it.
    let output = hiero::agent_hook::session_end(&config);
    assert!(
        output.json.contains(&format!("http://127.0.0.1:{port}")),
        "{}",
        output.json
    );
    assert!(
        !output.json.contains("9768"),
        "integrations must not hard-code the default port: {}",
        output.json
    );
    daemon.shutdown().unwrap();
}

// ------------------------------------------------------------- lifecycle CLI

#[test]
fn hiero_status_reports_a_live_daemon_and_never_prints_the_token() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let token = daemon.bearer().expose_secret().clone();
    let root_argument = root.path().to_str().unwrap();

    let (stdout, stderr, status) = hiero(&["status", "--data-root", root_argument]);
    assert!(status.success(), "{stderr}");
    assert!(stdout.contains("daemon: running"), "{stdout}");
    assert!(stdout.contains(PROTOCOL_REVISION), "{stdout}");
    assert!(!stdout.contains(&token) && !stderr.contains(&token));

    let (stdout, stderr, status) = hiero(&["status", "--json", "--data-root", root_argument]);
    assert!(status.success(), "{stderr}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["running"], serde_json::json!(true));
    assert_eq!(
        payload["status"]["instance_id"].as_str().unwrap(),
        daemon.discovery_record().instance_id
    );
    assert_eq!(
        payload["status"]["protocol_revision"].as_str().unwrap(),
        PROTOCOL_REVISION
    );
    assert!(!stdout.contains(&token), "the token must never be printed");
    daemon.shutdown().unwrap();
}

#[test]
fn hiero_status_reports_a_stopped_daemon_without_failing() {
    let root = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = hiero(&["status", "--data-root", root.path().to_str().unwrap()]);
    assert!(status.success(), "{stderr}");
    assert!(stdout.contains("daemon: not running"), "{stdout}");
    assert!(stdout.contains("no discovery record"), "{stdout}");
}

#[test]
fn hiero_stop_requests_authenticated_shutdown_through_the_discovered_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut ready,
    )
    .unwrap();
    let token = std::fs::read_to_string(root.path().join("daemon.token")).unwrap();

    // A foreground daemon is reached exactly like a managed one: through the
    // authenticated discovered endpoint.
    let unit_dir = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = hiero(&[
        "stop",
        "--data-root",
        root.path().to_str().unwrap(),
        "--unit-dir",
        unit_dir.path().to_str().unwrap(),
    ]);
    assert!(status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("shutdown requested"), "{stdout}");
    assert!(stdout.contains("daemon stopped"), "{stdout}");
    assert!(!stdout.contains(token.trim()), "{stdout}");

    assert_eq!(child.wait().unwrap().code(), Some(0));
    assert!(!root.path().join("daemon.json").exists());
    assert!(RootOwnership::acquire(&config, "probe").is_ok());
    // Stopping is not rotating.
    assert_eq!(
        std::fs::read_to_string(root.path().join("daemon.token")).unwrap(),
        token
    );
}

#[test]
fn hiero_stop_confirms_absence_and_refuses_an_unverifiable_unit() {
    let root = tempfile::tempdir().unwrap();
    let unit_dir = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = hiero(&[
        "stop",
        "--data-root",
        root.path().to_str().unwrap(),
        "--unit-dir",
        unit_dir.path().to_str().unwrap(),
    ]);
    // No panic, no stack trace, and a message that says what happened.
    assert_ne!(status.code(), Some(101), "stderr: {stderr}");
    assert!(status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("daemon stopped: no discovery record"),
        "{stdout}"
    );
    assert!(RootOwnership::acquire(&HieronymusConfig::new(root.path()), "stopped").is_ok());

    // A broken registration cannot authorize any fallback manager action.
    #[cfg(any(target_os = "linux", windows))]
    {
        std::fs::write(
            unit_dir.path().join(hiero::service::SERVICE_UNIT_NAME),
            "[Unit]\nDescription=Hieronymus\n",
        )
        .unwrap();
        let (stdout, stderr, status) = hiero(&[
            "stop",
            "--data-root",
            root.path().to_str().unwrap(),
            "--unit-dir",
            unit_dir.path().to_str().unwrap(),
        ]);
        assert_ne!(status.code(), Some(101), "stderr: {stderr}");
        #[cfg(target_os = "linux")]
        let reason = "unit file has no ExecStart";
        #[cfg(windows)]
        let reason = "Invalid native registration state";
        assert!(
            !status.success() && stderr.contains(reason),
            "stdout: {stdout}\nstderr: {stderr}"
        );
    }
}

#[test]
fn the_lifecycle_commands_route_and_the_service_subcommands_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let unit_dir = tempfile::tempdir().unwrap();
    let root_argument = root.path().to_str().unwrap();
    let unit_argument = unit_dir.path().to_str().unwrap();

    // Every lifecycle verb is a known command (never "unknown command").
    for command in ["start", "stop", "restart", "status"] {
        let (stdout, stderr, _) = hiero(&[
            command,
            "--data-root",
            root_argument,
            "--unit-dir",
            unit_argument,
        ]);
        assert!(
            !stderr.contains("unknown command"),
            "{command}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    #[cfg(any(target_os = "linux", windows))]
    {
        // `start` installs the unit through the service integration. With a
        // custom `--unit-dir` the manager is deliberately never contacted, so the
        // start step refuses — and the refusal still reports the install step
        // that did happen.
        let fresh_unit_dir = tempfile::tempdir().unwrap();
        let (stdout, stderr, _) = hiero(&[
            "start",
            "--data-root",
            root_argument,
            "--unit-dir",
            fresh_unit_dir.path().to_str().unwrap(),
        ]);
        let reported = format!("{stdout}{stderr}");
        #[cfg(target_os = "linux")]
        let installed = "service unit written";
        #[cfg(windows)]
        let installed = "Windows daemon task installed";
        assert!(reported.contains(installed), "{reported}");
        assert!(
            fresh_unit_dir
                .path()
                .join(hiero::service::SERVICE_UNIT_NAME)
                .exists()
        );
        assert!(
            unit_dir
                .path()
                .join(hiero::service::SERVICE_UNIT_NAME)
                .exists()
        );

        // The lower-level `hiero service <...>` surface is untouched.
        let (stdout, stderr, status) = hiero(&[
            "service",
            "status",
            "--json",
            "--data-root",
            root_argument,
            "--unit-dir",
            unit_argument,
        ]);
        assert!(
            status.code() == Some(0) || status.code() == Some(1),
            "{stderr}"
        );
        let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(payload["installed"], serde_json::json!(true));

        let (stdout, stderr, status) = hiero(&[
            "service",
            "uninstall",
            "--data-root",
            root_argument,
            "--unit-dir",
            unit_argument,
        ]);
        assert!(status.success(), "{stderr}");
        #[cfg(target_os = "linux")]
        let removed = "service unit removed";
        #[cfg(windows)]
        let removed = "Windows daemon task removed";
        assert!(stdout.contains(removed), "{stdout}");
    }

    // The usage line advertises both surfaces.
    let (_, stderr, _) = hiero(&["nonsense-command"]);
    assert!(stderr.contains("start|stop|restart|status"), "{stderr}");
    assert!(stderr.contains("service"), "{stderr}");
}

#[test]
fn lifecycle_errors_never_leak_the_installation_token() {
    let root = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(&options(root.path())).unwrap();
    let token = daemon.bearer().expose_secret().clone();
    let config = HieronymusConfig::new(root.path());

    // Every error surface the lifecycle module can produce.
    let mut texts = vec![
        lifecycle::probe(&config).detail(),
        format!("{:?}", lifecycle::connect(&config, false)),
    ];
    seed_record(
        &config,
        foreign_listener(),
        &"ab".repeat(16),
        PROTOCOL_REVISION,
    );
    texts.push(lifecycle::probe(&config).detail());
    texts.push(format!("{:?}", lifecycle::connect(&config, false)));
    texts.push(format!("{:?}", lifecycle::status(&config).to_json()));
    let report = hiero::doctor::run(&config);
    texts.push(format!("{:?}", report.to_json()));
    texts.push(report.render_human());

    for text in texts {
        assert!(!text.contains(&token), "a credential leaked into: {text}");
    }
    daemon.request_shutdown();
    drop(daemon);
}

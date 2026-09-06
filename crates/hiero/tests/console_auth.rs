//! Console authentication contract (plan W1):
//!
//! - `hiero admin` / `hiero config` mint a one-time launch grant through the
//!   local daemon and hand it to the browser opener in a URL *fragment*,
//!   never on stdout/stderr and never in a query string;
//! - the daemon accepts an authenticated safe read (`GET`) with **no**
//!   `Origin` (a legitimate top-level navigation) but still rejects an
//!   explicit foreign `Origin`, a missing session, and any mutating request
//!   without the exact `Origin`;
//! - grants are single-use and sessions live only as long as the daemon
//!   process.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::{
    browser_session, same_origin, send_request, start_daemon, start_daemon_on_ephemeral_port,
    start_daemon_with_browser_session,
};
use serde_json::json;

// --------------------------------------------------------------- Origin reads

#[test]
fn authenticated_get_without_origin_succeeds_with_the_session() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &[(
            "Cookie".to_string(),
            format!("hieronymus_session={}", fixture.session),
        )],
        b"",
    );
    assert_eq!(
        response.status, 200,
        "a top-level navigation GET carries no Origin and must still be served: {:?}",
        response.raw_body
    );
    assert!(response.body().get("providers").is_some());
}

#[test]
fn a_read_without_a_session_is_unauthorized_even_without_an_origin() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(fixture.port, "GET", "/api/providers", &[], b"");
    assert_eq!(response.status, 401);
    assert_eq!(response.body(), json!({"error": "unauthorized"}));
}

#[test]
fn a_read_with_an_explicit_foreign_origin_is_forbidden() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={}", fixture.session),
            ),
            ("Origin".to_string(), "https://attacker.invalid".to_string()),
        ],
        b"",
    );
    assert_eq!(response.status, 403);
    assert_eq!(response.body(), json!({"error": "forbidden_origin"}));
}

#[test]
fn a_read_with_the_exact_origin_succeeds() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={}", fixture.session),
            ),
            ("Origin".to_string(), same_origin(fixture.port)),
        ],
        b"",
    );
    assert_eq!(response.status, 200);
}

// ----------------------------------------------------------- Origin mutations

#[test]
fn a_mutating_request_without_an_origin_is_forbidden() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={}", fixture.session),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        br#"{"provider": {"id": "x", "name": "X", "type": "openai", "url": "https://example.test"}}"#,
    );
    assert_eq!(
        response.status, 403,
        "a state change must require the exact Origin: {:?}",
        response.raw_body
    );
    assert_eq!(response.body(), json!({"error": "forbidden_origin"}));
}

#[test]
fn a_mutating_request_with_a_foreign_origin_is_forbidden() {
    let (fixture, _root, _daemon) = start_daemon_with_browser_session();
    let response = send_request(
        fixture.port,
        "POST",
        "/api/providers",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={}", fixture.session),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), "https://attacker.invalid".to_string()),
        ],
        br#"{"provider": {"id": "x", "name": "X", "type": "openai", "url": "https://example.test"}}"#,
    );
    assert_eq!(response.status, 403);
}

// -------------------------------------------------------------- grant / session

#[test]
fn a_launch_grant_exchanges_once_then_reports_reuse() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let mint = send_request(
        port,
        "POST",
        "/auth/launch-grant",
        &[(
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        )],
        b"",
    );
    assert_eq!(mint.status, 200);
    let grant = mint.body()["launch_grant"].as_str().unwrap().to_string();
    let body = format!(r#"{{"launch_grant": "{grant}"}}"#).into_bytes();

    let first = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &body,
    );
    assert_eq!(first.status, 200);

    let replay = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        &body,
    );
    assert_eq!(replay.status, 401);
    assert_eq!(replay.body(), json!({"error": "launch_grant_already_used"}));

    let unknown = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Origin".to_string(), same_origin(port)),
        ],
        br#"{"launch_grant": "never-minted-grant"}"#,
    );
    assert_eq!(unknown.status, 401);
    assert_eq!(unknown.body(), json!({"error": "launch_grant_invalid"}));
}

#[test]
fn a_grant_exchange_still_requires_the_exact_origin() {
    let (_root, daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let mint = send_request(
        port,
        "POST",
        "/auth/launch-grant",
        &[(
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        )],
        b"",
    );
    let grant = mint.body()["launch_grant"].as_str().unwrap().to_string();
    let missing_origin = send_request(
        port,
        "POST",
        "/auth/launch-grant/exchange",
        &[("Content-Type".to_string(), "application/json".to_string())],
        format!(r#"{{"launch_grant": "{grant}"}}"#).as_bytes(),
    );
    assert_eq!(
        missing_origin.status, 403,
        "the grant exchange is a state change and keeps the strict Origin rule"
    );
}

#[test]
fn sessions_do_not_survive_a_daemon_restart() {
    let (fixture, root, daemon) = start_daemon_with_browser_session();
    // The session works while this daemon runs.
    let ok = send_request(
        fixture.port,
        "GET",
        "/api/providers",
        &[(
            "Cookie".to_string(),
            format!("hieronymus_session={}", fixture.session),
        )],
        b"",
    );
    assert_eq!(ok.status, 200);
    daemon.shutdown().unwrap();

    // A fresh daemon on the same data root has an empty in-memory session
    // store, so the old cookie is no longer a live session.
    let restarted = start_daemon(root.path());
    let stale = send_request(
        restarted.local_addr().port(),
        "GET",
        "/api/providers",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={}", fixture.session),
            ),
            (
                "Origin".to_string(),
                same_origin(restarted.local_addr().port()),
            ),
        ],
        b"",
    );
    assert_eq!(stale.status, 401);
    assert_eq!(stale.body(), json!({"error": "unauthorized"}));

    // And a brand-new grant/session flow works against the restarted daemon.
    let (_grant, session) = browser_session(&restarted);
    let fresh = send_request(
        restarted.local_addr().port(),
        "GET",
        "/api/providers",
        &[(
            "Cookie".to_string(),
            format!("hieronymus_session={session}"),
        )],
        b"",
    );
    assert_eq!(fresh.status, 200);
    restarted.shutdown().unwrap();
}

// ------------------------------------------------------------------- CLI launch

/// Write an executable opener stub that records its single URL argument to
/// `out_path` and exits 0.
fn write_capture_opener(dir: &std::path::Path, out_path: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("capture-opener.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", out_path.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn run_console_cli(
    data_root: &std::path::Path,
    page: &str,
    opener: &std::ffi::OsStr,
) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([page, "--data-root", data_root.to_str().unwrap()])
        .env("HIERO_CONSOLE_BROWSER", opener)
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status.code(),
    )
}

#[test]
fn cli_opens_the_selected_page_with_the_grant_only_in_the_fragment() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    let port = daemon.local_addr().port();
    let bearer = daemon.bearer().expose_secret().clone();

    for page in ["admin", "config"] {
        let out = root.path().join(format!("opened-{page}.txt"));
        let opener = write_capture_opener(root.path(), &out);
        let (stdout, stderr, code) = run_console_cli(root.path(), page, opener.as_os_str());
        assert_eq!(code, Some(0), "stdout={stdout} stderr={stderr}");
        assert!(
            stdout.contains(&format!("opening the {page} console")),
            "stdout={stdout}"
        );

        let url = std::fs::read_to_string(&out).unwrap();
        let (base, fragment) = url
            .split_once('#')
            .expect("the opener URL must have a fragment");
        assert_eq!(base, format!("http://127.0.0.1:{port}/{page}"));
        let grant = fragment
            .strip_prefix("launch_grant=")
            .expect("the fragment carries the launch grant");
        assert_eq!(grant.len(), 64, "grant is 32 random bytes hex: {grant:?}");

        // The grant and the bearer never reach the command's own output.
        for stream in [&stdout, &stderr] {
            assert!(
                !stream.contains(grant),
                "grant leaked into CLI output: {stream}"
            );
            assert!(
                !stream.contains(&bearer),
                "bearer leaked into CLI output: {stream}"
            );
            assert!(
                !stream.contains("launch_grant"),
                "the grant parameter name leaked into CLI output: {stream}"
            );
            assert!(
                !stream.contains('#'),
                "the URL fragment leaked into CLI output: {stream}"
            );
        }
    }

    daemon.shutdown().unwrap();
}

#[test]
fn cli_reports_opener_failure_without_exposing_the_grant() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_daemon(root.path());
    let port = daemon.local_addr().port();
    let bearer = daemon.bearer().expose_secret().clone();

    let (stdout, stderr, code) = run_console_cli(
        root.path(),
        "admin",
        std::ffi::OsStr::new("/nonexistent/hiero-xdg-open"),
    );

    assert_eq!(code, Some(2), "an opener failure is a CLI error");
    assert!(
        stderr.contains(&format!("http://127.0.0.1:{port}")),
        "the failure message names the bare origin: {stderr}"
    );
    assert!(
        !stderr.contains("launch_grant") && !stderr.contains('#'),
        "the failure message must not carry the grant or the fragment: {stderr}"
    );
    assert!(!stdout.contains(&bearer) && !stderr.contains(&bearer));
}

#[test]
fn cli_rejects_unknown_flags_like_its_siblings() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "admin",
            "--json",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--json"), "{stderr}");
}

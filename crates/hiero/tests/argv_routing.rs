//! argv[0] command routing (distribution spec, installer section): one binary
//! serves the historical entry points. `hieronymus` routes to the canonical
//! CLI, `hieronymus-mcp` routes to `hiero mcp`, and `hieronymus-agent-hook`
//! routes to the `agent-hook` subcommand (`session-start`/`session-end`).
//! Routes are exercised through the real `argv[0]` (CommandExt::arg0).

use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

/// Runs the binary with `argv[0]` set to `name` and the given arguments,
/// feeding `stdin_text` and returning (stdout, stderr, status).
fn run_as(
    name: &str,
    arguments: &[&str],
    stdin_text: &str,
) -> (String, String, std::process::ExitStatus) {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hiero"));
        command.arg0(name);
        command
    };
    #[cfg(windows)]
    let fixture = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let mut command = {
        // Windows aliases are real selection-record launchers, not argv[0]
        // overrides or renamed copies of the console executable.
        let layout = hiero::app::AppLayout::new(fixture.path());
        let payload = layout.version_dir(env!("CARGO_PKG_VERSION"));
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_hiero"), payload.join("hiero.exe")).unwrap();
        std::fs::copy(
            env!("CARGO_BIN_EXE_hiero-launcher"),
            payload.join("hiero-launcher.exe"),
        )
        .unwrap();
        layout
            .switch_stable_links(env!("CARGO_PKG_VERSION"))
            .unwrap();
        Command::new(layout.stable_link(name))
    };
    let mut child = command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_text.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status,
    )
}

#[test]
fn hieronymus_arg0_behaves_as_the_canonical_cli() {
    let (stdout, stderr, status) = run_as("hieronymus", &["version"], "");
    assert!(status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("hiero v"), "{stdout}");
    assert!(stdout.contains('\u{03b1}'), "{stdout}");
}

#[test]
fn hieronymus_mcp_arg0_routes_to_the_stdio_adapter() {
    // With no discovery record the routed adapter reports the daemon error —
    // the unrouted CLI would have said "missing command" instead.
    let root = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = run_as(
        "hieronymus-mcp",
        &["--data-root", root.path().to_str().unwrap()],
        "",
    );
    assert_eq!(status.code(), Some(2), "{stdout}{stderr}");
    assert!(
        stderr.contains("no running local service discovered"),
        "{stderr}"
    );
    assert!(!stdout.contains("missing command"), "{stdout}");
}

#[test]
fn canonical_hiero_mcp_matches_the_routed_behavior() {
    let root = tempfile::tempdir().unwrap();
    let (_, stderr, status) = run_as(
        "hiero",
        &["mcp", "--data-root", root.path().to_str().unwrap()],
        "",
    );
    assert_eq!(status.code(), Some(2));
    assert!(
        stderr.contains("no running local service discovered"),
        "{stderr}"
    );
}

#[test]
fn hieronymus_agent_hook_arg0_routes_to_session_end() {
    let (stdout, stderr, status) = run_as("hieronymus-agent-hook", &["session-end"], "");
    assert!(status.success(), "{stdout}{stderr}");
    assert_eq!(stdout, "Hieronymus session hook complete\n");
}

#[test]
fn hieronymus_agent_hook_session_start_reports_missing_context() {
    let cwd = tempfile::tempdir().unwrap();
    let (stdout, stderr, status) = run_as(
        "hieronymus-agent-hook",
        &["session-start", "--cwd", cwd.path().to_str().unwrap()],
        "",
    );
    assert!(status.success(), "{stdout}{stderr}");
    assert_eq!(stdout, "no .hieronymus.json context found\n");
}

#[test]
fn hieronymus_agent_hook_session_start_json_matches_the_frozen_payload() {
    let cwd = tempfile::tempdir().unwrap();
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-start",
            "--cwd",
            cwd.path().to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["event"], "session-start");
    assert_eq!(payload["handled"], false);
    assert_eq!(payload["reason"], "no .hieronymus.json context found");
    assert_eq!(payload["service"]["available"], false);
    assert_eq!(payload["service"]["mode"], "direct-local");
    assert_eq!(
        payload["service"]["reason"],
        "no running local service discovered"
    );
}

#[test]
fn hieronymus_agent_hook_session_start_reports_discovered_context() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join(".hieronymus.json"),
        r#"{"series_slug": "demo", "source_language": "ja", "target_language": "en"}"#,
    )
    .unwrap();
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &["session-start", "--cwd", workspace.path().to_str().unwrap()],
        "",
    );
    assert!(status.success(), "{stdout}");
    assert_eq!(stdout, "Hieronymus context loaded\n");

    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-start",
            "--cwd",
            workspace.path().to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["handled"], true);
    assert_eq!(payload["series_slug"], "demo");
    assert_eq!(payload["source_language"], "ja");
    assert_eq!(payload["target_language"], "en");
    assert_eq!(payload["task_type"], "translation");
    assert_eq!(payload["volume"], "");
    assert_eq!(payload["chapter"], "");
}

#[test]
fn hieronymus_agent_hook_session_end_json_includes_the_service() {
    let (stdout, _, status) = run_as("hieronymus-agent-hook", &["session-end", "--json"], "");
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["event"], "session-end");
    assert_eq!(payload["handled"], true);
    assert_eq!(payload["service"]["available"], false);
}

#[test]
fn agent_hook_without_a_known_subcommand_is_a_usage_error() {
    let (_, stderr, status) = run_as("hieronymus-agent-hook", &[], "");
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("agent-hook"), "{stderr}");
    assert!(stderr.contains("session-start"), "{stderr}");
}

#[test]
fn canonical_hiero_agent_hook_works_too() {
    let (stdout, _, status) = run_as("hiero", &["agent-hook", "session-end"], "");
    assert!(status.success());
    assert_eq!(stdout, "Hieronymus session hook complete\n");
}

#[test]
fn agent_hook_rejects_a_nonexistent_cwd() {
    let (_, stderr, status) = run_as(
        "hieronymus-agent-hook",
        &["session-start", "--cwd", "/nonexistent/hieronymus/cwd"],
        "",
    );
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("--cwd"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Frozen fixture byte parity (Python `json.dumps(ensure_ascii=False,
// sort_keys=True)`) and content parity vs `service_discovery.py`
// ---------------------------------------------------------------------------

#[test]
fn session_start_json_byte_matches_the_frozen_fixture() {
    let cwd = tempfile::tempdir().unwrap();
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-start",
            "--cwd",
            cwd.path().to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    assert_eq!(
        stdout,
        "{\"event\": \"session-start\", \"handled\": false, \
         \"reason\": \"no .hieronymus.json context found\", \"service\": \
         {\"available\": false, \"mode\": \"direct-local\", \
         \"reason\": \"no running local service discovered\"}}\n"
    );
}

#[test]
fn session_end_json_byte_matches_the_frozen_fixture() {
    let (stdout, _, status) = run_as("hieronymus-agent-hook", &["session-end", "--json"], "");
    assert!(status.success(), "{stdout}");
    assert_eq!(
        stdout,
        "{\"event\": \"session-end\", \"handled\": true, \"service\": \
         {\"available\": false, \"mode\": \"direct-local\", \
         \"reason\": \"no running local service discovered\"}}\n"
    );
}

#[test]
fn handled_context_json_renders_python_style_escapes_and_key_order() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join(".hieronymus.json"),
        "{\"series_slug\": \"d\\u00e9mo \\\"x\\\"\"}",
    )
    .unwrap();
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-start",
            "--cwd",
            workspace.path().to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    // sort_keys order (chapter < event < handled < series_slug < service <
    // source_language < target_language < task_type < volume), ", "/": "
    // separators, quote escapes, and raw non-ASCII (ensure_ascii=False).
    assert_eq!(
        stdout,
        "{\"chapter\": \"\", \"event\": \"session-start\", \"handled\": true, \
         \"series_slug\": \"démo \\\"x\\\"\", \"service\": \
         {\"available\": false, \"mode\": \"direct-local\", \
         \"reason\": \"no running local service discovered\"}, \
         \"source_language\": \"ja\", \"target_language\": \"en\", \
         \"task_type\": \"translation\", \"volume\": \"\"}\n"
    );
}

fn seed_discovery(config: &hieronymus::data_root::HieronymusConfig, port: u16, pid: u32) {
    use hiero::daemon::discovery::{DISCOVERY_VERSION, DiscoveryRecord};
    use hiero::daemon::registry::PROTOCOL_REVISION;
    let record = DiscoveryRecord {
        discovery_version: DISCOVERY_VERSION,
        protocol_version: PROTOCOL_REVISION.to_string(),
        host: "127.0.0.1".to_string(),
        port,
        pid,
        instance_id: "ab".repeat(16),
        started_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    hiero::daemon::discovery::write_discovery(config, &record).unwrap();
}

#[test]
fn no_discovery_record_reports_no_running_service() {
    // The frozen payload for a root no daemon ever ran in.
    let root = tempfile::tempdir().unwrap();
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-end",
            "--json",
            "--data-root",
            root.path().to_str().unwrap(),
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(
        payload["service"]["reason"],
        "no running local service discovered"
    );
}

#[test]
fn a_record_with_a_live_pid_but_no_daemon_is_still_unavailable() {
    // ADR 0009: liveness is never decided by PID existence. This record
    // carries a pid that IS alive (our own) and a port nothing serves, so the
    // old pid check would have called it live. The authenticated probe does
    // not.
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    seed_discovery(&config, 1, std::process::id());
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-end",
            "--json",
            "--data-root",
            root.path().to_str().unwrap(),
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["service"]["available"], serde_json::json!(false));
    assert_eq!(payload["service"]["mode"], "direct-local");
}

#[test]
fn unreachable_service_failure_reason_carries_the_probe_error() {
    // A live pid with a refused port is Python's `{exc}` branch: the reason
    // must name the failure, not just the generic prefix.
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    seed_discovery(&config, 1, std::process::id());
    let (stdout, _, status) = run_as(
        "hieronymus-agent-hook",
        &[
            "session-end",
            "--json",
            "--data-root",
            root.path().to_str().unwrap(),
        ],
        "",
    );
    assert!(status.success(), "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let reason = payload["service"]["reason"].as_str().unwrap();
    let prefix = "local service state exists but health check failed: ";
    assert!(reason.starts_with(prefix), "{reason}");
    assert!(reason.len() > prefix.len(), "{reason}");
}

#[test]
fn tray_missing_installed_helper_has_actionable_diagnostic() {
    // Same-filesystem hard link avoids a writable executable descriptor leaking
    // into a concurrently forked test child (ETXTBSY on immediate exec).
    let fixture = tempfile::tempdir_in(
        std::path::Path::new(env!("CARGO_BIN_EXE_hiero"))
            .parent()
            .unwrap(),
    )
    .unwrap();
    let cli = fixture
        .path()
        .join(format!("hiero{}", std::env::consts::EXE_SUFFIX));
    std::fs::hard_link(env!("CARGO_BIN_EXE_hiero"), &cli).unwrap();
    let output = Command::new(cli)
        .args(["tray", "--data-root"])
        .arg(fixture.path().join("data"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("hiero-desktop") && error.contains("desktop package"),
        "{error}"
    );
}

#[test]
#[cfg(unix)]
fn tray_forwards_absolute_root_as_one_literal_argument_to_sibling() {
    use std::os::unix::fs::PermissionsExt;
    // Same-filesystem hard link avoids a writable executable descriptor leaking
    // into a concurrently forked test child (ETXTBSY on immediate exec).
    let fixture = tempfile::tempdir_in(
        std::path::Path::new(env!("CARGO_BIN_EXE_hiero"))
            .parent()
            .unwrap(),
    )
    .unwrap();
    let cli = fixture.path().join("hiero");
    std::fs::hard_link(env!("CARGO_BIN_EXE_hiero"), &cli).unwrap();
    let helper = fixture.path().join("hiero-desktop");
    std::fs::write(
        &helper,
        "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" > \"$2.args\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = "literal $(touch BAD) ; data";
    let output = Command::new(cli)
        .current_dir(fixture.path())
        .args(["tray", "--data-root", root])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = fixture.path().join(root);
    assert_eq!(
        std::fs::read_to_string(fixture.path().join(format!("{root}.args"))).unwrap(),
        format!("2\n--data-root\n{}\n", expected.display())
    );
    assert!(!fixture.path().join("BAD").exists());
}

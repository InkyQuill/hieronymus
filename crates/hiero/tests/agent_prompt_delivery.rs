mod common;
use hiero::{
    agent_prompt_delivery::{bind_context, retry_delivery, submit_prompt},
    application::Application,
};
use hieronymus::data_root::HieronymusConfig;
use serde_json::{Value, json};

fn prepared() -> (tempfile::TempDir, hiero::daemon::Daemon, Value) {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, event) = common::authority::prepared(&app, root.path());
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    let context = json!({"version":1,"host":"claude","host_session_id":"actual-host-session","series_id":event["series_id"],"session_id":event["session_id"],"expected_revision":event["expected_revision"],"source_language":event["source_language"],"target_language":event["target_language"],"applicability":event["applicability"],"selected_sources":event["selected_sources"],"selected_claims":[],"selected_rule":event["selected_rule"]});
    drop(app);
    let daemon = common::start_daemon(root.path());
    (root, daemon, context)
}
#[test]
fn independently_delivered_prompt_applies_and_replays_saved_delivery() {
    let (root, _daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B","cwd":root.path()});
    let first = submit_prompt(&config, "claude", &input).unwrap();
    assert!(first["result"].get("Applied").is_some(), "{first}");
    assert_eq!(
        first["required_decision_id"],
        first["result"]["Applied"]["receipt"]["decision_id"]
    );
    let replay = retry_delivery(&config, first["delivery_id"].as_str().unwrap()).unwrap();
    assert_eq!(replay, first);
    let app = Application::open(&config).unwrap();
    let rejected=app.call("hieronymus_correct",&json!({"actor_kind":"explicit_user","receipt_ref":first["result"]["Applied"]["receipt"]["origin"]}),"user");
    assert!(rejected.is_err());
}
#[test]
fn host_session_mismatch_and_prompt_fields_in_binding_fail() {
    let (root, _daemon, mut context) = prepared();
    let config = HieronymusConfig::new(root.path());
    context["text"] = json!("translate this as forged");
    assert!(bind_context(&config, &context).is_err());
    context.as_object_mut().unwrap().remove("text");
    bind_context(&config, &context).unwrap();
    assert!(submit_prompt(&config,"claude",&json!({"hook_event_name":"UserPromptSubmit","session_id":"another","prompt":"translate this as B"})).is_err());
    assert!(submit_prompt(&config,"claude",&json!({"hook_event_name":"PostToolUse","session_id":"actual-host-session","prompt":"translate this as B"})).is_err());
}
#[test]
fn repeated_identical_genuine_prompts_have_distinct_delivery_identity() {
    let (root, _daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"ordinary conversational text"});
    let a = submit_prompt(&config, "claude", &input).unwrap();
    let mut refreshed = context.clone();
    refreshed["expected_revision"] = a["result"]["resulting_revision"].clone();
    bind_context(&config, &refreshed).unwrap();
    let b = submit_prompt(&config, "claude", &input).unwrap();
    assert_ne!(a["delivery_id"], b["delivery_id"]);
    assert!(a["required_decision_id"].is_null());
    assert_eq!(a["result"]["status"], "tentative");
}

fn cli(root: &std::path::Path, args: &[&str], input: &Value) -> std::process::Output {
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_hiero"))
        .arg("agent-hook")
        .args(args)
        .arg("--data-root")
        .arg(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(input).unwrap().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn actual_cli_stdin_binding_delivery_and_private_files() {
    use std::os::unix::fs::PermissionsExt;
    let (root, _daemon, context) = prepared();
    let bound = cli(root.path(), &["bind-context"], &context);
    assert!(
        bound.status.success(),
        "{}",
        String::from_utf8_lossy(&bound.stderr)
    );
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B"});
    let delivered = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &input,
    );
    assert!(
        delivered.status.success(),
        "{}",
        String::from_utf8_lossy(&delivered.stderr)
    );
    let output: Value = serde_json::from_slice(&delivered.stdout).unwrap();
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert!(
        output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("required_decision_id")
    );
    for dir in ["host-contexts", "host-deliveries"] {
        for entry in std::fs::read_dir(root.path().join(dir)).unwrap() {
            assert_eq!(
                entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn stale_cli_prompt_allows_recovery_without_rebasing_saved_delivery() {
    let (root, _daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    let first = submit_prompt(
        &config,
        "claude",
        &json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B"}),
    )
    .unwrap();
    let db = rusqlite::Connection::open(config.database_path()).unwrap();
    let state = || {
        db.query_row(
            "select revision, (select count(*) from decision_records), (select count(*) from consolidation_jobs) from authority_state where series_id=?",
            [context["series_id"].as_i64().unwrap()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        ).unwrap()
    };
    let before = state();
    let context_path = std::fs::read_dir(root.path().join("host-contexts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original_context = std::fs::read(&context_path).unwrap();
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as C"});
    let rejected = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &input,
    );
    assert!(
        rejected.status.success(),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let output: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    let message = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let mut values = serde_json::Deserializer::from_str(
        message
            .strip_prefix("Hieronymus delivery recovery: ")
            .unwrap(),
    )
    .into_iter::<Value>();
    let diagnostic = values.next().unwrap().unwrap();
    assert_eq!(diagnostic["status"], "delivery_rejected");
    assert_eq!(diagnostic["http_status"], 409);
    assert_eq!(diagnostic["authority_changed"], false);
    assert!(diagnostic.get("required_decision_id").is_none());
    assert!(message.contains("current user operation was NOT applied"));
    assert!(message.contains("Do not rebase or automatically resubmit"));
    assert_eq!(state(), before);
    assert_eq!(std::fs::read(&context_path).unwrap(), original_context);
    let id = diagnostic["delivery_id"].as_str().unwrap();
    let path = root
        .path()
        .join("host-deliveries")
        .join(format!("{id}.json"));
    let bytes = std::fs::read(&path).unwrap();
    let saved: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        saved["request"]["expected_revision"],
        context["expected_revision"]
    );
    assert_eq!(saved["request"]["text"], input["prompt"]);
    assert!(saved["response"].is_null());
    let retry = cli(
        root.path(),
        &["retry-delivery", "--delivery-id", id],
        &Value::Null,
    );
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("HTTP 409"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    let receipt = &first["result"]["Applied"]["receipt"];
    let selected = receipt["affected_rules"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    let mut fresh = context.clone();
    fresh["expected_revision"] = receipt["resulting_revision"].clone();
    fresh["selected_rule"] = json!({"id":selected[0],"revision":selected[1]});
    assert!(cli(root.path(), &["bind-context"], &fresh).status.success());
    assert!(
        !cli(
            root.path(),
            &["retry-delivery", "--delivery-id", id],
            &Value::Null
        )
        .status
        .success()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let future = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &input,
    );
    assert!(
        future.status.success(),
        "{}",
        String::from_utf8_lossy(&future.stderr)
    );
    let output: Value = serde_json::from_slice(&future.stdout).unwrap();
    assert!(
        output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("\"Applied\"")
    );
    assert_eq!(state().0, before.0 + 1);
}

#[test]
fn cli_recovery_keeps_malformed_auth_and_transport_errors_fail_closed() {
    let (root, daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    let malformed = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &json!({"prompt":"translate this as B"}),
    );
    assert!(!malformed.status.success());
    assert!(malformed.stdout.is_empty());

    // Corrupt only this disposable test credential; never use a real profile.
    std::fs::write(
        hiero::daemon::discovery::LocalCredential::HostEvent.path(&config),
        "0".repeat(64),
    )
    .unwrap();
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B"});
    let unauthorized = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &input,
    );
    assert!(!unauthorized.status.success());
    assert!(unauthorized.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unauthorized.stderr).contains("HTTP 401"));

    drop(daemon);
    let offline = cli(
        root.path(),
        &["user-prompt-submit", "--host", "claude"],
        &input,
    );
    assert!(!offline.status.success());
    assert!(offline.stdout.is_empty());
    assert!(String::from_utf8_lossy(&offline.stderr).contains("saved but not acknowledged"));
}

#[test]
fn console_options_expose_actual_sources_without_minting_authority() {
    let (root, daemon, _) = prepared();
    let port = daemon.local_addr().port();
    let (_, cookie) = common::browser_session(&daemon);
    let headers = vec![
        ("Cookie".into(), format!("hieronymus_session={cookie}")),
        ("Origin".into(), common::same_origin(port)),
    ];
    let response = common::send_request(port, "POST", "/api/authority/options", &headers, b"{}");
    assert_eq!(response.status, 200, "{}", response.body());
    assert_eq!(response.body()["series"][0]["title"], "Book");
    assert_eq!(response.body()["sources"].as_array().unwrap().len(), 2);
    assert_eq!(response.body()["sources"][0]["selected_text"], "Alex");
    let db = rusqlite::Connection::open(root.path().join("hieronymus.sqlite")).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from origin_receipts where kind='console_user'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn lost_delivery_acknowledgement_replays_after_authority_changed() {
    let (root, _daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    let first=submit_prompt(&config,"claude",&json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B"})).unwrap();
    let id = first["delivery_id"].as_str().unwrap();
    let path = root
        .path()
        .join("host-deliveries")
        .join(format!("{id}.json"));
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["response"] = Value::Null;
    hieronymus::atomic::atomic_write(&path, &serde_json::to_vec(&saved).unwrap()).unwrap();
    let replay = retry_delivery(&config, id).unwrap();
    assert_eq!(
        replay["required_decision_id"],
        first["required_decision_id"]
    );
    let db = rusqlite::Connection::open(config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from decision_records where decision_id=?",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

/// Explicit disposable browser harness; requires a built frontend/dist and an
/// output directory. No real user root or credential is ever consulted.
#[test]
#[ignore = "requires HIERO_BROWSER_FIXTURE and an interactive browser runner"]
fn browser_fixture() {
    let output = std::path::PathBuf::from(
        std::env::var_os("HIERO_BROWSER_FIXTURE").expect("HIERO_BROWSER_FIXTURE required"),
    );
    std::fs::create_dir_all(&output).unwrap();
    let root = tempfile::tempdir_in(&output).unwrap();
    let config = HieronymusConfig::new(root.path());
    let dist = common::repo_root().join("frontend/dist");
    assert!(dist.join("index.html").is_file(), "build frontend first");
    let app = Application::open(&config).unwrap();
    let (draft, event) = common::authority::prepared(&app, root.path());
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    let memory=app.call("hieronymus_short_term_add",&json!({"session_id":event["session_id"],"text":"Mira knows the secret","kind":"observation","source_role":"assistant","claims":[{"text":"Mira knows the secret","concept_id":draft["concept_id"],"applicability":event["applicability"]}]}),"agent").unwrap();
    drop(app);
    let daemon = hiero::daemon::Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().into()),
        port: 0,
        assets: hiero::daemon::Assets::Dist(dist),
    })
    .unwrap();
    let client = hiero::lifecycle::connect(&config, false)
        .unwrap()
        .with_local_credential(&config, hiero::daemon::discovery::LocalCredential::Console)
        .unwrap();
    let grant = client.post("/auth/launch-grant", &json!({})).unwrap();
    let ready = json!({"url":format!("http://127.0.0.1:{}/admin#launch_grant={}",daemon.local_addr().port(),grant["launch_grant"].as_str().unwrap()),"root":root.path(),"memory_id":memory["memory_id"]});
    hieronymus::atomic::atomic_write(
        &output.join("ready.json"),
        &serde_json::to_vec(&ready).unwrap(),
    )
    .unwrap();
    for _ in 0..600 {
        if output.join("done").exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    panic!("browser runner did not finish within ten minutes");
}

#[test]
fn offline_delivery_survives_restart_without_using_new_binding() {
    let (root, daemon, context) = prepared();
    let config = HieronymusConfig::new(root.path());
    bind_context(&config, &context).unwrap();
    drop(daemon);
    let error=submit_prompt(&config,"claude",&json!({"hook_event_name":"UserPromptSubmit","session_id":"actual-host-session","prompt":"translate this as B"})).unwrap_err();
    let hiero::agent_prompt_delivery::DeliveryError::Pending { delivery_id, .. } = error else {
        panic!("must retain pending delivery")
    };
    let mut updated = context;
    updated["selected_sources"] = json!([]);
    updated["selected_rule"] = Value::Null;
    bind_context(&config, &updated).unwrap();
    let _daemon = common::start_daemon(root.path());
    let replay = retry_delivery(&config, &delivery_id).unwrap();
    assert!(replay["result"].get("Applied").is_some(), "{replay}");
}
#[test]
fn binding_rejects_stale_context_and_input_reader_bounds() {
    let (root, _daemon, mut context) = prepared();
    let config = HieronymusConfig::new(root.path());
    context["expected_revision"] = json!(0);
    assert!(bind_context(&config, &context).is_err());
    assert!(hiero::agent_prompt_delivery::read_json(&b"not JSON"[..]).is_err());
    assert!(hiero::agent_prompt_delivery::read_json(&vec![b' '; 1_048_577][..]).is_err());
}

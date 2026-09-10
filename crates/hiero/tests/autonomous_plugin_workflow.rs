//! A deterministic transcript driver exercises installed entrypoints and real
//! application state. It is never evidence of native host/model acceptance.
mod common;
use serde_json::{Value, json};
use std::io::Write;

fn prompt(root: &std::path::Path, input: &Value) -> std::process::Output {
    hook_cli(root, &["user-prompt-submit", "--host", "claude"], input)
}
fn hook_cli(root: &std::path::Path, args: &[&str], input: &Value) -> std::process::Output {
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
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn first_host_prompt_exposes_actual_identity_without_minting_authority() {
    let root = tempfile::tempdir().unwrap();
    let input = json!({"hook_event_name":"UserPromptSubmit","session_id":"independently-observed-session","prompt":"Please translate this chapter."});
    let output = prompt(root.path(), &input);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("independently-observed-session"));
    assert!(context.contains("binding_required"));
    assert!(context.contains("bind-context"));
    assert!(!root.path().join("host-contexts").exists());
    assert!(!root.path().join("host-deliveries").exists());
    assert!(!root.path().join("hieronymus.sqlite").exists());
}

#[test]
fn generated_optional_prompt_hook_executes_installed_handler() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let files = hiero::agent_plugins::render(&config).unwrap();
    let contents = &files
        .iter()
        .find(|(p, _)| p.ends_with("codex/hooks/hooks.codex.json"))
        .unwrap()
        .1;
    let hooks: Value = serde_json::from_str(contents).unwrap();
    let command = hooks["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
        .as_str()
        .expect("supported command hook schema");
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_hiero"),
        bin.join("hieronymus-agent-hook"),
    )
    .unwrap();
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .env("PATH", &bin)
        .env("HIERONYMUS_DATA_ROOT", root.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"UserPromptSubmit","session_id":"native-shaped-session","prompt":"Read chapter one"}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("native-shaped-session")
    );
}

#[test]
fn ordinary_work_then_independent_correction_uses_receipt_before_validation() {
    use hiero::application::Application;
    use hieronymus::data_root::HieronymusConfig;
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let mut calls = Vec::new();
    let (draft, event) = common::authority::prepared_with(
        |name, args| {
            calls.push(name.to_string());
            app.call(name, &args, "agent").unwrap()
        },
        root.path(),
    );
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    let observed = "Alex speaks in clipped phrases.";
    let claim = json!({"text":observed,"concept_id":null,"applicability":event["applicability"]});
    app.call(
        "hieronymus_short_term_add",
        &json!({"session_id":event["session_id"],"kind":"note","text":observed,"claims":[claim]}),
        "agent",
    )
    .unwrap();
    let recall_args =
        json!({"series_slug":"book","session_id":event["session_id"],"query":"clipped phrases"});
    let recalled = app
        .call("hieronymus_recall", &recall_args, "agent")
        .unwrap();
    assert!(
        recalled["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["text"] == observed),
        "{recalled}"
    );
    // Context is assembled only from actual producer/session/recall outputs.
    let binding = json!({"version":1,"host":"claude","host_session_id":"observed-work-session","series_id":event["series_id"],"session_id":event["session_id"],"expected_revision":recalled["resulting_revision"],"source_language":event["source_language"],"target_language":event["target_language"],"applicability":event["applicability"],"selected_sources":event["selected_sources"],"selected_claims":[],"selected_rule":event["selected_rule"]});
    let _daemon = common::start_daemon(root.path());
    let bound = hook_cli(root.path(), &["bind-context"], &binding);
    assert!(
        bound.status.success(),
        "{}",
        String::from_utf8_lossy(&bound.stderr)
    );
    let delivered = prompt(
        root.path(),
        &json!({"hook_event_name":"UserPromptSubmit","session_id":"observed-work-session","prompt":"translate this as Б"}),
    );
    assert!(
        delivered.status.success(),
        "{}",
        String::from_utf8_lossy(&delivered.stderr)
    );
    let hook: Value = serde_json::from_slice(&delivered.stdout).unwrap();
    let context = hook["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let encoded = context
        .strip_prefix("Hieronymus trusted correction result: ")
        .unwrap()
        .split_once(". If required_decision_id")
        .unwrap()
        .0;
    let result: Value = serde_json::from_str(encoded).unwrap();
    let dependency = &result["required_decision_id"];
    assert!(dependency.is_string(), "{result}");
    for (rendering, valid) in [("Б", true), ("А", false)] {
        let args = json!({"series_slug":"book","raw_text":"Alex","translated_text":rendering,"volume":"I","chapter":"1","story_timeline_id":event["applicability"]["timeline_id"],"story_scene_key":"a","required_decision_id":dependency});
        let validated = app
            .call("hieronymus_termbase_validate", &args, "agent")
            .unwrap();
        assert_eq!(
            validated["results"].as_array().unwrap().is_empty(),
            valid,
            "{validated}"
        );
        calls.push("hieronymus_termbase_validate".into());
    }
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let jobs: i64 = db
        .query_row(
            "select count(*) from consolidation_jobs where decision_id=?",
            [dependency.as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    let state: String = db
        .query_row(
            "select state from consolidation_jobs where decision_id=?",
            [dependency.as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "pending",
        "no provider ran, so no completion can be claimed"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|n| n.as_str() == "hieronymus_evidence_capture")
            .count(),
        4
    );
    assert_eq!(
        calls
            .iter()
            .filter(|n| n.as_str() == "hieronymus_termbase_validate")
            .count(),
        2
    );
    let reopened = Application::open(&config).unwrap();
    let mut recall_args = recall_args;
    recall_args["required_decision_id"] = dependency.clone();
    assert!(
        reopened
            .call("hieronymus_recall", &recall_args, "agent")
            .is_ok()
    );
}

#[test]
fn local_marketplace_points_to_generated_codex_bundle_and_manifest_hook() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    hiero::agent_plugins::generate(&config).unwrap();
    let index = config
        .agent_plugins_root()
        .join(".agents/plugins/marketplace.json");
    let index: Value =
        serde_json::from_slice(&std::fs::read(index).expect("local supported marketplace"))
            .unwrap();
    let source = index["plugins"][0]["source"]["path"].as_str().unwrap();
    let bundle = config.agent_plugins_root().join(source);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(bundle.join(".codex-plugin/plugin.json")).unwrap())
            .unwrap();
    assert!(bundle.join(manifest["hooks"].as_str().unwrap()).is_file());
    assert!(
        bundle
            .join(manifest["mcpServers"].as_str().unwrap())
            .is_file()
    );
}

#[test]
fn claude_marketplace_points_to_generated_bundle() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    hiero::agent_plugins::generate(&config).unwrap();
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(
            config
                .agent_plugins_root()
                .join(".claude-plugin/marketplace.json"),
        )
        .expect("Claude local marketplace"),
    )
    .unwrap();
    let plugin = &catalog["plugins"][0];
    assert_eq!(plugin["source"], "./claude");
    let bundle = config.agent_plugins_root().join("claude");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(bundle.join(".claude-plugin/plugin.json")).unwrap())
            .unwrap();
    assert_eq!(plugin["name"], manifest["name"]);
    assert_eq!(plugin["version"], manifest["version"]);
    assert_eq!(plugin["description"], manifest["description"]);
}

#[test]
fn ambiguous_prompt_records_one_tentative_job_without_claiming_applied_dependency() {
    use hiero::{agent_prompt_delivery, application::Application};
    use hieronymus::data_root::HieronymusConfig;
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (draft, event) = common::authority::prepared(&app, root.path());
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    let binding = json!({"version":1,"host":"claude","host_session_id":"ambiguous-host-session","series_id":event["series_id"],"session_id":event["session_id"],"expected_revision":event["expected_revision"],"source_language":event["source_language"],"target_language":event["target_language"],"applicability":event["applicability"],"selected_sources":[],"selected_claims":[],"selected_rule":null});
    let _daemon = common::start_daemon(root.path());
    let bound = hook_cli(root.path(), &["bind-context"], &binding);
    assert!(
        bound.status.success(),
        "{}",
        String::from_utf8_lossy(&bound.stderr)
    );
    let result=agent_prompt_delivery::submit_prompt(&config,"claude",&json!({"hook_event_name":"UserPromptSubmit","session_id":"ambiguous-host-session","prompt":"Maybe Alex means the other person?"})).unwrap();
    assert_eq!(result["result"]["status"], "tentative");
    assert!(result["required_decision_id"].is_null());
    assert_eq!(
        agent_prompt_delivery::retry_delivery(&config, result["delivery_id"].as_str().unwrap())
            .unwrap(),
        result
    );
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from consolidation_jobs where decision_id=?",
            [result["result"]["decision_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let forms: Vec<String> = db
        .prepare("select canonical_translation from term_rules where status='active'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(forms, vec!["А"]);
    assert!(app.call("hieronymus_recall",&json!({"series_slug":"book","session_id":event["session_id"],"query":"Alex","required_decision_id":result["result"]["decision_id"]}),"agent").is_err());
}

#[path = "common/multilingual.rs"]
mod multilingual;
#[test]
fn multilingual_baseline_setup_uses_public_typed_context_and_learned_authority() {
    let root = tempfile::tempdir().unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(root.path());
    let app = hiero::application::Application::open(&config).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/hybrid-relevance.json")).unwrap();
    assert_eq!(fixture["documents"].as_array().unwrap().len(), 35);
    assert_eq!(fixture["queries"].as_array().unwrap().len(), 11);
    let contexts = multilingual::seed(
        |n, a| app.call(n, &a, "agent").unwrap(),
        &fixture,
        &root.path().join("sources"),
        false,
    );
    let c = &contexts["lighthouse-chronicles"];
    let contract = app
        .call(
            "hieronymus_termbase_contract",
            &multilingual::query_context(c, json!({"raw_text":"penicillin"})),
            "agent",
        )
        .unwrap();
    assert!(contract.to_string().contains("пенициллин"), "{contract}");
    let mut args = multilingual::query_context(c, json!({"query":"electrified"}));
    args["session_id"] = c["session_id"].clone();
    let recall = app.call("hieronymus_recall", &args, "agent").unwrap();
    assert!(
        recall["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["text"] == fixture["memories"][0]["text"]),
        "{recall}"
    );
}

#[test]
fn unhelpful_recall_uses_installed_feedback_without_invalidating_the_claim() {
    use hiero::application::Application;
    use hieronymus::{
        crystals::{CrystalStore, NewCrystal},
        data_root::HieronymusConfig,
        workspace::WorkspaceStore,
    };
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (draft, event) = common::authority::prepared(&app, root.path());
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    let context = WorkspaceStore::open(&config)
        .unwrap()
        .get_session(event["session_id"].as_i64().unwrap())
        .unwrap()
        .context;
    // Normal learned store ingestion; no authority/activation rows are seeded.
    let mut note = NewCrystal::new("lesson", "Alex speaks in clipped phrases.");
    note.claims = vec![
        serde_json::from_value(
            json!({"text":note.text,"concept_id":null,"applicability":event["applicability"]}),
        )
        .unwrap(),
    ];
    let store = CrystalStore::open(&config).unwrap();
    let crystal = store.add_crystal(&context, "lesson", &note).unwrap();
    let recall=app.call("hieronymus_recall",&json!({"session_id":event["session_id"],"series_slug":"book","query":"clipped phrases"}),"agent").unwrap();
    let hit = recall["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["activation_id"].is_i64())
        .expect("actual recalled activation");
    let _daemon = common::start_daemon(root.path());
    let run = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args([
                "recall-feedback",
                "--recall-id",
                recall["recall_id"].as_str().unwrap(),
                "--idempotency-key",
                "workflow-unhelpful",
                "--miss",
                &hit["activation_id"].to_string(),
                "--json",
                "--data-root",
            ])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let before = store.get(crystal).unwrap();
    assert_eq!(run()["applied"], true);
    let after = store.get(crystal).unwrap();
    assert_ne!(before.strength, after.strength);
    assert_eq!(run()["applied"], false);
    assert_eq!(after.strength, store.get(crystal).unwrap().strength);
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select count(*) from claim_effects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from term_rules where status='active'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

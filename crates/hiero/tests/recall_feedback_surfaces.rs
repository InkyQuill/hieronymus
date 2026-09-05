//! Recall-feedback surfaces: the one new REST route (`POST /recall/feedback`,
//! native bearer + Host) and the `hiero recall-feedback` CLI subcommand over
//! the same at-most-once `FeedbackStore` contract (ADR 0011).

use std::process::Command;

mod common;

use common::send_request;
use hiero::daemon::Daemon;
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::feedback::FeedbackStore;
use hieronymus::memory_models::TranslationContext;
use hieronymus::recall::RecallService;
use hieronymus::registry::Registry;
use hieronymus::workspace::WorkspaceStore;

/// A data root with one recalled crystal: returns the config, the recall id,
/// and the crystal's activation id.
fn recall_fixture(root: &std::path::Path) -> (HieronymusConfig, String, i64) {
    let config = HieronymusConfig::new(root.to_path_buf());
    Registry::open(&config)
        .unwrap()
        .create_series("demo", "demo", "ja", "en", None)
        .unwrap();
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let session = WorkspaceStore::open(&config)
        .unwrap()
        .start_session(&context)
        .unwrap();
    CrystalStore::open(&config)
        .unwrap()
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal::new("lesson", "Feedback surfaces cover chalk binding."),
        )
        .unwrap();
    let response = RecallService::open(&config)
        .unwrap()
        .recall(session.id, &context, "chalk binding", 10)
        .unwrap();
    let activation_id = response
        .hits
        .iter()
        .find_map(|hit| match hit {
            hieronymus::recall::RecallHit::LongTerm {
                crystal,
                activation_id,
                ..
            } if crystal.crystal_type == "lesson" => Some(*activation_id),
            _ => None,
        })
        .unwrap();
    (config, response.recall_id, activation_id)
}

fn bearer_header(daemon: &Daemon) -> (String, String) {
    (
        "Authorization".to_string(),
        format!("Bearer {}", daemon.bearer().expose_secret()),
    )
}

#[test]
fn rest_route_applies_replays_and_rejects_mismatches() {
    let root = tempfile::tempdir().unwrap();
    let (config, recall_id, activation_id) = recall_fixture(root.path());
    let daemon = Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap();
    let port = daemon.local_addr().port();
    let auth = vec![bearer_header(&daemon)];
    let body = format!(
        r#"{{"recall_id": "{recall_id}", "useful": [{activation_id}], "miss": [], "idempotency_key": "rest-k1"}}"#
    );

    // Native route: missing bearer fails before anything else.
    let unauthorized = send_request(port, "POST", "/recall/feedback", &[], body.as_bytes());
    assert_eq!(unauthorized.status, 401);

    let applied = send_request(port, "POST", "/recall/feedback", &auth, body.as_bytes());
    assert_eq!(applied.status, 200, "{:?}", applied.raw_body);
    assert_eq!(
        applied.body(),
        serde_json::json!({
            "recall_id": recall_id,
            "applied": true,
            "useful": 1,
            "miss": 0,
        })
    );

    // The same key replays to an explicit already-applied response.
    let replay = send_request(port, "POST", "/recall/feedback", &auth, body.as_bytes());
    assert_eq!(replay.status, 200, "{:?}", replay.raw_body);
    assert_eq!(replay.body()["applied"], serde_json::json!(false));

    // Activation ids that do not belong to the recall_id fail with 400.
    let mismatched = send_request(
        port,
        "POST",
        "/recall/feedback",
        &auth,
        format!(
            r#"{{"recall_id": "{recall_id}", "useful": [424242], "miss": [], "idempotency_key": "rest-k2"}}"#
        )
        .as_bytes(),
    );
    assert_eq!(mismatched.status, 400, "{:?}", mismatched.raw_body);
    assert_eq!(
        mismatched.body()["error"],
        serde_json::json!("activation_mismatch")
    );

    // Unknown recall ids report 404.
    let unknown = send_request(
        port,
        "POST",
        "/recall/feedback",
        &auth,
        br#"{"recall_id": "rc-unknown", "useful": [], "miss": [], "idempotency_key": "rest-k3"}"#,
    );
    assert_eq!(unknown.status, 404, "{:?}", unknown.raw_body);
    assert_eq!(
        unknown.body()["error"],
        serde_json::json!("unknown_recall_id")
    );

    // Wrong method reports the frozen not-found shape.
    let wrong_method = send_request(port, "GET", "/recall/feedback", &auth, b"");
    assert_eq!(wrong_method.status, 404);

    let _ = config;
}

#[test]
fn cli_applies_feedback_and_replays_noop() {
    let root = tempfile::tempdir().unwrap();
    let (_config, recall_id, activation_id) = recall_fixture(root.path());
    let data_root = root.path().to_str().unwrap();

    // Since plan M5 the CLI writes nothing itself: it posts to the daemon's
    // `POST /recall/feedback` route, so a daemon must be running over the
    // data root (the command reports the fact honestly otherwise).
    let daemon = Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap();

    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
            .args(["recall-feedback", "--data-root", data_root])
            .args(extra)
            .output()
            .unwrap();
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
            output.status,
        )
    };

    let useful = activation_id.to_string();
    let (stdout, stderr, status) = run(&[
        "--json",
        "--recall-id",
        &recall_id,
        "--useful",
        &useful,
        "--idempotency-key",
        "cli-k1",
    ]);
    assert!(status.success(), "{stdout}{stderr}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["applied"], serde_json::json!(true));
    assert_eq!(payload["useful"], serde_json::json!(1));

    // The same key replays as an explicit no-op with a zero exit code.
    let (stdout, stderr, status) = run(&[
        "--json",
        "--recall-id",
        &recall_id,
        "--useful",
        &useful,
        "--idempotency-key",
        "cli-k1",
    ]);
    assert!(status.success(), "{stdout}{stderr}");
    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(payload["applied"], serde_json::json!(false));

    // Mismatched activation ids fail with exit code 2 and a clear message.
    let (_stdout, stderr, status) = run(&[
        "--recall-id",
        &recall_id,
        "--useful",
        "424242",
        "--idempotency-key",
        "cli-k3",
    ]);
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("activation"), "{stderr}");

    // A missing idempotency key fails with the usage hint.
    let (_stdout, stderr, status) = run(&["--recall-id", &recall_id, "--useful", &useful]);
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("recall-feedback"), "{stderr}");

    daemon.shutdown().unwrap();
}

#[test]
fn cli_without_a_running_daemon_reports_how_to_start_one() {
    let root = tempfile::tempdir().unwrap();
    let (_config, recall_id, _activation_id) = recall_fixture(root.path());

    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "recall-feedback",
            "--data-root",
            root.path().to_str().unwrap(),
            "--json",
            "--recall-id",
            &recall_id,
            "--idempotency-key",
            "offline-k",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("no running local service discovered"),
        "{stderr}"
    );
    assert!(stderr.contains("hiero daemon"), "{stderr}");
}

#[test]
fn cli_and_rest_share_the_at_most_once_ledger() {
    let root = tempfile::tempdir().unwrap();
    let (config, recall_id, activation_id) = recall_fixture(root.path());
    // The CLI posts to the daemon (plan M5), so one must be running; the
    // independent read-back below then verifies the shared ledger.
    let daemon = Daemon::start(&hiero::daemon::DaemonOptions {
        data_root: Some(root.path().to_path_buf()),
        port: 0,
        assets: hiero::daemon::Assets::default(),
    })
    .unwrap();

    // The CLI applies first; the same key through the store (as the REST
    // route would) must stay a no-op.
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "recall-feedback",
            "--data-root",
            root.path().to_str().unwrap(),
            "--json",
            "--recall-id",
            &recall_id,
            "--useful",
            &activation_id.to_string(),
            "--idempotency-key",
            "shared-k",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let replay = FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&hieronymus::feedback::RecallFeedback {
            recall_id: recall_id.clone(),
            useful_activation_ids: vec![activation_id],
            missed_activation_ids: Vec::new(),
            idempotency_key: "shared-k".to_string(),
        })
        .unwrap();
    assert!(!replay.applied);

    daemon.shutdown().unwrap();
}

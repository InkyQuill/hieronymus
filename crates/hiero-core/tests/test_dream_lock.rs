use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use hiero_core::{
    config::HieronymusConfig,
    db,
    dreaming::{
        DreamAuditStore, DreamCycleState, DreamRunCompletion, PhaseRunStart,
        acquire_dream_cycle_lock, dream_cycle_paths,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn config(root: &TempDir) -> HieronymusConfig {
    HieronymusConfig::load(Some(root.path().to_owned())).expect("test configuration")
}

#[test]
fn dream_cycle_paths_live_under_the_data_root() {
    let root = TempDir::new().unwrap();
    let config = config(&root);

    let paths = dream_cycle_paths(&config);

    assert_eq!(paths.lock_file, root.path().join("dream-cycle.lock"));
    assert_eq!(paths.state_json, root.path().join("dream-cycle.json"));
}

#[test]
fn guard_owns_os_lock_and_cleans_only_its_state() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let guard = acquire_dream_cycle_lock(&config, "manual", false).unwrap();
    let paths = dream_cycle_paths(&config);
    let state: DreamCycleState =
        serde_json::from_slice(&fs::read(&paths.state_json).unwrap()).unwrap();
    assert_eq!(state, *guard.state());

    let replacement = DreamCycleState {
        owner: "replacement".to_owned(),
        pid: std::process::id(),
        started_at: chrono::Utc::now(),
        token: uuid::Uuid::new_v4(),
    };
    fs::write(&paths.state_json, serde_json::to_vec(&replacement).unwrap()).unwrap();
    drop(guard);

    let remaining: DreamCycleState =
        serde_json::from_slice(&fs::read(paths.state_json).unwrap()).unwrap();
    assert_eq!(remaining, replacement);
}

#[test]
fn non_waiting_contender_gets_typed_redacted_diagnostic() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let _guard = acquire_dream_cycle_lock(&config, "manual", false).unwrap();

    let error = acquire_dream_cycle_lock(&config, "autostart", false).unwrap_err();

    assert_eq!(
        error.state.as_ref().map(|state| state.owner.as_str()),
        Some("manual")
    );
    let display = error.to_string();
    assert_eq!(display, "dream cycle already running");
    assert!(!display.contains("manual"));
    assert!(!display.contains(&std::process::id().to_string()));
}

#[test]
fn stale_or_corrupt_metadata_is_replaced_only_after_os_lock_acquisition() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    fs::create_dir_all(root.path()).unwrap();
    fs::write(&paths.state_json, b"not json").unwrap();

    let guard = acquire_dream_cycle_lock(&config, "fresh", false).unwrap();
    let state: DreamCycleState =
        serde_json::from_slice(&fs::read(&paths.state_json).unwrap()).unwrap();

    assert_eq!(state.owner, "fresh");
    drop(guard);
    assert!(!paths.state_json.exists());
}

#[test]
fn invalid_live_metadata_is_not_reported_or_removed_by_a_contender() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let guard = acquire_dream_cycle_lock(&config, "manual", false).unwrap();
    let paths = dream_cycle_paths(&config);
    fs::write(
        &paths.state_json,
        br#"{"owner":"bad\nowner","pid":0,"started_at":"2026-01-01T00:00:00Z","token":"00000000-0000-0000-0000-000000000000"}"#,
    )
    .unwrap();

    let error = acquire_dream_cycle_lock(&config, "other", false).unwrap_err();

    assert!(error.is_already_running());
    assert!(error.state.is_none());
    assert!(paths.state_json.exists());
    drop(guard);
}

#[test]
fn guard_releases_lock_during_panic_unwind() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let _ = std::panic::catch_unwind(|| {
        let _guard = acquire_dream_cycle_lock(&config, "panic", false).unwrap();
        panic!("simulated cycle panic");
    });

    assert!(acquire_dream_cycle_lock(&config, "recovered", false).is_ok());
}

#[test]
fn second_process_cannot_acquire_until_guard_drops() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let guard = acquire_dream_cycle_lock(&config, "parent", false).unwrap();

    let blocked = run_child(root.path(), "try");
    assert_eq!(blocked.status.code(), Some(23), "{blocked:?}");

    drop(guard);
    let acquired = run_child(root.path(), "try");
    assert!(acquired.status.success(), "{acquired:?}");
}

#[test]
fn waiting_process_acquires_after_release() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let guard = acquire_dream_cycle_lock(&config, "parent", false).unwrap();
    let mut child = child_command(root.path(), "wait")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none());

    drop(guard);
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn symlink_lock_file_is_rejected_without_touching_target() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let config = config(&root);
    let target = root.path().join("target");
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, dream_cycle_paths(&config).lock_file).unwrap();

    assert!(acquire_dream_cycle_lock(&config, "manual", false).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[test]
fn hard_linked_or_public_lock_file_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let root = TempDir::new().unwrap();
    let config = config(&root);
    let paths = dream_cycle_paths(&config);
    fs::write(&paths.lock_file, b"").unwrap();
    fs::set_permissions(&paths.lock_file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(acquire_dream_cycle_lock(&config, "manual", false).is_err());

    fs::set_permissions(&paths.lock_file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&paths.lock_file, root.path().join("alias")).unwrap();
    assert!(acquire_dream_cycle_lock(&config, "manual", false).is_err());
}

#[cfg(unix)]
#[test]
fn data_root_reached_through_symlink_ancestor_is_rejected() {
    use std::os::unix::fs::symlink;

    let outer = TempDir::new().unwrap();
    let real = outer.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = outer.path().join("alias");
    symlink(&real, &alias).unwrap();
    let config = HieronymusConfig::load(Some(alias)).unwrap();

    assert!(acquire_dream_cycle_lock(&config, "manual", false).is_err());
}

#[cfg(unix)]
#[test]
fn nested_symlink_ancestor_is_rejected_without_touching_external_directory() {
    use std::os::unix::fs::symlink;

    let outer = TempDir::new().unwrap();
    let external = TempDir::new().unwrap();
    let external_child = external.path().join("child");
    fs::create_dir(&external_child).unwrap();
    let alias = outer.path().join("alias");
    symlink(external.path(), &alias).unwrap();
    let config = HieronymusConfig::load(Some(alias.join("child"))).unwrap();

    assert!(acquire_dream_cycle_lock(&config, "manual", false).is_err());
    assert!(fs::read_dir(external_child).unwrap().next().is_none());
}

#[test]
fn windows_lock_validation_uses_the_shared_file_identity_helper() {
    let lock_source = include_str!("../src/dreaming/lock.rs");
    let identity_source = include_str!("../src/file_identity.rs");

    assert!(lock_source.contains("file_identity::{"));
    assert!(identity_source.contains("volume_serial_number"));
    assert!(identity_source.contains("file_index"));
    assert!(identity_source.contains("number_of_links"));
}

fn child_command(root: &std::path::Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--ignored")
        .arg("--exact")
        .arg("dream_lock_child_helper")
        .env("HIERO_DREAM_LOCK_CHILD_ROOT", root)
        .env("HIERO_DREAM_LOCK_CHILD_MODE", mode);
    command
}

fn run_child(root: &std::path::Path, mode: &str) -> std::process::Output {
    child_command(root, mode).output().unwrap()
}

#[test]
#[ignore]
fn dream_lock_child_helper() {
    let Some(root) = std::env::var_os("HIERO_DREAM_LOCK_CHILD_ROOT") else {
        return;
    };
    let config = HieronymusConfig::load(Some(root.into())).unwrap();
    let wait = std::env::var("HIERO_DREAM_LOCK_CHILD_MODE").unwrap() == "wait";
    match acquire_dream_cycle_lock(&config, "child", wait) {
        Ok(_guard) => {}
        Err(error) if error.is_already_running() => std::process::exit(23),
        Err(error) => panic!("unexpected acquisition error: {error}"),
    }
}

#[test]
#[ignore]
fn dream_audit_child_helper() {
    let Some(root) = std::env::var_os("HIERO_DREAM_AUDIT_CHILD_ROOT") else {
        return;
    };
    let run_id: i64 = std::env::var("HIERO_DREAM_AUDIT_CHILD_RUN")
        .unwrap()
        .parse()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let config = HieronymusConfig::load(Some(root.into())).unwrap();
        let pool = db::connect(&config).await.unwrap();
        DreamAuditStore::new(&pool)
            .record(
                run_id,
                None,
                "child_append",
                "concurrent child append",
                &json!({"pid": std::process::id()}),
            )
            .await
            .unwrap();
    });
}

#[tokio::test]
async fn audit_lifecycle_is_transactional_redacted_and_monotonic() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(41, "fake").await.unwrap();
    let phase = store
        .start_phase(PhaseRunStart {
            dream_run_id: run.id,
            phase: "crystallization",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 2,
            prompt_hash: "abc",
        })
        .await
        .unwrap();
    store
        .record(
            run.id,
            Some(phase.id),
            "provider_request",
            "Authorization: Bearer summary-secret",
            &json!({"Authorization": "Bearer secret", "nested": {"access_token": "secret", "safe": 1}}),
        )
        .await
        .unwrap();
    store.complete_phase(phase.id, 1).await.unwrap();
    store
        .complete_run(run.id, DreamRunCompletion::new(2, 1, 0))
        .await
        .unwrap();
    // Repeating the same terminal transition is idempotent.
    store.complete_phase(phase.id, 1).await.unwrap();
    store
        .complete_run(run.id, DreamRunCompletion::new(2, 1, 0))
        .await
        .unwrap();

    let row = sqlx::query("SELECT status, completed_at FROM dream_runs WHERE id = ?")
        .bind(run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    use sqlx::Row;
    assert_eq!(row.get::<String, _>("status"), "completed");
    assert!(row.get::<Option<String>, _>("completed_at").is_some());
    let payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM dream_audit_entries WHERE event_type = 'provider_request'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!payload.contains("secret"));
    assert!(payload.contains("[REDACTED]"));
    let summary: String = sqlx::query_scalar(
        "SELECT summary FROM dream_audit_entries WHERE event_type = 'provider_request'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(summary, "[REDACTED]");
    assert!(
        store
            .fail_run(run.id, DreamRunCompletion::new(2, 1, 0), "late")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audit_redacts_extended_secret_keys_and_sensitive_string_values() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(49, "fake").await.unwrap();
    store
        .record(
            run.id,
            None,
            "secrets",
            "safe",
            &json!({
                "password": "p",
                "client-secret": "c",
                "private_key": "k",
                "sessionCookie": "cookie",
                "credential": "cred",
                "nested": [{"accessKey": "key"}],
                "message": "Authorization: Bearer plain-value-secret",
                "safe": "ordinary"
            }),
        )
        .await
        .unwrap();

    let payload: Value = serde_json::from_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT payload_json FROM dream_audit_entries WHERE event_type = 'secrets'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["safe"], "ordinary");
    assert_eq!(payload["password"], "[REDACTED]");
    assert_eq!(payload["client-secret"], "[REDACTED]");
    assert_eq!(payload["private_key"], "[REDACTED]");
    assert_eq!(payload["sessionCookie"], "[REDACTED]");
    assert_eq!(payload["credential"], "[REDACTED]");
    assert_eq!(payload["nested"][0]["accessKey"], "[REDACTED]");
    assert_eq!(payload["message"], "[REDACTED]");
}

#[tokio::test]
async fn audit_rejects_deep_wide_and_huge_values_before_append() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(50, "fake").await.unwrap();

    let mut deep = Value::Null;
    for _ in 0..80 {
        deep = json!({"next": deep});
    }
    let wide = Value::Array((0..5_000).map(|_| Value::Null).collect());
    let huge_string = json!({"safe": "x".repeat(70_000)});
    let huge_key = Value::Object([("k".repeat(70_000), Value::Null)].into_iter().collect());
    for (event, payload) in [
        ("too_deep", deep),
        ("too_wide", wide),
        ("huge_string", huge_string),
        ("huge_key", huge_key),
    ] {
        assert!(
            store
                .record(run.id, None, event, "bounded", &payload)
                .await
                .is_err()
        );
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dream_audit_entries WHERE event_type IN ('too_deep', 'too_wide', 'huge_string', 'huge_key')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn error_redaction_scans_full_input_then_truncates_on_utf8_boundary() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);

    let late_secret_run = store.start_run(51, "fake").await.unwrap();
    let late_secret = format!("{} client_secret=late", "x".repeat(5_000));
    store
        .fail_run(
            late_secret_run.id,
            DreamRunCompletion::new(0, 0, 0),
            &late_secret,
        )
        .await
        .unwrap();
    let late: String = sqlx::query_scalar("SELECT error FROM dream_runs WHERE id = ?")
        .bind(late_secret_run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(late, "[REDACTED]");

    let unicode_run = store.start_run(52, "fake").await.unwrap();
    store
        .fail_run(
            unicode_run.id,
            DreamRunCompletion::new(0, 0, 0),
            &"я".repeat(3_000),
        )
        .await
        .unwrap();
    let unicode: String = sqlx::query_scalar("SELECT error FROM dream_runs WHERE id = ?")
        .bind(unicode_run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(unicode.len() <= 4_096);
    assert!(unicode.is_char_boundary(unicode.len()));
}

#[tokio::test]
async fn negative_cycle_and_counts_are_rejected_without_writes_or_transitions() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    assert!(store.start_run(-1, "fake").await.is_err());
    let run = store.start_run(53, "fake").await.unwrap();
    assert!(
        store
            .start_phase(PhaseRunStart {
                dream_run_id: run.id,
                phase: "crystallization",
                provider_profile: "default",
                provider_type: "fake",
                model: "test-model",
                input_count: -1,
                prompt_hash: "",
            })
            .await
            .is_err()
    );
    assert!(
        store
            .complete_run(run.id, DreamRunCompletion::new(-1, 0, 0))
            .await
            .is_err()
    );
    let phase = store
        .start_phase(PhaseRunStart {
            dream_run_id: run.id,
            phase: "crystallization",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 0,
            prompt_hash: "",
        })
        .await
        .unwrap();
    assert!(store.complete_phase(phase.id, -1).await.is_err());

    let run_status: String = sqlx::query_scalar("SELECT status FROM dream_runs WHERE id = ?")
        .bind(run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let phase_status: String =
        sqlx::query_scalar("SELECT status FROM dream_phase_runs WHERE id = ?")
            .bind(phase.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let negative_cycles: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE cycle_id < 0")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (run_status, phase_status, negative_cycles),
        ("running".into(), "running".into(), 0)
    );
}

#[tokio::test]
async fn failure_apis_always_leave_run_and_phase_terminal() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(42, "fake").await.unwrap();
    let phase = store
        .start_phase(PhaseRunStart {
            dream_run_id: run.id,
            phase: "maintenance",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 0,
            prompt_hash: "",
        })
        .await
        .unwrap();

    store
        .fail_phase(phase.id, "api_key=very-secret")
        .await
        .unwrap();
    store
        .fail_run(
            run.id,
            DreamRunCompletion::new(0, 0, 0),
            "Bearer very-secret",
        )
        .await
        .unwrap();

    let phase_error: (String, String, Option<String>) =
        sqlx::query_as("SELECT status, error, completed_at FROM dream_phase_runs WHERE id = ?")
            .bind(phase.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let run_error: (String, String, Option<String>) =
        sqlx::query_as("SELECT status, error, completed_at FROM dream_runs WHERE id = ?")
            .bind(run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(phase_error.0, "failed");
    assert_eq!(run_error.0, "failed");
    assert!(phase_error.2.is_some() && run_error.2.is_some());
    assert!(!phase_error.1.contains("very-secret"));
    assert!(!run_error.1.contains("very-secret"));
}

#[tokio::test]
async fn failing_run_closes_every_still_running_phase_in_one_transaction() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(43, "fake").await.unwrap();
    let phase = store
        .start_phase(PhaseRunStart {
            dream_run_id: run.id,
            phase: "crystallization",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 1,
            prompt_hash: "",
        })
        .await
        .unwrap();

    store
        .fail_run(run.id, DreamRunCompletion::new(0, 0, 0), "cancelled")
        .await
        .unwrap();

    let phase_status: (String, Option<String>) =
        sqlx::query_as("SELECT status, completed_at FROM dream_phase_runs WHERE id = ?")
            .bind(phase.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(phase_status.0, "failed");
    assert!(phase_status.1.is_some());
}

#[tokio::test]
async fn oversized_audit_payload_is_rejected_without_partial_append() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(44, "fake").await.unwrap();

    let error = store
        .record(
            run.id,
            None,
            "too_large",
            "bounded",
            &json!({"safe": "x".repeat(70_000)}),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("64 KiB"));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dream_audit_entries WHERE event_type = 'too_large'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn run_cannot_complete_while_a_phase_is_running() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(45, "fake").await.unwrap();
    store
        .start_phase(PhaseRunStart {
            dream_run_id: run.id,
            phase: "crystallization",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 1,
            prompt_hash: "",
        })
        .await
        .unwrap();

    assert!(
        store
            .complete_run(run.id, DreamRunCompletion::new(1, 0, 0))
            .await
            .is_err()
    );
    let status: String = sqlx::query_scalar("SELECT status FROM dream_runs WHERE id = ?")
        .bind(run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "running");
}

#[tokio::test]
async fn audit_entry_rejects_a_phase_from_another_run() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let first = store.start_run(46, "fake").await.unwrap();
    let second = store.start_run(47, "fake").await.unwrap();
    let phase = store
        .start_phase(PhaseRunStart {
            dream_run_id: first.id,
            phase: "crystallization",
            provider_profile: "default",
            provider_type: "fake",
            model: "test-model",
            input_count: 1,
            prompt_hash: "",
        })
        .await
        .unwrap();

    assert!(
        store
            .record(
                second.id,
                Some(phase.id),
                "mismatched",
                "must roll back",
                &json!({}),
            )
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dream_audit_entries WHERE event_type = 'mismatched'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn audit_appends_are_safe_across_real_processes() {
    let root = TempDir::new().unwrap();
    let config = config(&root);
    let pool = db::connect(&config).await.unwrap();
    let store = DreamAuditStore::new(&pool);
    let run = store.start_run(48, "fake").await.unwrap();
    let mut children = Vec::new();
    for _ in 0..4 {
        let mut command = Command::new(std::env::current_exe().unwrap());
        children.push(
            command
                .arg("--ignored")
                .arg("--exact")
                .arg("dream_audit_child_helper")
                .env("HIERO_DREAM_AUDIT_CHILD_ROOT", root.path())
                .env("HIERO_DREAM_AUDIT_CHILD_RUN", run.id.to_string())
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dream_audit_entries WHERE dream_run_id = ? AND event_type = 'child_append'",
    )
    .bind(run.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 4);
}

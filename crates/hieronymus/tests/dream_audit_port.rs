//! Behavior ported from `tests/test_dream_audit.py`: audit append/list and
//! recursive payload redaction of secret-keyed fields.

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_audit::DreamAuditStore;

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

fn dream_run_id(config: &HieronymusConfig, cycle_id: i64) -> i64 {
    let connection = open_migrated(&config.database_path()).unwrap();
    connection
        .execute(
            "insert into dream_runs(cycle_id, status, provider, created_at)
             values (?1, 'running', 'test', '2026-06-09T00:00:00+00:00')",
            rusqlite::params![cycle_id],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn phase_run_id(config: &HieronymusConfig, dream_run_id: i64) -> i64 {
    let connection = open_migrated(&config.database_path()).unwrap();
    connection
        .execute(
            "insert into dream_phase_runs(
               dream_run_id, phase, provider_profile, provider_type, model, status, created_at
             )
             values (?1, 'extract', 'default', 'anthropic', 'claude-test', 'running',
                     '2026-06-09T00:00:00+00:00')",
            rusqlite::params![dream_run_id],
        )
        .unwrap();
    connection.last_insert_rowid()
}

#[test]
fn append_list_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);
    let phase_run = phase_run_id(&config, dream_run);

    let entry_id = store
        .append(
            dream_run,
            Some(phase_run),
            "provider_request",
            "info",
            "queued crystallization request",
            &json!({"model": "claude-test", "temperature": 0.2}),
        )
        .unwrap();

    let entries = store.list_for_run(dream_run).unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.id, entry_id);
    assert_eq!(entry.dream_run_id, dream_run);
    assert_eq!(entry.phase_run_id, Some(phase_run));
    assert_eq!(entry.event_type, "provider_request");
    assert_eq!(entry.severity, "info");
    assert_eq!(entry.summary, "queued crystallization request");
    assert_eq!(
        entry.payload,
        json!({"model": "claude-test", "temperature": 0.2})
    );
    assert!(!entry.created_at.is_empty());
}

#[test]
fn append_redacts_nested_secret_keys() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    store
        .append(
            dream_run,
            None,
            "provider_request",
            "info",
            "sent request",
            &json!({
                "model": "claude-test",
                "headers": {
                    "Authorization": "Bearer secret",
                    "anthropic-version": "2023-06-01",
                    "x-api-key": "secret-key"
                },
                "messages": [
                    {"role": "user", "token": "nested-secret", "content": "safe"},
                    {"apiKey": "camel-secret", "bearer": "bearer-secret"}
                ]
            }),
        )
        .unwrap();

    let payload = store.list_for_run(dream_run).unwrap().remove(0).payload;
    assert_eq!(
        payload,
        json!({
            "headers": {
                "Authorization": "[REDACTED]",
                "anthropic-version": "[REDACTED]",
                "x-api-key": "[REDACTED]"
            },
            "messages": [
                {"content": "safe", "role": "user", "token": "[REDACTED]"},
                {"apiKey": "[REDACTED]", "bearer": "[REDACTED]"}
            ],
            "model": "claude-test"
        })
    );
}

#[test]
fn append_redacts_gemini_api_key_header() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    store
        .append(
            dream_run,
            None,
            "provider_request",
            "info",
            "sent gemini request",
            &json!({
                "model": "gemini-2.5-pro",
                "headers": {"x-goog-api-key": "gemini-secret"}
            }),
        )
        .unwrap();

    assert_eq!(
        store.list_for_run(dream_run).unwrap()[0].payload,
        json!({
            "headers": {"x-goog-api-key": "[REDACTED]"},
            "model": "gemini-2.5-pro"
        })
    );
}

#[test]
fn append_redacts_secret_keys_inside_arrays() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    // The Python suite passes a tuple payload; the JSON projection is an array.
    store
        .append(
            dream_run,
            None,
            "provider_request",
            "info",
            "sent tuple payload",
            &json!({
                "messages": [
                    {"role": "user", "content": "safe"},
                    {"Authorization": "Bearer tuple-secret", "model": "claude-test"}
                ]
            }),
        )
        .unwrap();

    assert_eq!(
        store.list_for_run(dream_run).unwrap()[0].payload,
        json!({
            "messages": [
                {"content": "safe", "role": "user"},
                {"Authorization": "[REDACTED]", "model": "claude-test"}
            ]
        })
    );
}

#[test]
fn append_redacts_nested_token_like_keys() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    store
        .append(
            dream_run,
            None,
            "provider_response",
            "info",
            "received token payload",
            &json!({
                "provider": {
                    "model": "oauth-model",
                    "access_token": "access-secret",
                    "refresh_token": "refresh-secret"
                },
                "headers": [{"bearer_token": "bearer-secret", "status": "safe"}]
            }),
        )
        .unwrap();

    assert_eq!(
        store.list_for_run(dream_run).unwrap()[0].payload,
        json!({
            "headers": [{"bearer_token": "[REDACTED]", "status": "safe"}],
            "provider": {
                "access_token": "[REDACTED]",
                "model": "oauth-model",
                "refresh_token": "[REDACTED]"
            }
        })
    );
}

#[test]
fn list_for_run_orders_entries_by_id() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    let first_id = store
        .append(
            dream_run,
            None,
            "first",
            "info",
            "first entry",
            &Value::Null,
        )
        .unwrap();
    let second_id = store
        .append(
            dream_run,
            None,
            "second",
            "warning",
            "second entry",
            &Value::Null,
        )
        .unwrap();

    let ids: Vec<i64> = store
        .list_for_run(dream_run)
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(ids, vec![first_id, second_id]);
}

#[test]
fn phase_run_id_can_be_none() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let store = DreamAuditStore::open(&config).unwrap();
    let dream_run = dream_run_id(&config, 1);

    store
        .append(
            dream_run,
            None,
            "run_started",
            "info",
            "started run",
            &json!({"model": "deterministic"}),
        )
        .unwrap();

    assert_eq!(store.list_for_run(dream_run).unwrap()[0].phase_run_id, None);
}

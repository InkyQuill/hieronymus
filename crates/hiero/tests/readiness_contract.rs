use hiero::daemon::readiness::RuntimeReadiness;
use hiero::readiness::{ProviderCondition, ReadinessLevel};
use hieronymus::provider_observation::{ProviderKey, ProviderObserver, ProviderOutcome};

fn key(revision: u64) -> ProviderKey {
    ProviderKey {
        profile: "primary".into(),
        model: "model".into(),
        revision,
    }
}

#[test]
fn active_provider_failure_recovers_only_on_new_success() {
    let runtime = RuntimeReadiness::default();
    runtime.activate(vec![key(1)]);
    assert_eq!(
        runtime.snapshot().providers[0].condition,
        ProviderCondition::Untested
    );
    assert!(
        runtime.snapshot().providers[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("not yet verified")
    );
    runtime.completed(&key(1), 2, ProviderOutcome::Unavailable);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Degraded);
    runtime.completed(&key(1), 1, ProviderOutcome::Success);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Degraded);
    runtime.completed(&key(1), 2, ProviderOutcome::Success);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Degraded);
    runtime.completed(&key(1), 3, ProviderOutcome::Success);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Ready);
    assert!(runtime.snapshot().providers[0].observed_at.is_some());
}

#[test]
fn stale_provider_success_cannot_clear_current_failure() {
    let runtime = RuntimeReadiness::default();
    runtime.activate(vec![key(2)]);
    runtime.completed(&key(2), 2, ProviderOutcome::Unavailable);
    runtime.completed(&key(1), 3, ProviderOutcome::Success);
    assert_eq!(
        runtime.snapshot().providers[0].condition,
        ProviderCondition::Failed
    );
}

#[test]
fn inactive_provider_does_not_contribute_or_accumulate() {
    let runtime = RuntimeReadiness::default();
    runtime.activate(vec![key(2)]);
    for revision in 3..10_000 {
        runtime.completed(&key(revision), revision, ProviderOutcome::Authentication);
    }
    assert_eq!(runtime.snapshot().providers.len(), 1);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Ready);
    runtime.activate(vec![]);
    assert!(runtime.snapshot().providers.is_empty());
}

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::provider_config::{
    ProviderCatalog, ProviderDefaults, ProviderProfile, save_provider_catalog,
};
use std::sync::{Arc, atomic::AtomicBool};

fn configured() -> (
    tempfile::TempDir,
    HieronymusConfig,
    RuntimeReadiness,
    ProviderCatalog,
) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let mut catalog = ProviderCatalog::default().with_provider(
        "primary",
        ProviderProfile::new(
            "Primary",
            "openai",
            "https://example.invalid/v1",
            "SECRET-KEY",
            3.0,
        ),
    );
    catalog.defaults = ProviderDefaults::new("primary", "model");
    save_provider_catalog(&config, &catalog).unwrap();
    let mut dream = default_dream_config();
    dream.enabled = false;
    for (name, workflow) in &mut dream.workflows {
        workflow.enabled = name == "coverage_audit";
        workflow.provider.clear();
        workflow.model = " ".into();
    }
    save_dream_config(&config, &dream).unwrap();
    let runtime = RuntimeReadiness::for_config(config.clone(), Arc::new(AtomicBool::new(false)));
    (root, config, runtime, catalog)
}

#[test]
fn external_edits_defaults_and_unused_profiles_reconcile_without_rest() {
    let (_root, config, runtime, mut catalog) = configured();
    let before = runtime.snapshot().providers[0].clone();
    assert_eq!(before.capabilities, ["coverage_audit"]);
    let old = key(before.revision);
    runtime.completed(&old, 10, ProviderOutcome::Authentication);
    catalog.providers.insert(
        "unused".into(),
        ProviderProfile::new(
            "Unused",
            "openai",
            "https://unused.invalid",
            "UNUSED-SECRET",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    assert_eq!(
        runtime.snapshot().providers[0].condition,
        ProviderCondition::Failed
    );
    assert_eq!(runtime.snapshot().providers[0].revision, old.revision);
    catalog.providers.insert(
        "primary".into(),
        ProviderProfile::new(
            "Primary",
            "openai",
            "https://example.invalid/v1",
            "REPLACEMENT-SECRET",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    runtime.starting();
    let after = runtime.snapshot().providers[0].clone();
    assert!(after.revision > before.revision);
    assert_eq!(after.condition, ProviderCondition::Untested);
    runtime.completed(&old, 11, ProviderOutcome::Success);
    assert_eq!(
        runtime.snapshot().providers[0].condition,
        ProviderCondition::Untested
    );
    catalog.defaults.model = "replacement-model".into();
    save_provider_catalog(&config, &catalog).unwrap();
    assert_eq!(runtime.snapshot().providers[0].model, "replacement-model");
}

#[test]
fn disabled_and_deterministic_assignments_never_warn_and_state_is_ephemeral() {
    let (_root, config, runtime, _catalog) = configured();
    let before = runtime.snapshot().providers[0].clone();
    runtime.completed(&key(before.revision), 1, ProviderOutcome::Unavailable);
    let fresh = RuntimeReadiness::for_config(config.clone(), Arc::new(AtomicBool::new(false)));
    assert_eq!(
        fresh.snapshot().providers[0].condition,
        ProviderCondition::Untested
    );
    let mut dream = default_dream_config();
    for workflow in dream.workflows.values_mut() {
        workflow.provider = "deterministic".into();
        workflow.model = "deterministic".into();
        workflow.enabled = true;
    }
    save_dream_config(&config, &dream).unwrap();
    assert!(runtime.snapshot().providers.is_empty());
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Ready);
    for workflow in dream.workflows.values_mut() {
        workflow.enabled = false;
    }
    save_dream_config(&config, &dream).unwrap();
    assert!(runtime.snapshot().providers.is_empty());
}

#[test]
fn semantic_status_is_derived_from_worker_state_and_sanitized() {
    use hiero::daemon::semantic_worker::RequiredSemanticState as S;
    let runtime = RuntimeReadiness::default();
    assert_eq!(
        runtime.snapshot_with_semantic(&S::Ready).level,
        ReadinessLevel::Ready
    );
    assert_eq!(
        runtime.snapshot_with_semantic(&S::Acquiring).level,
        ReadinessLevel::Starting
    );
    assert_eq!(
        runtime.snapshot_with_semantic(&S::Rebuilding).level,
        ReadinessLevel::Starting
    );
    let failed = runtime.snapshot_with_semantic(&S::Failed("SENTINEL-RAW-ERROR".into()));
    assert_eq!(failed.level, ReadinessLevel::Degraded);
    assert!(!serde_json::to_string(&failed).unwrap().contains("SENTINEL"));
}

#[test]
fn shutdown_does_not_publish_a_provider_failure() {
    use std::sync::atomic::Ordering;
    let stop = Arc::new(AtomicBool::new(false));
    // A configured runtime supplies the daemon's cancellation edge.
    let (_fixture, config, _, _) = configured();
    let runtime = RuntimeReadiness::for_config(config, Arc::clone(&stop));
    let revision = runtime.snapshot().providers[0].revision;
    stop.store(true, Ordering::Release);
    runtime.completed(&key(revision), 1, ProviderOutcome::Unavailable);
    assert_eq!(
        runtime.snapshot().providers[0].condition,
        ProviderCondition::Untested
    );
}

mod common;
#[test]
fn authenticated_status_is_additive_and_excludes_secret_diagnostics() {
    let (_root, config, _runtime, mut catalog) = configured();
    catalog.providers.insert(
        "primary".into(),
        ProviderProfile::new(
            "Primary",
            "openai",
            "https://URL-USER:URL-PASSWORD@example.invalid/v1?api_key=URL-QUERY#URL-FRAGMENT",
            "SENTINEL-KEY",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    std::fs::write(
        config.dream_autostart_path(),
        r#"{"last_error":"SENTINEL-PROMPT SENTINEL-RAW-BODY SENTINEL-KEY"}"#,
    )
    .unwrap();
    let daemon = common::start_daemon(config.data_root());
    let port = daemon.local_addr().port();
    let headers = [(
        "Authorization".into(),
        format!("Bearer {}", daemon.bearer().expose_secret()),
    )];
    let response = common::send_request(port, "GET", "/status", &headers, &[]);
    assert_eq!(response.status, 200);
    let payload = response.body();
    assert_eq!(
        payload["instance_id"],
        daemon.discovery_record().instance_id
    );
    assert_eq!(payload["protocol_revision"], common::PROTOCOL_REVISION);
    assert!(payload["readiness"].is_object());
    for retained in [
        "providers",
        "dreaming",
        "semantic",
        "mcp_adapter",
        "running",
    ] {
        assert!(payload.get(retained).is_some());
    }
    let serialized = payload.to_string();
    for secret in [
        "SENTINEL-KEY",
        "SENTINEL-PROMPT",
        "SENTINEL-RAW-BODY",
        "URL-USER",
        "URL-PASSWORD",
        "URL-QUERY",
        "URL-FRAGMENT",
    ] {
        assert!(!serialized.contains(secret), "secret leaked: {secret}");
    }
    let unauthorized = common::send_request(port, "GET", "/status", &[], &[]);
    assert_eq!(unauthorized.status, 401);
    assert!(unauthorized.body().get("readiness").is_none());
    let invalid_host = common::send_request(
        port,
        "GET",
        "/status",
        &[
            ("Host".into(), "foreign.invalid".into()),
            headers[0].clone(),
        ],
        &[],
    );
    assert_eq!(invalid_host.status, 400);
    assert!(invalid_host.body().get("readiness").is_none());
    catalog.providers.insert(
        "primary".into(),
        ProviderProfile::new(
            "Primary",
            "openai",
            "https://example.invalid/v1",
            "2026",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    let short_key = common::send_request(port, "GET", "/status", &headers, &[]).body();
    assert_eq!(short_key["protocol_revision"], common::PROTOCOL_REVISION);
    assert_eq!(
        short_key["instance_id"],
        daemon.discovery_record().instance_id
    );
    std::fs::write(
        config.provider_config_path(),
        "SENTINEL-MALFORMED-CONFIG = [",
    )
    .unwrap();
    let malformed = common::send_request(port, "GET", "/status", &headers, &[]).body();
    assert_eq!(malformed["readiness"]["level"], "degraded");
    assert!(!malformed.to_string().contains("SENTINEL"));
    daemon.shutdown().unwrap();
}

#[test]
fn editing_another_active_provider_preserves_failure_and_rotated_keys_reject_old_results() {
    let (_root, config, runtime, mut catalog) = configured();
    catalog.providers.insert(
        "secondary".into(),
        ProviderProfile::new(
            "Secondary",
            "openai",
            "https://other.invalid",
            "SECONDARY-KEY",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    let mut dream = hieronymus::dream_config::load_dream_config(&config).unwrap();
    let secondary = dream.workflows.get_mut("knowledge_crystals").unwrap();
    secondary.enabled = true;
    secondary.provider = "secondary".into();
    secondary.model = "model".into();
    save_dream_config(&config, &dream).unwrap();
    let first = runtime
        .snapshot()
        .providers
        .iter()
        .find(|p| p.provider == "primary")
        .unwrap()
        .clone();
    let old = key(first.revision);
    runtime.completed(&old, 1, ProviderOutcome::Unavailable);
    catalog.providers.insert(
        "secondary".into(),
        ProviderProfile::new(
            "Secondary",
            "openai",
            "https://changed.invalid",
            "SECONDARY-KEY",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    let after = runtime.snapshot();
    let first = after
        .providers
        .iter()
        .find(|p| p.provider == "primary")
        .unwrap();
    assert_eq!(first.revision, old.revision);
    assert_eq!(first.condition, ProviderCondition::Failed);
    let other = after
        .providers
        .iter()
        .find(|p| p.provider == "secondary")
        .unwrap();
    runtime.completed(
        &ProviderKey {
            profile: other.provider.clone(),
            model: other.model.clone(),
            revision: other.revision,
        },
        2,
        ProviderOutcome::Success,
    );
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Degraded);
    catalog.providers.insert(
        "primary".into(),
        ProviderProfile::new(
            "Primary",
            "openai",
            "https://example.invalid/v1",
            "ROTATED-KEY",
            3.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();
    let after = runtime.snapshot();
    let first = after
        .providers
        .iter()
        .find(|p| p.provider == "primary")
        .unwrap();
    assert_eq!(first.condition, ProviderCondition::Untested);
    runtime.completed(&key(first.revision), 3, ProviderOutcome::Authentication);
    runtime.completed(&old, 4, ProviderOutcome::Success);
    assert_eq!(runtime.snapshot().level, ReadinessLevel::Degraded);
}

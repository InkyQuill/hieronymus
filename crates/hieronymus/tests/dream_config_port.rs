//! Behavior ported from `tests/test_dream_config.py`: dream.conf defaults,
//! workflow round-trips, legacy workflow migration, and validation errors.

use std::fs;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::{
    WorkflowProfile, default_dream_config, load_dream_config, redacted_dream_config_payload,
    save_dream_config,
};

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

fn write_dream_config(config: &HieronymusConfig, raw: &str) {
    fs::create_dir_all(config.config_root()).unwrap();
    fs::write(config.dream_config_path(), raw).unwrap();
}

#[test]
fn dream_config_paths_live_under_config_root() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);

    assert_eq!(
        config.dream_config_path(),
        config.config_root().join("dream.conf")
    );
    assert_eq!(
        config.llm_cache_path(),
        config.config_root().join("llmcache.tmp")
    );
}

#[test]
fn default_dream_config_matches_memory_spec() {
    let root = tempfile::tempdir().unwrap();
    let dream_config = load_dream_config(&config(&root)).unwrap();

    assert!(!dream_config.enabled);
    assert_eq!(dream_config.schedule_interval_minutes, 30);
    assert_eq!(dream_config.min_pending_short_term_memories, 20);
    assert_eq!(dream_config.max_pending_short_term_memories, 200);
    assert_eq!(dream_config.max_short_term_memories_per_cycle, 50);
    assert_eq!(dream_config.not_enough_memories_cycle_threshold, 5);
    let names: Vec<&str> = dream_config.workflows.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            "concepts",
            "terminology_candidates",
            "rule_crystals",
            "knowledge_crystals",
            "relations",
            "reinforcement",
            "coverage_audit",
        ]
    );
    assert_eq!(dream_config.workflows["knowledge_crystals"].provider, "");
    assert_eq!(dream_config.max_short_term_memories_per_run, 500);
}

#[test]
fn save_dream_config_does_not_write_provider_profiles_or_secrets() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let dream_config = default_dream_config();

    save_dream_config(&config, &dream_config).unwrap();

    let raw = fs::read_to_string(config.dream_config_path()).unwrap();
    assert!(!raw.contains("api_key"), "{raw}");
    assert!(!raw.contains("providers"), "{raw}");
    assert!(!redacted_dream_config_payload(&dream_config).contains_key("providers"));
}

#[test]
fn load_save_dream_config_round_trips_workflows_without_providers() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile::new("local_llm", "local-model", true),
    );

    save_dream_config(&config, &dream_config).unwrap();
    let loaded = load_dream_config(&config).unwrap();

    assert_eq!(
        loaded.workflows["knowledge_crystals"],
        WorkflowProfile::new("local_llm", "local-model", true)
    );
    assert!(!loaded.workflows.contains_key("providers"));
}

#[test]
fn load_dream_config_migrates_legacy_workflows_to_disk() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_dream_config(
        &config,
        r#"
[dreaming]
enabled = false

[workflows.crystallization]
provider = "legacy_provider"
model = "legacy-model"
enabled = true

[workflows.relation_discovery]
provider = "relations_provider"
model = "relations-model"
enabled = true

[workflows.reinforcement_compaction]
provider = "maintenance_provider"
model = "maintenance-model"
enabled = true
"#,
    );

    let loaded = load_dream_config(&config).unwrap();

    let names: Vec<&str> = loaded.workflows.keys().map(String::as_str).collect();
    assert_eq!(names.len(), 7);
    assert_eq!(
        loaded.workflows["knowledge_crystals"].provider,
        "legacy_provider"
    );
    assert_eq!(loaded.workflows["relations"].model, "relations-model");
    assert_eq!(
        loaded.workflows["coverage_audit"].provider,
        "maintenance_provider"
    );
    let saved = fs::read_to_string(config.dream_config_path()).unwrap();
    assert!(saved.contains("[workflows.coverage_audit]"), "{saved}");
    assert!(!saved.contains("[workflows.crystallization]"), "{saved}");
}

#[test]
fn load_dream_config_rejects_unknown_workflow_without_migrating() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    fs::create_dir_all(config.data_root()).unwrap();
    fs::write(
        config.dream_config_path(),
        "[workflows.unknown]\nprovider = 'openai'\nmodel = 'gpt-4.1-mini'\nenabled = true\n",
    )
    .unwrap();

    let error = load_dream_config(&config).unwrap_err();

    assert!(
        error.to_string().contains("workflows must contain exactly"),
        "{error}"
    );
}

#[test]
fn load_dream_config_rejects_invalid_threshold_order() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_dream_config(
        &config,
        "[dreaming]\n\
         enabled = true\n\
         schedule_interval_minutes = 30\n\
         min_pending_short_term_memories = 20\n\
         max_pending_short_term_memories = 10\n\
         max_short_term_memories_per_cycle = 50\n\
         not_enough_memories_cycle_threshold = 5\n",
    );

    let error = load_dream_config(&config).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("max_pending_short_term_memories")
    );
}

#[test]
fn load_dream_config_rejects_non_positive_thresholds() {
    for (raw, expected) in [
        (
            "[dreaming]\nschedule_interval_minutes = 0\n",
            "schedule_interval_minutes must be at least 1",
        ),
        (
            "[dreaming]\nmin_pending_short_term_memories = -1\n",
            "min_pending_short_term_memories must be at least 1",
        ),
        (
            "[dreaming]\nmax_pending_short_term_memories = 0\n",
            "max_pending_short_term_memories must be at least 1",
        ),
        (
            "[dreaming]\nmax_short_term_memories_per_cycle = 0\n",
            "max_short_term_memories_per_cycle must be at least 1",
        ),
        (
            "[dreaming]\nnot_enough_memories_cycle_threshold = 0\n",
            "not_enough_memories_cycle_threshold must be at least 1",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_dream_config(&config, raw);

        let error = load_dream_config(&config).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}: {raw}");
    }
}

#[test]
fn deprecated_provider_payload_is_ignored() {
    for raw in [
        "[providers.local_llm]\nendpoint = \"http://localhost:11434\"\n",
        "[providers.ollama]\nendpoint = \"http://localhost:11435\"\n",
        "[providers.local_llm]\ntype = \"local\"\n",
        "[providers.ollama]\ntimeout_seconds = 0\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_dream_config(&config, raw);

        let dream_config = load_dream_config(&config).unwrap();

        assert!(
            !dream_config
                .workflows
                .iter()
                .any(|(name, _)| name == "providers")
        );
        assert!(!redacted_dream_config_payload(&dream_config).contains_key("providers"));
    }
}

#[test]
fn workflow_provider_existence_is_validated_outside_dream_config() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile::new("missing", "model", true),
    );

    save_dream_config(&config, &dream_config).unwrap();

    assert_eq!(
        load_dream_config(&config).unwrap().workflows["knowledge_crystals"].provider,
        "missing"
    );
}

#[test]
fn enabled_workflow_rejects_empty_model() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile::new("anthropic", "", true),
    );

    let error = save_dream_config(&config, &dream_config).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("enabled workflow must have a model"),
        "{error}"
    );
}

#[test]
fn load_dream_config_rejects_toml_type_mismatches() {
    for (raw, expected) in [
        ("[dreaming]\nenabled = 'yes'\n", "enabled must be a boolean"),
        (
            "[dreaming]\nschedule_interval_minutes = true\n",
            "schedule_interval_minutes must be an integer",
        ),
        (
            "[workflows.knowledge_crystals]\nprovider = 123\n",
            "workflows.knowledge_crystals.provider must be a string",
        ),
        (
            "[workflows.knowledge_crystals]\nenabled = 'true'\n",
            "workflows.knowledge_crystals.enabled must be a boolean",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_dream_config(&config, raw);

        let error = load_dream_config(&config).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}: {raw}");
    }
}

#[test]
fn load_dream_config_migrates_removed_workflow_names() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    fs::create_dir_all(config.config_root()).unwrap();
    fs::write(
        config.dream_config_path(),
        r#"
[providers.openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"

[workflows.crystallization]
provider = "openai"
model = "deepseek-v4-flash"
enabled = true
"#,
    )
    .unwrap();

    let loaded = load_dream_config(&config).unwrap();

    assert_eq!(loaded.workflows["knowledge_crystals"].provider, "openai");
}

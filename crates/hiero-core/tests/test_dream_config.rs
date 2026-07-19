use std::{collections::BTreeMap, fs};

use hiero_core::{
    dreaming::{
        DreamConfig, DreamConfigError, DreamPhase, PhaseProfile, WorkflowProfile,
        build_phase_prompt, resolve_workflows,
    },
    provider::{PassName, ProviderCatalog, ProviderDefaults, ProviderProfile},
};

fn catalog() -> ProviderCatalog {
    let mut catalog = ProviderCatalog::default();
    catalog
        .upsert(ProviderProfile::new(
            "remote",
            "Remote",
            "openai",
            "https://example.test/v1",
        ))
        .expect("profile should be valid");
    catalog.set_defaults(ProviderDefaults {
        provider: "remote".into(),
        model: "default-model".into(),
    });
    catalog
}

#[test]
fn defaults_match_the_seven_pass_memory_contract() {
    let config = DreamConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.schedule_interval_minutes, 30);
    assert_eq!(config.min_pending_short_term_memories, 20);
    assert_eq!(config.max_pending_short_term_memories, 200);
    assert_eq!(config.max_short_term_memories_per_cycle, 50);
    assert_eq!(config.not_enough_memories_cycle_threshold, 5);
    assert_eq!(config.reconsolidation_diff_threshold, 0.20);
    assert_eq!(config.workflows.len(), PassName::ALL.len());
    assert!(config.workflows.values().all(|phase| !phase.enabled));
}

#[test]
fn dream_phase_is_the_provider_pass_identifier() {
    let phase: DreamPhase = PassName::KnowledgeCrystals;
    assert_eq!(phase.as_str(), "knowledge_crystals");
}

#[test]
fn resolver_applies_catalog_defaults_and_explicit_phase_overrides() {
    let config = DreamConfig::default()
        .with_phase(
            DreamPhase::Concepts,
            PhaseProfile {
                enabled: true,
                ..PhaseProfile::default()
            },
        )
        .with_phase(
            DreamPhase::KnowledgeCrystals,
            PhaseProfile {
                provider: "remote".into(),
                model: "explicit-model".into(),
                enabled: true,
                max_records_per_pass: 42,
            },
        );

    let resolved = resolve_workflows(&config, &catalog()).expect("workflow should resolve");

    assert_eq!(
        resolved,
        vec![
            WorkflowProfile {
                phase: DreamPhase::Concepts,
                provider: "remote".into(),
                model: "default-model".into(),
                max_records_per_pass: 500,
            },
            WorkflowProfile {
                phase: DreamPhase::KnowledgeCrystals,
                provider: "remote".into(),
                model: "explicit-model".into(),
                max_records_per_pass: 42,
            },
        ]
    );
}

#[test]
fn resolver_validates_every_enabled_phase_before_returning_any() {
    let config = DreamConfig::default()
        .with_phase(
            DreamPhase::Concepts,
            PhaseProfile {
                enabled: true,
                ..PhaseProfile::default()
            },
        )
        .with_phase(
            DreamPhase::Relations,
            PhaseProfile {
                provider: "missing".into(),
                model: "model".into(),
                enabled: true,
                ..PhaseProfile::default()
            },
        );

    let error = resolve_workflows(&config, &catalog()).expect_err("all phases must validate");
    assert!(
        error
            .to_string()
            .contains("provider profile missing: missing")
    );
}

#[test]
fn resolver_fails_closed_for_missing_provider_or_model() {
    let enabled = PhaseProfile {
        enabled: true,
        ..PhaseProfile::default()
    };
    let no_defaults = ProviderCatalog::default();
    let missing_provider = DreamConfig::default().with_phase(DreamPhase::Concepts, enabled.clone());
    assert!(
        resolve_workflows(&missing_provider, &no_defaults)
            .expect_err("provider is required")
            .to_string()
            .contains("enabled workflow must have a provider: concepts")
    );

    let mut no_model = ProviderCatalog::default();
    no_model
        .upsert(ProviderProfile::new(
            "remote",
            "Remote",
            "openai",
            "https://example.test/v1",
        ))
        .expect("profile should be valid");
    let missing_model = DreamConfig::default().with_phase(
        DreamPhase::Concepts,
        PhaseProfile {
            provider: "remote".into(),
            ..enabled
        },
    );
    assert!(
        resolve_workflows(&missing_model, &no_model)
            .expect_err("model is required")
            .to_string()
            .contains("enabled workflow must have a model: concepts")
    );
}

#[test]
fn disabled_phases_do_not_require_provider_resolution() {
    let config = DreamConfig::default().with_phase(
        DreamPhase::Concepts,
        PhaseProfile {
            provider: "missing".into(),
            model: String::new(),
            enabled: false,
            max_records_per_pass: 500,
        },
    );
    assert!(
        resolve_workflows(&config, &ProviderCatalog::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn validation_rejects_invalid_intervals_thresholds_and_phase_sets() {
    let invalid = DreamConfig {
        schedule_interval_minutes: 0,
        ..DreamConfig::default()
    };
    assert!(matches!(
        invalid.validate(),
        Err(DreamConfigError::Invalid(_))
    ));

    let invalid = DreamConfig {
        max_pending_short_term_memories: 10,
        ..DreamConfig::default()
    };
    assert!(invalid.validate().is_err());

    let mut invalid = DreamConfig::default();
    invalid.workflows.remove(&DreamPhase::CoverageAudit);
    assert!(invalid.validate().is_err());
}

#[test]
fn strict_toml_rejects_unknown_phase_and_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let unknown_phase = dir.path().join("unknown-phase.conf");
    fs::write(&unknown_phase, "[workflows.unknown]\nenabled = true\n").unwrap();
    assert!(DreamConfig::load_from(&unknown_phase).is_err());

    let unknown_field = dir.path().join("unknown-field.conf");
    fs::write(&unknown_field, "[dreaming]\nsurprise = 1\n").unwrap();
    assert!(DreamConfig::load_from(&unknown_field).is_err());
}

#[test]
fn load_merges_partial_tables_with_defaults_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dream.conf");
    fs::write(
        &path,
        "[dreaming]\nenabled = true\n\n[workflows.knowledge_crystals]\nprovider = 'remote'\nmodel = 'model'\nenabled = true\nmax_records_per_pass = 12\n",
    )
    .unwrap();

    let loaded = DreamConfig::load_from(&path).expect("partial config should load");
    assert!(loaded.enabled);
    assert_eq!(loaded.workflows.len(), 7);
    assert_eq!(
        loaded.workflows[&DreamPhase::KnowledgeCrystals].max_records_per_pass,
        12
    );

    loaded
        .save_to(&path)
        .expect("config should save atomically");
    assert_eq!(DreamConfig::load_from(&path).unwrap(), loaded);
    assert!(!fs::read_to_string(path).unwrap().contains("providers"));
}

#[test]
fn loading_a_missing_file_returns_validated_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        DreamConfig::load_from(dir.path().join("missing.conf")).unwrap(),
        DreamConfig::default()
    );
}

#[test]
fn build_prompt_is_typed_stable_and_rejects_no_phase() {
    let prompt = build_phase_prompt(
        &DreamConfig::default(),
        DreamPhase::RuleCrystals,
        &serde_json::json!({"memory_ids": [2, 1]}),
    );
    assert!(prompt.contains("deterministic translation rules"));
    assert!(prompt.contains("Return a single JSON object."));
    assert!(prompt.contains("\"memory_ids\":[2,1]"));
}

#[test]
fn serde_shape_contains_no_provider_profiles() {
    let value = toml::Value::try_from(DreamConfig::default()).unwrap();
    let root = value.as_table().unwrap();
    assert_eq!(
        root.keys().cloned().collect::<Vec<_>>(),
        vec!["dreaming", "workflows"]
    );
}

#[test]
fn direct_toml_deserialize_merges_partial_settings_and_phases() {
    let config: DreamConfig = toml::from_str(
        "[dreaming]\nenabled = true\n\n[workflows.relations]\nprovider = 'remote'\nmodel = 'model'\nenabled = true\n",
    )
    .expect("typed TOML should merge over defaults");

    assert!(config.enabled);
    assert_eq!(config.schedule_interval_minutes, 30);
    assert_eq!(config.workflows.len(), PassName::ALL.len());
    assert!(config.workflows[&DreamPhase::Relations].enabled);
    assert!(!config.workflows[&DreamPhase::Concepts].enabled);
}

#[test]
fn direct_json_deserialize_merges_partial_settings_and_phases() {
    let config: DreamConfig = serde_json::from_value(serde_json::json!({
        "dreaming": {"schedule_interval_minutes": 45},
        "workflows": {"coverage_audit": {"enabled": true, "provider": "remote", "model": "model"}}
    }))
    .expect("typed JSON should merge over defaults");

    assert_eq!(config.schedule_interval_minutes, 45);
    assert_eq!(config.workflows.len(), PassName::ALL.len());
    assert!(config.workflows[&DreamPhase::CoverageAudit].enabled);
}

#[test]
fn direct_deserialize_round_trips_the_full_serialized_contract() {
    let expected = DreamConfig::default().with_phase(
        DreamPhase::KnowledgeCrystals,
        PhaseProfile {
            provider: "remote".into(),
            model: "model".into(),
            enabled: true,
            max_records_per_pass: 17,
        },
    );

    let toml = toml::to_string(&expected).unwrap();
    let json = serde_json::to_string(&expected).unwrap();

    assert_eq!(toml::from_str::<DreamConfig>(&toml).unwrap(), expected);
    assert_eq!(
        serde_json::from_str::<DreamConfig>(&json).unwrap(),
        expected
    );
}

#[test]
fn direct_deserialize_rejects_unknown_and_invalid_configuration() {
    let cases = [
        "[dreaming]\nunknown = 1\n",
        "[workflows.unknown]\nenabled = false\n",
        "[dreaming]\nschedule_interval_minutes = 0\n",
    ];

    for raw in cases {
        assert!(
            toml::from_str::<DreamConfig>(raw).is_err(),
            "accepted {raw}"
        );
    }
    assert!(
        serde_json::from_value::<DreamConfig>(serde_json::json!({
            "dreaming": {"schedule_interval_minutes": 0}
        }))
        .is_err()
    );
}

#[test]
fn oversized_save_fails_before_replacing_an_existing_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dream.conf");
    DreamConfig::default().save_to(&path).unwrap();
    let original = fs::read(&path).unwrap();
    let oversized = DreamConfig {
        general_prompt: "x".repeat(1024 * 1024),
        ..DreamConfig::default()
    };

    let error = oversized
        .save_to(&path)
        .expect_err("serialized configs over the read bound must be rejected");

    assert!(error.to_string().contains("exceeds"));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn save_accepts_a_serialized_config_at_the_exact_read_boundary() {
    const LIMIT: usize = 1024 * 1024;
    let default = DreamConfig::default();
    let base = toml::to_string_pretty(&default).unwrap();
    let fixed_bytes = base.len() - default.general_prompt.len();
    let exact = DreamConfig {
        general_prompt: "x".repeat(LIMIT - fixed_bytes),
        ..default
    };
    assert_eq!(toml::to_string_pretty(&exact).unwrap().len(), LIMIT);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dream.conf");

    exact.save_to(&path).expect("the boundary is inclusive");

    assert_eq!(fs::metadata(&path).unwrap().len(), LIMIT as u64);
    assert_eq!(DreamConfig::load_from(path).unwrap(), exact);
}

#[cfg(unix)]
#[test]
fn save_uses_private_permissions_and_rejects_symlink_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dream.conf");
    DreamConfig::default().save_to(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let target = dir.path().join("target");
    fs::write(&target, "sentinel").unwrap();
    let link = dir.path().join("link.conf");
    symlink(&target, &link).unwrap();
    assert!(DreamConfig::default().save_to(&link).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "sentinel");
}

#[cfg(unix)]
#[test]
fn load_rejects_special_nodes_and_symlinked_ancestors() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    assert!(DreamConfig::load_from(dir.path()).is_err());

    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    DreamConfig::default()
        .save_to(real.join("dream.conf"))
        .unwrap();
    let linked = dir.path().join("linked");
    symlink(&real, &linked).unwrap();
    assert!(DreamConfig::load_from(linked.join("dream.conf")).is_err());
}

#[cfg(unix)]
#[test]
fn save_rejects_existing_special_nodes_instead_of_replacing_them() {
    use std::os::unix::{fs::FileTypeExt, net::UnixListener};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dream.conf");
    let listener = UnixListener::bind(&path).unwrap();

    assert!(DreamConfig::default().save_to(&path).is_err());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());

    drop(listener);
}

#[test]
fn btree_input_order_cannot_change_resolved_phase_order() {
    let mut workflows = BTreeMap::new();
    for phase in PassName::ALL.into_iter().rev() {
        workflows.insert(
            phase,
            PhaseProfile {
                enabled: true,
                ..PhaseProfile::default()
            },
        );
    }
    let config = DreamConfig {
        workflows,
        ..DreamConfig::default()
    };
    let phases = resolve_workflows(&config, &catalog())
        .unwrap()
        .into_iter()
        .map(|workflow| workflow.phase)
        .collect::<Vec<_>>();
    assert_eq!(phases, PassName::ALL);
}

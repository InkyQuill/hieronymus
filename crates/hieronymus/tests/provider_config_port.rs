//! Behavior ported from `tests/test_provider_config.py`: the provider.conf
//! catalog contract, legacy dream-provider migration, and secret redaction.

use std::fs;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::provider_config::{
    ProviderCatalog, ProviderDefaults, ProviderProfile, default_provider_catalog,
    load_provider_catalog, migrate_dream_provider_payload, redacted_provider_catalog_payload,
    save_provider_catalog, validate_provider_catalog,
};

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

fn write_provider_config(config: &HieronymusConfig, text: &str) {
    fs::create_dir_all(config.config_root()).unwrap();
    fs::write(config.provider_config_path(), text).unwrap();
}

fn profile(name: &str, provider_type: &str, url: &str) -> ProviderProfile {
    ProviderProfile::new(name, provider_type, url, "", 30.0)
}

#[test]
fn load_provider_catalog_defaults_when_missing() {
    let root = tempfile::tempdir().unwrap();
    let catalog = load_provider_catalog(&config(&root)).unwrap();

    assert!(catalog.providers.is_empty());
    assert_eq!(catalog.defaults, ProviderDefaults::new("", ""));
}

#[test]
fn load_provider_catalog_rejects_legacy_dream_providers_without_migrating() {
    // ADR 0009/0010: a `dream.conf` `[providers]` block is
    // `config_migration_required`, never a silent rewrite of two files on
    // read. Only the staged `hiero migrate` protocol converts it.
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    fs::create_dir_all(config.config_root()).unwrap();
    let dream_raw = r#"
[providers.openai]
name = "DeepSeek"
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "raw-secret"
timeout_seconds = 12

[workflows.knowledge_crystals]
provider = "openai"
model = "deepseek-v4-flash"
enabled = true
"#;
    fs::write(config.dream_config_path(), dream_raw).unwrap();

    let error = load_provider_catalog(&config).unwrap_err();

    assert!(error.is_migration_required(), "{error}");
    assert!(error.to_string().contains("hiero migrate"), "{error}");
    assert_eq!(
        fs::read_to_string(config.dream_config_path()).unwrap(),
        dream_raw
    );
    assert!(!config.provider_config_path().exists());
}

#[test]
fn load_provider_catalog_rejects_legacy_dream_providers_before_collision_check() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    save_provider_catalog(
        &config,
        &ProviderCatalog {
            providers: [(
                "openai".to_string(),
                ProviderProfile::new(
                    "OpenAI",
                    "openai",
                    "https://api.openai.com/v1",
                    "existing-secret",
                    30.0,
                ),
            )]
            .into(),
            defaults: ProviderDefaults::default(),
        },
    )
    .unwrap();
    let provider_before = fs::read_to_string(config.provider_config_path()).unwrap();
    fs::write(
        config.dream_config_path(),
        r#"
[providers.openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "legacy-secret"
"#,
    )
    .unwrap();

    let error = load_provider_catalog(&config).unwrap_err();

    assert!(error.is_migration_required(), "{error}");
    let provider_raw = fs::read_to_string(config.provider_config_path()).unwrap();
    assert_eq!(provider_raw, provider_before);
    assert!(!provider_raw.contains("legacy-secret"));
}

#[test]
fn default_provider_catalog_returns_empty_providers_and_defaults() {
    assert_eq!(
        default_provider_catalog(),
        ProviderCatalog {
            providers: Default::default(),
            defaults: ProviderDefaults::new("", ""),
        }
    );
}

#[test]
fn load_provider_catalog_rejects_invalid_toml() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_provider_config(&config, "[deepseek\n");

    let error = load_provider_catalog(&config).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("provider.conf is not valid TOML")
    );
}

#[test]
fn save_and_load_provider_catalog_round_trips_profiles_and_defaults() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let catalog = ProviderCatalog {
        providers: [
            (
                "deepseek-api".to_string(),
                ProviderProfile::new(
                    "Deepseek",
                    "openai",
                    "https://api.deepseek.com",
                    "raw-secret",
                    45.0,
                ),
            ),
            (
                "local-ollama".to_string(),
                ProviderProfile::new("Ollama", "openai", "http://127.0.0.1:6000/v1", "", 30.0),
            ),
        ]
        .into(),
        defaults: ProviderDefaults::new("deepseek-api", "deepseek-v4-flash"),
    };

    save_provider_catalog(&config, &catalog).unwrap();

    let raw = fs::read_to_string(config.provider_config_path()).unwrap();
    assert!(raw.contains("[deepseek-api]"), "{raw}");
    assert!(raw.contains("[local-ollama]"), "{raw}");
    assert!(raw.contains("[defaults]"), "{raw}");
    assert!(!raw.contains("[providers."), "{raw}");
    assert!(raw.contains("raw-secret"), "{raw}");

    assert_eq!(load_provider_catalog(&config).unwrap(), catalog);
}

#[test]
fn redacted_provider_catalog_payload_redacts_keys() {
    let catalog = ProviderCatalog {
        providers: [
            (
                "deepseek-api".to_string(),
                ProviderProfile::new(
                    "Deepseek",
                    "openai",
                    "https://api.deepseek.com",
                    "raw-secret",
                    30.0,
                ),
            ),
            (
                "local-ollama".to_string(),
                ProviderProfile::new("Ollama", "ollama", "http://127.0.0.1:11434", "", 30.0),
            ),
        ]
        .into(),
        defaults: ProviderDefaults::new("deepseek-api", "deepseek-chat"),
    };

    let payload = redacted_provider_catalog_payload(&catalog).to_string();

    assert!(!payload.contains("raw-secret"), "{payload}");
    assert!(payload.contains("\"***\""), "{payload}");
}

#[test]
fn provider_catalog_validates_default_provider_exists() {
    let error = validate_provider_catalog(&ProviderCatalog {
        providers: Default::default(),
        defaults: ProviderDefaults::new("deepseek-api", "deepseek-v4-flash"),
    })
    .unwrap_err();

    assert!(error.to_string().contains("default provider is missing"));
}

#[test]
fn provider_catalog_accepts_supported_provider_types() {
    for provider_type in ["openai", "google", "anthropic", "ollama"] {
        let catalog = ProviderCatalog {
            providers: [(
                provider_type.to_string(),
                profile(provider_type, provider_type, "https://example.test"),
            )]
            .into(),
            defaults: ProviderDefaults::default(),
        };
        assert_eq!(validate_provider_catalog(&catalog).unwrap(), catalog);
    }
}

#[test]
fn provider_catalog_rejects_unknown_provider_type() {
    let error = validate_provider_catalog(&ProviderCatalog {
        providers: [(
            "bad".to_string(),
            profile("Bad", "made-up", "https://example.test"),
        )]
        .into(),
        defaults: ProviderDefaults::default(),
    })
    .unwrap_err();

    assert!(error.to_string().contains("unsupported provider type"));
}

#[test]
fn load_provider_catalog_canonicalizes_gemini_alias_without_rewriting() {
    // The deprecated `gemini` spelling is an accepted alias: it resolves to
    // `google` in memory, and the file on disk is left byte-identical
    // (ADR 0010: no mutation on read).
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let raw = "[gemini]\ntype = \"gemini\"\nurl = \"https://generativelanguage.googleapis.com\"\n";
    write_provider_config(&config, raw);

    let catalog = load_provider_catalog(&config).unwrap();

    assert_eq!(catalog.providers["gemini"].provider_type(), "google");
    assert_eq!(
        fs::read_to_string(config.provider_config_path()).unwrap(),
        raw
    );
}

#[test]
fn load_provider_catalog_defaults_provider_name_to_table_id() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_provider_config(
        &config,
        "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\n",
    );

    let catalog = load_provider_catalog(&config).unwrap();

    assert_eq!(
        catalog.providers["deepseek"],
        ProviderProfile::new("deepseek", "openai", "https://api.deepseek.com", "", 30.0)
    );
}

#[test]
fn load_provider_catalog_rejects_missing_required_provider_fields() {
    for (raw, expected) in [
        (
            "[deepseek]\nurl = \"https://api.deepseek.com\"\n",
            "deepseek.type is required",
        ),
        (
            "[deepseek]\ntype = \"openai\"\n",
            "deepseek.url is required",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_provider_config(&config, raw);

        let error = load_provider_catalog(&config).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn load_provider_catalog_rejects_unknown_keys() {
    for (raw, expected) in [
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\nextra = \"nope\"\n",
            "unknown provider config setting: deepseek.extra",
        ),
        (
            "[defaults]\nprovider = \"\"\nmodel = \"\"\nextra = \"nope\"\n",
            "unknown provider config setting: defaults.extra",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_provider_config(&config, raw);

        let error = load_provider_catalog(&config).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn provider_catalog_rejects_invalid_provider_ids() {
    for provider_id in ["", "defaults", "deep.seek", "deep seek"] {
        let error = validate_provider_catalog(&ProviderCatalog {
            providers: [(
                provider_id.to_string(),
                profile("Bad", "openai", "https://example.test"),
            )]
            .into(),
            defaults: ProviderDefaults::default(),
        })
        .unwrap_err();

        assert!(error.to_string().contains("invalid provider id"), "{error}");
    }
}

#[test]
fn provider_catalog_allows_default_model_without_default_provider() {
    let catalog = ProviderCatalog {
        providers: Default::default(),
        defaults: ProviderDefaults::new("", "deepseek-v4-flash"),
    };
    assert_eq!(validate_provider_catalog(&catalog).unwrap(), catalog);
}

#[test]
fn load_provider_catalog_rejects_field_type_mismatches() {
    for (raw, expected) in [
        (
            "[deepseek]\nname = 1\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\n",
            "providers.deepseek.name must be a string",
        ),
        (
            "[deepseek]\ntype = 1\nurl = \"https://api.deepseek.com\"\n",
            "providers.deepseek.type must be a string",
        ),
        (
            "[deepseek]\ntype = \"openai\"\nurl = 1\n",
            "providers.deepseek.url must be a string",
        ),
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\nkey = 1\n",
            "providers.deepseek.key must be a string",
        ),
        (
            "[defaults]\nprovider = 1\n",
            "defaults.provider must be a string",
        ),
        ("[defaults]\nmodel = 1\n", "defaults.model must be a string"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_provider_config(&config, raw);

        let error = load_provider_catalog(&config).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn load_provider_catalog_rejects_invalid_timeout_values() {
    for (raw, expected) in [
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\ntimeout_seconds = \"slow\"\n",
            "providers.deepseek.timeout_seconds must be a number",
        ),
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\ntimeout_seconds = 0\n",
            "providers.deepseek.timeout_seconds must be greater than 0",
        ),
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\ntimeout_seconds = -1\n",
            "providers.deepseek.timeout_seconds must be greater than 0",
        ),
        (
            "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\ntimeout_seconds = inf\n",
            "providers.deepseek.timeout_seconds must be finite and greater than 0",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = config(&root);
        write_provider_config(&config, raw);

        let error = load_provider_catalog(&config).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn migrate_dream_provider_payload_preserves_secret_and_endpoint() {
    let payload: toml::Table = toml::from_str(
        r#"[openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"
timeout_seconds = 12
"#,
    )
    .unwrap();

    let catalog = migrate_dream_provider_payload(&payload, &ProviderCatalog::default()).unwrap();

    assert_eq!(
        catalog.providers["openai"],
        ProviderProfile::new(
            "Openai",
            "openai",
            "https://api.deepseek.com",
            "secret",
            12.0
        )
    );
}

#[test]
fn migrate_dream_provider_payload_rejects_profile_collision() {
    let existing = ProviderCatalog {
        providers: [(
            "openai".to_string(),
            ProviderProfile::new("Existing", "openai", "https://api.openai.com/v1", "", 30.0),
        )]
        .into(),
        defaults: ProviderDefaults::default(),
    };
    let payload: toml::Table = toml::from_str(
        r#"[openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"
"#,
    )
    .unwrap();

    let error = migrate_dream_provider_payload(&payload, &existing).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("would overwrite provider profile")
    );
}

#[test]
fn migrate_dream_provider_payload_rejects_missing_type() {
    let payload: toml::Table = toml::from_str(
        r#"[openai]
endpoint = "https://api.deepseek.com"
api_key = "secret"
"#,
    )
    .unwrap();

    let error = migrate_dream_provider_payload(&payload, &ProviderCatalog::default()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("providers.openai.type is required"),
        "{error}"
    );
}

#[test]
fn migrate_dream_provider_payload_rejects_type_mismatch() {
    let payload: toml::Table = toml::from_str(
        r#"[openai]
type = 123
endpoint = "https://api.deepseek.com"
api_key = "secret"
"#,
    )
    .unwrap();

    let error = migrate_dream_provider_payload(&payload, &ProviderCatalog::default()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("providers.openai.type must be a string"),
        "{error}"
    );
}

#[test]
fn migrate_dream_provider_payload_rejects_unknown_keys() {
    let payload: toml::Table = toml::from_str(
        r#"[openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"
extra = "nope"
"#,
    )
    .unwrap();

    let error = migrate_dream_provider_payload(&payload, &ProviderCatalog::default()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown provider config setting: providers.openai.extra"
    );
}

//! Startup-state classification (ADR 0009 "shared bounded StateClassifier";
//! ADR 0010 config preservation; Astra findings 15-16): the bounded gate that
//! runs before the daemon binds anything must reject every non-current data
//! root and must never rewrite a config file while classifying it.

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::{RUST_META_TABLE, SUPPORTED_RUST_SCHEMA_VERSION};
use hieronymus::state_classifier::{StartupState, classify};

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("data"))
}

/// A fresh Rust database at the current schema, created the same way
/// `open_migrated` creates one on first daemon start.
fn write_current_database(config: &HieronymusConfig) {
    std::fs::create_dir_all(config.data_root()).unwrap();
    hieronymus::db::open_migrated(&config.database_path()).unwrap();
}

#[test]
fn invalid_config_is_rejected_without_rewriting_it() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let bytes = b"[not valid TOML\n";
    std::fs::write(config.dream_config_path(), bytes).unwrap();
    assert!(classify(&config).is_err());
    assert_eq!(std::fs::read(config.dream_config_path()).unwrap(), bytes);
    assert!(!config.daemon_discovery_path().exists());
}

#[test]
fn empty_data_root_classifies_as_fresh() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(classify(&config(&root)).unwrap(), StartupState::Fresh);
}

#[test]
fn current_database_and_config_classify_as_current() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    assert_eq!(classify(&config).unwrap(), StartupState::Current);
}

#[test]
fn python_schema_database_requires_migration() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    std::fs::create_dir_all(config.data_root()).unwrap();
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    for table in [
        "series",
        "task_sessions",
        "short_term_memories",
        "strict_terms",
    ] {
        connection
            .execute(
                &format!("create table {table} (id integer primary key)"),
                [],
            )
            .unwrap();
    }
    drop(connection);

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "migration_required");
    assert!(error.remediation().contains("hiero migrate"));
}

#[test]
fn newer_schema_database_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    std::fs::create_dir_all(config.data_root()).unwrap();
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute_batch(&format!(
            "create table {RUST_META_TABLE} (schema_version integer not null);
             insert into {RUST_META_TABLE} (schema_version) values ({});",
            SUPPORTED_RUST_SCHEMA_VERSION + 1
        ))
        .unwrap();
    drop(connection);

    assert!(classify(&config).is_err());
}

#[test]
fn marker_only_database_without_domain_tables_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    std::fs::create_dir_all(config.data_root()).unwrap();
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute_batch(&format!(
            "create table {RUST_META_TABLE} (schema_version integer not null unique);
             insert into {RUST_META_TABLE} (schema_version) values ({SUPPORTED_RUST_SCHEMA_VERSION});"
        ))
        .unwrap();
    connection
        .pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION)
        .unwrap();
    drop(connection);

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "invalid_startup_state");
}

#[test]
fn mixed_version_markers_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    // Meta says the supported version, user_version disagrees.
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION + 5)
        .unwrap();
    drop(connection);

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "invalid_startup_state");
}

#[test]
fn legacy_dream_workflow_names_require_config_migration() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let dream =
        "[workflows.crystallization]\nprovider = \"openai\"\nmodel = \"gpt\"\nenabled = true\n";
    std::fs::write(config.dream_config_path(), dream).unwrap();

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "config_migration_required");
    assert!(error.remediation().contains("hiero migrate"));
    // The classifier never rewrites the file it rejected.
    assert_eq!(
        std::fs::read_to_string(config.dream_config_path()).unwrap(),
        dream
    );
}

#[test]
fn current_database_with_legacy_provider_block_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let dream = "[providers.openai]\ntype = \"openai\"\nendpoint = \"https://api.openai.com\"\napi_key = \"secret\"\n";
    std::fs::write(config.dream_config_path(), dream).unwrap();

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "config_migration_required");
    assert_eq!(
        std::fs::read_to_string(config.dream_config_path()).unwrap(),
        dream
    );
}

#[test]
fn standalone_gemini_provider_type_stays_current_and_untouched() {
    // The deprecated `gemini` alias is not a structural legacy shape: it
    // canonicalizes to `google` in memory, so a current database with a
    // standalone `gemini` provider still starts and the file is untouched.
    // (A dead-end `config_migration_required` here would error out of
    // `hiero migrate`, which refuses an already-current database.)
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let provider =
        "[gemini]\ntype = \"gemini\"\nurl = \"https://generativelanguage.googleapis.com\"\n";
    std::fs::write(config.provider_config_path(), provider).unwrap();

    assert_eq!(classify(&config).unwrap(), StartupState::Current);
    assert_eq!(
        std::fs::read_to_string(config.provider_config_path()).unwrap(),
        provider
    );
}

#[test]
fn database_committed_journal_requires_config_promotion() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let journal = hieronymus::upgrade::CutoverJournal {
        journal_version: 1,
        state: "database_committed".to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-test".to_string(),
        staged: Vec::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: std::collections::BTreeMap::new(),
        updated_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        config.data_root().join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();

    let error = classify(&config).unwrap_err();
    assert_eq!(error.code(), "config_promotion_required");
    assert!(error.remediation().contains("hiero migrate"));
}

#[test]
fn complete_journal_still_classifies_as_current() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let journal = hieronymus::upgrade::CutoverJournal {
        journal_version: 1,
        state: "complete".to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-test".to_string(),
        staged: Vec::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: std::collections::BTreeMap::new(),
        updated_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        config.data_root().join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();

    assert_eq!(classify(&config).unwrap(), StartupState::Current);
}

#[test]
fn prepared_journal_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);
    let journal = hieronymus::upgrade::CutoverJournal {
        journal_version: 1,
        state: "prepared".to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-test".to_string(),
        staged: Vec::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: std::collections::BTreeMap::new(),
        updated_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        config.data_root().join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();

    assert!(classify(&config).is_err());
}

#[test]
fn ordinary_current_config_load_leaves_every_byte_untouched() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    write_current_database(&config);

    // A canonical current-format dream.conf written by the save path, plus a
    // hand-written current provider.conf: classification must not touch either.
    hieronymus::dream_config::save_dream_config(
        &config,
        &hieronymus::dream_config::default_dream_config(),
    )
    .unwrap();
    let dream_before = std::fs::read(config.dream_config_path()).unwrap();
    let provider = "[deepseek]\ntype = \"openai\"\nurl = \"https://api.deepseek.com\"\n\n[defaults]\nprovider = \"\"\nmodel = \"\"\n";
    std::fs::write(config.provider_config_path(), provider).unwrap();

    assert_eq!(classify(&config).unwrap(), StartupState::Current);
    assert_eq!(
        std::fs::read(config.dream_config_path()).unwrap(),
        dream_before
    );
    assert_eq!(
        std::fs::read_to_string(config.provider_config_path()).unwrap(),
        provider
    );
}

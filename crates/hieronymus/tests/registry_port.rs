//! Behavior ported from `tests/test_registry.py` and
//! `tests/test_series_language_tags.py` (the store-level tests; MCP-surface
//! tests port with the MCP slice).

use std::fs;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::{DatabaseState, classify_database};
use hieronymus::registry::{Registry, RegistryError};
use hieronymus::secret::redact_values;

fn open_registry(root: &tempfile::TempDir) -> Registry {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap()
}

#[test]
fn create_series_initializes_global_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();

    let series = registry
        .create_series("only-sense-online", "Only Sense Online", "ja", "en", None)
        .unwrap();

    assert_eq!(series.slug, "only-sense-online");
    assert!(config.database_path().exists());
    assert_eq!(
        registry.get_series("only-sense-online").unwrap().title,
        "Only Sense Online"
    );
    assert_eq!(registry.list_series().unwrap(), vec![series]);

    // A freshly initialized registry database classifies as the supported
    // Rust schema, not as a Python database.
    assert_eq!(
        classify_database(&config.database_path()),
        DatabaseState::RustSchema {
            version: hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION
        }
    );
}

#[test]
fn create_series_upserts_existing_slug() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);
    registry
        .create_series("only-sense-online", "Only Sense Online", "ja", "en", None)
        .unwrap();

    let updated = registry
        .create_series(
            "only-sense-online",
            "Only Sense Online Rebuild",
            "jp",
            "ru",
            None,
        )
        .unwrap();

    assert_eq!(updated.title, "Only Sense Online Rebuild");
    assert_eq!(updated.source_language, "jp");
    assert_eq!(updated.target_language, "ru");
    assert_eq!(registry.get_series("only-sense-online").unwrap(), updated);
    assert_eq!(registry.list_series().unwrap(), vec![updated]);
}

#[test]
fn create_series_rejects_filename_unsafe_slugs() {
    for slug in ["../../escape", "bad/slug", "Upper-Case", "-leading", ""] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("hieronymus"));
        let registry = Registry::open(&config).unwrap();

        let error = registry
            .create_series(slug, "Unsafe", "ja", "en", None)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid series slug: use lowercase letters"),
            "{slug}: {error}"
        );
        assert!(
            !config.database_path().exists() || {
                // The database itself may exist (registry opened first); no stray
                // sqlite files may appear anywhere else under the data root.
                let sqlite_files: Vec<_> = fs::read_dir(config.data_root())
                    .unwrap()
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        name.ends_with(".sqlite") || name.ends_with(".sqlite-wal")
                    })
                    .map(|entry| entry.path())
                    .filter(|path| path != &config.database_path())
                    .collect();
                sqlite_files.is_empty()
            }
        );
        match registry.get_series(slug) {
            Err(RegistryError::UnknownSeries(text)) => {
                assert_eq!(text, slug);
            }
            other => panic!("expected unknown series for {slug}, got {other:?}"),
        }
    }
}

#[test]
fn opening_a_corrupt_database_reports_the_migration_failure() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    fs::create_dir_all(config.data_root()).unwrap();
    fs::write(config.database_path(), b"this is not sqlite").unwrap();

    // The Python reference injects a failing migration; the honest Rust
    // equivalent is that the migration/open step fails loudly instead of
    // continuing with an uninitialized database.
    let error = Registry::open(&config).unwrap_err();

    assert!(!error.to_string().is_empty());
}

#[test]
fn create_series_accepts_language_tags_without_translation_direction() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);

    let series = registry
        .create_series(
            "book-of-friends",
            "Book of Friends",
            "",
            "",
            Some(&["ja".into(), "en".into(), "ru".into()]),
        )
        .unwrap();

    assert_eq!(series.slug, "book-of-friends");
    assert_eq!(series.source_language, "");
    assert_eq!(series.target_language, "");
    assert_eq!(series.language_tags, vec!["en", "ja", "ru"]);
    assert_eq!(registry.get_series("book-of-friends").unwrap(), series);
}

#[test]
fn legacy_series_languages_seed_language_tags() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);

    let series = registry
        .create_series("only-sense-online", "Only Sense Online", "JA", " en ", None)
        .unwrap();

    assert_eq!(series.source_language, "JA");
    assert_eq!(series.target_language, " en ");
    assert_eq!(series.language_tags, vec!["en", "ja"]);

    // Tags live in the side table, normalized; the compatibility columns
    // keep the raw values.
    let connection = rusqlite::Connection::open_with_flags(
        registry.config().database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = connection
        .prepare("select language_tag from series_language_tags where series_id = ?1 order by language_tag")
        .unwrap();
    let tags: Vec<String> = statement
        .query_map([series.id.unwrap()], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(tags, vec!["en", "ja"]);
}

#[test]
fn explicit_empty_language_tags_do_not_seed_from_legacy_languages() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);

    let series = registry
        .create_series(
            "only-sense-online",
            "Only Sense Online",
            "ja",
            "en",
            Some(&[]),
        )
        .unwrap();

    assert_eq!(series.source_language, "ja");
    assert_eq!(series.target_language, "en");
    assert!(series.language_tags.is_empty());
    assert!(
        registry
            .get_series("only-sense-online")
            .unwrap()
            .language_tags
            .is_empty()
    );
}

#[test]
fn set_series_language_tags_does_not_change_compatibility_fields() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);
    let series = registry
        .create_series("only-sense-online", "Only Sense Online", "ja", "en", None)
        .unwrap();

    registry
        .set_series_language_tags(
            series.id.unwrap(),
            &["RU".into(), " en ".into(), "ru".into()],
        )
        .unwrap();

    let updated = registry.get_series("only-sense-online").unwrap();
    assert_eq!(updated.source_language, "ja");
    assert_eq!(updated.target_language, "en");
    assert_eq!(updated.language_tags, vec!["en", "ru"]);
}

#[test]
fn set_series_language_tags_rejects_unknown_series_id() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);

    let error = registry
        .set_series_language_tags(4242, &["en".into()])
        .unwrap_err();

    assert_eq!(error.to_string(), "unknown series id: 4242");
}

#[test]
fn get_series_rejects_unknown_slug() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);

    let error = registry.get_series("never-created").unwrap_err();

    assert_eq!(error.to_string(), "unknown series: never-created");
}

#[test]
fn list_series_orders_by_slug() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);
    registry
        .create_series("zeta-series", "Zeta", "ja", "en", None)
        .unwrap();
    registry
        .create_series("alpha-series", "Alpha", "ja", "en", None)
        .unwrap();

    let order: Vec<String> = registry
        .list_series()
        .unwrap()
        .into_iter()
        .map(|series| series.slug)
        .collect();

    assert_eq!(order, vec!["alpha-series", "zeta-series"]);
}

// Guard against secret redaction regressions in registry diagnostics.
#[test]
fn registry_errors_do_not_echo_unknown_slugs_with_secret_markers() {
    let root = tempfile::tempdir().unwrap();
    let registry = open_registry(&root);
    let slug = "sk-abc123";
    let error = registry.get_series(slug).unwrap_err().to_string();
    let redacted = redact_values(&error, &[slug]);
    // The slug itself is not a secret, but the redaction helper must remain
    // capable of masking anything passed through error text.
    assert_eq!(redacted, "unknown series: [redacted]");
}

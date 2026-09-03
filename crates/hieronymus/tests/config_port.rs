//! Behavior ported from the Python suite: `tests/test_config.py`,
//! `tests/test_release_config.py`, and `tests/test_ingest_config.py`.
//! Assertion values and error messages are kept identical to the Python
//! reference; these tests are the behavioral contract for the Rust port.

use std::fs;
use std::path::Path;

use hieronymus::data_root::{HieronymusConfig, load_config};
use hieronymus::ingest_config::{
    IngestConfig, LearnLimits, ShortMemoryLimits, default_ingest_config, load_ingest_config,
    save_ingest_config,
};
use hieronymus::release_config::{
    ReleaseConfig, default_release_config, load_release_config, save_release_config,
};
use hieronymus::secret::Secret;

fn write(root: &Path, name: &str, text: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(name), text).unwrap();
}

// ---------------------------------------------------------------- data root

#[test]
fn config_exposes_single_global_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());

    assert_eq!(
        config.database_path(),
        root.path().join("hieronymus.sqlite")
    );
}

#[test]
fn load_config_uses_explicit_data_root() {
    let root = tempfile::tempdir().unwrap();
    let config = load_config(Some(root.path()));

    assert_eq!(config.data_root(), root.path());
    assert_eq!(
        config.database_path(),
        root.path().join("hieronymus.sqlite")
    );
}

#[test]
fn load_config_defaults_to_config_home_when_unset() {
    // SAFETY: tests run single-threaded per process env mutation discipline.
    unsafe { std::env::remove_var("HIERONYMUS_DATA_ROOT") };
    let config = load_config(None);

    let home = home::home_dir().unwrap();
    assert_eq!(config.data_root(), home.join(".config").join("hieronymus"));
}

#[test]
fn load_config_uses_environment_root() {
    let root = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("HIERONYMUS_DATA_ROOT", root.path()) };

    let config = load_config(None);

    assert_eq!(config.data_root(), root.path());
    unsafe { std::env::remove_var("HIERONYMUS_DATA_ROOT") };
}

#[test]
fn load_config_expands_home_shorthand() {
    unsafe { std::env::remove_var("HIERONYMUS_DATA_ROOT") };
    let config = load_config(Some(Path::new("~/hieronymus-root")));

    let home = home::home_dir().unwrap();
    assert_eq!(config.data_root(), home.join("hieronymus-root"));
}

// ----------------------------------------------------------- release config

#[test]
fn release_config_path_lives_under_config_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    assert_eq!(
        config.release_config_path(),
        config.config_root().join("release.conf")
    );
}

#[test]
fn default_release_config_uses_stable_channel() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    let release_config = load_release_config(&config).unwrap();

    assert_eq!(release_config.update_channel(), "stable");
    assert_eq!(release_config.update_target(), "latest");
    assert!(!release_config.allows_dev_updates());
}

#[test]
fn saved_default_release_config_matches_reference_defaults() {
    let release_config = default_release_config();
    assert_eq!(release_config.update_channel(), "stable");
}

#[test]
fn save_release_config_round_trips_dev_channel() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    save_release_config(&config, &ReleaseConfig::new("dev")).unwrap();

    assert_eq!(
        load_release_config(&config).unwrap().update_channel(),
        "dev"
    );
    let raw = fs::read_to_string(config.release_config_path()).unwrap();
    assert!(raw.contains("channel = \"dev\""));
}

#[test]
fn load_release_config_rejects_unknown_channel() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(
        config.config_root(),
        "release.conf",
        "[updates]\nchannel = \"nightly\"\n",
    );

    let error = load_release_config(&config).unwrap_err();

    assert!(error.to_string().contains("updates.channel"), "{error}");
}

#[test]
fn load_release_config_rejects_unknown_keys() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(
        config.config_root(),
        "release.conf",
        "[unknown]\nvalue = 1\n",
    );

    let error = load_release_config(&config).unwrap_err();

    assert_eq!(error.to_string(), "unknown release config setting: unknown");
}

#[test]
fn load_release_config_rejects_non_table_updates() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(config.config_root(), "release.conf", "updates = 1\n");

    let error = load_release_config(&config).unwrap_err();

    assert_eq!(error.to_string(), "updates must be a table");
}

#[test]
fn load_release_config_defaults_when_channel_missing() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(config.config_root(), "release.conf", "[updates]\n");

    let release_config = load_release_config(&config).unwrap();

    assert_eq!(release_config.update_channel(), "stable");
}

#[test]
fn release_config_rejects_invalid_channel_on_save() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    let error = save_release_config(&config, &ReleaseConfig::new("nightly")).unwrap_err();

    assert_eq!(
        error.to_string(),
        "updates.channel must be one of: dev, stable"
    );
}

// ------------------------------------------------------------ ingest config

#[test]
fn ingest_config_path_lives_under_config_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    assert_eq!(
        config.ingest_config_path(),
        config.config_root().join("ingest.conf")
    );
}

#[test]
fn default_ingest_config_preserves_current_behavior() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));

    let ingest_config = load_ingest_config(&config).unwrap();

    assert_eq!(ingest_config, default_ingest_config());
    assert_eq!(ingest_config.short_memory.warning_sentence_count, 6);
    assert_eq!(ingest_config.short_memory.rejection_sentence_count, 30);
    assert_eq!(ingest_config.short_memory.warning_symbol_count, 0);
    assert_eq!(ingest_config.short_memory.rejection_symbol_count, 0);
    assert_eq!(ingest_config.learn.max_block_chars, 1200);
}

#[test]
fn save_load_ingest_config_round_trips_plaintext_limits() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let ingest_config = IngestConfig::new(
        ShortMemoryLimits::new(5, 12, 1000, 3000),
        LearnLimits::new(900),
    );

    save_ingest_config(&config, &ingest_config).unwrap();

    let raw = fs::read_to_string(config.ingest_config_path()).unwrap();
    assert!(raw.contains("warning_sentence_count = 5"), "{raw}");
    assert!(raw.contains("rejection_sentence_count = 12"), "{raw}");
    assert!(raw.contains("warning_symbol_count = 1000"), "{raw}");
    assert!(raw.contains("rejection_symbol_count = 3000"), "{raw}");
    assert!(raw.contains("max_block_chars = 900"), "{raw}");
    assert_eq!(load_ingest_config(&config).unwrap(), ingest_config);
}

#[test]
fn load_ingest_config_rejects_invalid_sentence_threshold_order() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(
        config.config_root(),
        "ingest.conf",
        "[short_memory]\nwarning_sentence_count = 30\nrejection_sentence_count = 6\n",
    );

    let error = load_ingest_config(&config).unwrap_err();

    assert!(error.to_string().contains("rejection_sentence_count"));
}

#[test]
fn load_ingest_config_rejects_invalid_symbol_threshold_order() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(
        config.config_root(),
        "ingest.conf",
        "[short_memory]\nwarning_symbol_count = 100\nrejection_symbol_count = 50\n",
    );

    let error = load_ingest_config(&config).unwrap_err();

    assert!(error.to_string().contains("rejection_symbol_count"));
    assert_eq!(
        error.to_string(),
        "short_memory.rejection_symbol_count must be greater than or equal to \
         short_memory.warning_symbol_count"
    );
}

#[test]
fn load_ingest_config_rejects_unknown_keys() {
    for (raw, expected) in [
        (
            "[unknown]\nvalue = 1\n",
            "unknown ingest config setting: unknown",
        ),
        (
            "[short_memory]\nextra = 1\n",
            "unknown ingest config setting: short_memory.extra",
        ),
        (
            "[learn]\nextra = 1\n",
            "unknown ingest config setting: learn.extra",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("hieronymus"));
        write(config.config_root(), "ingest.conf", raw);

        let error = load_ingest_config(&config).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn load_ingest_config_rejects_non_integer_values() {
    for (raw, expected) in [
        (
            "[short_memory]\nwarning_sentence_count = true\n",
            "short_memory.warning_sentence_count must be an integer",
        ),
        (
            "[short_memory]\nrejection_symbol_count = 1.5\n",
            "short_memory.rejection_symbol_count must be an integer",
        ),
        (
            "[learn]\nmax_block_chars = '1200'\n",
            "learn.max_block_chars must be an integer",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("hieronymus"));
        write(config.config_root(), "ingest.conf", raw);

        let error = load_ingest_config(&config).unwrap_err();

        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn load_ingest_config_rejects_minimum_value_failures() {
    for (raw, expected) in [
        (
            "[short_memory]\nwarning_sentence_count = 0\n",
            "short_memory.warning_sentence_count must be at least 1",
        ),
        (
            "[short_memory]\nrejection_sentence_count = 0\n",
            "short_memory.rejection_sentence_count must be at least 1",
        ),
        (
            "[short_memory]\nwarning_symbol_count = -1\n",
            "short_memory.warning_symbol_count must be at least 0",
        ),
        (
            "[short_memory]\nrejection_symbol_count = -1\n",
            "short_memory.rejection_symbol_count must be at least 0",
        ),
        (
            "[learn]\nmax_block_chars = 0\n",
            "learn.max_block_chars must be at least 1",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::new(root.path().join("hieronymus"));
        write(config.config_root(), "ingest.conf", raw);

        let error = load_ingest_config(&config).unwrap_err();

        assert_eq!(error.to_string(), expected, "config: {raw}");
    }
}

#[test]
fn load_ingest_config_rejects_non_table_sections() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    write(config.config_root(), "ingest.conf", "short_memory = 1\n");

    let error = load_ingest_config(&config).unwrap_err();

    assert_eq!(error.to_string(), "short_memory must be a table");
}

// ---------------------------------------------------------------- secret<T>

#[test]
fn secret_never_leaks_value_through_debug_display_or_json() {
    let secret = Secret::new("sentinel-provider-key".to_string());

    let debug = format!("{secret:?}");
    let display = format!("{secret}");
    let json = serde_json::to_string(&secret).unwrap();

    assert!(!debug.contains("sentinel"), "{debug}");
    assert!(!display.contains("sentinel"), "{display}");
    assert!(!json.contains("sentinel"), "{json}");
    assert_eq!(secret.expose_secret(), "sentinel-provider-key");
}

#[test]
fn redact_replaces_secret_values_in_text() {
    let redacted = hieronymus::secret::redact_values("key is abc123 and abc123", &["abc123"]);

    assert_eq!(redacted, "key is [redacted] and [redacted]");
}

use std::str::FromStr;

use sqlx::{SqliteConnection, sqlite::SqliteConnectOptions};

use super::{
    DbError, Fts5Probe, Fts5ProbeError, RequiredFts5, build_pool_with_probe, config_path_options,
    connect_options, memory_settings, resolve_config_path,
};

#[test]
fn memory_url_detection_uses_sqlite_url_semantics() {
    for url in [
        "sqlite::memory:",
        "sqlite://:memory:",
        "sqlite://?mode=memory",
        "sqlite://file:shared?cache=shared&mode=memory",
    ] {
        assert!(
            memory_settings(url)
                .expect("test URL should parse")
                .is_memory,
            "{url} should be an in-memory URL"
        );
    }
    for url in [
        "sqlite://memory.sqlite",
        "sqlite://mode=memory.sqlite",
        "sqlite://file.sqlite?cache=shared",
    ] {
        assert!(
            !memory_settings(url)
                .expect("test URL should parse")
                .is_memory,
            "{url} should be a file URL"
        );
    }
}

struct MissingFts5Probe;

impl Fts5Probe for MissingFts5Probe {
    async fn check(&self, _connection: &mut SqliteConnection) -> Result<(), Fts5ProbeError> {
        Err(Fts5ProbeError::Missing)
    }
}

#[tokio::test]
async fn injected_preflight_failure_exits_pool_construction_as_missing_fts5() {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("test connection options should parse");

    let error = build_pool_with_probe(
        options,
        "injected preflight test".to_owned(),
        &MissingFts5Probe,
    )
    .await
    .expect_err("missing FTS5 should fail before constructing the pool");

    assert!(matches!(error, DbError::MissingFts5));
}

#[tokio::test]
async fn relative_file_prefix_path_is_resolved_as_a_literal_file() {
    let directory = tempfile::TempDir::new().expect("temporary directory should be created");
    let relative = std::path::Path::new("file:name?mode=memory#literal").join("hieronymus.db");
    let absolute = resolve_config_path(&relative, || Ok(directory.path().to_owned()))
        .expect("relative test path should resolve");
    std::fs::create_dir_all(
        absolute
            .parent()
            .expect("test database path should have a parent"),
    )
    .expect("literal database parent should be created");

    let pool = connect_options(
        super::configure_options(config_path_options(&absolute), false),
        "relative file-prefix test".to_owned(),
        &RequiredFts5,
    )
    .await
    .expect("relative file-prefix path should remain file-backed");
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .expect("journal mode should be readable");

    assert!(absolute.is_file());
    assert_eq!(journal_mode, "wal");
    pool.close().await;
}

#[test]
fn absolute_config_path_bypasses_failing_cwd_provider() {
    let absolute = std::env::temp_dir().join("absolute/hieronymus.db");
    assert!(absolute.is_absolute());
    let resolved = resolve_config_path(&absolute, || {
        Err(std::io::Error::other("cwd lookup must not run"))
    })
    .expect("absolute path should bypass cwd lookup");

    assert_eq!(resolved, absolute);
}

#[test]
fn relative_config_path_maps_cwd_provider_failure() {
    let error = resolve_config_path(std::path::Path::new("relative/hieronymus.db"), || {
        Err(std::io::Error::other("injected cwd failure"))
    })
    .expect_err("relative path requires cwd lookup");

    assert!(matches!(error, DbError::CurrentDirectory { .. }));
}

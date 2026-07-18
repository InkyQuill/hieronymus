use std::fs;

use hiero_core::{
    config::HieronymusConfig,
    db::{DbError, connect, connect_url, migrate},
};
use sqlx::migrate::Migrator;
use tempfile::TempDir;

fn file_url(directory: &TempDir, name: &str) -> String {
    format!("sqlite://{}", directory.path().join(name).display())
}

#[tokio::test]
async fn connect_configures_each_connection_and_uses_wal_for_files() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let pool = connect_url(&file_url(&directory, "configured.sqlite"))
        .await
        .expect("file database should connect");

    let mut connections = Vec::new();
    for _ in 0..8 {
        connections.push(
            pool.acquire()
                .await
                .expect("configured connection should be acquired"),
        );
    }

    for connection in &mut connections {
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .expect("foreign_keys should be readable");
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut **connection)
            .await
            .expect("busy_timeout should be readable");
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **connection)
            .await
            .expect("journal_mode should be readable");

        assert_eq!(foreign_keys, 1);
        assert_eq!(busy_timeout, 5_000);
        assert_eq!(journal_mode, "wal");
    }

    assert_eq!(pool.size(), 8);
    tokio::select! {
        biased;
        result = pool.acquire() => panic!("ninth acquisition unexpectedly completed: {result:?}"),
        () = tokio::task::yield_now() => {}
    }

    drop(connections.pop());
    pool.acquire()
        .await
        .expect("a released pool slot should be reusable");
}

#[tokio::test]
async fn connect_creates_the_configured_database_parent() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let data_root = directory.path().join("nested/data");
    let config =
        HieronymusConfig::load(Some(data_root.clone())).expect("explicit data root should resolve");

    let pool = connect(&config)
        .await
        .expect("configured database should connect");

    assert!(data_root.is_dir());
    assert!(config.database_path().is_file());
    pool.close().await;
}

#[tokio::test]
async fn connect_preserves_directory_creation_failure_context() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let blocking_file = directory.path().join("not-a-directory");
    fs::write(&blocking_file, b"file").expect("blocking file should be created");
    let config = HieronymusConfig::load(Some(blocking_file.clone()))
        .expect("explicit data root should resolve without filesystem access");

    let error = connect(&config)
        .await
        .expect_err("a regular file cannot contain the database");

    match error {
        DbError::CreateDirectory { path, source } => {
            assert_eq!(path, blocking_file);
            assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
        }
        other => panic!("expected a directory creation error, got {other:?}"),
    }
}

#[tokio::test]
async fn memory_urls_do_not_request_wal() {
    for url in [
        "sqlite::memory:",
        "sqlite://:memory:",
        "sqlite://?mode=memory",
        "sqlite://file:shared-memory?mode=memory&cache=shared",
    ] {
        let pool = connect_url(url)
            .await
            .unwrap_or_else(|error| panic!("{url} should connect: {error}"));
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .expect("journal_mode should be readable");

        assert_eq!(journal_mode, "memory", "unexpected mode for {url}");
        pool.close().await;
    }
}

#[tokio::test]
async fn embedded_migrations_are_idempotent() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let pool = connect_url(&file_url(&directory, "migrations.sqlite"))
        .await
        .expect("database should connect");

    migrate(&pool)
        .await
        .expect("first migration run should pass");
    migrate(&pool)
        .await
        .expect("second migration run should pass");

    let migration_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await
    .expect("migration metadata should be readable");
    assert_eq!(migration_table, 1);
}

#[tokio::test]
async fn failed_sqlx_migration_rolls_back_its_partial_changes() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let migrations = directory.path().join("test-migrations");
    fs::create_dir(&migrations).expect("migration fixture directory should be created");
    fs::write(
        migrations.join("0001_broken.sql"),
        "CREATE TABLE doomed (id INTEGER PRIMARY KEY);\n\
         INSERT INTO doomed DEFAULT VALUES;\n\
         THIS IS NOT SQL;",
    )
    .expect("broken migration fixture should be written");
    let migrator = Migrator::new(migrations.as_path())
        .await
        .expect("migration fixture should resolve");
    let pool = connect_url(&file_url(&directory, "rollback.sqlite"))
        .await
        .expect("database should connect");

    migrator
        .run(&pool)
        .await
        .expect_err("broken migration should fail");

    let doomed_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'doomed'",
    )
    .fetch_one(&pool)
    .await
    .expect("schema should remain readable after rollback");
    assert_eq!(doomed_table, 0);
}

#[test]
fn missing_fts5_error_is_actionable_and_distinct() {
    let error = DbError::MissingFts5;

    assert!(error.to_string().contains("FTS5"));
    assert!(error.to_string().contains("SQLite"));
}

#[tokio::test]
async fn invalid_urls_return_typed_errors() {
    let error = connect_url("postgres://not-sqlite")
        .await
        .expect_err("a non-SQLite URL should fail");

    assert!(matches!(error, DbError::InvalidUrl { .. }));
}

use std::str::FromStr;

use hiero_core::db::{DbError, migrate};
use sqlx::{
    AssertSqlSafe, Executor, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

const SQLX_SCHEMA: &str = r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
"#;

async fn pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("metadata fixture pool should connect")
}

async fn schema_snapshot(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
    sqlx::query(
        "SELECT type, name, tbl_name, coalesce(sql, '') AS sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("schema should inspect")
    .into_iter()
    .map(|row| {
        (
            row.get("type"),
            row.get("name"),
            row.get("tbl_name"),
            row.get("sql"),
        )
    })
    .collect()
}

async fn assert_invalid_metadata(pool: &SqlitePool) {
    let before = schema_snapshot(pool).await;
    let error = migrate(pool)
        .await
        .expect_err("malformed SQLx metadata must be rejected");
    assert!(
        matches!(error, DbError::InvalidMigrationMetadata { .. }),
        "unexpected error: {error}"
    );
    assert_eq!(schema_snapshot(pool).await, before);
}

#[tokio::test]
async fn empty_malformed_sqlx_table_is_rejected_before_migrator() {
    let pool = pool().await;
    sqlx::query("CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("malformed metadata table should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn lookalike_sqlx_table_without_primary_key_is_rejected() {
    let pool = pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        CREATE TABLE _sqlx_migrations (
            version BIGINT,
            description TEXT NOT NULL,
            installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL,
            execution_time BIGINT NOT NULL
        );
        "#,
    ))
    .await
    .expect("lookalike metadata table should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn sqlx_table_with_wrong_default_is_rejected() {
    let pool = pool().await;
    pool.execute(sqlx::raw_sql(AssertSqlSafe(
        SQLX_SCHEMA.replace("DEFAULT CURRENT_TIMESTAMP", "DEFAULT (datetime('now'))"),
    )))
    .await
    .expect("wrong-default metadata table should install");
    assert_invalid_metadata(&pool).await;
}

async fn migrated_pool() -> SqlitePool {
    let pool = pool().await;
    migrate(&pool).await.expect("fresh schema should migrate");
    pool
}

#[tokio::test]
async fn dirty_sqlx_history_is_rejected_with_actionable_error() {
    let pool = migrated_pool().await;
    sqlx::query("UPDATE _sqlx_migrations SET success = 0 WHERE version = 4")
        .execute(&pool)
        .await
        .expect("dirty history should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn checksum_mismatch_is_rejected_with_actionable_error() {
    let pool = migrated_pool().await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 3")
        .execute(&pool)
        .await
        .expect("checksum mismatch should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn unknown_future_version_is_rejected_with_actionable_error() {
    let pool = migrated_pool().await;
    sqlx::query("UPDATE _sqlx_migrations SET version = 999 WHERE version = 4")
        .execute(&pool)
        .await
        .expect("future version should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn non_prefix_history_gap_is_rejected_with_actionable_error() {
    let pool = migrated_pool().await;
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 3")
        .execute(&pool)
        .await
        .expect("history gap should install");
    assert_invalid_metadata(&pool).await;
}

#[tokio::test]
async fn every_current_embedded_history_prefix_is_valid() {
    let source = migrated_pool().await;
    let history: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT version, description, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&source)
    .await
    .expect("current history should read");
    assert_eq!(
        history.iter().map(|row| row.0).collect::<Vec<_>>(),
        [1, 3, 4]
    );

    for prefix_len in 0..=history.len() {
        let pool = pool().await;
        pool.execute(sqlx::raw_sql(SQLX_SCHEMA))
            .await
            .expect("exact metadata schema should install");
        for (version, description, checksum) in &history[..prefix_len] {
            let sql = match version {
                1 => include_str!("../../../migrations/0001_initial_schema.sql"),
                3 => include_str!("../../../migrations/0003_compound_indexes.sql"),
                4 => include_str!("../../../migrations/0004_semantic_index_state.sql"),
                _ => panic!("unexpected embedded migration {version}"),
            };
            pool.execute(sqlx::raw_sql(sql))
                .await
                .expect("prefix migration should install");
            sqlx::query("INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time) VALUES (?, ?, 1, ?, 0)")
                .bind(version)
                .bind(description)
                .bind(checksum)
                .execute(&pool)
                .await
                .expect("prefix metadata should insert");
        }
        migrate(&pool)
            .await
            .unwrap_or_else(|error| panic!("prefix length {prefix_len} rejected: {error}"));
    }
}

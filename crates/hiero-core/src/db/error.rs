use std::path::PathBuf;

use sqlx::migrate::MigrateError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    #[error("invalid SQLite URL `{url}`: {reason}")]
    InvalidUrl { url: String, reason: String },

    #[error("failed to create database directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to resolve the current directory for a relative database path: {source}")]
    CurrentDirectory {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to connect to SQLite database `{url}`: {source}")]
    Connect {
        url: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("SQLite was built without required FTS5 support; install an FTS5-enabled SQLite build")]
    MissingFts5,

    #[error("failed to verify required SQLite FTS5 support: {source}")]
    Fts5Probe {
        #[source]
        source: sqlx::Error,
    },

    #[error("failed to apply embedded SQLite migrations: {source}")]
    Migration {
        #[source]
        source: MigrateError,
    },

    #[error("failed to inspect legacy SQLite schema: {source}")]
    LegacyInspection {
        #[source]
        source: sqlx::Error,
    },

    #[error("invalid SQLx migration metadata: {reason}")]
    InvalidMigrationMetadata { reason: String },

    #[error("unsupported legacy SQLite schema: {reason}")]
    UnsupportedLegacySchema { reason: String },

    #[error(
        "legacy timestamp `{table}.{column}` row {rowid} is not RFC 3339 UTC or SQLite UTC text: `{value}`"
    )]
    LegacyTimestamp {
        table: String,
        column: String,
        rowid: i64,
        value: String,
    },

    #[error("failed to baseline legacy SQLite schema: {source}")]
    LegacyUpgrade {
        #[source]
        source: sqlx::Error,
    },

    #[error("failed to convert legacy strict terms: {source}")]
    LegacyTermConversion {
        #[source]
        source: sqlx::Error,
    },

    #[error("legacy strict term {source_id} has invalid {field}: {reason}")]
    InvalidLegacyTerm {
        source_id: String,
        field: &'static str,
        reason: String,
    },

    #[error("legacy strict term {source_id} conflicts with crystal {target_id}: {reason}")]
    LegacyTermConflict {
        source_id: String,
        target_id: i64,
        reason: String,
    },

    #[error(
        "legacy strict-term conversion parity mismatch: source={source_rows}, converted={converted_rows}, existing={existing_rows}, ledger={ledger_rows}"
    )]
    LegacyTermCountMismatch {
        source_rows: i64,
        converted_rows: i64,
        existing_rows: i64,
        ledger_rows: i64,
    },

    #[error("failed to roll back legacy strict-term conversion after `{original}`: {source}")]
    LegacyTermRollback {
        original: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("failed to restore foreign-key enforcement after legacy migration: {reason}")]
    ForeignKeyRestore { reason: String },
}

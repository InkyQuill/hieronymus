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
}

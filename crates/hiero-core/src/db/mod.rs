mod error;

#[cfg(test)]
mod test_support;

use std::{str::FromStr, time::Duration};

pub use error::DbError;
use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::config::HieronymusConfig;

const MISSING_FTS5_PROTOCOL_ERROR: &str = "hieronymus: SQLite FTS5 support is unavailable";

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(config: &HieronymusConfig) -> Result<SqlitePool, DbError> {
    let database_path = config.database_path();
    let parent = database_path
        .parent()
        .expect("the configured database path always has a data-root parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| DbError::CreateDirectory {
            path: parent.to_owned(),
            source,
        })?;

    connect_url(&format!("sqlite://{}", database_path.display())).await
}

pub async fn connect_url(url: &str) -> Result<SqlitePool, DbError> {
    if !url.starts_with("sqlite:") {
        return Err(DbError::InvalidUrl {
            url: url.to_owned(),
            reason: "expected a URL beginning with `sqlite:`".to_owned(),
        });
    }

    let is_memory = is_memory_url(url);
    let mut options = SqliteConnectOptions::from_str(url)
        .map_err(|source| DbError::InvalidUrl {
            url: url.to_owned(),
            reason: source.to_string(),
        })?
        .create_if_missing(!is_memory)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    if !is_memory {
        options = options.journal_mode(SqliteJournalMode::Wal);
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                match sqlx::query(
                    "CREATE VIRTUAL TABLE temp.__hieronymus_fts5_probe USING fts5(probe)",
                )
                .execute(&mut *connection)
                .await
                {
                    Ok(_) => {
                        sqlx::query("DROP TABLE temp.__hieronymus_fts5_probe")
                            .execute(&mut *connection)
                            .await?;
                        Ok(())
                    }
                    Err(error) if error.to_string().contains("no such module: fts5") => Err(
                        sqlx::Error::Protocol(MISSING_FTS5_PROTOCOL_ERROR.to_owned()),
                    ),
                    Err(error) => Err(error),
                }
            })
        })
        .connect_with(options)
        .await
        .map_err(|source| map_connect_error(url, source))?;

    migrate(&pool).await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), DbError> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|source| DbError::Migration { source })
}

fn is_memory_url(url: &str) -> bool {
    let without_scheme = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let (database, query) = without_scheme
        .split_once('?')
        .map_or((without_scheme, ""), |parts| parts);

    database == ":memory:"
        || query
            .split('&')
            .filter_map(|parameter| parameter.split_once('='))
            .any(|(key, value)| key == "mode" && value == "memory")
}

fn map_connect_error(url: &str, source: sqlx::Error) -> DbError {
    if matches!(&source, sqlx::Error::Protocol(message) if message == MISSING_FTS5_PROTOCOL_ERROR) {
        DbError::MissingFts5
    } else {
        DbError::Connect {
            url: url.to_owned(),
            source,
        }
    }
}

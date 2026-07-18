mod error;

#[cfg(test)]
mod test_support;

use std::{path::Path, str::FromStr, time::Duration};

pub use error::DbError;
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, SqliteConnection, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use url::form_urlencoded;
use uuid::Uuid;

use crate::config::HieronymusConfig;

const MISSING_FTS5_PROTOCOL_ERROR: &str = "hieronymus: SQLite FTS5 support is unavailable";

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(config: &HieronymusConfig) -> Result<SqlitePool, DbError> {
    let configured_path = config.database_path();
    let current_directory =
        std::env::current_dir().map_err(|source| DbError::CurrentDirectory { source })?;
    let database_path = absolute_config_path(&configured_path, &current_directory);
    let parent = database_path
        .parent()
        .expect("the configured database path always has a data-root parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| DbError::CreateDirectory {
            path: parent.to_owned(),
            source,
        })?;

    let options = configure_options(config_path_options(&database_path), false);
    connect_options(options, path_target(&database_path), &RequiredFts5).await
}

pub async fn connect_url(url: &str) -> Result<SqlitePool, DbError> {
    if !url.starts_with("sqlite:") {
        return Err(invalid_url(url, "expected a URL beginning with `sqlite:`"));
    }

    let options = SqliteConnectOptions::from_str(url).map_err(|source| DbError::InvalidUrl {
        url: url.to_owned(),
        reason: source.to_string(),
    })?;
    let memory = memory_settings(url)?;
    if memory.is_memory && !memory.shared_cache {
        return Err(invalid_url(
            url,
            "private-cache in-memory databases cannot safely back an eight-connection pool; use `cache=shared`",
        ));
    }

    let mut options = configure_options(options, memory.is_memory);
    if memory.is_memory {
        // SQLx's URL-generated in-memory filename is not guaranteed to share
        // schema state across concurrently opened SQLite handles. Normalize
        // every accepted form to one unique pool-local named memory database.
        options = options
            .filename(memory_filename(&memory))
            .in_memory(false)
            .shared_cache(true);
    }
    connect_options(options, url.to_owned(), &RequiredFts5).await
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), DbError> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|source| DbError::Migration { source })
}

async fn connect_options(
    options: SqliteConnectOptions,
    target: String,
    probe: &impl Fts5Probe,
) -> Result<SqlitePool, DbError> {
    let pool = build_pool_with_probe(options, target, probe).await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn build_pool_with_probe(
    options: SqliteConnectOptions,
    target: String,
    probe: &impl Fts5Probe,
) -> Result<SqlitePool, DbError> {
    // Preflight before pool construction makes a missing capability a direct,
    // typed startup error instead of letting pool retries hide it as a timeout.
    let mut preflight = SqliteConnection::connect_with(&options)
        .await
        .map_err(|source| connect_error(&target, source))?;
    probe.check(&mut preflight).await.map_err(probe_error)?;
    preflight
        .close()
        .await
        .map_err(|source| connect_error(&target, source))?;

    SqlitePoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move { probe_fts5(connection).await.map_err(probe_error_for_pool) })
        })
        .connect_with(options)
        .await
        .map_err(|source| connect_error(&target, source))
}

trait Fts5Probe {
    async fn check(&self, connection: &mut SqliteConnection) -> Result<(), Fts5ProbeError>;
}

struct RequiredFts5;

impl Fts5Probe for RequiredFts5 {
    async fn check(&self, connection: &mut SqliteConnection) -> Result<(), Fts5ProbeError> {
        probe_fts5(connection).await
    }
}

#[derive(Debug)]
enum Fts5ProbeError {
    Missing,
    Query(sqlx::Error),
}

async fn probe_fts5(connection: &mut SqliteConnection) -> Result<(), Fts5ProbeError> {
    match sqlx::query("CREATE VIRTUAL TABLE temp.__hieronymus_fts5_probe USING fts5(probe)")
        .execute(&mut *connection)
        .await
    {
        Ok(_) => {
            sqlx::query("DROP TABLE temp.__hieronymus_fts5_probe")
                .execute(&mut *connection)
                .await
                .map_err(Fts5ProbeError::Query)?;
            Ok(())
        }
        Err(error) if error.to_string().contains("no such module: fts5") => {
            Err(Fts5ProbeError::Missing)
        }
        Err(error) => Err(Fts5ProbeError::Query(error)),
    }
}

fn probe_error(error: Fts5ProbeError) -> DbError {
    match error {
        Fts5ProbeError::Missing => DbError::MissingFts5,
        Fts5ProbeError::Query(source) => DbError::Fts5Probe { source },
    }
}

fn probe_error_for_pool(error: Fts5ProbeError) -> sqlx::Error {
    match error {
        Fts5ProbeError::Missing => sqlx::Error::Protocol(MISSING_FTS5_PROTOCOL_ERROR.to_owned()),
        Fts5ProbeError::Query(source) => source,
    }
}

fn configure_options(mut options: SqliteConnectOptions, is_memory: bool) -> SqliteConnectOptions {
    options = options
        .create_if_missing(!is_memory)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    if is_memory {
        options
    } else {
        options.journal_mode(SqliteJournalMode::Wal)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MemorySettings {
    is_memory: bool,
    shared_cache: bool,
    canonical_name: Option<String>,
}

fn memory_settings(url: &str) -> Result<MemorySettings, DbError> {
    let without_scheme = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let (database, query) = without_scheme
        .split_once('?')
        .map_or((without_scheme, ""), |parts| parts);
    let decoded_database = percent_encoding::percent_decode_str(database)
        .decode_utf8()
        .map_err(|source| DbError::InvalidUrl {
            url: url.to_owned(),
            reason: source.to_string(),
        })?;
    let is_anonymous_name = decoded_database.is_empty() || decoded_database == ":memory:";
    let starts_in_memory = database == ":memory:";
    let mut settings = MemorySettings {
        is_memory: starts_in_memory,
        shared_cache: starts_in_memory,
        canonical_name: None,
    };

    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match (key.as_ref(), value.as_ref()) {
            ("mode", "memory") => {
                settings.is_memory = true;
                // This matches SQLx: mode=memory enables shared cache at the
                // point where it appears, so later parameters may override it.
                settings.shared_cache = true;
            }
            ("cache", "private") => settings.shared_cache = false,
            ("cache", "shared") => settings.shared_cache = true,
            _ => {}
        }
    }
    if settings.is_memory && !is_anonymous_name {
        settings.canonical_name = Some(decoded_database.into_owned());
    }
    Ok(settings)
}

fn memory_filename(settings: &MemorySettings) -> String {
    let identity = settings.canonical_name.as_ref().map_or_else(
        || format!("anonymous-{}", Uuid::new_v4()),
        |name| {
            let digest = Sha256::digest(name.as_bytes());
            format!("named-{digest:x}")
        },
    );
    format!("file:hieronymus-memory-{identity}?mode=memory&cache=shared")
}

fn invalid_url(url: &str, reason: &str) -> DbError {
    DbError::InvalidUrl {
        url: url.to_owned(),
        reason: reason.to_owned(),
    }
}

fn connect_error(target: &str, source: sqlx::Error) -> DbError {
    if is_missing_fts5_error(&source) {
        DbError::MissingFts5
    } else {
        DbError::Connect {
            url: target.to_owned(),
            source,
        }
    }
}

fn is_missing_fts5_error(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Protocol(message) if message == MISSING_FTS5_PROTOCOL_ERROR)
}

fn path_target(path: &Path) -> String {
    format!("database path `{}`", path.to_string_lossy())
}

fn config_path_options(path: &Path) -> SqliteConnectOptions {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        if path.to_str().is_none() {
            let encoded = percent_encoding::percent_encode(
                path.as_os_str().as_bytes(),
                percent_encoding::NON_ALPHANUMERIC,
            );
            return SqliteConnectOptions::new().filename(format!("file:{encoded}"));
        }
    }

    SqliteConnectOptions::new().filename(path)
}

fn absolute_config_path(path: &Path, current_directory: &Path) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        current_directory.join(path)
    }
}

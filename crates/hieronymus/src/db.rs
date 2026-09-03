use std::path::Path;

use rusqlite::OpenFlags;

/// The ported global schema (Python `migrations/global.sql`, verbatim). The
/// Rust line creates fresh databases at this schema plus its own
/// `hieronymus_meta` version marker; the compatibility import boundary
/// (ADR 0010) owns converting Python databases to it.
const GLOBAL_MIGRATION_SQL: &str = include_str!("../migrations/global.sql");
const TERMINOLOGY_MIGRATION_SQL: &str = include_str!("../migrations/terminology.sql");

/// Supported Rust schema version created by this line. Bumped only by an
/// accepted schema-upgrade decision; a database written by a newer binary
/// fails closed.
pub const SUPPORTED_RUST_SCHEMA_VERSION: i64 = 1;

const RUST_META_TABLE: &str = "hieronymus_meta";
/// Sentinel tables that identify a Python-era Hieronymus database. Python has
/// no schema-version marker; the ported migration table set is the fingerprint
/// (see `tests/test_config.py::test_global_migration_creates_memory_dreaming_schema`).
const PYTHON_SENTINEL_TABLES: [&str; 4] = [
    "series",
    "task_sessions",
    "short_term_memories",
    "strict_terms",
];

/// Read-only classification of a data root's SQLite database, per the
/// database-upgrade spec: empty, supported Python schema, supported or newer
/// Rust schema, unknown, or corrupt. Classification never mutates the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseState {
    /// No database file, or a file with no tables at all.
    Empty,
    /// A Python-era database identified by its table fingerprint.
    PythonSchema,
    /// A Rust database at the supported schema version.
    RustSchema { version: i64 },
    /// A Rust database written by a newer binary; fails closed.
    NewerSchema { version: i64 },
    /// Tables exist but match no known schema.
    Unknown,
    /// The file exists but cannot be opened as a SQLite database.
    Corrupt,
}

impl DatabaseState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DatabaseState::Empty => "empty",
            DatabaseState::PythonSchema => "python-schema",
            DatabaseState::RustSchema { .. } => "rust-schema",
            DatabaseState::NewerSchema { .. } => "newer-schema",
            DatabaseState::Unknown => "unknown",
            DatabaseState::Corrupt => "corrupt",
        }
    }

    pub fn schema_version(&self) -> Option<i64> {
        match self {
            DatabaseState::RustSchema { version } | DatabaseState::NewerSchema { version } => {
                Some(*version)
            }
            _ => None,
        }
    }
}

/// Open the database with the Python-compatible pragmas and apply the Rust
/// schema when it is not yet marked as migrated. Fresh databases are created
/// directly at the current Rust schema version (ADR 0010). An existing Python
/// or unsupported database is never auto-migrated here: the caller must run
/// the explicit upgrade path from the database-upgrade spec.
pub fn open_migrated(path: &Path) -> Result<rusqlite::Connection, OpenMigratedError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(OpenMigratedError::Io)?;
    }
    let connection = rusqlite::Connection::open(path)?;
    connection.execute_batch(
        "pragma foreign_keys = on;
         pragma journal_mode = wal;",
    )?;
    let state = classify_database(path);
    match state {
        DatabaseState::Empty => {
            connection.execute_batch(GLOBAL_MIGRATION_SQL)?;
            connection.execute_batch(TERMINOLOGY_MIGRATION_SQL)?;
            connection.execute_batch(&format!(
                "create table if not exists {RUST_META_TABLE} (
                     schema_version integer not null unique
                 );
                 insert or ignore into {RUST_META_TABLE} (schema_version)
                 values ({SUPPORTED_RUST_SCHEMA_VERSION});"
            ))?;
            connection.pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION)?;
            Ok(connection)
        }
        DatabaseState::RustSchema { version } if version == SUPPORTED_RUST_SCHEMA_VERSION => {
            Ok(connection)
        }
        other => Err(OpenMigratedError::UnsupportedState(other)),
    }
}

/// Errors from [`open_migrated`]: filesystem, SQLite, or an unsupported
/// database state that must not be opened for writes.
#[derive(Debug, thiserror::Error)]
pub enum OpenMigratedError {
    #[error("database could not be opened: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database requires an explicit upgrade path; refusing to open for writes: {0:?}")]
    UnsupportedState(DatabaseState),
}

/// Classify the database at `path` without writing to it.
pub fn classify_database(path: &Path) -> DatabaseState {
    if !path.exists() {
        return DatabaseState::Empty;
    }
    let connection = match rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => return DatabaseState::Corrupt,
    };
    let tables = match list_tables(&connection) {
        Ok(tables) => tables,
        Err(_) => return DatabaseState::Corrupt,
    };
    if tables.is_empty() {
        return DatabaseState::Empty;
    }
    if tables.contains(&RUST_META_TABLE.to_string()) {
        let version = read_rust_schema_version(&connection);
        return match version {
            Ok(version) if version <= SUPPORTED_RUST_SCHEMA_VERSION => {
                DatabaseState::RustSchema { version }
            }
            Ok(version) => DatabaseState::NewerSchema { version },
            Err(()) => DatabaseState::Unknown,
        };
    }
    if PYTHON_SENTINEL_TABLES
        .iter()
        .all(|table| tables.contains(&table.to_string()))
    {
        return DatabaseState::PythonSchema;
    }
    DatabaseState::Unknown
}

fn list_tables(connection: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "select name from sqlite_master where type = 'table' and name not like 'sqlite_%'",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(row?);
    }
    Ok(tables)
}

fn read_rust_schema_version(connection: &rusqlite::Connection) -> Result<i64, ()> {
    // The meta table layout is owned by this workspace; reading it on a
    // database that happens to contain a foreign `hieronymus_meta` without
    // the expected column classifies as Unknown rather than panicking.
    let mut statement = connection
        .prepare(&format!(
            "select schema_version from {RUST_META_TABLE} order by schema_version desc limit 1"
        ))
        .map_err(|_| ())?;
    let version = statement
        .query_row([], |row| row.get::<_, i64>(0))
        .map_err(|_| ())?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(name);
        (root, path)
    }

    #[test]
    fn missing_file_is_empty() {
        let (_root, path) = temp_db("missing.sqlite");
        assert_eq!(classify_database(&path), DatabaseState::Empty);
    }

    #[test]
    fn garbage_file_is_corrupt() {
        let (_root, path) = temp_db("garbage.sqlite");
        std::fs::write(&path, b"not a database at all, just text").unwrap();
        assert_eq!(classify_database(&path), DatabaseState::Corrupt);
    }

    #[test]
    fn zero_table_database_is_empty() {
        let (_root, path) = temp_db("empty.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        // Opening and closing without statements leaves a valid, table-less file.
        connection.execute("pragma user_version = 0", []).unwrap();
        drop(connection);
        assert_eq!(classify_database(&path), DatabaseState::Empty);
    }

    #[test]
    fn python_sentinel_tables_classify_as_python_schema() {
        let (_root, path) = temp_db("python.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        for table in PYTHON_SENTINEL_TABLES {
            connection
                .execute(
                    &format!("create table {table} (id integer primary key)"),
                    [],
                )
                .unwrap();
        }
        drop(connection);
        assert_eq!(classify_database(&path), DatabaseState::PythonSchema);
    }

    #[test]
    fn meta_table_with_supported_version_classifies_as_rust_schema() {
        let (_root, path) = temp_db("rust.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "create table {RUST_META_TABLE} (schema_version integer not null);
                 insert into {RUST_META_TABLE} (schema_version) values ({SUPPORTED_RUST_SCHEMA_VERSION});"
            ))
            .unwrap();
        drop(connection);
        assert_eq!(
            classify_database(&path),
            DatabaseState::RustSchema {
                version: SUPPORTED_RUST_SCHEMA_VERSION
            }
        );
    }

    #[test]
    fn meta_table_with_newer_version_fails_closed() {
        let (_root, path) = temp_db("newer.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "create table {RUST_META_TABLE} (schema_version integer not null);
                 insert into {RUST_META_TABLE} (schema_version) values ({});",
                SUPPORTED_RUST_SCHEMA_VERSION + 1
            ))
            .unwrap();
        drop(connection);
        assert_eq!(
            classify_database(&path),
            DatabaseState::NewerSchema {
                version: SUPPORTED_RUST_SCHEMA_VERSION + 1
            }
        );
    }

    #[test]
    fn foreign_tables_classify_as_unknown() {
        let (_root, path) = temp_db("foreign.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute("create table something_else (id integer primary key)", [])
            .unwrap();
        drop(connection);
        assert_eq!(classify_database(&path), DatabaseState::Unknown);
    }
}

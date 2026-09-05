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

/// The Rust schema-version marker table. Its presence is the primary signal
/// that a database was written by this line rather than Python.
pub const RUST_META_TABLE: &str = "hieronymus_meta";

/// Mandatory domain tables that a database at the current Rust schema must
/// carry. A `hieronymus_meta` marker without these is a partial or forged
/// schema and fails closed (ADR 0009: newer/unknown/partial state never
/// starts). This is a bounded `sqlite_master` check, never `integrity_check`
/// or a data scan; it names the load-bearing tables from every subsystem
/// (`migrations/global.sql` + `migrations/terminology.sql`), not all of them.
const RUST_MANDATORY_TABLES: [&str; 15] = [
    "series",
    "task_sessions",
    "short_term_memories",
    "crystals",
    "memory_events",
    "dream_runs",
    "strict_terms",
    "concepts",
    "concept_facets",
    "audit_log",
    "rag_sources",
    "rag_chunks",
    "memory_graph_migration_ledger",
    "term_rules",
    "term_rule_forms",
];
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
    let mut connection = rusqlite::Connection::open(path)?;
    connection.execute_batch(
        "pragma foreign_keys = on;
         pragma journal_mode = wal;",
    )?;
    let state = classify_database(path);
    match state {
        DatabaseState::Empty => {
            create_fresh_schema(&mut connection)?;
            Ok(connection)
        }
        DatabaseState::RustSchema { version } if version == SUPPORTED_RUST_SCHEMA_VERSION => {
            Ok(connection)
        }
        other => Err(OpenMigratedError::UnsupportedState(other)),
    }
}

/// Build the current Rust schema on an empty database in a single transaction,
/// publishing the `hieronymus_meta` version marker last. A failure at any step
/// rolls the whole thing back, so an interrupted fresh create never leaves a
/// half-built database that would later classify as a usable Rust schema
/// (ADR 0010).
fn create_fresh_schema(connection: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(GLOBAL_MIGRATION_SQL)?;
    transaction.execute_batch(TERMINOLOGY_MIGRATION_SQL)?;
    transaction.pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION)?;
    // Marker last: every domain table exists before the version is claimed.
    transaction.execute_batch(&format!(
        "create table if not exists {RUST_META_TABLE} (
             schema_version integer not null unique
         );
         insert or ignore into {RUST_META_TABLE} (schema_version)
         values ({SUPPORTED_RUST_SCHEMA_VERSION});"
    ))?;
    transaction.commit()
}

/// Why a database that carries the current Rust schema-version marker is not
/// actually a usable current schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDefect {
    /// Mandatory domain tables are absent (a marker-only or partial database).
    MissingTables(Vec<String>),
    /// A mandatory table is missing mandatory columns.
    MissingColumns {
        table: &'static str,
        columns: Vec<String>,
    },
    /// `PRAGMA user_version` and the `hieronymus_meta` marker disagree.
    InconsistentVersionMarkers {
        user_version: i64,
        meta_version: i64,
    },
    /// The database could not be read to complete the check.
    Unreadable,
}

impl std::fmt::Display for SchemaDefect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaDefect::MissingTables(tables) => {
                write!(formatter, "missing mandatory tables: {}", tables.join(", "))
            }
            SchemaDefect::MissingColumns { table, columns } => write!(
                formatter,
                "table {table} is missing mandatory columns: {}",
                columns.join(", ")
            ),
            SchemaDefect::InconsistentVersionMarkers {
                user_version,
                meta_version,
            } => write!(
                formatter,
                "version markers disagree: user_version {user_version}, hieronymus_meta {meta_version}"
            ),
            SchemaDefect::Unreadable => write!(formatter, "database could not be read"),
        }
    }
}

/// Verify a database that already classified as [`DatabaseState::RustSchema`]
/// at the supported version actually carries the current mandatory
/// tables/columns and consistent version markers. Bounded and read-only: uses
/// `sqlite_master` and `PRAGMA table_info`/`user_version` only — never
/// `integrity_check`, foreign-key scans, or row reads (ADR 0009).
pub fn verify_current_rust_schema(path: &Path) -> Result<(), SchemaDefect> {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| SchemaDefect::Unreadable)?;

    let tables = list_tables(&connection).map_err(|_| SchemaDefect::Unreadable)?;
    let missing: Vec<String> = RUST_MANDATORY_TABLES
        .iter()
        .filter(|table| !tables.contains(&table.to_string()))
        .map(|table| table.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(SchemaDefect::MissingTables(missing));
    }

    // A representative column check on the Rust-owned terminology table and
    // the marker table; the full column contract is the upgrade path's job.
    // TODO(schema-v2): widen the mandatory-column check when R3 lands in-place
    // ALTERs — a partially applied column add on another mandatory table
    // currently still classifies as Current.
    for (table, required) in [
        (
            "term_rules",
            &["id", "status", "source_text", "canonical_translation"][..],
        ),
        (
            "term_rule_forms",
            &["id", "rule_id", "form_kind", "surface"][..],
        ),
        (RUST_META_TABLE, &["schema_version"][..]),
    ] {
        let present = table_columns(&connection, table).map_err(|_| SchemaDefect::Unreadable)?;
        let missing: Vec<String> = required
            .iter()
            .filter(|column| !present.contains(&column.to_string()))
            .map(|column| column.to_string())
            .collect();
        if !missing.is_empty() {
            return Err(SchemaDefect::MissingColumns {
                table,
                columns: missing,
            });
        }
    }

    let meta_version =
        read_rust_schema_version(&connection).map_err(|()| SchemaDefect::Unreadable)?;
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| SchemaDefect::Unreadable)?;
    if user_version != meta_version {
        return Err(SchemaDefect::InconsistentVersionMarkers {
            user_version,
            meta_version,
        });
    }

    Ok(())
}

fn table_columns(connection: &rusqlite::Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("select name from pragma_table_info(?1)")?;
    let rows = statement.query_map([table], |row| row.get::<_, String>(0))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row?);
    }
    Ok(columns)
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

/// Apply the target-schema SQL steps of an upgrade to `connection`: the
/// terminology rule tables plus the schema-version metadata. Idempotent, and
/// safe inside the caller's transaction — the upgrade protocol owns the
/// commit. This is the same statement set a fresh Rust database receives.
pub(crate) fn apply_terminology_schema_steps(
    connection: &rusqlite::Connection,
) -> rusqlite::Result<()> {
    connection.execute_batch(TERMINOLOGY_MIGRATION_SQL)?;
    connection.execute_batch(&format!(
        "create table if not exists {RUST_META_TABLE} (
             schema_version integer not null unique
         );
         insert or ignore into {RUST_META_TABLE} (schema_version)
         values ({SUPPORTED_RUST_SCHEMA_VERSION});"
    ))?;
    connection.pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION)
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
    fn fresh_schema_is_built_in_one_transaction_marker_last() {
        let (_root, path) = temp_db("fresh-tx.sqlite");
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("pragma foreign_keys = on; pragma journal_mode = wal;")
            .unwrap();

        // A rolled-back fresh create leaves neither the marker nor any table.
        {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch(GLOBAL_MIGRATION_SQL).unwrap();
            transaction
                .execute_batch(TERMINOLOGY_MIGRATION_SQL)
                .unwrap();
            transaction
                .execute_batch(&format!(
                    "create table {RUST_META_TABLE} (schema_version integer not null unique);
                     insert into {RUST_META_TABLE} (schema_version) values \
                     ({SUPPORTED_RUST_SCHEMA_VERSION});"
                ))
                .unwrap();
            // Drop without commit.
        }
        assert_eq!(classify_database(&path), DatabaseState::Empty);

        // The real path commits and passes the deeper schema verification.
        create_fresh_schema(&mut connection).unwrap();
        assert_eq!(
            classify_database(&path),
            DatabaseState::RustSchema {
                version: SUPPORTED_RUST_SCHEMA_VERSION
            }
        );
        drop(connection);
        assert_eq!(verify_current_rust_schema(&path), Ok(()));
    }

    #[test]
    fn marker_only_database_is_missing_mandatory_tables() {
        let (_root, path) = temp_db("marker-only.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "create table {RUST_META_TABLE} (schema_version integer not null unique);
                 insert into {RUST_META_TABLE} (schema_version) values \
                 ({SUPPORTED_RUST_SCHEMA_VERSION});"
            ))
            .unwrap();
        connection
            .pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION)
            .unwrap();
        drop(connection);

        assert!(matches!(
            verify_current_rust_schema(&path),
            Err(SchemaDefect::MissingTables(_))
        ));
    }

    #[test]
    fn inconsistent_version_markers_are_a_defect() {
        let (_root, path) = temp_db("mixed-markers.sqlite");
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch("pragma foreign_keys = on; pragma journal_mode = wal;")
            .unwrap();
        create_fresh_schema(&mut connection).unwrap();
        connection
            .pragma_update(None, "user_version", SUPPORTED_RUST_SCHEMA_VERSION + 3)
            .unwrap();
        drop(connection);

        assert!(matches!(
            verify_current_rust_schema(&path),
            Err(SchemaDefect::InconsistentVersionMarkers { .. })
        ));
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

//! `hiero export` (plan M5): a read-only JSON serialization of the user's
//! memory content at an explicit destination. Export NEVER copies or touches
//! the live SQLite file: it opens a read-only connection (safe next to a
//! running daemon, WAL readers never write) and serializes rows to one JSON
//! document, so the destination holds plain data, not a database that could
//! corrupt or diverge.
//!
//! This is a *content export*, not a backup: it is not restorable, and it
//! deliberately omits the operational state a complete backup would need.
//! The exported tables are the documented content set — series, sessions,
//! short-term memories, crystals, concepts and facets, terminology rules,
//! RAG sources and chunks, and dream runs. Operational/audit tables
//! (`audit_log`, cutover journals, tag side tables) are intentionally not
//! part of the content export. `hiero migrate`'s backups own the restorable
//! copies.
//!
//! The output is deterministic: the same database state always produces the
//! same bytes (fixed table order, `ORDER BY rowid`, sorted JSON keys). All
//! table queries run inside ONE deferred read transaction, so the document is
//! a single committed snapshot even while the daemon writes — never a row set
//! stitched together from several different moments.
//!
//! The write side is guarded twice, because a read-only diagnostic that can
//! destroy the memory it reads is worse than no diagnostic at all (finding
//! A1). Read-only SQLite flags protect the *connection*, not the destination
//! *path*, so before anything is created the destination guard refuses
//! every authoritative or runtime file of the data root — the database and its
//! WAL/SHM/journal sidecars, the configuration files, the daemon discovery
//! record and bearer token, the ownership and dream-cycle locks, the cutover
//! journal, and the managed asset trees (semantic, backups, agent plugins,
//! normalized RAG sources) — including symlink and hard-link aliases to any
//! of them. Publication then goes through a synced sibling
//! temporary file and an atomic no-clobber rename, so an export never
//! truncates, half-writes, or silently replaces a file that already exists.
//! Replacing a previous export is possible but never implicit: it is a second
//! entry point ([`run_overwriting`], the CLI's `--force`) that relaxes only
//! the no-clobber rule — the destination guard and the atomic publish are
//! identical, so `--force` can replace the user's own export and still cannot
//! touch the user's memory.

use crate::platform::export::{Destination, is_link, same_file};
use std::path::{Component, Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_locks::dream_cycle_paths;
use hieronymus::ownership::OWNER_LOCK_FILE;
use hieronymus::rag::RAG_NORMALIZED_DIR;
use hieronymus::semantic_arming::semantic_config_path;
use hieronymus::upgrade::JOURNAL_FILE;

/// The documented export format marker.
pub const EXPORT_FORMAT: &str = "hieronymus-export-v1";

/// The exported content tables, in output order.
pub const EXPORTED_TABLES: [&str; 10] = [
    "series",
    "task_sessions",
    "short_term_memories",
    "crystals",
    "concepts",
    "concept_facets",
    "term_rules",
    "rag_sources",
    "rag_chunks",
    "dream_runs",
];

/// What one export run produced: the destination and per-table row counts.
#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub format: &'static str,
    pub output: std::path::PathBuf,
    /// Table name → exported row count, in [`EXPORTED_TABLES`] order.
    pub tables: Vec<(String, usize)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("cannot open the database read-only: {0}")]
    Open(#[from] rusqlite::Error),
    #[error("cannot write {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    /// The destination is (or aliases) a file the installation owns. Writing
    /// there would destroy live state, so the export never begins.
    #[error(
        "refusing to export to {}: that path is, or aliases, a file this installation owns \
         (the database and its sidecars, configuration, daemon token/discovery, locks, or a \
         managed asset tree); choose a destination outside them",
        .0.display()
    )]
    UnsafeDestination(std::path::PathBuf),
}

/// How an export may treat a destination that already exists. Replacing a
/// file the user already has is a decision only the user can make, so it is a
/// separate entry point rather than a flag threaded through `run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Publication {
    /// Publish only if nothing is there. The default.
    NewFileOnly,
    /// Atomically replace whatever is there (`hiero export --force`).
    Overwrite,
}

/// Export the content tables to `output` as one pretty JSON document.
///
/// `output` must name a NEW file: an existing destination is refused rather
/// than overwritten, so re-exporting is a deliberate act — see
/// [`run_overwriting`].
pub fn run(config: &HieronymusConfig, output: &Path) -> Result<ExportReport, ExportError> {
    export(config, output, Publication::NewFileOnly)
}

/// Export as [`run`] does, but atomically REPLACE an existing `output`.
///
/// This is the deliberate overwrite the CLI's `--force` asks for, and it
/// relaxes exactly one rule: the destination may already exist. Every safety
/// property survives — the same destination guard runs first (a protected or
/// aliased path is refused no matter what the caller intends), and the publish
/// is still a synced temporary file renamed into place, never a truncation of
/// the file the user still has.
pub fn run_overwriting(
    config: &HieronymusConfig,
    output: &Path,
) -> Result<ExportReport, ExportError> {
    export(config, output, Publication::Overwrite)
}

fn export(
    config: &HieronymusConfig,
    output: &Path,
    publication: Publication,
) -> Result<ExportReport, ExportError> {
    // Validate before opening anything, creating any parent directory, or
    // writing one byte: a refused destination must leave the filesystem
    // exactly as it was. This runs for `Overwrite` too — an explicit
    // overwrite is permission to replace the user's own export, never
    // permission to replace the user's memory.
    let checked_output = ensure_safe_destination(config, output)?;
    let destination = Destination::open(&checked_output).map_err(|source| ExportError::Write {
        path: output.to_path_buf(),
        source,
    })?;
    let connection = Connection::open_with_flags(
        config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut tables = Vec::new();
    let mut document = serde_json::Map::new();
    document.insert(
        "format".to_string(),
        Value::String(EXPORT_FORMAT.to_string()),
    );
    let table_rows = serde_json::Map::new();
    document.insert("tables".to_string(), Value::Object(table_rows));
    // One deferred read transaction spans every table query. Without it each
    // `select` would run in its own implicit transaction and could observe a
    // different committed moment, so a concurrent daemon write could land a
    // `task_sessions` row in the export whose `series` row is missing.
    connection.execute_batch("BEGIN DEFERRED")?;
    for table in EXPORTED_TABLES {
        let rows = export_table(&connection, table)?;
        tables.push((table.to_string(), rows.len()));
        document
            .get_mut("tables")
            .and_then(Value::as_object_mut)
            .expect("tables is an object")
            .insert(table.to_string(), Value::Array(rows));
    }
    // Release the snapshot before serialization: the read transaction covers
    // the queries only, never the JSON encoding or the write. (An error above
    // drops the connection instead, which rolls the read transaction back.)
    connection.execute_batch("COMMIT")?;
    drop(connection);

    let mut text = serde_json::to_string_pretty(&Value::Object(document)).map_err(|error| {
        ExportError::Write {
            path: output.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        }
    })?;
    text.push('\n');
    destination
        .publish(&text, publication)
        .map_err(|source| ExportError::Write {
            path: output.to_path_buf(),
            source,
        })?;
    Ok(ExportReport {
        format: EXPORT_FORMAT,
        output: output.to_path_buf(),
        tables,
    })
}

/// Refuse destinations the installation owns. Read-only SQLite flags protect
/// the connection, never the path, so this is the only thing standing between
/// `hiero export --output ~/.config/hieronymus/hieronymus.sqlite` and a
/// destroyed memory (finding A1).
///
/// Three independent checks, because no single one covers every alias:
/// symlink components are refused outright, lexical *and* canonical path
/// comparison catches protected names (including ones that do not exist yet,
/// like a WAL sidecar), and device/inode comparison catches hard links, which
/// no amount of path arithmetic can see.
///
/// One deliberate limit: hard-link detection covers the enumerated
/// [`protected_files`] only, never files *inside* [`protected_trees`], since
/// walking those trees on every export is impractical. Directories cannot be
/// hard-linked, so the trees themselves stay protected by path; a hard link to
/// an individual file under `backups/` would not be recognized.
fn ensure_safe_destination(
    config: &HieronymusConfig,
    output: &Path,
) -> Result<PathBuf, ExportError> {
    let refuse = || ExportError::UnsafeDestination(output.to_path_buf());
    let candidate = lexical_absolute(output).map_err(|source| ExportError::Write {
        path: output.to_path_buf(),
        source,
    })?;

    // 1. Never write through a symlink. The destination itself may be an alias
    //    to a protected file, and a symlinked ancestor makes every path
    //    comparison below meaningless — refuse both rather than try to reason
    //    about where they point.
    let raw_absolute = std::path::absolute(output).map_err(|source| ExportError::Write {
        path: output.to_path_buf(),
        source,
    })?;
    for ancestor in raw_absolute.ancestors() {
        if is_link(ancestor) {
            return Err(refuse());
        }
    }

    let files = protected_files(config);
    let trees = protected_trees(config);

    // 2. Path identity, lexically and then canonically. The canonical pass
    //    matters when the data root sits behind a symlinked ancestor of its
    //    own (`/var` → `/private/var`, a symlinked home): the caller can then
    //    name a protected file by a spelling the lexical pass does not match.
    let resolved = resolved_target(&candidate);
    for file in &files {
        if lexical_absolute(file).is_ok_and(|protected| protected == candidate) {
            return Err(refuse());
        }
        if let (Some(resolved), Some(protected)) = (&resolved, resolved_target(file))
            && *resolved == protected
        {
            return Err(refuse());
        }
    }
    for tree in &trees {
        if lexical_absolute(tree).is_ok_and(|protected| candidate.starts_with(protected)) {
            return Err(refuse());
        }
        if let (Some(resolved), Some(protected)) = (&resolved, resolved_target(tree))
            && resolved.starts_with(protected)
        {
            return Err(refuse());
        }
    }

    // 3. Inode identity. A hard link to the database is a different path with
    //    the same bytes: writing through it destroys the database just as
    //    surely, and only device+inode reveals it.
    for file in &files {
        match same_file(&candidate, file) {
            Ok(true) => return Err(refuse()),
            Ok(false) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ExportError::Write {
                    path: output.to_path_buf(),
                    source,
                });
            }
        }
    }
    Ok(candidate)
}

/// Every individual file the installation owns. Sidecars and locks are listed
/// even when absent: the WAL and journal appear and disappear with the
/// daemon's writes, and a destination that would collide with one later is
/// refused now.
fn protected_files(config: &HieronymusConfig) -> Vec<PathBuf> {
    let database = config.database_path();
    let sidecar = |suffix: &str| {
        let mut name = database.clone().into_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    let dream = dream_cycle_paths(config);
    vec![
        database.clone(),
        sidecar("-wal"),
        sidecar("-shm"),
        sidecar("-journal"),
        config.dream_config_path(),
        config.provider_config_path(),
        config.ingest_config_path(),
        config.release_config_path(),
        semantic_config_path(config),
        config.llm_cache_path(),
        config.daemon_discovery_path(),
        config.daemon_token_path(),
        config.data_root().join(OWNER_LOCK_FILE),
        config.data_root().join(JOURNAL_FILE),
        dream.lock_file,
        dream.state_json,
        config.dream_autostart_path(),
    ]
}

/// Managed asset trees: everything inside them belongs to the installation
/// (acquired embedding models and the semantic index, migration backups,
/// generated agent plugins, normalized RAG intermediates), so the whole
/// subtree is off limits. The data root itself is NOT protected wholesale —
/// an export beside the runtime state is a reasonable thing to ask for.
fn protected_trees(config: &HieronymusConfig) -> Vec<PathBuf> {
    vec![
        config.semantic_root(),
        config.backups_root(),
        config.agent_plugins_root(),
        config.data_root().join(RAG_NORMALIZED_DIR),
    ]
}

/// Make `path` absolute and resolve `.`/`..` textually. Purely lexical, so it
/// works for paths that do not exist yet; it is sound only alongside the
/// symlink-component refusal above, which is why the two always run together.
fn lexical_absolute(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// The real location `path` names: its own canonical path when it exists, and
/// otherwise its canonical parent plus its file name. `None` when not even the
/// parent exists, in which case the path cannot alias anything on disk.
fn resolved_target(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let parent = std::fs::canonicalize(path.parent()?).ok()?;
    Some(parent.join(path.file_name()?))
}

/// Serialize one table: every column of every row, stable `rowid` order.
/// Values map as INTEGER → number, REAL → number, TEXT → string, BLOB →
/// lowercase hex, NULL → null.
fn export_table(connection: &Connection, table: &str) -> Result<Vec<Value>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        // Table names come from the fixed EXPORTED_TABLES list, never from
        // user input, so the interpolation is static.
        "select * from {table} order by rowid"
    ))?;
    let column_names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut rows = statement.query([])?;
    let mut exported = Vec::new();
    while let Some(row) = rows.next()? {
        let mut object = serde_json::Map::new();
        for (index, name) in column_names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(integer) => Value::from(integer),
                rusqlite::types::ValueRef::Real(real) => Value::from(real),
                rusqlite::types::ValueRef::Text(text) => {
                    Value::from(String::from_utf8_lossy(text).into_owned())
                }
                rusqlite::types::ValueRef::Blob(blob) => Value::from(
                    blob.iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>(),
                ),
            };
            object.insert(name.clone(), value);
        }
        exported.push(Value::Object(object));
    }
    Ok(exported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hieronymus::registry::Registry;

    #[test]
    fn export_is_deterministic_readonly_json() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let config = HieronymusConfig::new(root.path());
        Registry::open(&config)
            .unwrap()
            .create_series("book", "Book", "ja", "en", None)
            .unwrap();

        let output = root.path().join("export").join("memory.json");
        let report = run(&config, &output).unwrap();
        assert_eq!(report.format, EXPORT_FORMAT);
        let count = |name: &str| {
            report
                .tables
                .iter()
                .find(|(table, _)| table == name)
                .unwrap()
                .1
        };
        assert_eq!(count("series"), 1);

        // Determinism is checked against a second, fresh destination: an
        // export never overwrites, so re-running onto the same path is a
        // refusal rather than a rewrite.
        let first = std::fs::read_to_string(&output).unwrap();
        let again = root.path().join("export").join("memory-again.json");
        run(&config, &again).unwrap();
        let second = std::fs::read_to_string(&again).unwrap();
        assert_eq!(first, second, "export must be deterministic");
        assert!(
            run(&config, &output).is_err(),
            "an existing destination is never overwritten"
        );
        let document: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(document["format"], EXPORT_FORMAT);
        assert_eq!(document["tables"]["series"][0]["slug"], "book");

        // The export is plain data, not a SQLite file.
        assert!(!first.starts_with("SQLite format 3"));
    }
}

#[cfg(all(test, unix))]
mod publication_tests {
    use super::*;

    #[test]
    fn publication_stays_in_opened_directory_after_ancestor_swap() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let outside = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let original = root.path().join("exports");
        std::fs::create_dir(&original).unwrap();
        let destination = Destination::open(&original.join("memory.json")).unwrap();
        let moved = root.path().join("moved");
        std::fs::rename(&original, &moved).unwrap();
        std::os::unix::fs::symlink(outside.path(), &original).unwrap();
        std::fs::write(outside.path().join("memory.json"), "protected").unwrap();
        destination
            .publish("export", Publication::Overwrite)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(moved.join("memory.json")).unwrap(),
            "export"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("memory.json")).unwrap(),
            "protected"
        );
    }
}

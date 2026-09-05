//! `hiero export` (plan M5): a read-only JSON serialization of the user's
//! memory content at an explicit destination. Export NEVER copies or touches
//! the live SQLite file: it opens a read-only connection (safe next to a
//! running daemon, WAL readers never write) and serializes rows to one JSON
//! document, so the destination holds plain data, not a database that could
//! corrupt or diverge.
//!
//! The output is deterministic: the same database state always produces the
//! same bytes (fixed table order, `ORDER BY rowid`, sorted JSON keys). The
//! exported tables are the documented content set — series, sessions,
//! short-term memories, crystals, concepts and facets, terminology rules,
//! RAG sources and chunks, and dream runs. Operational/audit tables
//! (`audit_log`, cutover journals, tag side tables) are intentionally not
//! part of the content export.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;

use hieronymus::data_root::HieronymusConfig;

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
}

/// Export the content tables to `output` as one pretty JSON document.
pub fn run(config: &HieronymusConfig, output: &Path) -> Result<ExportReport, ExportError> {
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
    for table in EXPORTED_TABLES {
        let rows = export_table(&connection, table)?;
        tables.push((table.to_string(), rows.len()));
        document
            .get_mut("tables")
            .and_then(Value::as_object_mut)
            .expect("tables is an object")
            .insert(table.to_string(), Value::Array(rows));
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(document)).map_err(|error| {
        ExportError::Write {
            path: output.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        }
    })?;
    text.push('\n');
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| ExportError::Write {
            path: output.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(output, text).map_err(|source| ExportError::Write {
        path: output.to_path_buf(),
        source,
    })?;
    Ok(ExportReport {
        format: EXPORT_FORMAT,
        output: output.to_path_buf(),
        tables,
    })
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
        let root = tempfile::tempdir().unwrap();
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

        let first = std::fs::read_to_string(&output).unwrap();
        let second = {
            let _ = run(&config, &output);
            std::fs::read_to_string(&output).unwrap()
        };
        assert_eq!(first, second, "export must be deterministic");
        let document: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(document["format"], EXPORT_FORMAT);
        assert_eq!(document["tables"]["series"][0]["slug"], "book");

        // The export is plain data, not a SQLite file.
        assert!(!first.starts_with("SQLite format 3"));
    }
}

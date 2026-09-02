//! Neutral probe import from a supported frozen fixture.
//!
//! The probe opens the source strictly read-only, reads every row of the
//! enumerated domain tables with typed reads, canonicalizes each row in
//! memory, and stores only its digest and field count in a disposable
//! neutral target. Every source row gets exactly one ledger outcome. It
//! never touches the production target rule schema or the cutover protocol.

use crate::classify::{
    DatabaseContract, classify_with_contract, open_read_only, quote_ident, validate_source,
    validate_target,
};
use crate::report::{
    FtsProbeResult, LedgerCounts, ProbeReceipt, hash_directory, sha256_file, sha256_hex,
};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Transaction};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The enumerated probe domains: series; sessions; strict terms, aliases and
/// tags; concepts and facets; memories, crystals and links; RAG sources,
/// chunks, tags and scopes; events and audit; and the memory-graph migration
/// ledger. Together they are exactly the projected content tables — every
/// non-FTS table of the current schema.
pub const DOMAIN_TABLES: [&str; 40] = [
    "audit_log",
    "concept_facet_language_tags",
    "concept_facet_semantic_tags",
    "concept_facet_story_scopes",
    "concept_facets",
    "concept_renames",
    "concept_semantic_tags",
    "concepts",
    "crystal_activations",
    "crystal_concepts",
    "crystal_language_tags",
    "crystal_links",
    "crystal_semantic_tags",
    "crystal_sources",
    "crystal_story_scopes",
    "crystals",
    "dream_audit_entries",
    "dream_phase_runs",
    "dream_runs",
    "memory_events",
    "memory_graph_migration_ledger",
    "rag_chunk_language_tags",
    "rag_chunk_semantic_tags",
    "rag_chunk_story_scopes",
    "rag_chunks",
    "rag_sources",
    "series",
    "series_language_tags",
    "short_term_memories",
    "short_term_memory_language_tags",
    "short_term_memory_semantic_tags",
    "short_term_memory_story_scopes",
    "strict_concept_proposals",
    "strict_term_aliases",
    "strict_term_tags",
    "strict_terms",
    "task_session_language_tags",
    "task_session_semantic_tags",
    "task_session_story_scopes",
    "task_sessions",
];

/// Integer columns documented as booleans; canonicalized as true/false.
const BOOL_COLUMNS: [&str; 4] = ["is_canonical", "is_inferred", "applied", "case_sensitive"];

/// One exact FTS probe per searchable domain: (fts table, term, projection
/// representative-row table, projection representative-row column). The term
/// is validated against the projected representative row before querying.
const FTS_PROBES: [(&str, &str, &str, &str); 5] = [
    ("strict_terms_fts", "センス", "strict_terms", "source_text"),
    (
        "short_term_memories_fts",
        "Council",
        "short_term_memories",
        "text",
    ),
    ("concepts_fts", "センス", "concepts", "canonical_name"),
    ("crystals_fts", "Чутьё", "crystals", "text"),
    ("rag_chunks_fts", "archivist", "rag_chunks", "text"),
];

/// The neutral probe schema, verbatim from the task contract.
const NEUTRAL_SCHEMA: &str = "
create table probe_rows (
    source_table text not null,
    source_id text not null,
    payload_sha256 text not null,
    field_count integer not null,
    primary key (source_table, source_id)
);
create table probe_ledger (
    source_table text not null,
    source_id text not null,
    outcome text not null check (outcome in ('read', 'skipped', 'blocking')),
    reason_code text not null,
    primary key (source_table, source_id)
);
";

/// Structured refusal for non-convertible fixtures; the probe fails closed
/// before the target is created.
#[derive(Debug)]
pub struct ProbeRefusal {
    pub classification: String,
    pub error_code: String,
}

impl std::fmt::Display for ProbeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "probe refused ({}) for classification {}",
            self.error_code, self.classification
        )
    }
}

impl std::error::Error for ProbeRefusal {}

fn refusal_code(classification: &str) -> &'static str {
    match classification {
        "corrupt" => "source-unreadable",
        "empty" => "empty-database",
        "partial-python" => "partial-schema",
        "unknown-schema" => "unknown-schema",
        _ => "unsafe-classification",
    }
}

/// Run the neutral probe import. Only `work_root` (already validated as a
/// descendant of the repo probe work root) receives writes.
pub fn probe_import(
    source: &Path,
    target: &Path,
    fixture_root: &Path,
    work_root: &Path,
    expected: &DatabaseContract,
) -> Result<ProbeReceipt> {
    let name = validate_source(source, fixture_root)?;
    let target_path = validate_target(target, work_root)?;
    let directory_before = hash_directory(fixture_root)?;
    let source_before = sha256_file(source)?;

    let classification = classify_with_contract(source, fixture_root, expected)?;
    ensure!(
        sha256_file(source)? == source_before,
        "source fixture {name} changed bytes during classification"
    );

    if !classification.safe_to_convert {
        return Err(ProbeRefusal {
            classification: classification.name.clone(),
            error_code: refusal_code(&classification.name).to_string(),
        }
        .into());
    }

    let deep = classification.name == "supported-python";
    if deep {
        let expected_name = expected.fixture.rsplit('/').next().unwrap_or_default();
        ensure!(
            name == expected_name,
            "full-schema checks apply to {}, not {name}",
            expected.fixture
        );
    }

    let source_conn = open_read_only(source)?;
    let mut target_conn = Connection::open_with_flags(
        &target_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("create probe target {}", target_path.display()))?;
    target_conn
        .execute_batch(NEUTRAL_SCHEMA)
        .context("create the neutral probe schema")?;

    let present = present_domain_tables(&source_conn)?;
    if deep {
        ensure!(
            present.len() == DOMAIN_TABLES.len(),
            "the full schema must expose every probe domain table"
        );
    }

    let mut rows_per_table = BTreeMap::new();
    {
        let tx = target_conn
            .transaction()
            .context("begin the probe transaction")?;
        for table in &present {
            let rows = probe_table(&source_conn, &tx, table)?;
            rows_per_table.insert(table.clone(), rows);
        }
        tx.commit().context("commit the probe transaction")?;
    }

    let probe_row_count: i64 =
        target_conn.query_row("select count(*) from probe_rows", [], |row| row.get(0))?;
    let ledger_row_count: i64 =
        target_conn.query_row("select count(*) from probe_ledger", [], |row| row.get(0))?;
    let ledger = ledger_counts(&target_conn)?;
    let total: u64 = rows_per_table.values().sum();
    ensure!(
        probe_row_count as u64 == total && ledger_row_count as u64 == total,
        "every source row must produce exactly one probe row and one ledger outcome"
    );
    ensure!(
        ledger.read == total && ledger.skipped == 0 && ledger.blocking == 0,
        "every source row must be read cleanly (read={}, skipped={}, blocking={})",
        ledger.read,
        ledger.skipped,
        ledger.blocking
    );

    let mut row_counts = BTreeMap::new();
    for table in all_table_names(&source_conn)? {
        let count: i64 = source_conn.query_row(
            &format!("select count(*) from {}", quote_ident(&table)),
            [],
            |row| row.get(0),
        )?;
        row_counts.insert(table, count);
    }
    if deep {
        let mismatches: Vec<String> = row_counts
            .iter()
            .filter(|(table, count)| expected.row_counts.get(*table) != Some(*count))
            .map(|(table, count)| {
                format!(
                    "{table}={count} (projected {})",
                    expected.row_counts.get(table).unwrap_or(&-1)
                )
            })
            .collect();
        ensure!(
            row_counts.len() == expected.row_counts.len() && mismatches.is_empty(),
            "row counts contradict the verified projection: {mismatches:?}"
        );
    }

    let migration_sources_digest =
        sha256_hex(serde_json::to_string(&expected.migration_sources)?.as_bytes());

    let mut representative_row_digests = BTreeMap::new();
    let mut fts = BTreeMap::new();
    if deep {
        for (table, projected_rows) in &expected.representative_rows {
            let observed = read_rows(&source_conn, table)?;
            let (matches, digest) = compare_rows(observed, projected_rows)
                .with_context(|| format!("compare representative rows for {table}"))?;
            ensure!(
                matches,
                "representative rows for {table} contradict the verified projection"
            );
            representative_row_digests.insert(table.clone(), digest);
        }
        for (ledger_table, projection) in &expected.application_migration_ledgers {
            let columns = schema_column_names(&source_conn, ledger_table)?;
            ensure!(
                columns == projection.columns,
                "application ledger {ledger_table} columns contradict the projection"
            );
            let observed = read_rows(&source_conn, ledger_table)?;
            let (matches, _) = compare_rows(observed, &projection.rows)?;
            ensure!(
                matches,
                "application ledger {ledger_table} rows contradict the projection"
            );
        }
        fts = run_fts_probes(&source_conn, expected)?;
    } else {
        for ledger_table in expected.application_migration_ledgers.keys() {
            ensure!(
                !present.contains(ledger_table),
                "a legacy source must not contain the applied {ledger_table}"
            );
        }
    }

    drop(source_conn);
    drop(target_conn);

    let source_bytes_identical = sha256_file(source)? == source_before;
    ensure!(
        source_bytes_identical,
        "source fixture {name} changed bytes during the probe"
    );
    let fixture_directory_identical = hash_directory(fixture_root)? == directory_before;
    ensure!(
        fixture_directory_identical,
        "the fixture directory changed during the probe"
    );

    let work_dir = target_path
        .parent()
        .context("the target must have a parent directory")?
        .to_path_buf();
    let mut siblings: Vec<String> = fs::read_dir(&work_dir)
        .with_context(|| format!("list work root {}", work_dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    siblings.sort();
    let expected_sibling = target_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    ensure!(
        siblings == vec![expected_sibling],
        "the work root must contain only the probe target"
    );

    Ok(ProbeReceipt {
        ok: true,
        source_name: classification.source_name,
        classification: classification.name,
        safe_to_convert: classification.safe_to_convert,
        source_bytes_identical,
        fixture_directory_identical,
        source_sha256: source_before,
        schema_digest: classification.schema_digest,
        migration_sources_digest,
        probe_row_count: probe_row_count as u64,
        ledger,
        rows_per_table,
        row_counts,
        representative_row_digests,
        fts,
        target_sha256: sha256_file(&target_path)?,
        error_code: None,
    })
}

fn present_domain_tables(source: &Connection) -> Result<Vec<String>> {
    let mut names = BTreeMap::new();
    let mut stmt = source.prepare("select name from sqlite_master where type = 'table'")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        names.insert(row.get::<_, String>(0)?, ());
    }
    let mut present = Vec::new();
    for table in DOMAIN_TABLES {
        if names.contains_key(table) {
            present.push(table.to_string());
        }
    }
    Ok(present)
}

/// Every table of the source, ordered by name.
fn all_table_names(source: &Connection) -> Result<Vec<String>> {
    let mut stmt =
        source.prepare("select name from sqlite_master where type = 'table' order by name")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn probe_table(source: &Connection, tx: &Transaction, table: &str) -> Result<u64> {
    let primary_key = primary_key_columns(source, table)?;
    let rowid_backed = primary_key.is_empty();
    let select = if rowid_backed {
        format!(
            "select rowid as __probe_rowid, * from {} order by rowid",
            quote_ident(table)
        )
    } else {
        format!("select * from {} order by rowid", quote_ident(table))
    };
    let mut stmt = source.prepare(&select)?;
    let column_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let schema_columns: Vec<String> = column_names
        .iter()
        .filter(|column| column.as_str() != "__probe_rowid")
        .cloned()
        .collect();
    let mut rows = stmt.query([])?;
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        let mut cells = Map::new();
        for (index, column) in column_names.iter().enumerate() {
            if column == "__probe_rowid" {
                continue;
            }
            let value = canonical_cell(column, row.get_ref(index)?)?;
            cells.insert(column.clone(), value);
        }
        let source_id = if rowid_backed {
            row.get_ref(0)?.as_i64()?.to_string()
        } else {
            primary_key
                .iter()
                .map(|column| match cells.get(column) {
                    Some(Value::Number(number)) => Ok(number.to_string()),
                    Some(Value::Bool(flag)) => Ok((*flag as u8).to_string()),
                    Some(Value::String(text)) => Ok(text.clone()),
                    other => bail!(
                        "primary key column {column} of {table} must be textual or integral, got {other:?}"
                    ),
                })
                .collect::<Result<Vec<_>>>()?
                .join(":")
        };
        let payload = Value::Object(cells);
        let digest = sha256_hex(serde_json::to_string(&payload)?.as_bytes());
        let field_count = schema_columns.len() as i64;
        tx.execute(
            "insert into probe_rows (source_table, source_id, payload_sha256, field_count) values (?1, ?2, ?3, ?4)",
            rusqlite::params![table, source_id, digest, field_count],
        )?;
        tx.execute(
            "insert into probe_ledger (source_table, source_id, outcome, reason_code) values (?1, ?2, 'read', 'ok')",
            rusqlite::params![table, source_id],
        )?;
        count += 1;
    }
    Ok(count)
}

fn primary_key_columns(source: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = source.prepare(&format!("pragma table_info({})", quote_ident(table)))?;
    let mut rows = stmt.query([])?;
    let mut columns: Vec<(i64, String)> = Vec::new();
    while let Some(row) = rows.next()? {
        let ordinal: i64 = row.get("pk")?;
        if ordinal > 0 {
            columns.push((ordinal, row.get("name")?));
        }
    }
    columns.sort_by_key(|(ordinal, _)| *ordinal);
    Ok(columns.into_iter().map(|(_, name)| name).collect())
}

fn schema_column_names(source: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = source.prepare(&format!("pragma table_info({})", quote_ident(table)))?;
    let mut rows = stmt.query([])?;
    let mut names = Vec::new();
    while let Some(row) = rows.next()? {
        names.push(row.get("name")?);
    }
    Ok(names)
}

/// REAL values are canonicalized at 9 significant digits. The projection was
/// generated from Python float reprs while SQLite stores IEEE doubles, so
/// last-ulp drift (for example 0.9500000000000001 vs 0.95) must be absorbed;
/// integers, booleans, text, and blobs still compare exactly.
fn round_real(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    const DIGITS: i32 = 9;
    let magnitude = value.abs().log10().floor() as i32;
    let factor = 10f64.powi(DIGITS - 1 - magnitude);
    (value * factor).round() / factor
}

/// Canonicalize one typed cell. Integers, reals, text, blobs, booleans, JSON
/// columns, and documented timestamps all map to a deterministic JSON value.
fn canonical_cell(column: &str, value: ValueRef<'_>) -> Result<Value> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(raw) => Ok(if BOOL_COLUMNS.contains(&column) {
            Value::Bool(raw != 0)
        } else {
            Value::from(raw)
        }),
        ValueRef::Real(raw) => Ok(Value::Number(
            serde_json::Number::from_f64(round_real(raw))
                .with_context(|| format!("column {column} holds a non-finite real"))?,
        )),
        ValueRef::Text(raw) => {
            let text = std::str::from_utf8(raw)
                .with_context(|| format!("column {column} holds invalid utf-8 text"))?;
            if column.ends_with("_json") {
                Ok(serde_json::from_str(text)
                    .with_context(|| format!("column {column} holds invalid JSON"))?)
            } else {
                Ok(Value::String(text.to_string()))
            }
        }
        ValueRef::Blob(raw) => Ok(Value::String(format!(
            "blob:{}",
            crate::report::hex_encode(raw)
        ))),
    }
}

/// Read every row of a table as canonical JSON objects, in rowid order.
fn read_rows(source: &Connection, table: &str) -> Result<Vec<Value>> {
    let mut stmt = source.prepare(&format!(
        "select * from {} order by rowid",
        quote_ident(table)
    ))?;
    let column_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut rows = stmt.query([])?;
    let mut observed = Vec::new();
    while let Some(row) = rows.next()? {
        let mut cells = Map::new();
        for (index, column) in column_names.iter().enumerate() {
            let value = canonical_cell(column, row.get_ref(index)?)?;
            cells.insert(column.clone(), value);
        }
        observed.push(Value::Object(cells));
    }
    Ok(observed)
}

fn normalize_projected_cell(key: &str, value: &Value) -> Result<Value> {
    Ok(match value {
        Value::String(text) if key.ends_with("_json") => serde_json::from_str(text)
            .with_context(|| format!("projected JSON column {key} must hold valid JSON"))?,
        Value::Number(number) if BOOL_COLUMNS.contains(&key) => {
            Value::Bool(number.as_i64().unwrap_or_default() != 0)
        }
        Value::Number(number) if number.is_f64() => {
            let rounded = round_real(number.as_f64().unwrap_or_default());
            Value::Number(
                serde_json::Number::from_f64(rounded).context("projected real must be finite")?,
            )
        }
        other => other.clone(),
    })
}

fn normalize_projected_row(row: &Value) -> Result<Value> {
    let object = row.as_object().context("projected rows must be objects")?;
    let mut normalized = Map::new();
    for (key, value) in object {
        normalized.insert(key.clone(), normalize_projected_cell(key, value)?);
    }
    Ok(Value::Object(normalized))
}

/// Order-insensitive comparison of observed and projected rows, plus the
/// canonical digest of the observed rows.
fn compare_rows(observed: Vec<Value>, projected: &[Value]) -> Result<(bool, String)> {
    let mut observed_lines: Vec<String> = observed
        .iter()
        .map(|row| Ok(serde_json::to_string(row)?))
        .collect::<Result<_>>()?;
    observed_lines.sort();
    let mut projected_lines: Vec<String> = projected
        .iter()
        .map(|row| -> Result<String> {
            let normalized = normalize_projected_row(row)?;
            Ok(serde_json::to_string(&normalized)?)
        })
        .collect::<Result<_>>()?;
    projected_lines.sort();
    let digest = sha256_hex(observed_lines.join("\n").as_bytes());
    Ok((observed_lines == projected_lines, digest))
}

fn run_fts_probes(
    source: &Connection,
    expected: &DatabaseContract,
) -> Result<BTreeMap<String, FtsProbeResult>> {
    let mut results = BTreeMap::new();
    for (table, term, ref_table, ref_column) in FTS_PROBES {
        let rows = expected
            .representative_rows
            .get(ref_table)
            .with_context(|| format!("projection lacks representative rows for {ref_table}"))?;
        let first = rows
            .first()
            .with_context(|| format!("projection representative rows for {ref_table} are empty"))?;
        let haystack = first
            .get(ref_column)
            .and_then(Value::as_str)
            .with_context(|| {
                format!("projection representative row for {ref_table} lacks column {ref_column}")
            })?;
        ensure!(
            haystack.contains(term),
            "FTS probe term {term:?} must occur in the projected representative row for {ref_table}"
        );
        let mut stmt = source.prepare(&format!(
            "select rowid from {} where {} match ?1",
            quote_ident(table),
            quote_ident(table)
        ))?;
        let mut ids: Vec<i64> = stmt
            .query_map([term], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.sort_unstable();
        ids.dedup();
        let digest = sha256_hex(
            ids.iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",")
                .as_bytes(),
        );
        results.insert(table.to_string(), FtsProbeResult { ids, digest });
    }
    Ok(results)
}

fn ledger_counts(target: &Connection) -> Result<LedgerCounts> {
    let mut counts = LedgerCounts {
        read: 0,
        skipped: 0,
        blocking: 0,
    };
    let mut stmt = target.prepare("select outcome, count(*) from probe_ledger group by outcome")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let outcome: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        match outcome.as_str() {
            "read" => counts.read = count as u64,
            "skipped" => counts.skipped = count as u64,
            "blocking" => counts.blocking = count as u64,
            other => bail!("unexpected ledger outcome {other:?}"),
        }
    }
    Ok(counts)
}

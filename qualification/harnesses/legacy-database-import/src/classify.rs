//! Read-only classification of the frozen SQLite fixtures against Task 3's
//! verified database projection (`qualification/compatibility/
//! legacy-database-import.json`).
//!
//! This is a qualification classifier: it derives a classification from a
//! strict read-only introspection of the source database and cross-checks it
//! against the projection's `variants` expectations. It is deliberately not
//! the production `StateClassifier` and never falls back to
//! `compatibility/snapshots/state.json`.

use crate::report::{Classification, sha256_file};
use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Repo-relative path of the only accepted projection contract.
pub const PROJECTION_CONTRACT: &str = "qualification/compatibility/legacy-database-import.json";
/// Repo-relative path of the only accepted fixture root.
pub const FIXTURE_ROOT: &str = "compatibility/fixtures/database";
/// Repo-relative parent of every validated probe work root.
pub const WORK_ROOT: &str = "qualification/.artifacts/work/legacy-database-import";
/// The fixture the projection's full schema/content expectations describe.
pub const CURRENT_FIXTURE: &str = "compatibility/fixtures/database/minimal-python.sqlite";

/// The six frozen fixture basenames; nothing else is ever opened.
pub const FROZEN_FIXTURES: [&str; 6] = [
    "minimal-python.sqlite",
    "legacy-python.sqlite",
    "empty.sqlite",
    "partial-python.sqlite",
    "corrupt.sqlite",
    "unknown-schema.sqlite",
];

/// The exact contract ids the projection must carry.
pub const CONTRACT_IDS: [&str; 3] = [
    "database.migrations.current",
    "database.schema.current",
    "database.upgrade.preflight",
];

/// The exact 12-field `database` object the projection must carry.
const DATABASE_FIELDS: [&str; 12] = [
    "application_migration_ledgers",
    "columns",
    "fixture",
    "foreign_keys",
    "indexes",
    "migration_sources",
    "object_contracts",
    "representative_rows",
    "row_counts",
    "tables",
    "triggers",
    "variants",
];

/// Each projected expectation must map to its frozen contract id.
const EXPECTED_OBJECT_CONTRACTS: [(&str, &str); 10] = [
    (
        "application_migration_ledgers",
        "database.migrations.current",
    ),
    ("columns", "database.schema.current"),
    ("foreign_keys", "database.schema.current"),
    ("indexes", "database.schema.current"),
    ("migration_sources", "database.migrations.current"),
    ("representative_rows", "database.schema.current"),
    ("row_counts", "database.schema.current"),
    ("tables", "database.schema.current"),
    ("triggers", "database.schema.current"),
    ("variants", "database.upgrade.preflight"),
];

/// The legacy pre-memory-graph schema signature: the memory-graph migration
/// consumes `strict_terms` rows into concepts, facets, and crystals, so a
/// legacy source carries exactly these three tables and no ledger.
const LEGACY_TABLES: [&str; 3] = ["series", "strict_terms", "crystals"];

/// Repo root, resolved from the compile-time manifest directory.
pub fn repo_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .expect("canonicalize CARGO_MANIFEST_DIR")
            .ancestors()
            .nth(3)
            .expect("harness lives three directories below the repo root")
            .to_path_buf()
    })
}

/// Quote an identifier for use inside a pragma or query.
pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or_default().to_string()
}

/// The projection normalizes DDL whitespace; observed SQL must be normalized
/// the same way before comparison.
fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The fixture root must canonicalize to the repo's frozen fixture root.
pub fn validate_fixture_root(fixture_root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(fixture_root)
        .with_context(|| format!("canonicalize fixture root {}", fixture_root.display()))?;
    let expected = fs::canonicalize(repo_root().join(FIXTURE_ROOT))
        .with_context(|| format!("canonicalize the repo fixture root {FIXTURE_ROOT}"))?;
    ensure!(
        canonical == expected,
        "fixture root must be exactly {FIXTURE_ROOT}"
    );
    Ok(canonical)
}

/// The projection contract must canonicalize to the one verified projection.
pub fn validate_contract_path(contract: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(contract)
        .with_context(|| format!("canonicalize contract {}", contract.display()))?;
    let expected = fs::canonicalize(repo_root().join(PROJECTION_CONTRACT))
        .with_context(|| format!("canonicalize the verified projection {PROJECTION_CONTRACT}"))?;
    ensure!(
        canonical == expected,
        "contract must be the verified projection at {PROJECTION_CONTRACT}, got {}",
        contract.display()
    );
    Ok(canonical)
}

/// The source must be one of the six frozen basenames and resolve to a
/// non-symlink regular file directly inside the validated fixture root.
pub fn validate_source(source: &Path, fixture_root: &Path) -> Result<String> {
    let root = validate_fixture_root(fixture_root)?;
    let name = source
        .file_name()
        .and_then(OsStr::to_str)
        .context("source name must be valid utf-8")?;
    ensure!(
        FROZEN_FIXTURES.contains(&name),
        "source name {name:?} is not one of the six frozen fixture basenames"
    );
    let owned = root.join(name);
    let meta = fs::symlink_metadata(&owned).with_context(|| format!("inspect fixture {name}"))?;
    ensure!(!meta.is_symlink(), "fixture {name} must not be a symlink");
    ensure!(meta.is_file(), "fixture {name} must be a regular file");
    let resolved =
        fs::canonicalize(&owned).with_context(|| format!("canonicalize fixture {name}"))?;
    ensure!(
        resolved.parent() == Some(root.as_path()),
        "fixture {name} must be a direct child of the fixture root"
    );
    let given =
        fs::canonicalize(source).with_context(|| format!("canonicalize {}", source.display()))?;
    ensure!(
        given == resolved,
        "source must resolve to the frozen fixture {name}"
    );
    Ok(name.to_string())
}

/// The work root must be an existing, non-symlink strict descendant of the
/// repo's probe work root.
pub fn validate_work_root(work_root: &Path) -> Result<PathBuf> {
    let allowed = repo_root().join(WORK_ROOT);
    fs::create_dir_all(&allowed)
        .with_context(|| format!("ensure the allowed work root {WORK_ROOT} exists"))?;
    let allowed = fs::canonicalize(&allowed)?;
    let meta = fs::symlink_metadata(work_root)
        .with_context(|| format!("inspect work root {}", work_root.display()))?;
    ensure!(
        !meta.is_symlink(),
        "work root must not be a symlink: {}",
        work_root.display()
    );
    ensure!(
        meta.is_dir(),
        "work root must be a directory: {}",
        work_root.display()
    );
    let canonical = fs::canonicalize(work_root)
        .with_context(|| format!("canonicalize work root {}", work_root.display()))?;
    ensure!(
        canonical != allowed && canonical.starts_with(&allowed),
        "work root must be a non-symlink strict descendant of {WORK_ROOT}"
    );
    Ok(canonical)
}

/// The target must be a new, non-symlink plain file directly beneath the
/// validated work root.
pub fn validate_target(target: &Path, work_root: &Path) -> Result<PathBuf> {
    let allowed = validate_work_root(work_root)?;
    let name = target
        .file_name()
        .and_then(OsStr::to_str)
        .context("target name must be valid utf-8")?;
    ensure!(
        !name.is_empty() && name != "." && name != "..",
        "target name must be a plain file name"
    );
    let parent = target
        .parent()
        .context("target must have a parent directory")?;
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("canonicalize target parent {}", parent.display()))?;
    ensure!(
        parent == allowed,
        "target must live directly beneath the validated work root"
    );
    match fs::symlink_metadata(target) {
        Ok(meta) => {
            ensure!(
                !meta.is_symlink(),
                "target must not be a symlink: {}",
                target.display()
            );
            anyhow::bail!(
                "target must be a new disposable file, but it already exists: {}",
                target.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(
                anyhow::Error::new(error).context(format!("inspect target {}", target.display()))
            );
        }
    }
    Ok(parent.join(name))
}

/// Open a source fixture strictly read-only and switch on `query_only`
/// before any statement runs. No source transaction or write pragma is ever
/// issued.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open read-only {}", path.display()))?;
    conn.execute_batch("PRAGMA query_only = ON")
        .context("set PRAGMA query_only = ON")?;
    Ok(conn)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedColumn {
    pub cid: i64,
    pub default: Option<String>,
    pub hidden: i64,
    pub name: String,
    pub not_null: bool,
    pub primary_key: i64,
    #[serde(rename = "type")]
    pub column_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedIndexColumn {
    pub cid: i64,
    pub name: String,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedIndex {
    pub columns: Vec<ProjectedIndexColumn>,
    pub name: String,
    pub origin: String,
    pub partial: bool,
    pub sql: Option<String>,
    pub table: String,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedTrigger {
    pub name: String,
    pub sql: String,
    pub table: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedForeignKey {
    pub from: String,
    pub id: i64,
    pub r#match: String,
    pub on_delete: String,
    pub on_update: String,
    pub seq: i64,
    pub table: String,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LedgerProjection {
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
}

/// Projection expectations for one fixture variant.
#[derive(Debug, Clone, Deserialize)]
pub struct VariantExpectation {
    pub classification: String,
    pub foreign_key_violations: u64,
    pub integrity: String,
    pub safe_to_convert: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VariantEntry {
    pub expected: VariantExpectation,
    pub fixture: String,
    pub id: String,
}

#[derive(Debug, Deserialize)]
struct TableEntry {
    name: String,
    sql: String,
}

/// The parsed, strictly validated Task 3 database projection.
#[derive(Debug)]
pub struct DatabaseContract {
    pub fixture: String,
    pub tables: BTreeMap<String, String>,
    pub columns: BTreeMap<String, Vec<ProjectedColumn>>,
    pub indexes: Vec<ProjectedIndex>,
    pub triggers: Vec<ProjectedTrigger>,
    pub foreign_keys: BTreeMap<String, Vec<ProjectedForeignKey>>,
    pub row_counts: BTreeMap<String, i64>,
    pub representative_rows: BTreeMap<String, Vec<Value>>,
    pub migration_sources: Vec<String>,
    pub application_migration_ledgers: BTreeMap<String, LedgerProjection>,
    variants: BTreeMap<String, VariantEntry>,
}

impl DatabaseContract {
    /// Parse the closed projection. Requires the exact three contract ids,
    /// the exact 12-field `database` object, and the frozen object-contract
    /// mapping; nothing else is accepted.
    pub fn load(path: &Path) -> Result<DatabaseContract> {
        let raw: Value = serde_json::from_str(
            &fs::read_to_string(path)
                .with_context(|| format!("read projection contract {}", path.display()))?,
        )
        .with_context(|| format!("projection contract {} must be valid JSON", path.display()))?;

        ensure!(
            raw.get("projection_version") == Some(&Value::from(1)),
            "projection_version must be 1"
        );
        ensure!(
            raw.get("risk").and_then(Value::as_str) == Some("legacy-database-import"),
            "projection risk must be legacy-database-import"
        );

        let mut contract_ids = BTreeSet::new();
        let contracts = raw
            .get("contracts")
            .context("projection must list contracts")?
            .as_array()
            .context("contracts must be an array")?;
        for entry in contracts {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .context("every contract needs an id")?;
            contract_ids.insert(id.to_string());
        }
        let expected_ids: BTreeSet<String> =
            CONTRACT_IDS.iter().map(|id| (*id).to_string()).collect();
        ensure!(
            contract_ids == expected_ids,
            "projection must carry exactly the three frozen contract ids"
        );

        let database = raw
            .get("database")
            .context("projection must carry a database object")?
            .as_object()
            .context("database must be an object")?;
        let fields: BTreeSet<&str> = database.keys().map(String::as_str).collect();
        let expected_fields: BTreeSet<&str> = DATABASE_FIELDS.into_iter().collect();
        ensure!(
            fields == expected_fields,
            "database object must carry exactly the 12 projected fields"
        );

        let object_contracts: BTreeMap<String, String> =
            serde_json::from_value(database["object_contracts"].clone())
                .context("object_contracts must map fields to contract ids")?;
        let expected_object_contracts: BTreeMap<String, String> = EXPECTED_OBJECT_CONTRACTS
            .iter()
            .map(|(field, id)| ((*field).to_string(), (*id).to_string()))
            .collect();
        ensure!(
            object_contracts == expected_object_contracts,
            "object_contracts must map each expectation to its frozen contract id"
        );

        let fixture: String = serde_json::from_value(database["fixture"].clone())
            .context("database.fixture must be a string")?;
        ensure!(
            fixture == CURRENT_FIXTURE,
            "database.fixture must be {CURRENT_FIXTURE}"
        );

        let table_entries: Vec<TableEntry> =
            serde_json::from_value(database["tables"].clone()).context("database.tables")?;
        let tables: BTreeMap<String, String> = table_entries
            .into_iter()
            .map(|entry| (entry.name, entry.sql))
            .collect();
        let columns: BTreeMap<String, Vec<ProjectedColumn>> =
            serde_json::from_value(database["columns"].clone()).context("database.columns")?;
        let indexes: Vec<ProjectedIndex> =
            serde_json::from_value(database["indexes"].clone()).context("database.indexes")?;
        let triggers: Vec<ProjectedTrigger> =
            serde_json::from_value(database["triggers"].clone()).context("database.triggers")?;
        let foreign_keys: BTreeMap<String, Vec<ProjectedForeignKey>> =
            serde_json::from_value(database["foreign_keys"].clone())
                .context("database.foreign_keys")?;
        let row_counts: BTreeMap<String, i64> =
            serde_json::from_value(database["row_counts"].clone())
                .context("database.row_counts")?;
        let representative_rows: BTreeMap<String, Vec<Value>> =
            serde_json::from_value(database["representative_rows"].clone())
                .context("database.representative_rows")?;

        let migration_sources: Vec<String> =
            serde_json::from_value(database["migration_sources"].clone())
                .context("database.migration_sources")?;
        ensure!(
            !migration_sources.is_empty(),
            "migration_sources must not be empty"
        );
        ensure!(
            migration_sources
                .iter()
                .all(|entry| entry.ends_with(".sql")),
            "migration sources must be .sql files"
        );

        let application_migration_ledgers: BTreeMap<String, LedgerProjection> =
            serde_json::from_value(database["application_migration_ledgers"].clone())
                .context("database.application_migration_ledgers")?;

        let variant_entries: Vec<VariantEntry> =
            serde_json::from_value(database["variants"].clone()).context("database.variants")?;
        let mut variants = BTreeMap::new();
        for entry in variant_entries {
            variants.insert(basename(&entry.fixture), entry);
        }

        Ok(DatabaseContract {
            fixture,
            tables,
            columns,
            indexes,
            triggers,
            foreign_keys,
            row_counts,
            representative_rows,
            migration_sources,
            application_migration_ledgers,
            variants,
        })
    }

    /// Variant expectation keyed by fixture basename.
    pub fn variant(&self, fixture_name: &str) -> Option<&VariantEntry> {
        self.variants.get(fixture_name)
    }
}

/// Read-only introspection of one source database, shaped exactly like the
/// projection's schema expectations.
#[derive(Debug, Default, Serialize)]
struct Inventory {
    tables: BTreeMap<String, String>,
    columns: BTreeMap<String, Vec<ProjectedColumn>>,
    indexes: Vec<ProjectedIndex>,
    triggers: Vec<ProjectedTrigger>,
    foreign_keys: BTreeMap<String, Vec<ProjectedForeignKey>>,
}

impl Inventory {
    /// Observe the full schema inventory of an open read-only connection.
    fn observe(conn: &Connection) -> Result<Inventory> {
        let mut tables = BTreeMap::new();
        {
            let mut stmt = conn.prepare(
                "select name, sql from sqlite_master where type = 'table' order by name",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let name: String = row.get(0)?;
                let sql: Option<String> = row.get(1)?;
                tables.insert(name, normalize_sql(&sql.unwrap_or_default()));
            }
        }

        let mut columns = BTreeMap::new();
        let mut indexes = Vec::new();
        let mut foreign_keys = BTreeMap::new();
        for name in tables.keys() {
            let mut cols = Vec::new();
            {
                let mut stmt =
                    conn.prepare(&format!("pragma table_xinfo({})", quote_ident(name)))?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    cols.push(ProjectedColumn {
                        cid: row.get("cid")?,
                        default: row.get("dflt_value")?,
                        hidden: row.get("hidden")?,
                        name: row.get("name")?,
                        not_null: row.get::<_, i64>("notnull")? != 0,
                        primary_key: row.get("pk")?,
                        column_type: row.get("type")?,
                    });
                }
            }
            columns.insert(name.clone(), cols);

            {
                let mut stmt =
                    conn.prepare(&format!("pragma index_list({})", quote_ident(name)))?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let index_name: String = row.get("name")?;
                    indexes.push(ProjectedIndex {
                        columns: index_columns(conn, &index_name)?,
                        name: index_name.clone(),
                        origin: row.get("origin")?,
                        partial: row.get::<_, i64>("partial")? != 0,
                        sql: index_sql(conn, &index_name)?.as_deref().map(normalize_sql),
                        table: name.clone(),
                        unique: row.get::<_, i64>("unique")? != 0,
                    });
                }
            }

            let mut fks = Vec::new();
            {
                let mut stmt =
                    conn.prepare(&format!("pragma foreign_key_list({})", quote_ident(name)))?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    fks.push(ProjectedForeignKey {
                        from: row.get("from")?,
                        id: row.get("id")?,
                        r#match: row.get("match")?,
                        on_delete: row.get("on_delete")?,
                        on_update: row.get("on_update")?,
                        seq: row.get("seq")?,
                        table: row.get("table")?,
                        to: row.get("to")?,
                    });
                }
            }
            if !fks.is_empty() {
                foreign_keys.insert(name.clone(), fks);
            }
        }
        indexes.sort_by(|left, right| left.name.cmp(&right.name));

        let mut triggers = Vec::new();
        {
            let mut stmt = conn.prepare(
                "select name, sql, tbl_name from sqlite_master where type = 'trigger' order by name",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                triggers.push(ProjectedTrigger {
                    name: row.get(0)?,
                    sql: normalize_sql(&row.get::<_, Option<String>>(1)?.unwrap_or_default()),
                    table: row.get(2)?,
                });
            }
        }

        Ok(Inventory {
            tables,
            columns,
            indexes,
            triggers,
            foreign_keys,
        })
    }

    /// Canonical digest of the full schema inventory.
    fn digest(&self) -> Result<String> {
        Ok(crate::report::sha256_hex(
            serde_json::to_string(self)?.as_bytes(),
        ))
    }

    /// Full inventory equality with the projected current schema.
    fn matches_current(&self, contract: &DatabaseContract) -> bool {
        self.tables == contract.tables
            && self.columns == contract.columns
            && self.indexes == contract.indexes
            && self.triggers == contract.triggers
            && self.foreign_keys == contract.foreign_keys
    }

    /// The legacy signature: exactly the three pre-memory-graph tables, with
    /// columns that are order-preserving subsets of the projected columns.
    fn matches_legacy(&self, contract: &DatabaseContract) -> bool {
        if self.tables.len() != LEGACY_TABLES.len() {
            return false;
        }
        for table in LEGACY_TABLES {
            if !self.tables.contains_key(table) {
                return false;
            }
            let (Some(projected), Some(actual)) =
                (contract.columns.get(table), self.columns.get(table))
            else {
                return false;
            };
            if !is_ordered_subset(actual, projected) {
                return false;
            }
        }
        true
    }

    fn subset_of_known(&self, contract: &DatabaseContract) -> bool {
        self.tables
            .keys()
            .all(|name| contract.tables.contains_key(name))
    }

    fn classify(&self, contract: &DatabaseContract) -> &'static str {
        if self.tables.is_empty() {
            "empty"
        } else if self.matches_current(contract) {
            "supported-python"
        } else if self.matches_legacy(contract) {
            "supported-legacy-python"
        } else if !self.tables.is_empty() && self.subset_of_known(contract) {
            "partial-python"
        } else {
            "unknown-schema"
        }
    }
}

/// Legacy columns are checked on the attributes the memory-graph import
/// actually reads: name, type, nullability, and key position. DDL defaults
/// are intentionally ignored because the legacy schema added defaults (for
/// example `default_source_language text not null default ''`) that the
/// current schema does not carry.
fn legacy_column_matches(actual: &ProjectedColumn, projected: &ProjectedColumn) -> bool {
    actual.name == projected.name
        && actual.column_type == projected.column_type
        && actual.not_null == projected.not_null
        && actual.primary_key == projected.primary_key
        && actual.hidden == projected.hidden
}

fn is_ordered_subset(actual: &[ProjectedColumn], projected: &[ProjectedColumn]) -> bool {
    let mut projected = projected.iter();
    actual
        .iter()
        .all(|column| projected.any(|candidate| legacy_column_matches(candidate, column)))
}

fn index_columns(conn: &Connection, index: &str) -> Result<Vec<ProjectedIndexColumn>> {
    let mut stmt = conn.prepare(&format!("pragma index_info({})", quote_ident(index)))?;
    let mut rows = stmt.query([])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        columns.push(ProjectedIndexColumn {
            cid: row.get("cid")?,
            name: row.get("name")?,
            seq: row.get("seqno")?,
        });
    }
    Ok(columns)
}

fn index_sql(conn: &Connection, index: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "select sql from sqlite_master where type = 'index' and name = ?1",
            [index],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .unwrap_or_default())
}

fn integrity_check(conn: &Connection) -> Result<String> {
    let mut stmt = conn.prepare("pragma integrity_check")?;
    let mut rows = stmt.query([])?;
    let mut results = Vec::new();
    while let Some(row) = rows.next()? {
        results.push(row.get::<_, String>(0)?);
    }
    if results.len() == 1 && results[0] == "ok" {
        Ok("ok".to_string())
    } else {
        Ok("degraded".to_string())
    }
}

fn foreign_key_violation_count(conn: &Connection) -> Result<u64> {
    let mut stmt = conn.prepare("pragma foreign_key_check")?;
    let mut rows = stmt.query([])?;
    let mut violations = 0u64;
    while rows.next()?.is_some() {
        violations += 1;
    }
    Ok(violations)
}

/// Classify a validated source against an already loaded projection.
pub fn classify_with_contract(
    source: &Path,
    fixture_root: &Path,
    contract: &DatabaseContract,
) -> Result<Classification> {
    let name = validate_source(source, fixture_root)?;
    let source_sha256 = sha256_file(source)?;
    let variant = contract
        .variant(&name)
        .ok_or_else(|| anyhow!("projection has no variant expectation for {name}"))?;

    // Open read-only; any read failure classifies as unreadable and must
    // match the projected corrupt expectation.
    enum Reading {
        Unreadable,
        Readable(Inventory, String, u64),
    }
    let reading = match open_read_only(source) {
        Err(_) => Reading::Unreadable,
        Ok(conn) => match Inventory::observe(&conn) {
            Err(_) => Reading::Unreadable,
            Ok(inventory) => {
                let integrity = integrity_check(&conn)?;
                let violations = foreign_key_violation_count(&conn)?;
                Reading::Readable(inventory, integrity, violations)
            }
        },
    };

    let classification = match reading {
        Reading::Unreadable => {
            ensure!(
                variant.expected.integrity == "unreadable"
                    && variant.expected.classification == "corrupt"
                    && variant.expected.foreign_key_violations == 0,
                "projection expects {:?}/{:?} for {name}, but the source is unreadable",
                variant.expected.classification,
                variant.expected.integrity
            );
            Classification {
                name: "corrupt".to_string(),
                safe_to_convert: false,
                source_name: name,
                integrity: "unreadable".to_string(),
                foreign_key_violations: 0,
                schema_digest: Inventory::default().digest()?,
                source_sha256,
                variant_id: variant.id.clone(),
            }
        }
        Reading::Readable(inventory, integrity, violations) => {
            let classification = inventory.classify(contract);
            ensure!(
                integrity == variant.expected.integrity,
                "integrity check {integrity:?} contradicts the projected expectation {:?} for {name}",
                variant.expected.integrity
            );
            ensure!(
                violations == variant.expected.foreign_key_violations,
                "foreign-key violations {violations} contradict the projected expectation {} for {name}",
                variant.expected.foreign_key_violations
            );
            ensure!(
                classification == variant.expected.classification,
                "derived classification {classification:?} contradicts the projected variant {:?} for {name}",
                variant.expected.classification
            );
            let safe = classification.starts_with("supported-");
            ensure!(
                safe == variant.expected.safe_to_convert,
                "derived safe_to_convert {safe} contradicts the projected variant for {name}"
            );
            Classification {
                name: classification.to_string(),
                safe_to_convert: variant.expected.safe_to_convert,
                source_name: name,
                integrity,
                foreign_key_violations: violations,
                schema_digest: inventory.digest()?,
                source_sha256,
                variant_id: variant.id.clone(),
            }
        }
    };
    Ok(classification)
}

/// Classify a fixture strictly read-only, accepting only the frozen fixture
/// root and the one verified projection contract.
pub fn classify_read_only(
    source: &Path,
    fixture_root: &Path,
    projection_contract: &Path,
) -> Result<Classification> {
    validate_contract_path(projection_contract)?;
    let contract = DatabaseContract::load(projection_contract)?;
    classify_with_contract(source, fixture_root, &contract)
}

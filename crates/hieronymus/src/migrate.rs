//! Read-only upgrade tooling (database-upgrade design, 2026-08-31;
//! ADR 0010): preflight classification and reporting, the typed strict-terms
//! converter, and the disposable dry-run.
//!
//! Part 1 of the upgrade protocol writes nothing to the data root: the
//! preflight opens the source strictly read-only, and the dry-run rehearses
//! the conversion on a byte-identical disposable copy outside the data root
//! that is removed after reporting. The write-side protocol (backup, cutover
//! journal, single transaction, config promotion) is a follow-up task.
//!
//! Transaction discipline is designed in for part 2: [`convert_strict_terms`]
//! takes a `&Connection` and can neither commit nor open a second connection
//! nor touch configuration. It additionally refuses autocommit connections,
//! so the caller must own a transaction (`&Transaction` derefs to
//! `&Connection`), which is exactly the handle the future upgrade
//! transaction will pass.
//!
//! Idempotence: the converter blocks (`MigrateError::AlreadyConverted`)
//! against a target that already carries converted rules. Recovery after an
//! interrupted upgrade is the cutover journal's job, not a silent re-run, so
//! a second conversion attempt can never double-apply.
//!
//! The dry-run verdict is honest for scripting: post-conversion verification
//! (foreign keys, integrity, row accounting, FTS equivalence) must pass, or
//! the report carries `refused: "verification-failed"` and the conversion-safe
//! verdict flips to false — the CLI exits nonzero.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use rusqlite::OpenFlags;
use serde::Serialize;

use crate::data_root::HieronymusConfig;
use crate::db::{
    DatabaseState, SUPPORTED_RUST_SCHEMA_VERSION, apply_terminology_schema_steps, classify_database,
};
use crate::terminology::rule_text;

/// The ledger records one outcome for every legacy source row and the stable
/// legacy-to-target identity mapping (design: Terminology Conversion).
const LEDGER_DDL: &str = "
create table if not exists term_migration_ledger (
  source_table text not null,
  source_id text not null,
  outcome text not null check (outcome in ('converted', 'skipped', 'blocking')),
  reason_code text not null,
  target_rule_id integer,
  target_form_id integer,
  legacy_concept_id integer,
  legacy_crystal_id integer,
  primary key (source_table, source_id)
);";

const LEDGER_TABLE: &str = "term_migration_ledger";

// Bounded ledger reason codes. Nothing outside this set is ever written, so
// reports stay aggregable and downstream tooling can match exhaustively.
pub const REASON_OK: &str = "ok";
pub const REASON_UNKNOWN_STATUS: &str = "unknown-status";
pub const REASON_EMPTY_FORM: &str = "empty-form";
pub const REASON_UNSUPPORTED_ALIAS_KIND: &str = "unsupported-alias-kind";
pub const REASON_CASE_INSENSITIVE_ALIAS: &str = "case-insensitive-alias";
pub const REASON_TOO_MANY_FORBIDDEN_VARIANTS: &str = "too-many-forbidden-variants";
pub const REASON_APPROVED_VARIANT_MISMATCH: &str = "approved-variant-mismatch";
pub const REASON_RULE_SHAPE_ROUND_TRIP: &str = "rule-shape-round-trip";
pub const REASON_BLANK_ALIAS: &str = "blank-alias";
pub const REASON_INVALID_VALUE: &str = "invalid-value";
pub const REASON_INSERT_FAILED: &str = "insert-failed";

// Bounded preflight/dry-run refusal codes (fail-closed states).
pub const REFUSAL_EMPTY_DATABASE: &str = "empty-database";
pub const REFUSAL_SOURCE_UNREADABLE: &str = "source-unreadable";
pub const REFUSAL_UNKNOWN_SCHEMA: &str = "unknown-schema";
pub const REFUSAL_NEWER_SCHEMA: &str = "newer-schema";
pub const REFUSAL_ALREADY_CURRENT: &str = "already-current";
pub const REFUSAL_UNSUPPORTED_LEGACY_SCHEMA: &str = "unsupported-legacy-schema";
pub const REFUSAL_INTEGRITY_DEGRADED: &str = "integrity-degraded";
pub const REFUSAL_FOREIGN_KEY_VIOLATIONS: &str = "foreign-key-violations";
pub const REFUSAL_DAEMON_ACTIVE: &str = "daemon-active";
pub const REFUSAL_VERIFICATION_FAILED: &str = "verification-failed";

/// Provenance recorded on every converted rule; legacy strict terms carry no
/// provenance column, so the migration itself is the provenance.
const CONVERTED_PROVENANCE: &str = "migrated:strict_terms";

/// The exact legacy column fingerprint the converter reads (Python
/// `migrations/global.sql`). Extra or missing columns fail closed: data this
/// converter does not know about must never be silently dropped.
const LEGACY_FINGERPRINT: &[(&str, &[&str])] = &[
    (
        "strict_terms",
        &[
            "id",
            "series_slug",
            "source_language",
            "target_language",
            "category",
            "source_text",
            "canonical_translation",
            "status",
            "notes",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "strict_term_aliases",
        &[
            "id",
            "term_id",
            "language",
            "text",
            "kind",
            "case_sensitive",
        ],
    ),
    ("strict_term_tags", &["term_id", "tag"]),
];

/// The legacy external-content FTS projection that must exist for the
/// searchable strict-term projection to be rebuildable.
const LEGACY_FTS_TABLE: &str = "strict_terms_fts";

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("legacy schema is unsupported: {0}")]
    UnsupportedLegacySchema(String),
    #[error("target schema is missing: {0}")]
    TargetSchemaMissing(String),
    #[error("converter requires a caller-owned transaction, not autocommit")]
    TransactionRequired,
    #[error("target already contains converted terminology; refusing to convert twice")]
    AlreadyConverted,
    #[error("source database changed during the dry-run")]
    SourceChanged,
    #[error("dry-run work root must live outside the data root")]
    InvalidWorkRoot,
    // --- write-side upgrade protocol (part 2) ---
    #[error("upgrade refused: {0}")]
    Refused(String),
    #[error("configuration is invalid: {0}")]
    ConfigInvalid(String),
    #[error("credential file has unsafe permissions: {0}")]
    UnsafeCredentialPermissions(PathBuf),
    #[error("enabled workflow cannot resolve its provider: {0}")]
    WorkflowUnresolved(String),
    #[error(
        "staged checksum mismatch for {path}: journal expects {expected}, staging holds {actual}"
    )]
    StagedChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("cutover journal is inconsistent: {0}")]
    JournalInconsistent(String),
    #[error("data-root ownership is held by another process: {0}")]
    RootOwnership(String),
    #[error("no verified pre-upgrade backup was found")]
    BackupMissing,
    #[error("recovery is blocked: {0}")]
    RecoveryBlocked(String),
    #[error("the durable semantic rebuild job could not be created: {0}")]
    SemanticJob(String),
    #[error("failure injected at protocol step {0:?}")]
    Injected(crate::upgrade::InjectionPoint),
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// The design's preflight report: schema, size, integrity, foreign keys,
/// table counts, pending conversion counts, unsupported rows with bounded
/// reason codes, required free space, target schema, backup path, and the
/// conversion-safe verdict. Only counts, ids, paths, and codes are recorded —
/// API keys and memory text are never copied into diagnostics by construction.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightReport {
    pub source_path: PathBuf,
    pub detected_state: String,
    pub detected_schema_version: Option<i64>,
    pub database_size_bytes: u64,
    pub integrity: String,
    pub foreign_key_violations: u64,
    pub table_counts: BTreeMap<String, i64>,
    pub pending_conversions: PendingConversionCounts,
    /// The typed conversion plan when the state is a supported legacy schema.
    pub plan: Option<TermConversionReport>,
    pub daemon_active: bool,
    /// Two database images: the disposable working copy plus backup headroom.
    pub required_free_space_bytes: u64,
    pub target_schema_version: i64,
    /// Reserved for the write-side upgrade protocol; always `None` in part 1.
    pub backup_path: Option<String>,
    pub conversion_safe: bool,
    pub refusal_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PendingConversionCounts {
    pub strict_terms: u64,
    pub strict_term_aliases: u64,
    pub strict_term_tags: u64,
}

/// Outcome of planning or applying the terminology conversion. One outcome
/// exists for every legacy source row: converted, skipped with a bounded
/// reason, or blocking.
#[derive(Debug, Clone, Serialize)]
pub struct TermConversionReport {
    pub source_terms: u64,
    pub source_aliases: u64,
    pub source_tags: u64,
    pub converted: u64,
    pub skipped: u64,
    pub blocking: u64,
    pub skip_reasons: BTreeMap<String, u64>,
    pub forms_inserted: u64,
    pub tags_inserted: u64,
    /// Ledger rows written; a plan-only report never writes and stays at 0.
    pub ledger_rows: u64,
    pub converted_terms: Vec<ConvertedTermLink>,
    pub skipped_terms: Vec<SkippedRow>,
    pub blocking_rows: Vec<SkippedRow>,
    /// `true` while the report describes a read-only plan with no writes.
    pub planned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvertedTermLink {
    pub legacy_term_id: i64,
    pub rule_id: i64,
    pub concept_id: Option<i64>,
    pub rule_crystal_id: Option<i64>,
}

/// One skipped or blocking source row, identified by table and legacy id with
/// its bounded reason code. Row text is deliberately not included.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedRow {
    pub source_table: String,
    pub source_id: String,
    pub reason_code: String,
}

#[derive(Debug, Serialize)]
pub struct DryRunReport {
    pub preflight: PreflightReport,
    /// Bounded refusal code: a pre-conversion classification refusal, an
    /// active daemon, or a failed post-conversion verification
    /// (`verification-failed`). `None` only when the dry-run ran and verified
    /// cleanly.
    pub refused: Option<String>,
    pub conversion: Option<TermConversionReport>,
    pub verification: Option<VerificationReport>,
    pub temp_artifacts_removed: bool,
}

#[derive(Debug, Serialize)]
pub struct VerificationReport {
    pub foreign_key_violations: u64,
    pub integrity: String,
    pub row_accounting_ok: bool,
    pub fts_equivalent: bool,
    pub fts_probes: u64,
    /// Domain invariants over the converted target: rule statuses within the
    /// lifecycle set, parseable forbidden-variant payloads, and every
    /// converted ledger row resolving to an existing rule.
    pub domain_invariants_ok: bool,
}

impl VerificationReport {
    /// The report is honest for scripting only when every check passed.
    pub fn all_checks_pass(&self) -> bool {
        self.row_accounting_ok
            && self.fts_equivalent
            && self.domain_invariants_ok
            && self.foreign_key_violations == 0
            && self.integrity == "ok"
    }
}

/// Full post-conversion verification through one connection handle (the
/// caller's transaction during the upgrade, a plain connection in the
/// dry-run): foreign keys, integrity, row accounting, FTS equivalence, and
/// the domain invariants of the design's step 9.
pub(crate) fn verify_upgraded_target(
    connection: &Connection,
    conversion: &TermConversionReport,
) -> Result<VerificationReport, MigrateError> {
    let foreign_key_violations = foreign_key_violation_count(connection)?;
    let integrity = integrity_result(connection)?;

    // Row accounting: every preserved and converted row is exactly where the
    // ledger says it is.
    let row_accounting_ok = conversion.ledger_rows
        == conversion.source_terms + conversion.source_aliases + conversion.source_tags
        && connection.query_row("select count(*) from term_rules", [], |row| {
            row.get::<_, i64>(0)
        })? as u64
            == conversion.converted
        && connection.query_row("select count(*) from term_rule_forms", [], |row| {
            row.get::<_, i64>(0)
        })? as u64
            == conversion.forms_inserted
        && connection.query_row("select count(*) from term_rule_semantic_tags", [], |row| {
            row.get::<_, i64>(0)
        })? as u64
            == conversion.tags_inserted;

    let (fts_equivalent, fts_probes) = verify_fts_equivalence(connection, conversion)?;
    let domain_invariants_ok = domain_invariants_hold(connection)?;

    Ok(VerificationReport {
        foreign_key_violations,
        integrity,
        row_accounting_ok,
        fts_equivalent,
        fts_probes,
        domain_invariants_ok,
    })
}

/// The design's domain invariants: lifecycle statuses stay inside the typed
/// set, every forbidden-variants payload parses as a string array, and every
/// converted strict-term ledger row resolves to an existing rule.
fn domain_invariants_hold(connection: &Connection) -> Result<bool, MigrateError> {
    let bad_status: i64 = connection.query_row(
        "select count(*) from term_rules
         where status not in ('candidate', 'active', 'superseded', 'archived')",
        [],
        |row| row.get(0),
    )?;
    if bad_status > 0 {
        return Ok(false);
    }
    let mut statement = connection.prepare(
        "select forbidden_variants_json from term_rules
         where forbidden_variants_json != '[]'",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let payload: String = row.get(0)?;
        let parsed = match serde_json::from_str::<serde_json::Value>(&payload) {
            Ok(parsed) => parsed,
            Err(_) => return Ok(false),
        };
        let valid = parsed
            .as_array()
            .map(|items| items.iter().all(|item| item.is_string()))
            .unwrap_or(false);
        if !valid {
            return Ok(false);
        }
    }
    let unresolved: i64 = connection.query_row(
        "select count(*) from term_migration_ledger
         where source_table = 'strict_terms' and outcome = 'converted'
           and (target_rule_id is null
                or not exists (select 1 from term_rules where id = target_rule_id))",
        [],
        |row| row.get(0),
    )?;
    Ok(unresolved == 0)
}

impl DryRunReport {
    /// Human-readable rendering; the library never prints, the CLI does.
    pub fn render_human(&self) -> String {
        let preflight = &self.preflight;
        let mut out = String::new();
        out.push_str(&format!(
            "migrate dry-run (state: {}, integrity: {}, foreign key violations: {})\n",
            preflight.detected_state, preflight.integrity, preflight.foreign_key_violations
        ));
        out.push_str(&format!("source: {}\n", preflight.source_path.display()));
        if let Some(code) = &self.refused {
            out.push_str(&format!("refused: {code}\n"));
            return out;
        }
        if let Some(conversion) = &self.conversion {
            out.push_str(&format!(
                "conversion: {} converted, {} skipped, {} blocking\n",
                conversion.converted, conversion.skipped, conversion.blocking
            ));
        }
        if let Some(verification) = &self.verification {
            out.push_str(&format!(
                "verification: row accounting {}, fts equivalent {} ({} probes)\n",
                if verification.row_accounting_ok {
                    "ok"
                } else {
                    "failed"
                },
                if verification.fts_equivalent {
                    "yes"
                } else {
                    "no"
                },
                verification.fts_probes
            ));
        }
        out.push_str(&format!(
            "temp artifacts removed: {}\n",
            if self.temp_artifacts_removed {
                "yes"
            } else {
                "no"
            }
        ));
        out
    }
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

/// Read-only preflight over the data root's database. Uses the shared bounded
/// `StateClassifier` (daemon-start behavior is untouched) and, for supported
/// legacy schemas, plans the terminology conversion to report pending counts
/// and unsupported rows. Never mutates the source.
pub fn run_preflight(
    config: &HieronymusConfig,
    daemon_active: bool,
) -> Result<PreflightReport, MigrateError> {
    let path = config.database_path();
    let state = classify_database(&path);
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let mut report = PreflightReport {
        source_path: path.clone(),
        detected_state: state.as_str().to_string(),
        detected_schema_version: state.schema_version(),
        database_size_bytes: size,
        integrity: "unreadable".to_string(),
        foreign_key_violations: 0,
        table_counts: BTreeMap::new(),
        pending_conversions: PendingConversionCounts::default(),
        plan: None,
        daemon_active,
        required_free_space_bytes: size.saturating_mul(2),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        backup_path: None,
        conversion_safe: false,
        refusal_code: None,
    };

    match state {
        DatabaseState::Empty => {
            report.refusal_code = Some(REFUSAL_EMPTY_DATABASE.to_string());
            return Ok(report);
        }
        DatabaseState::Corrupt => {
            report.refusal_code = Some(REFUSAL_SOURCE_UNREADABLE.to_string());
            return Ok(report);
        }
        DatabaseState::Unknown => {
            report.refusal_code = Some(REFUSAL_UNKNOWN_SCHEMA.to_string());
        }
        DatabaseState::NewerSchema { .. } => {
            report.refusal_code = Some(REFUSAL_NEWER_SCHEMA.to_string());
        }
        DatabaseState::RustSchema { .. } => {
            report.refusal_code = Some(REFUSAL_ALREADY_CURRENT.to_string());
        }
        DatabaseState::PythonSchema => {
            return inspect_legacy_database(config, report);
        }
    }

    // Readable-but-unsupported states still report health facts when the file
    // can be opened read-only.
    if let Ok(connection) = open_read_only(&path) {
        if let Ok(integrity) = integrity_result(&connection) {
            report.integrity = integrity;
        }
        if let Ok(violations) = foreign_key_violation_count(&connection) {
            report.foreign_key_violations = violations;
        }
        if let Ok(counts) = count_all_tables(&connection) {
            report.table_counts = counts;
        }
    }
    Ok(report)
}

/// Full inspection of a supported legacy Python database: fingerprint,
/// integrity, foreign keys, table counts, and the read-only conversion plan.
fn inspect_legacy_database(
    config: &HieronymusConfig,
    mut report: PreflightReport,
) -> Result<PreflightReport, MigrateError> {
    let connection = open_read_only(&config.database_path())?;
    report.integrity = integrity_result(&connection)?;
    report.foreign_key_violations = foreign_key_violation_count(&connection)?;
    report.table_counts = count_all_tables(&connection)?;

    if !legacy_fingerprint_matches(&connection) {
        report.refusal_code = Some(REFUSAL_UNSUPPORTED_LEGACY_SCHEMA.to_string());
        return Ok(report);
    }
    report.pending_conversions = PendingConversionCounts {
        strict_terms: report
            .table_counts
            .get("strict_terms")
            .copied()
            .unwrap_or(0) as u64,
        strict_term_aliases: report
            .table_counts
            .get("strict_term_aliases")
            .copied()
            .unwrap_or(0) as u64,
        strict_term_tags: report
            .table_counts
            .get("strict_term_tags")
            .copied()
            .unwrap_or(0) as u64,
    };
    let plan = plan_strict_term_conversion(&connection)?;
    report.conversion_safe = report.integrity == "ok" && report.foreign_key_violations == 0;
    if !report.conversion_safe {
        report.refusal_code = Some(if report.integrity != "ok" {
            REFUSAL_INTEGRITY_DEGRADED.to_string()
        } else {
            REFUSAL_FOREIGN_KEY_VIOLATIONS.to_string()
        });
    }
    report.plan = Some(plan);
    Ok(report)
}

/// Open a source strictly read-only and switch on `query_only` before any
/// statement runs. No source transaction or write pragma is ever issued
/// (the qualified import harness's proven approach).
pub(crate) fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("pragma query_only = on;")?;
    Ok(connection)
}

pub(crate) fn integrity_result(connection: &Connection) -> rusqlite::Result<String> {
    let mut statement = connection.prepare("pragma integrity_check")?;
    let mut rows = statement.query([])?;
    let mut first = String::new();
    let mut count = 0;
    while let Some(row) = rows.next()? {
        if count == 0 {
            first = row.get::<_, String>(0)?;
        }
        count += 1;
    }
    if count == 1 && first == "ok" {
        Ok("ok".to_string())
    } else {
        Ok("degraded".to_string())
    }
}

pub(crate) fn foreign_key_violation_count(connection: &Connection) -> rusqlite::Result<u64> {
    let mut statement = connection.prepare("pragma foreign_key_check")?;
    let mut rows = statement.query([])?;
    let mut violations = 0_u64;
    while rows.next()?.is_some() {
        violations += 1;
    }
    Ok(violations)
}

fn count_all_tables(connection: &Connection) -> rusqlite::Result<BTreeMap<String, i64>> {
    let mut statement =
        connection.prepare("select name from sqlite_master where type = 'table' order by name")?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut counts = BTreeMap::new();
    for name in names {
        let count = connection.query_row(
            &format!("select count(*) from \"{0}\"", name.replace('"', "\"\"")),
            [],
            |row| row.get::<_, i64>(0),
        )?;
        counts.insert(name, count);
    }
    Ok(counts)
}

fn legacy_fingerprint_matches(connection: &Connection) -> bool {
    for (table, required) in LEGACY_FINGERPRINT {
        let mut statement = match connection.prepare(&format!("pragma table_info({table})")) {
            Ok(statement) => statement,
            Err(_) => return false,
        };
        let actual = match statement.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => match rows.collect::<rusqlite::Result<std::collections::BTreeSet<_>>>() {
                Ok(names) => names,
                Err(_) => return false,
            },
            Err(_) => return false,
        };
        let expected: std::collections::BTreeSet<&str> = required.iter().copied().collect();
        // Exact column-set equality: unknown columns mean unknown data that a
        // conversion would silently drop.
        if actual.len() != expected.len()
            || !actual.iter().all(|name| expected.contains(name.as_str()))
        {
            return false;
        }
    }
    let fts_present: i64 = connection
        .query_row(
            "select count(*) from sqlite_master where type = 'table' and name = ?1",
            [LEGACY_FTS_TABLE],
            |row| row.get(0),
        )
        .unwrap_or(0);
    fts_present > 0
}

// ---------------------------------------------------------------------------
// Conversion planning and application
// ---------------------------------------------------------------------------

struct LegacyTerm {
    id: i64,
    series_slug: String,
    source_language: String,
    target_language: String,
    source_text: String,
    canonical_translation: String,
    status: String,
    notes: String,
    created_at: String,
    updated_at: String,
}

struct LegacyAlias {
    id: i64,
    language: String,
    text: String,
    kind: String,
    case_sensitive: i64,
}

/// A planned conversion: mapped lifecycle status plus the forbidden variants
/// carried into `forbidden_variants_json`.
struct PlannedTerm {
    status: &'static str,
    forbidden: Vec<String>,
}

/// The explicit legacy-to-Rust lifecycle mapping. Python wrote `pending` on
/// propose, `approved` on approve, and historical databases may hold `active`
/// (the legacy memory migration accepted both); anything else is a ledger
/// skip, never a guess.
fn map_status(status: &str) -> Option<&'static str> {
    match status {
        "pending" => Some("candidate"),
        "approved" | "active" => Some("active"),
        _ => None,
    }
}

/// Deterministically plan one term. The gating mirrors the legacy approval
/// shape checks: unsupported alias kinds and case-insensitive aliases cannot
/// be represented by the runtime rule model, at most one forbidden variant
/// exists, approved variants must equal the canonical rendering, and the rule
/// sentence must round-trip through the runtime parser.
fn plan_term(term: &LegacyTerm, aliases: &[LegacyAlias]) -> Result<PlannedTerm, &'static str> {
    let status = map_status(&term.status).ok_or(REASON_UNKNOWN_STATUS)?;
    if term.source_text.trim().is_empty() || term.canonical_translation.trim().is_empty() {
        return Err(REASON_EMPTY_FORM);
    }
    for alias in aliases {
        match alias.kind.as_str() {
            "approved_variant" | "forbidden_variant" => {}
            _ => return Err(REASON_UNSUPPORTED_ALIAS_KIND),
        }
        if alias.case_sensitive == 0 {
            return Err(REASON_CASE_INSENSITIVE_ALIAS);
        }
    }
    // The legacy shape check compares stripped approved variants against the
    // canonical rendering but keeps forbidden texts verbatim.
    let approved: Vec<&str> = aliases
        .iter()
        .filter(|alias| alias.kind == "approved_variant" && !alias.text.trim().is_empty())
        .map(|alias| alias.text.trim())
        .collect();
    let forbidden: Vec<String> = aliases
        .iter()
        .filter(|alias| alias.kind == "forbidden_variant" && !alias.text.trim().is_empty())
        .map(|alias| alias.text.clone())
        .collect();
    if forbidden.len() > 1 {
        return Err(REASON_TOO_MANY_FORBIDDEN_VARIANTS);
    }
    if approved
        .iter()
        .any(|variant| *variant != term.canonical_translation)
    {
        return Err(REASON_APPROVED_VARIANT_MISMATCH);
    }
    let text = rule_text(&term.source_text, &term.canonical_translation, &forbidden);
    match crate::terminology::parse_rule_crystal(&text) {
        Some((source, canonical, parsed_forbidden))
            if source == term.source_text
                && canonical == term.canonical_translation
                && parsed_forbidden == forbidden =>
        {
            Ok(PlannedTerm { status, forbidden })
        }
        _ => Err(REASON_RULE_SHAPE_ROUND_TRIP),
    }
}

/// Everything read from the legacy tables for one term. Per-row read
/// failures become blocking ledger outcomes directly on the report.
struct SourceEntry {
    term: LegacyTerm,
    aliases: Vec<LegacyAlias>,
    tags: Vec<String>,
}

/// Read-only plan of the terminology conversion. Requires only the legacy
/// tables, so the preflight can plan on a source that has no target schema
/// yet. Never writes.
pub fn plan_strict_term_conversion(
    connection: &Connection,
) -> Result<TermConversionReport, MigrateError> {
    require_legacy_schema(connection)?;
    let mut report = TermConversionReport {
        planned: true,
        ..TermConversionReport::empty()
    };
    let (entries, failures) = read_source_entries(connection, &mut report)?;
    // Plan-mode reports carry the same row-level detail as the applied
    // conversion: every blocking row with its bounded reason code.
    for failure in &failures {
        report.blocking_rows.push(failure.clone());
    }
    for entry in &entries {
        match plan_term(&entry.term, &entry.aliases) {
            Ok(_planned) => report.converted += 1,
            Err(reason) => {
                report.skipped += 1;
                *report.skip_reasons.entry(reason.to_string()).or_insert(0) += 1;
                report.skipped_terms.push(SkippedRow {
                    source_table: "strict_terms".to_string(),
                    source_id: entry.term.id.to_string(),
                    reason_code: reason.to_string(),
                });
            }
        }
    }
    Ok(report)
}

/// Apply the terminology conversion through `connection` inside the caller's
/// transaction. The converter cannot commit (a `&Connection` has no commit),
/// cannot open a second connection (it receives none), and cannot touch
/// configuration (it takes no paths): those are the upgrade transaction's
/// rules, enforced by construction.
pub fn convert_strict_terms(connection: &Connection) -> Result<TermConversionReport, MigrateError> {
    if connection.is_autocommit() {
        return Err(MigrateError::TransactionRequired);
    }
    require_legacy_schema(connection)?;
    require_target_schema(connection)?;
    ensure_not_already_converted(connection)?;

    let mut report = TermConversionReport::empty();
    let (entries, failures) = read_source_entries(connection, &mut report)?;

    connection.execute_batch(LEDGER_DDL)?;

    // Read failures reach the ledger like every other source row: blocking
    // with a bounded reason, never silent.
    for failure in &failures {
        report.blocking_rows.push(failure.clone());
        ledger_insert(
            connection,
            &failure.source_table,
            &failure.source_id,
            "blocking",
            &failure.reason_code,
            None,
            None,
            None,
            None,
        )?;
    }

    for entry in &entries {
        let planned = match plan_term(&entry.term, &entry.aliases) {
            Ok(planned) => planned,
            Err(reason) => {
                report.skipped += 1;
                *report.skip_reasons.entry(reason.to_string()).or_insert(0) += 1;
                report.skipped_terms.push(SkippedRow {
                    source_table: "strict_terms".to_string(),
                    source_id: entry.term.id.to_string(),
                    reason_code: reason.to_string(),
                });
                ledger_insert(
                    connection,
                    "strict_terms",
                    &entry.term.id.to_string(),
                    "skipped",
                    reason,
                    None,
                    None,
                    None,
                    None,
                )?;
                for alias in &entry.aliases {
                    ledger_insert(
                        connection,
                        "strict_term_aliases",
                        &alias.id.to_string(),
                        "skipped",
                        reason,
                        None,
                        None,
                        None,
                        None,
                    )?;
                }
                for tag in &entry.tags {
                    ledger_insert(
                        connection,
                        "strict_term_tags",
                        &format!("{}:{tag}", entry.term.id),
                        "skipped",
                        reason,
                        None,
                        None,
                        None,
                        None,
                    )?;
                }
                continue;
            }
        };

        let concept_id = find_concept_id(
            connection,
            entry.term.series_slug.as_str(),
            &entry.term.source_text,
        )?;
        let sentence = rule_text(
            &entry.term.source_text,
            &entry.term.canonical_translation,
            &planned.forbidden,
        );
        let rule_crystal_id = find_rule_crystal_id(connection, entry, &sentence)?;

        let rule_id = match insert_rule(connection, entry, &planned, concept_id, rule_crystal_id) {
            Ok(rule_id) => rule_id,
            Err(_) => {
                record_blocking(connection, entry, &mut report, REASON_INSERT_FAILED)?;
                continue;
            }
        };

        report.converted += 1;
        report.forms_inserted += insert_forms(connection, entry, rule_id, &mut report)?;
        report.tags_inserted += insert_tags(connection, entry, rule_id, &mut report)?;
        ledger_insert(
            connection,
            "strict_terms",
            &entry.term.id.to_string(),
            "converted",
            REASON_OK,
            Some(rule_id),
            None,
            concept_id,
            rule_crystal_id,
        )?;
        report.converted_terms.push(ConvertedTermLink {
            legacy_term_id: entry.term.id,
            rule_id,
            concept_id,
            rule_crystal_id,
        });
    }

    report.ledger_rows =
        connection.query_row(&format!("select count(*) from {LEDGER_TABLE}"), [], |row| {
            row.get::<_, i64>(0)
        })? as u64;
    Ok(report)
}

impl TermConversionReport {
    fn empty() -> Self {
        Self {
            source_terms: 0,
            source_aliases: 0,
            source_tags: 0,
            converted: 0,
            skipped: 0,
            blocking: 0,
            skip_reasons: BTreeMap::new(),
            forms_inserted: 0,
            tags_inserted: 0,
            ledger_rows: 0,
            converted_terms: Vec::new(),
            skipped_terms: Vec::new(),
            blocking_rows: Vec::new(),
            planned: false,
        }
    }
}

fn require_legacy_schema(connection: &Connection) -> Result<(), MigrateError> {
    for (table, _) in LEGACY_FINGERPRINT {
        let present: i64 = connection.query_row(
            "select count(*) from sqlite_master where type = 'table' and name = ?1",
            [table],
            |row| row.get(0),
        )?;
        if present == 0 {
            return Err(MigrateError::UnsupportedLegacySchema(format!(
                "missing table {table}"
            )));
        }
    }
    Ok(())
}

fn require_target_schema(connection: &Connection) -> Result<(), MigrateError> {
    for table in ["term_rules", "term_rule_forms", "term_rule_semantic_tags"] {
        let present: i64 = connection.query_row(
            "select count(*) from sqlite_master where type = 'table' and name = ?1",
            [table],
            |row| row.get(0),
        )?;
        if present == 0 {
            return Err(MigrateError::TargetSchemaMissing(format!(
                "missing table {table}; run the target-schema SQL steps first"
            )));
        }
    }
    Ok(())
}

fn ensure_not_already_converted(connection: &Connection) -> Result<(), MigrateError> {
    let ledger_present: i64 = connection.query_row(
        "select count(*) from sqlite_master where type = 'table' and name = ?1",
        [LEDGER_TABLE],
        |row| row.get(0),
    )?;
    if ledger_present > 0 {
        let rows: i64 =
            connection.query_row(&format!("select count(*) from {LEDGER_TABLE}"), [], |row| {
                row.get(0)
            })?;
        if rows > 0 {
            return Err(MigrateError::AlreadyConverted);
        }
    }
    let rules: i64 =
        connection.query_row("select count(*) from term_rules", [], |row| row.get(0))?;
    if rules > 0 {
        return Err(MigrateError::AlreadyConverted);
    }
    Ok(())
}

/// Read every legacy term with its aliases and tags. A term whose row cannot
/// be read as the typed record (corrupt cell) becomes a blocking outcome and
/// its child rows are blocked with it; reading continues with the rest. The
/// returned failures list carries every blocking outcome for ledger writing.
type SourceEntries = (Vec<SourceEntry>, Vec<SkippedRow>);

fn read_source_entries(
    connection: &Connection,
    report: &mut TermConversionReport,
) -> Result<SourceEntries, MigrateError> {
    let mut ids_statement = connection.prepare("select id from strict_terms order by id")?;
    let ids: Vec<i64> = ids_statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<_>>()?;
    report.source_terms = ids.len() as u64;
    report.source_aliases =
        connection.query_row("select count(*) from strict_term_aliases", [], |row| {
            row.get::<_, i64>(0)
        })? as u64;
    report.source_tags =
        connection.query_row("select count(*) from strict_term_tags", [], |row| {
            row.get::<_, i64>(0)
        })? as u64;

    let mut entries = Vec::new();
    let mut failures = Vec::new();
    for id in ids {
        match read_term(connection, id) {
            Ok(term) => {
                let aliases = read_aliases(connection, id, report, &mut failures)?;
                let tags = read_tags(connection, id, report, &mut failures)?;
                entries.push(SourceEntry {
                    term,
                    aliases,
                    tags,
                });
            }
            Err(_) => {
                report.blocking += 1;
                failures.push(SkippedRow {
                    source_table: "strict_terms".to_string(),
                    source_id: id.to_string(),
                    reason_code: REASON_INVALID_VALUE.to_string(),
                });
                for alias_id in alias_ids(connection, id) {
                    report.blocking += 1;
                    failures.push(SkippedRow {
                        source_table: "strict_term_aliases".to_string(),
                        source_id: alias_id.to_string(),
                        reason_code: REASON_INVALID_VALUE.to_string(),
                    });
                }
                for tag_id in blocked_term_tag_ids(connection, id) {
                    report.blocking += 1;
                    failures.push(SkippedRow {
                        source_table: "strict_term_tags".to_string(),
                        source_id: tag_id,
                        reason_code: REASON_INVALID_VALUE.to_string(),
                    });
                }
            }
        }
    }
    Ok((entries, failures))
}

fn read_term(connection: &Connection, id: i64) -> rusqlite::Result<LegacyTerm> {
    connection.query_row(
        "select series_slug, source_language, target_language, source_text,
                canonical_translation, status, notes, created_at, updated_at
         from strict_terms where id = ?1",
        [id],
        |row| {
            Ok(LegacyTerm {
                id,
                series_slug: row.get(0)?,
                source_language: row.get(1)?,
                target_language: row.get(2)?,
                source_text: row.get(3)?,
                canonical_translation: row.get(4)?,
                status: row.get(5)?,
                notes: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
}

fn alias_ids(connection: &Connection, term_id: i64) -> Vec<i64> {
    let mut statement = match connection
        .prepare("select id from strict_term_aliases where term_id = ?1 order by id")
    {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    match statement
        .query_map([term_id], |row| row.get::<_, i64>(0))
        .map(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    {
        Ok(rows) => rows.unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Ledger ids for the tags of a blocked term, so every source row still
/// reaches the ledger even when its parent row is unreadable.
fn blocked_term_tag_ids(connection: &Connection, term_id: i64) -> Vec<String> {
    let mut statement = match connection
        .prepare("select tag from strict_term_tags where term_id = ?1 order by tag")
    {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let mut rows = match statement.query([term_id]) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    let mut ids = Vec::new();
    let mut ordinal = 0;
    while let Some(row) = rows.next().unwrap_or(None) {
        ordinal += 1;
        match row.get::<_, String>(0) {
            Ok(tag) => ids.push(format!("{term_id}:{tag}")),
            Err(_) => ids.push(format!("{term_id}:tag-{ordinal}")),
        }
    }
    ids
}

fn read_aliases(
    connection: &Connection,
    term_id: i64,
    report: &mut TermConversionReport,
    failures: &mut Vec<SkippedRow>,
) -> Result<Vec<LegacyAlias>, MigrateError> {
    let mut statement = connection.prepare(
        "select id, language, text, kind, case_sensitive
         from strict_term_aliases where term_id = ?1 order by id",
    )?;
    let mut rows = statement.query([term_id])?;
    let mut aliases = Vec::new();
    let mut ordinal = 0;
    while let Some(row) = rows.next()? {
        ordinal += 1;
        match (|| -> rusqlite::Result<LegacyAlias> {
            Ok(LegacyAlias {
                id: row.get(0)?,
                language: row.get(1)?,
                text: row.get(2)?,
                kind: row.get(3)?,
                case_sensitive: row.get(4)?,
            })
        })() {
            Ok(alias) => aliases.push(alias),
            Err(_) => {
                report.blocking += 1;
                failures.push(SkippedRow {
                    source_table: "strict_term_aliases".to_string(),
                    source_id: format!("{term_id}:alias-{ordinal}"),
                    reason_code: REASON_INVALID_VALUE.to_string(),
                });
            }
        }
    }
    Ok(aliases)
}

fn read_tags(
    connection: &Connection,
    term_id: i64,
    report: &mut TermConversionReport,
    failures: &mut Vec<SkippedRow>,
) -> Result<Vec<String>, MigrateError> {
    let mut statement =
        connection.prepare("select tag from strict_term_tags where term_id = ?1 order by tag")?;
    let mut rows = statement.query([term_id])?;
    let mut tags = Vec::new();
    let mut ordinal = 0;
    while let Some(row) = rows.next()? {
        ordinal += 1;
        match row.get::<_, String>(0) {
            Ok(tag) => tags.push(tag),
            Err(_) => {
                report.blocking += 1;
                failures.push(SkippedRow {
                    source_table: "strict_term_tags".to_string(),
                    source_id: format!("{term_id}:tag-{ordinal}"),
                    reason_code: REASON_INVALID_VALUE.to_string(),
                });
            }
        }
    }
    Ok(tags)
}

/// Link a concept only when the legacy memory graph has exactly one
/// unambiguous match (canonical name, series scope, live status) — the same
/// deterministic predicate the legacy migration used to name concepts.
fn find_concept_id(
    connection: &Connection,
    series_slug: &str,
    source_text: &str,
) -> Result<Option<i64>, MigrateError> {
    let mut statement = connection.prepare(
        "select id from concepts
         where canonical_name = ?1 and scope_type = 'series' and scope_key = ?2
           and status not in ('archived', 'merged')
         order by id",
    )?;
    let ids: Vec<i64> = statement
        .query_map(
            rusqlite::params![source_text, format!("series:{series_slug}")],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(if ids.len() == 1 { Some(ids[0]) } else { None })
}

/// Link the searchable rule-crystal projection when the legacy database
/// already carries one with the exact rule sentence and scope.
fn find_rule_crystal_id(
    connection: &Connection,
    entry: &SourceEntry,
    text: &str,
) -> Result<Option<i64>, MigrateError> {
    let term = &entry.term;
    let mut statement = connection.prepare(
        "select id from crystals
         where crystal_type = 'rule' and status = 'active' and text = ?1
           and scope_type = 'series' and scope_key = ?2 and series_slug = ?3
           and source_language = ?4 and target_language = ?5
         order by id limit 1",
    )?;
    let id = statement
        .query_row(
            rusqlite::params![
                text,
                format!("series:{}", term.series_slug),
                term.series_slug,
                term.source_language,
                term.target_language
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(id)
}

fn insert_rule(
    connection: &Connection,
    entry: &SourceEntry,
    planned: &PlannedTerm,
    concept_id: Option<i64>,
    rule_crystal_id: Option<i64>,
) -> rusqlite::Result<i64> {
    let term = &entry.term;
    let forbidden_json =
        serde_json::to_string(&planned.forbidden).unwrap_or_else(|_| "[]".to_string());
    connection.execute(
        "insert into term_rules(
           concept_id, source_language, target_language, source_text,
           canonical_translation, forbidden_variants_json, matching_policy,
           status, provenance, notes, revision, rule_crystal_id,
           created_at, updated_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, 'surface', ?7, ?8, ?9, 1, ?10, ?11, ?12)",
        rusqlite::params![
            concept_id,
            term.source_language,
            term.target_language,
            term.source_text,
            term.canonical_translation,
            forbidden_json,
            planned.status,
            CONVERTED_PROVENANCE,
            term.notes,
            rule_crystal_id,
            term.created_at,
            term.updated_at,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn insert_one_form(
    connection: &Connection,
    rule_id: i64,
    kind: &str,
    surface: &str,
    language: &str,
    case_sensitive: i64,
) -> rusqlite::Result<i64> {
    connection.execute(
        "insert into term_rule_forms(rule_id, form_kind, surface, language, case_sensitive)
         values (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![rule_id, kind, surface, language, case_sensitive],
    )?;
    Ok(connection.last_insert_rowid())
}

/// Insert the source form, the canonical approved form, and one form per
/// non-blank alias with its language, kind and case-sensitivity preserved.
/// Returns the number of forms written. A failed form insert becomes a
/// blocking ledger row instead of aborting the conversion.
fn insert_forms(
    connection: &Connection,
    entry: &SourceEntry,
    rule_id: i64,
    report: &mut TermConversionReport,
) -> Result<u64, MigrateError> {
    let term = &entry.term;
    let mut written = 0_u64;
    for (kind, surface, language) in [
        (
            "source",
            term.source_text.as_str(),
            term.source_language.as_str(),
        ),
        (
            "approved",
            term.canonical_translation.as_str(),
            term.target_language.as_str(),
        ),
    ] {
        match insert_one_form(connection, rule_id, kind, surface, language, 0) {
            Ok(_) => written += 1,
            Err(_) => {
                report.blocking += 1;
                report.blocking_rows.push(SkippedRow {
                    source_table: "strict_terms".to_string(),
                    source_id: format!("{}:{kind}", term.id),
                    reason_code: REASON_INSERT_FAILED.to_string(),
                });
            }
        }
    }
    for alias in &entry.aliases {
        if alias.text.trim().is_empty() {
            // Blank aliases were ignored by the legacy contract as well; the
            // row still reaches the ledger as a deliberate skip.
            ledger_insert(
                connection,
                "strict_term_aliases",
                &alias.id.to_string(),
                "skipped",
                REASON_BLANK_ALIAS,
                Some(rule_id),
                None,
                None,
                None,
            )?;
            continue;
        }
        let kind = match alias.kind.as_str() {
            "approved_variant" => "approved",
            _ => "forbidden",
        };
        match insert_one_form(
            connection,
            rule_id,
            kind,
            &alias.text,
            &alias.language,
            alias.case_sensitive,
        ) {
            Ok(form_id) => {
                written += 1;
                ledger_insert(
                    connection,
                    "strict_term_aliases",
                    &alias.id.to_string(),
                    "converted",
                    REASON_OK,
                    Some(rule_id),
                    Some(form_id),
                    None,
                    None,
                )?;
            }
            Err(_) => {
                report.blocking += 1;
                report.blocking_rows.push(SkippedRow {
                    source_table: "strict_term_aliases".to_string(),
                    source_id: alias.id.to_string(),
                    reason_code: REASON_INSERT_FAILED.to_string(),
                });
                ledger_insert(
                    connection,
                    "strict_term_aliases",
                    &alias.id.to_string(),
                    "blocking",
                    REASON_INSERT_FAILED,
                    Some(rule_id),
                    None,
                    None,
                    None,
                )?;
            }
        }
    }
    Ok(written)
}

fn insert_tags(
    connection: &Connection,
    entry: &SourceEntry,
    rule_id: i64,
    report: &mut TermConversionReport,
) -> Result<u64, MigrateError> {
    let mut written = 0_u64;
    for tag in &entry.tags {
        let inserted = connection.execute(
            "insert into term_rule_semantic_tags(rule_id, tag) values (?1, ?2)",
            rusqlite::params![rule_id, tag],
        );
        match inserted {
            Ok(_) => {
                written += 1;
                ledger_insert(
                    connection,
                    "strict_term_tags",
                    &format!("{}:{tag}", entry.term.id),
                    "converted",
                    REASON_OK,
                    Some(rule_id),
                    None,
                    None,
                    None,
                )?;
            }
            Err(_) => {
                report.blocking += 1;
                report.blocking_rows.push(SkippedRow {
                    source_table: "strict_term_tags".to_string(),
                    source_id: format!("{}:{tag}", entry.term.id),
                    reason_code: REASON_INSERT_FAILED.to_string(),
                });
                ledger_insert(
                    connection,
                    "strict_term_tags",
                    &format!("{}:{tag}", entry.term.id),
                    "blocking",
                    REASON_INSERT_FAILED,
                    Some(rule_id),
                    None,
                    None,
                    None,
                )?;
            }
        }
    }
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
fn ledger_insert(
    connection: &Connection,
    source_table: &str,
    source_id: &str,
    outcome: &str,
    reason_code: &str,
    target_rule_id: Option<i64>,
    target_form_id: Option<i64>,
    legacy_concept_id: Option<i64>,
    legacy_crystal_id: Option<i64>,
) -> rusqlite::Result<()> {
    connection.execute(
        &format!(
            "insert or replace into {LEDGER_TABLE}(
               source_table, source_id, outcome, reason_code,
               target_rule_id, target_form_id, legacy_concept_id, legacy_crystal_id
             ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        ),
        rusqlite::params![
            source_table,
            source_id,
            outcome,
            reason_code,
            target_rule_id,
            target_form_id,
            legacy_concept_id,
            legacy_crystal_id
        ],
    )?;
    Ok(())
}

/// Record a blocking outcome for a term whose rule insert failed, cascading
/// to its child rows so every source row still reaches the ledger.
fn record_blocking(
    connection: &Connection,
    entry: &SourceEntry,
    report: &mut TermConversionReport,
    reason: &str,
) -> Result<(), MigrateError> {
    report.blocking += 1;
    report.blocking_rows.push(SkippedRow {
        source_table: "strict_terms".to_string(),
        source_id: entry.term.id.to_string(),
        reason_code: reason.to_string(),
    });
    ledger_insert(
        connection,
        "strict_terms",
        &entry.term.id.to_string(),
        "blocking",
        reason,
        None,
        None,
        None,
        None,
    )?;
    for alias in &entry.aliases {
        report.blocking += 1;
        ledger_insert(
            connection,
            "strict_term_aliases",
            &alias.id.to_string(),
            "blocking",
            reason,
            None,
            None,
            None,
            None,
        )?;
    }
    for tag in &entry.tags {
        report.blocking += 1;
        ledger_insert(
            connection,
            "strict_term_tags",
            &format!("{}:{tag}", entry.term.id),
            "blocking",
            reason,
            None,
            None,
            None,
            None,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Target-schema steps, FTS rebuild, verification
// ---------------------------------------------------------------------------

/// Apply the upgrade's target-schema SQL steps (terminology rule tables plus
/// the schema-version metadata) through the caller's connection. Idempotent;
/// the caller owns the transaction and the commit.
pub fn prepare_upgrade_target(connection: &Connection) -> Result<(), MigrateError> {
    Ok(apply_terminology_schema_steps(connection)?)
}

/// Rebuild the external-content strict-terms FTS projection from the
/// authoritative preserved rows (the upgrade protocol's FTS step).
pub fn rebuild_strict_terms_fts(connection: &Connection) -> Result<(), MigrateError> {
    connection.execute(
        "insert into strict_terms_fts(strict_terms_fts) values ('rebuild')",
        [],
    )?;
    Ok(())
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, MigrateError> {
    use sha2::Digest;
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", sha2::Sha256::digest(&bytes)))
}

/// Verify FTS query equivalence on the converted target: every converted
/// term's source surface resolves its own row, and the projection covers
/// exactly the preserved rows.
pub(crate) fn verify_fts_equivalence(
    connection: &Connection,
    conversion: &TermConversionReport,
) -> Result<(bool, u64), MigrateError> {
    let mut probes = 0_u64;
    for link in &conversion.converted_terms {
        let source_text: String = connection.query_row(
            "select source_text from term_rules where id = ?1",
            [link.rule_id],
            |row| row.get(0),
        )?;
        let ids = fts_probe(connection, &source_text)?;
        probes += 1;
        if !ids.contains(&link.legacy_term_id) {
            return Ok((false, probes));
        }
    }
    let indexed: i64 =
        connection.query_row("select count(*) from strict_terms_fts", [], |row| {
            row.get(0)
        })?;
    let preserved: i64 =
        connection.query_row("select count(*) from strict_terms", [], |row| row.get(0))?;
    Ok((indexed == preserved, probes))
}

fn fts_probe(connection: &Connection, term: &str) -> Result<Vec<i64>, MigrateError> {
    let quoted = format!("\"{}\"", term.replace('"', "\"\""));
    let mut statement = connection.prepare(
        "select rowid from strict_terms_fts where strict_terms_fts match ?1 order by rowid",
    )?;
    let ids = statement
        .query_map([quoted], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Removes its directory on drop unless disarmed: a failed dry-run leaves no
/// filesystem trace.
struct DirGuard {
    path: PathBuf,
    armed: bool,
}

impl DirGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------

/// `hiero migrate --dry-run`: preflight, then the full typed conversion and
/// verification against a disposable copy of the source in the default
/// temporary directory. Writes neither the source nor any authoritative
/// state; the copy is removed after reporting.
pub fn run_dry_run(
    config: &HieronymusConfig,
    daemon_active: bool,
) -> Result<DryRunReport, MigrateError> {
    run_dry_run_in(
        config,
        daemon_active,
        &std::env::temp_dir().join("hiero-migrate-dry-run"),
    )
}

/// Dry-run with an explicit work root (which must live outside the data
/// root). Every artifact is created beneath a unique run directory inside it
/// and removed after reporting, including on failure.
pub fn run_dry_run_in(
    config: &HieronymusConfig,
    daemon_active: bool,
    work_root: &Path,
) -> Result<DryRunReport, MigrateError> {
    let preflight = run_preflight(config, daemon_active)?;
    let mut report = DryRunReport {
        preflight,
        refused: None,
        conversion: None,
        verification: None,
        temp_artifacts_removed: false,
    };

    // Fail closed before any target exists: the classifier result gates the
    // dry-run exactly like it gates the real upgrade.
    if let Some(code) = report.preflight.refusal_code.clone() {
        report.refused = Some(code);
        return Ok(report);
    }
    if daemon_active {
        report.refused = Some(REFUSAL_DAEMON_ACTIVE.to_string());
        return Ok(report);
    }
    if work_root.starts_with(config.data_root()) {
        return Err(MigrateError::InvalidWorkRoot);
    }

    let run_dir = work_root.join(format!(
        "dry-run-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&run_dir)?;
    let guard = DirGuard::new(run_dir.clone());

    // Byte-identical disposable copy; the source is only ever read.
    let source = config.database_path();
    let source_digest = sha256_file(&source)?;
    let target = run_dir.join("hieronymus.sqlite");
    std::fs::copy(&source, &target)?;
    if sha256_file(&target)? != source_digest {
        return Err(MigrateError::SourceChanged);
    }

    let mut connection = Connection::open(&target)?;
    connection.execute_batch("pragma foreign_keys = on;")?;

    // Rehearse the upgrade's target-schema SQL steps.
    {
        let transaction = connection.transaction()?;
        apply_terminology_schema_steps(&transaction)?;
        transaction.commit()?;
    }

    // Rehearse the typed conversion and the FTS rebuild through one
    // transaction handle; verification then runs over the committed shape
    // exactly as the upgrade verifies its committed target.
    let conversion = {
        let transaction = connection.transaction()?;
        let conversion = convert_strict_terms(&transaction)?;
        rebuild_strict_terms_fts(&transaction)?;
        transaction.commit()?;
        conversion
    };

    let verification = verify_upgraded_target(&connection, &conversion)?;
    let verification_ok = verification.all_checks_pass();

    report.conversion = Some(conversion);
    report.verification = Some(verification);
    // The conversion-safe verdict is only trustworthy after verification: a
    // dry-run whose verification fails is refused (`verification-failed`) and
    // its verdict flips to unsafe, so scripting gets both a nonzero exit and
    // an honest report. Per-row ledger outcomes (skips/blocking) stay
    // reported data — the write-side protocol owns the upgrade gate.
    if !verification_ok {
        report.preflight.conversion_safe = false;
        report.preflight.refusal_code = Some(REFUSAL_VERIFICATION_FAILED.to_string());
        report.refused = Some(REFUSAL_VERIFICATION_FAILED.to_string());
    }

    drop(connection);
    guard.disarm();
    std::fs::remove_dir_all(&run_dir)?;
    report.temp_artifacts_removed = true;
    Ok(report)
}

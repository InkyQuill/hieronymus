//! Upgrade-tooling behavior (database-upgrade design, 2026-08-31): read-only
//! preflight, the strict-terms converter, and the disposable dry-run.
//! Classification/refusal shapes follow the qualified legacy-import harness
//! approaches without linking it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::migrate::{
    MigrateError, PreflightReport, convert_strict_terms, plan_strict_term_conversion,
    prepare_upgrade_target, run_dry_run_in, run_preflight,
};
use hieronymus::terminology::{Source, Termbase};
use rusqlite::Connection;

const GLOBAL_SQL: &str = include_str!("../migrations/global.sql");

const PROVIDER_SENTINEL: &str = "sk-sentinel-api-key-7f3a";
const MEMORY_SENTINEL: &str = "облигация-эскадрон-сакура";
const TERM_TEXT_SENTINEL: &str = "тактическое чутьё-маркер";

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn write_legacy_fixture(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let path = root.join("hieronymus.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(GLOBAL_SQL).unwrap();
    connection.close().unwrap();
    path
}

fn open(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
}

fn insert_series(connection: &Connection, slug: &str) {
    connection
        .execute(
            "insert into series(slug, title, default_source_language,
                                default_target_language, created_at, updated_at)
             values (?1, ?1, 'ja', 'en', '2026-01-01T00:00:00+00:00',
                     '2026-01-01T00:00:00+00:00')",
            [slug],
        )
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_term(
    connection: &Connection,
    series_slug: &str,
    source_text: &str,
    canonical: &str,
    status: &str,
    notes: &str,
    created_at: &str,
) -> i64 {
    connection
        .execute(
            "insert into strict_terms(series_slug, source_language, target_language,
                                      category, source_text, canonical_translation,
                                      status, notes, created_at, updated_at)
             values (?1, 'ja', 'en', 'name', ?2, ?3, ?4, ?5, ?6, ?6)",
            rusqlite::params![
                series_slug,
                source_text,
                canonical,
                status,
                notes,
                created_at
            ],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn insert_alias(
    connection: &Connection,
    term_id: i64,
    language: &str,
    text: &str,
    kind: &str,
    case_sensitive: i64,
) -> i64 {
    connection
        .execute(
            "insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
             values (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![term_id, language, text, kind, case_sensitive],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn insert_tag(connection: &Connection, term_id: i64, tag: &str) {
    connection
        .execute(
            "insert into strict_term_tags(term_id, tag) values (?1, ?2)",
            rusqlite::params![term_id, tag],
        )
        .unwrap();
}

fn insert_fts_row(
    connection: &Connection,
    term_id: i64,
    source: &str,
    canonical: &str,
    notes: &str,
) {
    connection
        .execute(
            "insert into strict_terms_fts(rowid, source_text, canonical_translation, notes)
             values (?1, ?2, ?3, ?4)",
            rusqlite::params![term_id, source, canonical, notes],
        )
        .unwrap();
}

fn insert_matching_concept(connection: &Connection, source_text: &str) -> i64 {
    connection
        .execute(
            "insert into concepts(canonical_name, description, scope_type, scope_key,
                                  status, confidence, created_at, updated_at)
             values (?1, '', 'series', 'series:demo', 'established', 0.95,
                     '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            [source_text],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn insert_matching_rule_crystal(
    connection: &Connection,
    text: &str,
    series_slug: &str,
    source_language: &str,
    target_language: &str,
) -> i64 {
    connection
        .execute(
            "insert into crystals(crystal_type, text, title, scope_type, scope_key,
                                  series_slug, source_language, target_language, tags_json,
                                  strength, confidence, source_credibility, rule_intent,
                                  malformed_penalty, supersedes_crystal_id, status,
                                  created_at, updated_at)
             values ('rule', ?1, '', 'series', ?2, ?3, ?4, ?5, '[]', 0.8, 0.95,
                     'user_rule', '', 0.0, null, 'active',
                     '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            rusqlite::params![
                text,
                format!("series:{series_slug}"),
                series_slug,
                source_language,
                target_language
            ],
        )
        .unwrap();
    connection.last_insert_rowid()
}

fn rule_text(source: &str, canonical: &str, forbidden: Option<&str>) -> String {
    match forbidden {
        Some(forbidden) => format!("{source} is translated as {canonical}, not {forbidden}."),
        None => format!("{source} is translated as {canonical}."),
    }
}

fn query_scalar(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap()
}

fn file_tree_digest(root: &Path) -> String {
    use sha2::Digest;
    let mut entries: Vec<PathBuf> = Vec::new();
    fn walk(dir: &Path, entries: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, entries);
            } else {
                entries.push(path);
            }
        }
    }
    walk(root, &mut entries);
    entries.sort();
    let mut digest = sha2::Sha256::new();
    for path in entries {
        use std::fmt::Write;
        let mut line = path.to_string_lossy().to_string();
        line.push(':');
        let bytes = std::fs::read(&path).unwrap();
        let hex: String =
            sha2::Sha256::digest(&bytes)
                .iter()
                .fold(String::new(), |mut out, byte| {
                    write!(out, "{byte:02x}").unwrap();
                    out
                });
        line.push_str(&hex);
        line.push('\n');
        digest.update(line.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn term_report(report: &PreflightReport) -> &hieronymus::migrate::TermConversionReport {
    report.plan.as_ref().expect("conversion plan present")
}

fn dir_entry_count(root: &Path) -> usize {
    std::fs::read_dir(root).unwrap().count()
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

#[test]
fn preflight_refuses_each_unsupported_state_fail_closed() {
    // Empty: no database at all.
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert!(!report.conversion_safe);
    assert_eq!(report.refusal_code.as_deref(), Some("empty-database"));
    assert_eq!(report.detected_state, "empty");

    // Corrupt: a file that is not a database.
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hieronymus.sqlite"), b"not a database").unwrap();
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert_eq!(report.refusal_code.as_deref(), Some("source-unreadable"));
    assert_eq!(report.detected_state, "corrupt");

    // Unknown schema: tables that match no known schema.
    let root = tempfile::tempdir().unwrap();
    let connection = Connection::open(root.path().join("hieronymus.sqlite")).unwrap();
    connection
        .execute(
            "create table unrelated_application (id integer primary key)",
            [],
        )
        .unwrap();
    drop(connection);
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert_eq!(report.refusal_code.as_deref(), Some("unknown-schema"));

    // Newer Rust schema fails closed.
    let root = tempfile::tempdir().unwrap();
    let connection = open(&write_legacy_fixture(root.path()));
    connection
        .execute_batch(
            "create table hieronymus_meta (schema_version integer not null unique);
             insert into hieronymus_meta (schema_version) values (99);",
        )
        .unwrap();
    drop(connection);
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert_eq!(report.refusal_code.as_deref(), Some("newer-schema"));
    assert_eq!(report.detected_schema_version, Some(99));

    // Current Rust schema: nothing to convert.
    let root = tempfile::tempdir().unwrap();
    hieronymus::db::open_migrated(&root.path().join("hieronymus.sqlite")).unwrap();
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert_eq!(report.refusal_code.as_deref(), Some("already-current"));
    assert_eq!(report.detected_state, "rust-schema");
}

#[test]
fn preflight_reports_supported_legacy_schema_and_counts() {
    let root = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(root.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    let first = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "core concept",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, first, "en", "talent", "forbidden_variant", 1);
    insert_tag(&connection, first, "narrative");
    insert_fts_row(&connection, first, "センス", "sense", "core concept");
    insert_term(
        &connection,
        "demo",
        " Grey Admiral ",
        "Серый Адмирал",
        "pending",
        "",
        "2026-02-02T00:00:00+00:00",
    );
    drop(connection);

    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert!(report.conversion_safe, "{report:?}");
    assert_eq!(report.refusal_code, None);
    assert_eq!(report.detected_state, "python-schema");
    assert_eq!(report.integrity, "ok");
    assert_eq!(report.foreign_key_violations, 0);
    assert_eq!(report.table_counts.get("strict_terms"), Some(&2));
    assert_eq!(report.pending_conversions.strict_terms, 2);
    assert_eq!(report.pending_conversions.strict_term_aliases, 1);
    assert_eq!(report.pending_conversions.strict_term_tags, 1);
    assert_eq!(report.target_schema_version, 1);
    assert!(report.backup_path.is_none());
    assert!(report.database_size_bytes > 0);
    assert!(report.required_free_space_bytes >= report.database_size_bytes);
    assert!(!report.daemon_active);
    let plan = term_report(&report);
    assert_eq!(plan.converted, 1);
    assert_eq!(plan.skipped, 1);
    assert_eq!(plan.blocking, 0);
    assert_eq!(plan.skip_reasons.get("rule-shape-round-trip"), Some(&1));
}

#[test]
fn preflight_report_redacts_secrets_and_memory_text() {
    let root = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(root.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    let term = insert_term(
        &connection,
        "demo",
        TERM_TEXT_SENTINEL,
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_fts_row(&connection, term, TERM_TEXT_SENTINEL, "sense", "");
    // A scratch table holding unknown user text: the preflight counts the
    // table but never reads row values, so the sentinel must not surface.
    connection
        .execute("create table scratch_note (text text)", [])
        .unwrap();
    connection
        .execute(
            "insert into scratch_note(text) values (?1)",
            [MEMORY_SENTINEL],
        )
        .unwrap();
    drop(connection);
    std::fs::write(
        root.path().join("provider.conf"),
        format!("api_key = \"{PROVIDER_SENTINEL}\"\n"),
    )
    .unwrap();

    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(PROVIDER_SENTINEL), "{rendered}");
    assert!(!rendered.contains(MEMORY_SENTINEL), "{rendered}");
    assert!(!rendered.contains(TERM_TEXT_SENTINEL), "{rendered}");
}

#[test]
fn preflight_flags_fingerprint_and_integrity_failures() {
    // A legacy database whose strict_term_aliases table lacks the `kind`
    // column fails the exact-column fingerprint and is refused.
    let root = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(root.path());
    let connection = open(&path);
    connection
        .execute("drop table strict_term_aliases", [])
        .unwrap();
    connection
        .execute(
            "create table strict_term_aliases (
                 id integer primary key,
                 term_id integer not null references strict_terms(id) on delete cascade,
                 language text not null,
                 text text not null,
                 case_sensitive integer not null default 1
             )",
            [],
        )
        .unwrap();
    insert_series(&connection, "demo");
    insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    drop(connection);
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert!(!report.conversion_safe);
    assert_eq!(
        report.refusal_code.as_deref(),
        Some("unsupported-legacy-schema")
    );

    // A foreign-key violation in the source refuses conversion.
    let root = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(root.path());
    let connection = open(&path);
    // Bypass foreign keys to plant a dangling term reference.
    connection
        .execute_batch("pragma foreign_keys = off;")
        .unwrap();
    insert_series(&connection, "demo");
    insert_term(
        &connection,
        "no-such-series",
        "センス",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    drop(connection);
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, false).unwrap();
    assert!(!report.conversion_safe);
    assert_eq!(
        report.refusal_code.as_deref(),
        Some("foreign-key-violations")
    );
    assert!(report.foreign_key_violations > 0);
}

#[test]
fn preflight_reports_active_daemon_fact() {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    let config = HieronymusConfig::new(root.path());
    let report = run_preflight(&config, true).unwrap();
    assert!(report.daemon_active);
    // Reporting only in part 1: the daemon fact does not flip data safety.
    assert!(report.conversion_safe);
}

// ---------------------------------------------------------------------------
// Converter
// ---------------------------------------------------------------------------

fn rich_fixture(root: &Path) -> Connection {
    let path = write_legacy_fixture(root);
    let connection = open(&path);
    insert_series(&connection, "demo");
    // T1: approved term with forbidden alias, tag, concept and crystal links.
    let t1 = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "core narrative concept",
        "2026-02-01T10:00:00+00:00",
    );
    let alias = insert_alias(&connection, t1, "en", "talent", "forbidden_variant", 1);
    insert_tag(&connection, t1, "narrative");
    insert_fts_row(&connection, t1, "センス", "sense", "core narrative concept");
    let concept = insert_matching_concept(&connection, "センス");
    let crystal = insert_matching_rule_crystal(
        &connection,
        &rule_text("センス", "sense", Some("talent")),
        "demo",
        "ja",
        "en",
    );
    // T2: pending term with an approved-variant alias equal to the canonical
    // rendering; no concept or crystal exists for it.
    let t2 = insert_term(
        &connection,
        "demo",
        "Grey Admiral",
        "Серый Адмирал",
        "pending",
        "",
        "2026-02-02T10:00:00+00:00",
    );
    let approved_alias = insert_alias(
        &connection,
        t2,
        "ru",
        "Серый Адмирал",
        "approved_variant",
        1,
    );
    insert_fts_row(&connection, t2, "Grey Admiral", "Серый Адмирал", "");
    let _ = (alias, approved_alias, concept, crystal);
    connection
}

fn prepared_copy(source_data_root: &Path, work: &Path) -> (tempfile::TempDir, Connection) {
    let target = tempfile::tempdir_in(work).unwrap();
    let path = target.path().join("hieronymus.sqlite");
    std::fs::copy(source_data_root.join("hieronymus.sqlite"), &path).unwrap();
    let connection = open(&path);
    prepare_upgrade_target(&connection).unwrap();
    (target, connection)
}

/// Convert inside a caller-owned transaction and commit; the converter itself
/// refuses autocommit connections.
fn convert_and_commit(connection: &mut Connection) -> hieronymus::migrate::TermConversionReport {
    let transaction = connection.transaction().unwrap();
    let report = convert_strict_terms(&transaction).unwrap();
    transaction.commit().unwrap();
    report
}

#[test]
fn converter_preserves_every_field() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let (_target, mut connection) = prepared_copy(data.path(), work.path());
    let legacy_t1: i64 = connection
        .query_row(
            "select id from strict_terms order by id limit 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let legacy_t2: i64 = connection
        .query_row(
            "select id from strict_terms order by id desc limit 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let legacy_alias_t1: i64 = connection
        .query_row(
            "select id from strict_term_aliases where term_id = ?1",
            [legacy_t1],
            |row| row.get(0),
        )
        .unwrap();

    let report = convert_and_commit(&mut connection);
    assert_eq!(report.converted, 2, "{report:?}");
    assert_eq!(report.skipped, 0);
    assert_eq!(report.blocking, 0);

    // Stable legacy identity: the ledger maps every source row to its target.
    let (rule_t1, concept, crystal): (i64, Option<i64>, Option<i64>) = connection
        .query_row(
            "select target_rule_id, legacy_concept_id, legacy_crystal_id
             from term_migration_ledger
             where source_table = 'strict_terms' and source_id = ?1",
            [legacy_t1.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let expected_concept: i64 = connection
        .query_row(
            "select id from concepts where canonical_name = 'センス'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(concept, Some(expected_concept));
    assert!(crystal.is_some());

    // term_rules row preserves languages, text, status, notes, provenance,
    // timestamps, matching policy and the stable identity links.
    #[derive(Debug)]
    struct RuleRow {
        source_language: String,
        target_language: String,
        source_text: String,
        canonical_translation: String,
        forbidden_json: String,
        status: String,
        provenance: String,
        notes: String,
        revision: i64,
        concept_id: Option<i64>,
        rule_crystal_id: Option<i64>,
    }
    let row: RuleRow = connection
        .query_row(
            "select source_language, target_language, source_text, canonical_translation,
                        forbidden_variants_json, status, provenance, notes, revision,
                        concept_id, rule_crystal_id
                 from term_rules where id = ?1",
            [rule_t1],
            |row| {
                Ok(RuleRow {
                    source_language: row.get(0)?,
                    target_language: row.get(1)?,
                    source_text: row.get(2)?,
                    canonical_translation: row.get(3)?,
                    forbidden_json: row.get(4)?,
                    status: row.get(5)?,
                    provenance: row.get(6)?,
                    notes: row.get(7)?,
                    revision: row.get(8)?,
                    concept_id: row.get(9)?,
                    rule_crystal_id: row.get(10)?,
                })
            },
        )
        .unwrap();
    assert_eq!(row.source_language, "ja");
    assert_eq!(row.target_language, "en");
    assert_eq!(row.source_text, "センス");
    assert_eq!(row.canonical_translation, "sense");
    assert_eq!(row.forbidden_json, "[\"talent\"]");
    assert_eq!(row.status, "active");
    assert_eq!(row.provenance, "migrated:strict_terms");
    assert_eq!(row.notes, "core narrative concept");
    assert_eq!(row.revision, 1);
    assert_eq!(row.concept_id, Some(expected_concept));
    assert_eq!(row.rule_crystal_id, crystal);

    // Forms carry the alias language, kind and case-sensitivity verbatim.
    let forms: Vec<(String, String, String, i64)> = connection
        .prepare("select form_kind, surface, language, case_sensitive from term_rule_forms where rule_id = ?1 order by form_kind, surface")
        .unwrap()
        .query_map([rule_t1], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        forms,
        vec![
            ("approved".into(), "sense".into(), "en".into(), 0),
            ("forbidden".into(), "talent".into(), "en".into(), 1),
            ("source".into(), "センス".into(), "ja".into(), 0),
        ]
    );

    // Alias ledger maps alias ids to form ids.
    let form_id: i64 = connection
        .query_row(
            "select target_form_id from term_migration_ledger
             where source_table = 'strict_term_aliases' and source_id = ?1",
            [legacy_alias_t1.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let surface: String = connection
        .query_row(
            "select surface from term_rule_forms where id = ?1",
            [form_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(surface, "talent");

    // Tags convert into term_rule_semantic_tags and are ledger-mapped.
    let tags: Vec<String> = connection
        .prepare("select tag from term_rule_semantic_tags where rule_id = ?1 order by tag")
        .unwrap()
        .query_map([rule_t1], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(tags, vec!["narrative".to_string()]);
    let tag_ledger: i64 = connection
        .query_row(
            "select count(*) from term_migration_ledger
             where source_table = 'strict_term_tags' and outcome = 'converted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tag_ledger, 1);

    // The pending term maps to candidate; its approved-variant alias converts.
    let status_t2: String = connection
        .query_row(
            "select tr.status from term_rules tr
             join term_migration_ledger l on l.target_rule_id = tr.id
             where l.source_table = 'strict_terms' and l.source_id = ?1",
            [legacy_t2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status_t2, "candidate");

    // The legacy rows themselves are preserved in the target copy.
    assert_eq!(
        query_scalar(&connection, "select count(*) from strict_terms"),
        2
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from strict_term_aliases"),
        2
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from strict_term_tags"),
        1
    );
}

#[test]
fn converter_ledger_records_skips_with_bounded_reasons() {
    let data = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(data.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    // Unsupported alias kind blocks the term.
    let a = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, a, "ja", "センス", "search_alias", 1);
    // Case-insensitive alias blocks the term.
    let b = insert_term(
        &connection,
        "demo",
        "talent",
        "gift",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, b, "en", "gifts", "forbidden_variant", 0);
    // Two forbidden variants block the term.
    let c = insert_term(
        &connection,
        "demo",
        "admiral",
        "адмирал",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, c, "ru", "враг", "forbidden_variant", 1);
    insert_alias(&connection, c, "ru", "противник", "forbidden_variant", 1);
    // Approved variant that differs from the canonical rendering blocks.
    let d = insert_term(
        &connection,
        "demo",
        "sensor",
        "сенсор",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, d, "ru", "датчик", "approved_variant", 1);
    // Unknown status blocks the term.
    insert_term(
        &connection,
        "demo",
        "ghost",
        "призрак",
        "retracted",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    // Blank source form blocks the term.
    insert_term(
        &connection,
        "demo",
        "   ",
        "пусто",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    // Rule text that cannot round-trip (contains the sentence separator).
    insert_term(
        &connection,
        "demo",
        "sense is translated as lie",
        "правда",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    drop(connection);

    let config = HieronymusConfig::new(data.path());
    let preflight = run_preflight(&config, false).unwrap();
    assert!(preflight.conversion_safe);
    let plan = term_report(&preflight);
    assert_eq!(plan.converted, 0);
    assert_eq!(plan.skipped, 7);
    let reasons: BTreeMap<&String, &u64> = plan.skip_reasons.iter().collect();
    assert_eq!(
        reasons.get(&"unsupported-alias-kind".to_string()),
        Some(&&1)
    );
    assert_eq!(
        reasons.get(&"case-insensitive-alias".to_string()),
        Some(&&1)
    );
    assert_eq!(
        reasons.get(&"too-many-forbidden-variants".to_string()),
        Some(&&1)
    );
    assert_eq!(
        reasons.get(&"approved-variant-mismatch".to_string()),
        Some(&&1)
    );
    assert_eq!(reasons.get(&"unknown-status".to_string()), Some(&&1));
    assert_eq!(reasons.get(&"empty-form".to_string()), Some(&&1));
    assert_eq!(reasons.get(&"rule-shape-round-trip".to_string()), Some(&&1));

    // Applied to a prepared copy, every skipped row still reaches the ledger.
    let work = tempfile::tempdir().unwrap();
    let (_target, mut connection) = prepared_copy(data.path(), work.path());
    let report = convert_and_commit(&mut connection);
    assert_eq!(report.converted, 0);
    assert_eq!(report.skipped, 7);
    let ledger_rows = query_scalar(&connection, "select count(*) from term_migration_ledger");
    let source_rows = query_scalar(&connection, "select count(*) from strict_terms")
        + query_scalar(&connection, "select count(*) from strict_term_aliases")
        + query_scalar(&connection, "select count(*) from strict_term_tags");
    assert_eq!(ledger_rows, source_rows);
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        0
    );
}

#[test]
fn converter_records_blocking_rows_without_stopping_others() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(data.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    let blocked = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, blocked, "en", "talent", "forbidden_variant", 1);
    insert_tag(&connection, blocked, "narrative");
    drop(connection);
    // Corrupt one cell to a blob the typed read cannot accept. SQLite stores
    // blobs in text-affinity columns as-is, so this survives without schema
    // surgery; the converter must isolate the row as blocking.
    let connection = open(&path);
    connection
        .execute("update strict_terms set notes = x'ff'", [])
        .unwrap();
    drop(connection);

    let (_target, mut connection) = prepared_copy(data.path(), work.path());
    let report = convert_and_commit(&mut connection);
    // The term and its child rows are all blocked, nothing else stops.
    assert_eq!(report.blocking, 3);
    assert_eq!(report.converted, 0);
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from term_migration_ledger where outcome = 'blocking' and reason_code = 'invalid-value'"
        ),
        3
    );
    // Every source row reached the ledger exactly once.
    let source_rows = query_scalar(&connection, "select count(*) from strict_terms")
        + query_scalar(&connection, "select count(*) from strict_term_aliases")
        + query_scalar(&connection, "select count(*) from strict_term_tags");
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_migration_ledger"),
        source_rows
    );
}

#[test]
fn converter_is_blocked_against_an_already_converted_target() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let (_target, mut connection) = prepared_copy(data.path(), work.path());
    convert_and_commit(&mut connection);
    let transaction = connection.transaction().unwrap();
    let second = convert_strict_terms(&transaction);
    assert!(
        matches!(second, Err(MigrateError::AlreadyConverted)),
        "{second:?}"
    );
}

#[test]
fn converter_refuses_autocommit_and_respects_the_caller_transaction() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let (target, mut connection) = prepared_copy(data.path(), work.path());
    let path = target.path().join("hieronymus.sqlite");

    // Autocommit connections are refused: the converter cannot commit behind
    // the caller's back because it never owns a transaction.
    let refused = convert_strict_terms(&connection);
    assert!(
        matches!(refused, Err(MigrateError::TransactionRequired)),
        "{refused:?}"
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        0
    );

    {
        let transaction = connection.transaction().unwrap();
        convert_strict_terms(&transaction).unwrap();
        // Deliberately dropped without commit: nothing may persist.
    }
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        0
    );
    let ledger_exists: i64 = connection
        .query_row(
            "select count(*) from sqlite_master where type = 'table' and name = 'term_migration_ledger'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ledger_exists, 0);
    std::mem::drop(connection);
    std::mem::drop(path);
}

// ---------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_no_filesystem_trace() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let config = HieronymusConfig::new(data.path());
    let before = file_tree_digest(data.path());

    let report = run_dry_run_in(&config, false, work.path()).unwrap();
    assert_eq!(report.refused, None, "{report:?}");
    assert!(report.temp_artifacts_removed);
    assert_eq!(file_tree_digest(data.path()), before, "data root changed");
    // The disposable work root holds no leftovers after reporting.
    assert_eq!(dir_entry_count(work.path()), 0, "work root not cleaned");

    // A refused dry-run never creates a target either.
    let empty = tempfile::tempdir().unwrap();
    let empty_config = HieronymusConfig::new(empty.path());
    let refused = run_dry_run_in(&empty_config, false, work.path()).unwrap();
    assert_eq!(refused.refused.as_deref(), Some("empty-database"));
    assert_eq!(dir_entry_count(work.path()), 0);
}

#[test]
fn dry_run_refuses_daemon_active_and_unsupported_states() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let config = HieronymusConfig::new(data.path());
    let report = run_dry_run_in(&config, true, work.path()).unwrap();
    assert_eq!(report.refused.as_deref(), Some("daemon-active"));
    assert!(report.conversion.is_none());
    assert!(report.verification.is_none());
    assert_eq!(dir_entry_count(work.path()), 0, "no target for a refusal");
}

#[test]
fn dry_run_converts_and_verifies() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let config = HieronymusConfig::new(data.path());
    let report = run_dry_run_in(&config, false, work.path()).unwrap();
    assert_eq!(report.refused, None, "{report:?}");
    let conversion = report.conversion.as_ref().unwrap();
    assert_eq!(conversion.converted, 2);
    assert_eq!(conversion.skipped, 0);
    let verification = report.verification.as_ref().unwrap();
    assert_eq!(verification.foreign_key_violations, 0);
    assert_eq!(verification.integrity, "ok");
    assert!(verification.row_accounting_ok);
    assert!(verification.fts_equivalent, "{verification:?}");
    assert!(verification.fts_probes >= 2);
    assert!(report.temp_artifacts_removed);
}

// ---------------------------------------------------------------------------
// Validation-fixture equivalence (design bar: counts are not enough)
// ---------------------------------------------------------------------------

fn contract_fixture(root: &Path) {
    let path = write_legacy_fixture(root);
    let connection = open(&path);
    insert_series(&connection, "demo");
    // Approved with a forbidden variant and a tag.
    let t1 = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "notes for センス",
        "2026-02-01T00:00:00+00:00",
    );
    insert_alias(&connection, t1, "en", "talent", "forbidden_variant", 1);
    insert_tag(&connection, t1, "narrative");
    insert_fts_row(&connection, t1, "センス", "sense", "notes for センス");
    // Approved with an approved variant equal to the canonical rendering.
    let t2 = insert_term(
        &connection,
        "demo",
        "Grey Admiral",
        "Серый Адмирал",
        "approved",
        "",
        "2026-02-02T00:00:00+00:00",
    );
    insert_alias(
        &connection,
        t2,
        "ru",
        "Серый Адмирал",
        "approved_variant",
        1,
    );
    insert_fts_row(&connection, t2, "Grey Admiral", "Серый Адмирал", "");
    // Pending: becomes a candidate, never an active contract rule.
    let t3 = insert_term(
        &connection,
        "demo",
        "ドック",
        "dock",
        "pending",
        "",
        "2026-02-03T00:00:00+00:00",
    );
    insert_alias(&connection, t3, "en", "dockyard", "forbidden_variant", 1);
    insert_fts_row(&connection, t3, "ドック", "dock", "");
    // Approved but unsupported alias shape: skipped, must not enforce.
    let t4 = insert_term(
        &connection,
        "demo",
        "mist",
        "туман",
        "approved",
        "",
        "2026-02-04T00:00:00+00:00",
    );
    insert_alias(&connection, t4, "en", "fog", "search_alias", 1);
    insert_fts_row(&connection, t4, "mist", "туман", "");
    drop(connection);
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExpectedFinding {
    legacy_term_id: i64,
    kind: String,
    severity: String,
    expected: String,
    observed: String,
}

/// The pre-conversion contract of the fixture, derived directly from the
/// legacy rows: only approved terms whose surface occurs (case-insensitively)
/// in the source enforce renderings, exactly like the legacy runtime.
fn expected_findings_from_legacy(path: &Path, raw: &str, translated: &str) -> Vec<ExpectedFinding> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("select id, source_text, canonical_translation from strict_terms where status = 'approved' order by id")
        .unwrap();
    let terms: Vec<(i64, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut findings = Vec::new();
    for (id, source_text, canonical) in terms {
        if !raw.to_lowercase().contains(&source_text.to_lowercase()) {
            continue;
        }
        let forbidden = forbidden_variants(&connection, id);
        for variant in forbidden {
            if translated.contains(&variant) {
                findings.push(ExpectedFinding {
                    legacy_term_id: id,
                    kind: "forbidden_variant".into(),
                    severity: "high".into(),
                    expected: canonical.clone(),
                    observed: variant,
                });
            }
        }
        if !translated.contains(&canonical) {
            findings.push(ExpectedFinding {
                legacy_term_id: id,
                kind: "missing_canonical".into(),
                severity: "medium".into(),
                expected: canonical,
                observed: String::new(),
            });
        }
    }
    findings.sort();
    findings
}

fn forbidden_variants(connection: &Connection, term_id: i64) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "select text from strict_term_aliases
             where term_id = ?1 and kind = 'forbidden_variant' order by id",
        )
        .unwrap();
    statement
        .query_map([term_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn validation_findings_are_equivalent_before_and_after_conversion() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    contract_fixture(data.path());
    let legacy_path = data.path().join("hieronymus.sqlite");

    // Convert a disposable copy into a Rust-schema target.
    let target = tempfile::tempdir_in(work.path()).unwrap();
    let target_path = target.path().join("hieronymus.sqlite");
    std::fs::copy(&legacy_path, &target_path).unwrap();
    let mut connection = open(&target_path);
    prepare_upgrade_target(&connection).unwrap();
    let report = {
        let transaction = connection.transaction().unwrap();
        let report = convert_strict_terms(&transaction).unwrap();
        transaction.commit().unwrap();
        report
    };

    // Before: the contract derived from the legacy rows.
    let raw = "センスを感じる。Grey Admiral が現れた。";
    let translated = "The talent was obvious; Серый Адмирал arrived.";
    let before = expected_findings_from_legacy(&legacy_path, raw, translated);
    assert_eq!(before.len(), 2, "{before:?}");

    // After: the deterministic runtime contract over the converted target.
    let config = HieronymusConfig::new(target.path());
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let termbase = Termbase::open(&config, &context).unwrap();
    let after = termbase
        .validate(translated, Source::SourceText(raw.to_string()))
        .unwrap();
    assert_eq!(after.len(), before.len(), "{after:?}");

    // Map legacy ids to rule ids through the conversion ledger and compare.
    let mut mapped: Vec<ExpectedFinding> = Vec::new();
    for finding in after {
        let legacy_term_id = report
            .converted_terms
            .iter()
            .find(|link| link.rule_id == finding.term_id)
            .map(|link| link.legacy_term_id)
            .unwrap_or_else(|| panic!("finding id {} not in ledger", finding.term_id));
        mapped.push(ExpectedFinding {
            legacy_term_id,
            kind: finding.kind,
            severity: finding.severity,
            expected: finding.expected,
            observed: finding.observed,
        });
    }
    mapped.sort();
    assert_eq!(mapped, before);

    // The pending term converted to candidate; the unsupported term skipped.
    let statuses: BTreeMap<i64, String> = report
        .converted_terms
        .iter()
        .map(|link| {
            let status: String = connection
                .query_row(
                    "select status from term_rules where id = ?1",
                    [link.rule_id],
                    |row| row.get(0),
                )
                .unwrap();
            (link.legacy_term_id, status)
        })
        .collect();
    let ordered: Vec<(i64, String)> = contract_fixture_ids(data.path());
    assert_eq!(
        statuses.get(&ordered[2].0).map(String::as_str),
        Some("candidate")
    );
    assert_eq!(report.skipped_terms.len(), 1);
    assert_eq!(report.skipped_terms[0].source_id, ordered[3].0.to_string());
    assert_eq!(
        report.skipped_terms[0].reason_code,
        "unsupported-alias-kind"
    );
}

fn contract_fixture_ids(root: &Path) -> Vec<(i64, String)> {
    let connection = Connection::open(root.join("hieronymus.sqlite")).unwrap();
    let mut statement = connection
        .prepare("select id, source_text from strict_terms order by id")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

// ---------------------------------------------------------------------------
// FTS query equivalence
// ---------------------------------------------------------------------------

#[test]
fn fts_queries_are_equivalent_before_and_after_conversion() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    contract_fixture(data.path());
    let legacy_path = data.path().join("hieronymus.sqlite");

    let target = tempfile::tempdir_in(work.path()).unwrap();
    let target_path = target.path().join("hieronymus.sqlite");
    std::fs::copy(&legacy_path, &target_path).unwrap();
    let mut connection = open(&target_path);
    prepare_upgrade_target(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    let report = convert_strict_terms(&transaction).unwrap();
    // The converter rebuild is part of the write path, like the upgrade's
    // FTS step: run it on the same transaction...
    hieronymus::migrate::rebuild_strict_terms_fts(&transaction).unwrap();
    transaction.commit().unwrap();

    let probes = [
        "センス",
        "sense",
        "Grey",
        "Серый",
        "Admiral",
        "dock",
        "mist",
    ];
    for probe in probes {
        let before = fts_probe(&legacy_path, probe);
        let after = fts_probe(&target_path, probe);
        assert_eq!(before, after, "fts probe {probe:?} diverged");
    }
    // The FTS projection covers exactly the preserved authoritative rows.
    assert_eq!(report.converted, 3);
    let indexed = fts_probe(&target_path, "sense");
    assert_eq!(indexed, vec![1]);
}

fn fts_probe(path: &Path, term: &str) -> Vec<i64> {
    let connection = Connection::open(path).unwrap();
    let quoted = format!("\"{}\"", term.replace('"', "\"\""));
    let ids = {
        let mut statement = connection
            .prepare(
                "select rowid from strict_terms_fts where strict_terms_fts match ?1 order by rowid",
            )
            .unwrap();
        statement
            .query_map([quoted], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    connection.close().unwrap();
    ids
}

// ---------------------------------------------------------------------------
// Plan-only entry (preflight reuse)
// ---------------------------------------------------------------------------

#[test]
fn plan_reports_the_same_shape_without_writing() {
    let data = tempfile::tempdir().unwrap();
    let connection = rich_fixture(data.path());
    drop(connection);
    let path = data.path().join("hieronymus.sqlite");
    let before = file_tree_digest(data.path());
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("pragma query_only = on;").unwrap();
    let report = plan_strict_term_conversion(&connection).unwrap();
    assert_eq!(report.converted, 2);
    assert_eq!(report.ledger_rows, 0);
    drop(connection);
    assert_eq!(
        file_tree_digest(data.path()),
        before,
        "plan mutated the source"
    );
}

// ---------------------------------------------------------------------------
// Review findings: verdict honesty and plan-mode blocking detail
// ---------------------------------------------------------------------------

#[test]
fn dry_run_verification_failure_flips_the_verdict_and_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(data.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    // The source form survives conversion (non-blank, round-trips) but has no
    // FTS tokens at all, so the rebuilt projection cannot resolve the row and
    // FTS verification must fail.
    insert_term(
        &connection,
        "demo",
        "!!!",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    drop(connection);

    let config = HieronymusConfig::new(data.path());
    let report = run_dry_run_in(&config, false, work.path()).unwrap();
    let verification = report.verification.as_ref().expect("verification ran");
    assert!(!verification.fts_equivalent, "{report:?}");
    assert_eq!(report.conversion.as_ref().map(|c| c.converted), Some(1));
    // The verdict is refused and the conversion-safe verdict flips to unsafe,
    // so scripting gets a nonzero exit and an honest report.
    assert_eq!(
        report.refused.as_deref(),
        Some("verification-failed"),
        "{report:?}"
    );
    assert!(!report.preflight.conversion_safe);
    assert_eq!(
        report.preflight.refusal_code.as_deref(),
        Some("verification-failed")
    );
    assert!(report.temp_artifacts_removed);
    assert_eq!(dir_entry_count(work.path()), 0);
}

#[test]
fn preflight_lists_blocking_rows_with_reason_codes() {
    let data = tempfile::tempdir().unwrap();
    let path = write_legacy_fixture(data.path());
    let connection = open(&path);
    insert_series(&connection, "demo");
    let readable = insert_term(
        &connection,
        "demo",
        "センス",
        "sense",
        "approved",
        "",
        "2026-02-01T00:00:00+00:00",
    );
    let corrupt = insert_term(
        &connection,
        "demo",
        "talent",
        "gift",
        "approved",
        "",
        "2026-02-02T00:00:00+00:00",
    );
    insert_alias(&connection, corrupt, "en", "gifts", "forbidden_variant", 1);
    let _ = readable;
    drop(connection);
    // Corrupt one cell to a blob the typed read cannot accept.
    let connection = open(&path);
    connection
        .execute(
            "update strict_terms set notes = x'ff' where id = ?1",
            [corrupt],
        )
        .unwrap();
    drop(connection);

    let config = HieronymusConfig::new(data.path());
    let report = run_preflight(&config, false).unwrap();
    assert!(report.conversion_safe);
    let plan = term_report(&report);
    // The unreadable term and its alias child row are both reported with
    // bounded codes.
    assert_eq!(plan.blocking, 2, "{plan:?}");
    let ids: Vec<(String, String, String)> = plan
        .blocking_rows
        .iter()
        .map(|row| {
            (
                row.source_table.clone(),
                row.source_id.clone(),
                row.reason_code.clone(),
            )
        })
        .collect();
    assert!(ids.contains(&(
        "strict_terms".to_string(),
        corrupt.to_string(),
        "invalid-value".to_string()
    )));
    assert_eq!(
        ids.iter()
            .filter(|(table, .., code)| table == "strict_term_aliases" && code == "invalid-value")
            .count(),
        1
    );
    // The reason codes survive into the serialized report.
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(rendered.contains("\"reason_code\""), "{rendered}");
}

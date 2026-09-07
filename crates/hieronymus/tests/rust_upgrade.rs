//! Ordered Rust-to-Rust schema upgrades (database-upgrade design, 2026-08-31;
//! ADR 0010): the step runner's transaction discipline, the v1 -> v2 upgrade
//! through the full protocol, schema equivalence between a fresh v2 database
//! and an upgraded v1 one, and the fail-closed states around both.
//!
//! `fixtures/rust-v1.sql` is a FROZEN snapshot of schema version 1. It must
//! never be regenerated from the current migrations: it is the only thing in
//! the suite that still knows what an old database looks like.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::{DatabaseState, SUPPORTED_RUST_SCHEMA_VERSION, classify_database};
use hieronymus::migrate::MigrateError;
use hieronymus::schema_upgrade::apply_steps;
use hieronymus::state_classifier::{CODE_SCHEMA_UPGRADE_REQUIRED, StartupState, classify};
use hieronymus::upgrade::{
    CutoverJournal, InjectionPoint, UpgradeOptions, daemon_start_blocker, read_cutover_journal,
    run_recovery, run_upgrade,
};
use rusqlite::Connection;

const RUST_V1_SQL: &str = include_str!("fixtures/rust-v1.sql");
const GLOBAL_SQL: &str = include_str!("../migrations/global.sql");

/// The four durable work tables schema version 2 introduces.
const V2_TABLES: [&str; 4] = [
    "dream_link_batches",
    "dream_link_members",
    "dream_link_pairs",
    "term_rule_actions",
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn config(root: &Path) -> HieronymusConfig {
    HieronymusConfig::new(root)
}

fn open(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
}

fn scalar(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap()
}

/// Representative rows across the subsystems a schema step must not disturb.
fn seed_v1_rows(connection: &Connection) {
    connection
        .execute_batch(
            "insert into series(slug, title, default_source_language,
                                default_target_language, created_at, updated_at)
             values ('demo', 'Demo', 'ja', 'en', '2026-01-01T00:00:00+00:00',
                     '2026-01-01T00:00:00+00:00');

             insert into task_sessions(series_slug, source_language, target_language,
                                       task_type, status, created_at, last_activity_at)
             values ('demo', 'ja', 'en', 'translation', 'active',
                     '2026-01-02T00:00:00+00:00', '2026-01-02T00:00:00+00:00');

             insert into crystals(crystal_type, text, scope_type, scope_key, series_slug,
                                  strength, confidence, status, created_at, updated_at)
             values ('fact', 'the admiral commands the fleet', 'series', 'series:demo',
                     'demo', 0.7, 0.9, 'active', '2026-01-03T00:00:00+00:00',
                     '2026-01-03T00:00:00+00:00'),
                    ('fact', 'the fleet sails at dawn', 'series', 'series:demo',
                     'demo', 0.6, 0.8, 'active', '2026-01-03T00:00:00+00:00',
                     '2026-01-03T00:00:00+00:00');

             insert into crystal_activations(crystal_id, session_id, recall_query, rank,
                                             score, created_at)
             values (1, 1, 'admiral', 1, 0.9, '2026-01-04T00:00:00+00:00');

             insert into term_rules(source_language, target_language, source_text,
                                    canonical_translation, status, created_at, updated_at)
             values ('ja', 'en', 'センス', 'Sense', 'active',
                     '2026-01-05T00:00:00+00:00', '2026-01-05T00:00:00+00:00');

             insert into term_rule_forms(rule_id, form_kind, surface, language)
             values (1, 'source', 'センス', 'ja');",
        )
        .unwrap();
}

/// A data root holding a Rust schema-version 1 database with real rows,
/// written the way this line really writes one: WAL journal mode, which
/// persists in the database header. Anything a version-1 database in the
/// field would carry must be carried here too.
fn v1_data_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let connection = open(&path);
    connection
        .query_row("pragma journal_mode = wal", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);
    // A cleanly closed WAL database: the sidecars are checkpointed away, which
    // is the shape a daemon that shut down properly leaves behind.
    connection
        .query_row("pragma wal_checkpoint(truncate)", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    drop(connection);
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 }
    );
    root
}

/// A Python-era data root: the ported global schema with no Rust marker.
fn python_data_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let connection = open(&root.path().join("hieronymus.sqlite"));
    connection.execute_batch(GLOBAL_SQL).unwrap();
    connection
        .execute_batch(
            "insert into series(slug, title, default_source_language,
                                default_target_language, created_at, updated_at)
             values ('demo', 'Demo', 'ja', 'en', '2026-01-01T00:00:00+00:00',
                     '2026-01-01T00:00:00+00:00');
             insert into strict_terms(series_slug, source_language, target_language,
                                      category, source_text, canonical_translation,
                                      status, notes, created_at, updated_at)
             values ('demo', 'ja', 'en', 'name', 'センス', 'sense', 'approved', '',
                     '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');
             insert into strict_terms_fts(rowid, source_text, canonical_translation, notes)
             values (1, 'センス', 'sense', '');",
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        classify_database(&root.path().join("hieronymus.sqlite")),
        DatabaseState::PythonSchema
    );
    root
}

fn table_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("select name from sqlite_master where type = 'table' order by name")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
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
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                // Excluded, because neither is data: the shared ownership lock
                // (created on every run, its inode never unlinked), and the
                // SQLite WAL sidecars. Opening a WAL database READ-ONLY
                // recreates a zero-length `-wal` and a `-shm` purely to
                // coordinate the read — no frame is written and the database
                // file is untouched. Tests that care assert the database bytes
                // and WAL emptiness directly (`assert_no_wal_frames`).
                let transient = name == hieronymus::ownership::OWNER_LOCK_FILE
                    || name.ends_with(".sqlite-wal")
                    || name.ends_with(".sqlite-shm");
                if !transient {
                    entries.push(path);
                }
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

/// No write-ahead log frames were produced: either there is no `-wal` at all,
/// or a read-only open recreated an EMPTY one. A non-empty `-wal` means
/// something wrote to the database.
fn assert_no_wal_frames(root: &Path, context: &str) {
    let wal = root.join("hieronymus.sqlite-wal");
    if wal.exists() {
        assert_eq!(
            std::fs::metadata(&wal).unwrap().len(),
            0,
            "{context}: write-ahead log frames were produced"
        );
    }
}

fn backup_sets(root: &Path) -> Vec<PathBuf> {
    let backups = root.join("backups");
    if !backups.exists() {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

// ---------------------------------------------------------------------------
// The step runner in isolation
// ---------------------------------------------------------------------------

#[test]
fn failed_upgrade_does_not_publish_the_new_version() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(RUST_V1_SQL).unwrap();
    {
        let tx = db.transaction().unwrap();
        apply_steps(&tx, 1, 2).unwrap();
        // Dropping the transaction simulates failure before the shared commit.
    }
    let version: i64 = db
        .query_row("select schema_version from hieronymus_meta", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, 1);
    let tables: i64 = db
        .query_row(
            "select count(*) from sqlite_master where name='dream_link_pairs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    let user_version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 1, "the header marker rolled back too");
}

#[test]
fn ordered_steps_preserve_every_row_and_add_empty_tables() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let mut connection = open(&path);
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);

    let before: BTreeMap<String, i64> = table_names(&connection)
        .into_iter()
        .map(|name| {
            let count = scalar(&connection, &format!("select count(*) from \"{name}\""));
            (name, count)
        })
        .collect();

    {
        let transaction = connection.transaction().unwrap();
        apply_steps(&transaction, 1, SUPPORTED_RUST_SCHEMA_VERSION).unwrap();
        transaction.commit().unwrap();
    }

    // Every pre-existing row is still exactly where it was.
    for (name, count) in &before {
        assert_eq!(
            scalar(&connection, &format!("select count(*) from \"{name}\"")),
            *count,
            "row count changed for {name}"
        );
    }
    // The new tables exist and are empty.
    for table in V2_TABLES {
        assert_eq!(
            scalar(
                &connection,
                &format!(
                    "select count(*) from sqlite_master where type = 'table' \
                     and name = '{table}'"
                )
            ),
            1,
            "{table} missing"
        );
        assert_eq!(
            scalar(&connection, &format!("select count(*) from {table}")),
            0,
            "{table} is not empty"
        );
    }
    assert_eq!(
        scalar(&connection, "select schema_version from hieronymus_meta"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    assert_eq!(
        scalar(&connection, "pragma user_version"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    drop(connection);
    assert_eq!(hieronymus::db::verify_current_rust_schema(&path), Ok(()));
}

#[test]
fn a_failing_step_rolls_back_the_whole_transaction() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let mut connection = open(&path);
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);
    // An object squatting on a name the step's `create index` needs: the step
    // fails part-way through, AFTER its first `create table` statements ran.
    connection
        .execute_batch("create table dream_link_batches_session_idx (a integer);")
        .unwrap();
    let crystals_before = scalar(&connection, "select count(*) from crystals");

    {
        let transaction = connection.transaction().unwrap();
        let error = apply_steps(&transaction, 1, SUPPORTED_RUST_SCHEMA_VERSION).unwrap_err();
        assert!(
            error.to_string().contains("dream_link_batches_session_idx"),
            "{error}"
        );
        // The protocol drops the transaction on any error; nothing commits.
    }

    assert_eq!(
        scalar(&connection, "select schema_version from hieronymus_meta"),
        1,
        "the old version stays published"
    );
    assert_eq!(scalar(&connection, "pragma user_version"), 1);
    for table in V2_TABLES {
        assert_eq!(
            scalar(
                &connection,
                &format!(
                    "select count(*) from sqlite_master where type = 'table' \
                     and name = '{table}'"
                )
            ),
            0,
            "{table} leaked past the rollback"
        );
    }
    assert_eq!(
        scalar(&connection, "select count(*) from crystals"),
        crystals_before,
        "original rows must survive untouched"
    );
    drop(connection);
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 }
    );
}

#[test]
fn a_fresh_current_database_and_an_upgraded_old_one_are_schema_identical() {
    /// Every schema object plus the full column contract of every table, in a
    /// deterministic order.
    fn schema_shape(path: &Path) -> Vec<String> {
        let connection = Connection::open(path).unwrap();
        let mut lines = Vec::new();
        let mut statement = connection
            .prepare(
                "select type, name, coalesce(sql, '') from sqlite_master
                 where name not like 'sqlite_%' order by type, name",
            )
            .unwrap();
        let objects: Vec<(String, String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        drop(statement);
        for (kind, name, sql) in &objects {
            lines.push(format!("{kind} {name} :: {sql}"));
        }
        for (kind, name, _) in &objects {
            if kind != "table" {
                continue;
            }
            let mut columns = connection
                .prepare("select cid, name, type, \"notnull\", dflt_value, pk from pragma_table_info(?1)")
                .unwrap();
            let rows: Vec<String> = columns
                .query_map([name], |row| {
                    Ok(format!(
                        "{} {} {} {} {:?} {}",
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            for column in rows {
                lines.push(format!("column {name} :: {column}"));
            }
        }
        let user_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        lines.push(format!("user_version {user_version}"));
        lines.push(format!(
            "schema_version {}",
            connection
                .query_row("select schema_version from hieronymus_meta", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap()
        ));
        lines
    }

    // A fresh database created at the current version.
    let fresh_root = tempfile::tempdir().unwrap();
    let fresh = fresh_root.path().join("hieronymus.sqlite");
    hieronymus::db::open_migrated(&fresh).unwrap();

    // A version-1 database moved forward by the ordered steps.
    let upgraded_root = tempfile::tempdir().unwrap();
    let upgraded = upgraded_root.path().join("hieronymus.sqlite");
    let mut connection = open(&upgraded);
    connection.execute_batch(RUST_V1_SQL).unwrap();
    {
        let transaction = connection.transaction().unwrap();
        apply_steps(&transaction, 1, SUPPORTED_RUST_SCHEMA_VERSION).unwrap();
        transaction.commit().unwrap();
    }
    drop(connection);

    let fresh_shape = schema_shape(&fresh);
    let upgraded_shape = schema_shape(&upgraded);
    assert_eq!(
        fresh_shape, upgraded_shape,
        "a fresh current database and an upgraded old one must be identical"
    );
    assert!(
        fresh_shape
            .iter()
            .any(|line| line.starts_with("table dream_link_pairs")),
        "the comparison must actually cover the new tables"
    );
}

/// The astra-3.2 property, stated as SQL: batch source ids are durable
/// SNAPSHOT identifiers, not live-row foreign keys. Deleting the session,
/// activation, or crystals a batch refers to must neither be blocked nor erase
/// the recorded pair progress — the dreaming phase revalidates the ids when it
/// resumes and records a skipped tombstone for whatever has gone. An audit
/// record in `term_rule_actions` likewise outlives the rule it describes.
#[test]
fn deleting_source_rows_neither_fails_nor_erases_durable_work() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let mut connection = open(&path);
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);
    {
        let transaction = connection.transaction().unwrap();
        apply_steps(&transaction, 1, SUPPORTED_RUST_SCHEMA_VERSION).unwrap();
        transaction.commit().unwrap();
    }

    // Foreign keys ON: any missing enforcement below would be a hard error.
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
        .execute_batch(
            "insert into dream_link_batches(id, session_id, created_cycle)
             values (1, 1, 7);
             insert into dream_link_members(batch_id, activation_id) values (1, 1);
             insert into dream_link_pairs(batch_id, left_id, right_id, status)
             values (1, 1, 2, 'queued');
             insert into term_rule_actions(idempotency_key, rule_id, actor, reason, action,
                                           expected_revision, resulting_revision,
                                           request_canonical, result_json, created_at)
             values ('key-1', 1, 'admin', 'approve on review', 'activate', 1, 2,
                     '{}', '{}', '2026-01-06T00:00:00+00:00');",
        )
        .unwrap();

    // Every BATCH SOURCE row is deleted. Each delete must SUCCEED — these are
    // snapshot ids, not live-row references.
    for statement in [
        "delete from crystal_activations",
        "delete from crystals",
        "delete from task_sessions",
    ] {
        connection
            .execute_batch(statement)
            .unwrap_or_else(|error| panic!("`{statement}` was blocked: {error}"));
    }

    // And the durable work is all still there.
    for table in [
        "dream_link_batches",
        "dream_link_members",
        "dream_link_pairs",
    ] {
        assert_eq!(
            scalar(&connection, &format!("select count(*) from {table}")),
            1,
            "{table} lost rows when its source records were deleted"
        );
    }
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from dream_link_pairs where status = 'queued'"
        ),
        1,
        "pair progress must survive the deletion of the crystals it names"
    );

    // The internal batch references DO still hold: a member or pair can never
    // point at a batch that is not there.
    let error = connection
        .execute_batch(
            "insert into dream_link_pairs(batch_id, left_id, right_id) values (99, 3, 4);",
        )
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("foreign key"),
        "{error}"
    );

    // An audit record is the opposite case (ADR 0011): it PINS the rule it
    // describes. No cascade can erase it, and the rule cannot be deleted out
    // from under it — terminology rules are archived or superseded through the
    // lifecycle, never hard-deleted.
    let error = connection
        .execute_batch("delete from term_rules;")
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("foreign key"),
        "an audited rule must not be deletable: {error}"
    );
    assert_eq!(
        scalar(&connection, "select count(*) from term_rule_actions"),
        1,
        "the audit record survived"
    );
}

// ---------------------------------------------------------------------------
// The full protocol over a version-1 data root
// ---------------------------------------------------------------------------

#[test]
fn a_version_1_data_root_upgrades_through_the_full_protocol() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    let connection = open(&path);
    let crystals_before = scalar(&connection, "select count(*) from crystals");
    let rules_before = scalar(&connection, "select count(*) from term_rules");
    let sessions_before = scalar(&connection, "select count(*) from task_sessions");
    let series_before = scalar(&connection, "select count(*) from series");
    drop(connection);

    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete", "{report:?}");
    assert_eq!(report.source_state, "rust-schema-upgradable");
    assert_eq!(report.target_schema_version, SUPPORTED_RUST_SCHEMA_VERSION);
    // No Python converter ran: an ordered schema step converts nothing.
    assert!(report.conversion.is_none(), "{report:?}");
    let verification = report.verification.as_ref().unwrap();
    assert_eq!(verification.foreign_key_violations, 0);
    assert_eq!(verification.integrity, "ok");
    assert!(verification.row_accounting_ok);
    assert!(verification.fts_equivalent);
    assert!(verification.domain_invariants_ok);
    // Spec step 10: the durable semantic-rebuild job committed with the rest.
    let job = report.semantic_job.as_ref().unwrap();
    assert_eq!(job.status, "queued");

    // Every row survived and the four new tables landed empty.
    let connection = open(&path);
    assert_eq!(
        scalar(&connection, "pragma user_version"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    assert_eq!(
        scalar(&connection, "select count(*) from crystals"),
        crystals_before
    );
    assert_eq!(
        scalar(&connection, "select count(*) from term_rules"),
        rules_before
    );
    assert_eq!(
        scalar(&connection, "select count(*) from task_sessions"),
        sessions_before
    );
    assert_eq!(
        scalar(&connection, "select count(*) from series"),
        series_before
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from crystals where text = 'the admiral commands the fleet'"
        ),
        1,
        "row content preserved, not just counts"
    );
    for table in V2_TABLES {
        assert_eq!(
            scalar(&connection, &format!("select count(*) from {table}")),
            0,
            "{table}"
        );
    }
    drop(connection);

    // Journal, backup and receipt describe this attempt.
    let journal = read_cutover_journal(&config(root.path())).unwrap().unwrap();
    assert_eq!(journal.state, "complete");
    assert_eq!(journal.source_state, "rust-schema-upgradable");
    assert_eq!(journal.target_schema_version, SUPPORTED_RUST_SCHEMA_VERSION);
    let sets = backup_sets(root.path());
    assert_eq!(sets.len(), 1, "{sets:?}");
    assert!(sets[0].join("receipt.json").exists());
    // The backup holds the PRE-upgrade database, still at version 1.
    assert_eq!(
        classify_database(&sets[0].join("hieronymus.sqlite")),
        DatabaseState::RustSchema { version: 1 }
    );

    // The daemon may start again, and the classifier agrees.
    assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);
    assert_eq!(
        classify(&config(root.path())).unwrap(),
        StartupState::Current
    );

    // Rerunning is a no-op and creates no second backup.
    let rerun = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(rerun.outcome.as_str(), "already-complete");
    assert_eq!(backup_sets(root.path()).len(), 1);
}

#[test]
fn a_precommit_failure_leaves_the_version_1_database_untouched() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::BeforeCommit),
    };
    let error = run_upgrade(&config(root.path()), false, &options).unwrap_err();
    assert!(
        matches!(error, MigrateError::Injected(InjectionPoint::BeforeCommit)),
        "{error}"
    );

    // The transaction rolled back: still version 1, no new tables, rows intact.
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 }
    );
    let connection = open(&path);
    for table in V2_TABLES {
        assert_eq!(
            scalar(
                &connection,
                &format!(
                    "select count(*) from sqlite_master where type = 'table' \
                     and name = '{table}'"
                )
            ),
            0,
            "{table} leaked past the rollback"
        );
    }
    assert_eq!(scalar(&connection, "select count(*) from crystals"), 2);
    drop(connection);

    // And the daemon refuses the mid-state: the `prepared` journal is shut.
    assert_eq!(
        daemon_start_blocker(&config(root.path()))
            .unwrap()
            .as_deref(),
        Some("prepared")
    );
    assert!(classify(&config(root.path())).is_err());

    // Rerunning without injection finishes the upgrade.
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema {
            version: SUPPORTED_RUST_SCHEMA_VERSION
        }
    );
}

/// The same failure-injection sweep `upgrade_port.rs` runs over a Python
/// cutover, applied to an ordered Rust→Rust upgrade: every interrupted attempt
/// leaves either the original version-1 database or a journal the daemon
/// refuses, and rerunning always finishes at the current schema.
#[test]
fn failure_injection_at_every_step_leaves_only_safe_states_for_an_ordered_upgrade() {
    const POINTS: &[InjectionPoint] = &[
        InjectionPoint::AfterPreflight,
        InjectionPoint::AfterStaging,
        InjectionPoint::AfterBackup,
        InjectionPoint::AfterJournalPrepared,
        InjectionPoint::BeforeCommit,
        InjectionPoint::AfterCommit,
        InjectionPoint::AfterDatabaseCommitted,
        InjectionPoint::AfterReceipt,
        InjectionPoint::AfterComplete,
    ];
    for point in POINTS {
        let root = v1_data_root();
        let path = root.path().join("hieronymus.sqlite");
        let options = UpgradeOptions {
            injection: Some(*point),
        };
        let result = run_upgrade(&config(root.path()), false, &options);
        assert!(
            matches!(&result, Err(MigrateError::Injected(injected)) if injected == point),
            "{point:?}: {result:?}"
        );

        if *point == InjectionPoint::AfterComplete {
            // A crash after the receipt is a finished cutover.
            assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);
            assert_eq!(
                classify(&config(root.path())).unwrap(),
                StartupState::Current
            );
            continue;
        }

        // Every earlier crash leaves a root the daemon refuses: the journal
        // gate is shut, or the database is still at version 1 (which the
        // classifier routes to `hiero migrate`). It is never startable.
        assert!(
            classify(&config(root.path())).is_err(),
            "{point:?}: daemon could start in the middle state"
        );

        // Before the commit, the version-1 database is completely untouched.
        if matches!(
            point,
            InjectionPoint::AfterPreflight
                | InjectionPoint::AfterStaging
                | InjectionPoint::AfterBackup
                | InjectionPoint::AfterJournalPrepared
                | InjectionPoint::BeforeCommit
        ) {
            assert_eq!(
                classify_database(&path),
                DatabaseState::RustSchema { version: 1 },
                "{point:?}: the new version leaked past the rollback"
            );
            let connection = open(&path);
            for table in V2_TABLES {
                assert_eq!(
                    scalar(
                        &connection,
                        &format!(
                            "select count(*) from sqlite_master where type = 'table' \
                             and name = '{table}'"
                        )
                    ),
                    0,
                    "{point:?}: {table} leaked past the rollback"
                );
            }
            assert_eq!(scalar(&connection, "select count(*) from crystals"), 2);
            drop(connection);
        }

        // Rerunning without injection always finishes the upgrade.
        let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
            .unwrap_or_else(|error| panic!("{point:?}: resume failed: {error}"));
        assert_eq!(report.outcome.as_str(), "complete", "{point:?}");
        assert_eq!(
            classify_database(&path),
            DatabaseState::RustSchema {
                version: SUPPORTED_RUST_SCHEMA_VERSION
            },
            "{point:?}"
        );
        let connection = open(&path);
        assert_eq!(
            scalar(&connection, "select count(*) from crystals"),
            2,
            "{point:?}: rows lost across the interrupted attempt"
        );
        drop(connection);
        assert_eq!(
            classify(&config(root.path())).unwrap(),
            StartupState::Current,
            "{point:?}"
        );
    }
}

/// The verification gate takes the real `verification-failed` path: the
/// transaction rolls back, the version-1 database is untouched, the `prepared`
/// journal keeps the daemon out, and the backup set from the failed attempt is
/// preserved for the operator.
#[test]
fn a_failed_verification_refuses_the_upgrade_and_rolls_back() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::VerificationFailed),
    };
    let error = run_upgrade(&config(root.path()), false, &options).unwrap_err();
    assert!(
        error.to_string().contains("verification-failed"),
        "the real refusal path, not an injected error type: {error}"
    );

    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 },
        "the new version was published despite failed verification"
    );
    let connection = open(&path);
    for table in V2_TABLES {
        assert_eq!(
            scalar(
                &connection,
                &format!(
                    "select count(*) from sqlite_master where type = 'table' \
                     and name = '{table}'"
                )
            ),
            0,
            "{table} leaked past the rollback"
        );
    }
    assert_eq!(scalar(&connection, "select count(*) from crystals"), 2);
    drop(connection);

    // The attempt's journal is `prepared`, so the daemon stays out, and the
    // backup taken before the transaction is kept.
    assert_eq!(
        daemon_start_blocker(&config(root.path()))
            .unwrap()
            .as_deref(),
        Some("prepared")
    );
    assert!(classify(&config(root.path())).is_err());
    let sets = backup_sets(root.path());
    assert_eq!(
        sets.len(),
        1,
        "the pre-upgrade backup must be kept: {sets:?}"
    );
    assert_eq!(
        classify_database(&sets[0].join("hieronymus.sqlite")),
        DatabaseState::RustSchema { version: 1 }
    );

    // Rerunning without the injection completes.
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
}

/// `hiero migrate --dry-run` over a version-1 root rehearses the ordered steps
/// against a disposable copy and writes NOTHING to the data root (an explicit
/// acceptance criterion of the database-upgrade spec).
#[test]
fn a_dry_run_over_a_version_1_root_writes_nothing() {
    let root = v1_data_root();
    let work = tempfile::tempdir().unwrap();
    let before = file_tree_digest(root.path());

    let report = hieronymus::migrate::run_dry_run_in(&config(root.path()), false, work.path())
        .map_err(|error| error.to_string())
        .unwrap();

    assert_eq!(report.refused, None, "{report:?}");
    assert_eq!(
        report.preflight.detected_state, "rust-schema-upgradable",
        "{report:?}"
    );
    assert_eq!(
        report.preflight.detected_schema_version,
        Some(1),
        "{report:?}"
    );
    // No Python converter is rehearsed for an ordered upgrade.
    assert!(report.conversion.is_none(), "{report:?}");
    let verification = report.verification.as_ref().unwrap();
    assert!(verification.all_checks_pass(), "{verification:?}");
    assert!(verification.row_accounting_ok);
    assert!(verification.fts_equivalent);
    assert!(report.temp_artifacts_removed);

    // The data root is byte-for-byte what it was, and still at version 1.
    assert_eq!(
        file_tree_digest(root.path()),
        before,
        "the dry-run wrote to the data root"
    );
    assert_eq!(
        classify_database(&root.path().join("hieronymus.sqlite")),
        DatabaseState::RustSchema { version: 1 }
    );
    // And the disposable work root was cleaned up.
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0);
}

#[test]
fn foreign_key_violations_refuse_the_upgrade_before_any_mutation() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    // A dangling child row: `foreign_key_check` reports it, and the joint
    // preflight refuses before a single byte of the data root is touched.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "pragma foreign_keys = off;
             insert into term_rule_forms(rule_id, form_kind, surface, language)
             values (9999, 'source', 'ghost', 'ja');",
        )
        .unwrap();
    drop(connection);

    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(
        error.to_string().contains("foreign-key-violations"),
        "{error}"
    );
    assert_eq!(file_tree_digest(root.path()), before, "data root changed");
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 }
    );
    // The daemon still refuses to start on the un-upgraded database.
    assert!(classify(&config(root.path())).is_err());
}

#[test]
fn a_completed_earlier_cutover_does_not_block_a_later_target() {
    let root = v1_data_root();
    // A cutover that completed under an older binary: its journal says the
    // target was version 1, and its backup set carries the receipt.
    let old_backup = root
        .path()
        .join("backups")
        .join("pre-upgrade-20260101T000000");
    std::fs::create_dir_all(&old_backup).unwrap();
    std::fs::write(old_backup.join("receipt.json"), "{\"receipt_version\": 1}").unwrap();
    std::fs::write(old_backup.join("hieronymus.sqlite"), b"an older image").unwrap();
    let old_receipt_bytes = std::fs::read(old_backup.join("receipt.json")).unwrap();
    let journal = CutoverJournal {
        journal_version: 1,
        state: "complete".to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: 1,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-20260101T000000".to_string(),
        staged: Vec::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: BTreeMap::new(),
        updated_at: "2026-01-01T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        root.path().join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();

    // The completed older cutover is history, not an answer: a new attempt runs.
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete", "{report:?}");
    assert_eq!(report.source_state, "rust-schema-upgradable");
    assert_eq!(report.target_schema_version, SUPPORTED_RUST_SCHEMA_VERSION);
    assert_eq!(
        classify_database(&root.path().join("hieronymus.sqlite")),
        DatabaseState::RustSchema {
            version: SUPPORTED_RUST_SCHEMA_VERSION
        }
    );

    // The earlier receipt and backup were preserved, and the new attempt got
    // its own numbered backup set beside them.
    assert_eq!(
        std::fs::read(old_backup.join("receipt.json")).unwrap(),
        old_receipt_bytes,
        "the earlier receipt must not be rewritten"
    );
    assert_eq!(
        std::fs::read(old_backup.join("hieronymus.sqlite")).unwrap(),
        b"an older image",
        "the earlier backup must not be touched"
    );
    let sets = backup_sets(root.path());
    assert_eq!(sets.len(), 2, "{sets:?}");
    assert!(sets.contains(&old_backup));

    // Now the journal really is terminal.
    let rerun = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(rerun.outcome.as_str(), "already-complete");
}

// ---------------------------------------------------------------------------
// The Python cutover lands directly at the current schema
// ---------------------------------------------------------------------------

#[test]
fn a_python_database_upgrades_straight_to_the_current_schema() {
    let root = python_data_root();
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
    assert_eq!(report.source_state, "python-schema");
    assert_eq!(report.target_schema_version, SUPPORTED_RUST_SCHEMA_VERSION);
    assert_eq!(report.conversion.as_ref().unwrap().converted, 1);

    let path = root.path().join("hieronymus.sqlite");
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema {
            version: SUPPORTED_RUST_SCHEMA_VERSION
        }
    );
    let connection = open(&path);
    for table in V2_TABLES {
        assert_eq!(
            scalar(
                &connection,
                &format!(
                    "select count(*) from sqlite_master where type = 'table' \
                     and name = '{table}'"
                )
            ),
            1,
            "{table} missing after a Python cutover"
        );
    }
    drop(connection);
    // A database that lands at the current schema starts immediately: no
    // second `hiero migrate` is required.
    assert_eq!(
        classify(&config(root.path())).unwrap(),
        StartupState::Current
    );
}

// ---------------------------------------------------------------------------
// Recovery from a version-1 Rust backup
// ---------------------------------------------------------------------------

#[test]
fn recovery_imports_a_version_1_backup_at_the_current_schema() {
    let root = v1_data_root();
    // Upgrade first, so `backups/` holds the immutable version-1 image.
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    let sets = backup_sets(root.path());
    assert_eq!(sets.len(), 1);
    let backup_digest = file_tree_digest(&sets[0]);
    assert_eq!(
        classify_database(&sets[0].join("hieronymus.sqlite")),
        DatabaseState::RustSchema { version: 1 },
        "the backup is the pre-upgrade image"
    );

    // The live database is destroyed; recovery reruns the CURRENT importer.
    std::fs::write(
        root.path().join("hieronymus.sqlite"),
        b"corrupted beyond repair",
    )
    .unwrap();
    let report = run_recovery(&config(root.path()), false)
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(
        report.recovered_from.file_name().unwrap(),
        sets[0].file_name().unwrap()
    );
    assert_eq!(report.converted_terms, 0, "no Python converter ran");

    let path = root.path().join("hieronymus.sqlite");
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema {
            version: SUPPORTED_RUST_SCHEMA_VERSION
        },
        "a version-1 backup must land at the current schema"
    );
    assert_eq!(hieronymus::db::verify_current_rust_schema(&path), Ok(()));
    let connection = open(&path);
    assert_eq!(scalar(&connection, "select count(*) from crystals"), 2);
    for table in V2_TABLES {
        assert_eq!(
            scalar(&connection, &format!("select count(*) from {table}")),
            0,
            "{table}"
        );
    }
    drop(connection);

    // The immutable backup was neither modified nor deleted.
    assert_eq!(file_tree_digest(&sets[0]), backup_digest);
    assert!(backup_sets(root.path()).contains(&sets[0]));
}

// ---------------------------------------------------------------------------
// The daemon gate
// ---------------------------------------------------------------------------

#[test]
fn the_daemon_refuses_a_version_1_database_and_writes_nothing() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    let before = std::fs::read(&path).unwrap();

    let error = classify(&config(root.path())).unwrap_err();
    assert_eq!(error.code(), CODE_SCHEMA_UPGRADE_REQUIRED);
    assert_eq!(error.remediation(), "hiero migrate");
    assert!(error.message().contains("version 1"), "{error}");

    // Opening for writes is refused too: no marker bump, no schema creation.
    let open_error = hieronymus::db::open_migrated(&path).unwrap_err();
    assert!(
        matches!(
            open_error,
            hieronymus::db::OpenMigratedError::UnsupportedState(DatabaseState::RustSchema {
                version: 1
            })
        ),
        "{open_error}"
    );

    // The guarantee, stated exactly: the database FILE BYTES are untouched and
    // not one write-ahead log frame is produced. Both gates open the file
    // read-only; on a WAL database SQLite recreates an EMPTY `-wal` plus a
    // `-shm` purely to coordinate that read, which writes no data.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the refused startup modified the database file"
    );
    assert_no_wal_frames(root.path(), "refused startup");
    assert!(!config(root.path()).daemon_discovery_path().exists());
    assert_eq!(
        classify_database(&path),
        DatabaseState::RustSchema { version: 1 }
    );
    // A version-1 database can never verify as the current schema.
    assert!(hieronymus::db::verify_current_rust_schema(&path).is_err());
}

/// The same guarantee against the harder shape: a WAL database a crashed
/// daemon left with a live `-wal` sidecar. The database file and the `-wal`
/// must both come back byte-identical; a transient `-shm` is acceptable
/// read-coordination state, not a write.
#[test]
fn a_crashed_version_1_database_is_also_left_byte_identical() {
    let root = v1_data_root();
    let path = root.path().join("hieronymus.sqlite");
    let wal = root.path().join("hieronymus.sqlite-wal");

    // Leave uncommitted-to-main-file frames behind, as a killed daemon would.
    let connection = open(&path);
    connection
        .execute_batch(
            "insert into crystals(crystal_type, text, scope_type, scope_key, series_slug,
                                  strength, confidence, status, created_at, updated_at)
             values ('fact', 'written just before the crash', 'series', 'series:demo',
                     'demo', 0.5, 0.5, 'active', '2026-01-09T00:00:00+00:00',
                     '2026-01-09T00:00:00+00:00');",
        )
        .unwrap();
    std::mem::forget(connection); // no checkpoint, no clean close
    assert!(wal.exists(), "the fixture must leave a live -wal");
    let database_before = std::fs::read(&path).unwrap();
    let wal_before = std::fs::read(&wal).unwrap();

    let error = classify(&config(root.path())).unwrap_err();
    assert_eq!(error.code(), CODE_SCHEMA_UPGRADE_REQUIRED);
    assert!(hieronymus::db::open_migrated(&path).is_err());

    assert_eq!(
        std::fs::read(&path).unwrap(),
        database_before,
        "the refused startup modified the database file"
    );
    assert_eq!(
        std::fs::read(&wal).unwrap(),
        wal_before,
        "the refused startup modified the write-ahead log"
    );
    assert!(!config(root.path()).daemon_discovery_path().exists());
}

#[test]
fn v3_materialized_batches_upgrade_without_replaying_terminal_pairs() {
    use hieronymus::dream_link_progress::LinkProgress;
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let mut connection = open(&config.database_path());
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);
    let transaction = connection.transaction().unwrap();
    apply_steps(&transaction, 1, 3).unwrap();
    transaction.commit().unwrap();
    connection
        .execute_batch(
            "insert into dream_link_batches(id, session_id, created_cycle) values(1, 999, 1);
      insert into dream_link_pairs(batch_id,left_id,right_id,status,applied_cycle,result_json)
      values(1, 10001, 10002, 'applied', 1, '{}'), (1,10001,10003,'queued',null,null);",
        )
        .unwrap();
    let transaction = connection.transaction().unwrap();
    apply_steps(&transaction, 3, SUPPORTED_RUST_SCHEMA_VERSION).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        scalar(&connection, "select lazy_pairs from dream_link_batches"),
        0
    );
    assert_eq!(
        scalar(&connection, "select count(*) from dream_link_crystals"),
        0
    );
    assert_eq!(
        scalar(
            &connection,
            "select applied_pair_count from dream_link_batches"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select skipped_pair_count from dream_link_batches"
        ),
        0
    );
    let mut progress = LinkProgress::open(&config).unwrap();
    assert_eq!(progress.process(2, 1).unwrap(), 1);
    assert_eq!(
        scalar(
            &connection,
            "select applied_cycle from dream_link_pairs where status='applied'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from dream_link_pairs where status='skipped'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select applied_pair_count from dream_link_batches"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select skipped_pair_count from dream_link_batches"
        ),
        1
    );
    assert_eq!(progress.process(3, 1).unwrap(), 0);
}

// Autonomous authority v5: migration effects are exercised through the real
// ordered runner, never by creating replacement test-only authority tables.
const V5_TABLES: &[&str] = &[
    "authority_state",
    "origin_receipts",
    "decision_records",
    "decision_evidence",
    "evidence_records",
    "story_timelines",
    "story_positions",
    "applicabilities",
    "knowledge_gates",
    "memory_claims",
    "claim_bindings",
    "claim_effects",
    "rule_authority",
    "rule_exclusions",
    "claim_derivations",
    "provider_recovery_state",
    "consolidation_results",
    "consolidation_jobs",
];

fn v4_authority_fixture() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection.execute_batch(RUST_V1_SQL).unwrap();
    seed_v1_rows(&connection);
    connection
        .execute_batch(
            "update term_rules set provenance = 'dream:learned' where id = 1;
         insert into term_rules(id,source_language,target_language,source_text,
           canonical_translation,status,provenance,created_at,updated_at)
         values(2,'ja','en','月','Moon','candidate','agent','2026-01-01','2026-01-01');
         insert into term_rule_story_scopes values(1,'Book I/Interlude α');",
        )
        .unwrap();
    let tx = connection.transaction().unwrap();
    apply_steps(&tx, 1, 4).unwrap();
    tx.commit().unwrap();
    connection.execute_batch(
        "insert into dream_link_batches(id,session_id,created_cycle,lazy_pairs) values(1,1,1,1);
         insert into dream_link_crystals values(1,0,1),(1,1,2);
         insert into dream_link_pairs values(1,1,2,'applied',1,'{}');"
    ).unwrap();
    connection
}

#[test]
fn v5_backfills_every_legacy_rule_without_inventing_series_ownership() {
    let mut connection = v4_authority_fixture();
    let tx = connection.transaction().unwrap();
    apply_steps(&tx, 4, 5).unwrap();
    tx.commit().unwrap();
    assert_eq!(scalar(&connection, "pragma user_version"), 5);
    assert_eq!(
        scalar(&connection, "select count(*) from rule_authority"),
        2
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from authority_state where revision=0"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from rule_authority where rule_id=1 and authority='explicit_user' and legacy_protected=1 and decision_id is null and consolidation_result_id is null"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from rule_authority where rule_id=2 and authority='learned' and legacy_protected=0"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from applicabilities where series_id is null and metadata_state='legacy_global' and scope_predicates_json='[]'"
        ),
        2
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from origin_receipts where kind='legacy_import'"
        ),
        1
    );
    assert_eq!(
        scalar(&connection, "select count(*) from pragma_foreign_key_check"),
        0
    );
    assert_eq!(
        scalar(&connection, "select count(*) from dream_link_crystals"),
        2
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from dream_link_pairs where status='applied' and applied_cycle=1 and result_json='{}'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from term_rule_story_scopes where story_scope='Book I/Interlude α'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &connection,
            "select count(*) from term_rules where status='active' and provenance='dream:learned'"
        ),
        1
    );
}

#[test]
fn v5_legacy_global_rules_upgrade_even_without_a_series() {
    let mut connection = v4_authority_fixture();
    connection
        .execute_batch(
            "delete from crystal_activations; delete from task_sessions; delete from series;",
        )
        .unwrap();
    let tx = connection.transaction().unwrap();
    apply_steps(&tx, 4, 5).unwrap();
    tx.commit().unwrap();
    assert_eq!(scalar(&connection, "select count(*) from series"), 0);
    assert_eq!(
        scalar(&connection, "select count(*) from rule_authority"),
        2
    );
    assert_eq!(
        scalar(&connection, "select count(*) from pragma_foreign_key_check"),
        0
    );
}

#[test]
fn v5_marker_failure_rolls_back_tables_backfill_and_markers() {
    let mut connection = v4_authority_fixture();
    connection.execute_batch("create trigger refuse_v5 before update on hieronymus_meta when new.schema_version=5 begin select raise(abort,'injected before v5 marker'); end;").unwrap();
    let tx = connection.transaction().unwrap();
    let error = apply_steps(&tx, 4, 5).unwrap_err();
    assert!(
        error.to_string().contains("injected before v5 marker"),
        "{error}"
    );
    tx.rollback().unwrap();
    assert_eq!(scalar(&connection, "pragma user_version"), 4);
    assert_eq!(
        scalar(&connection, "select schema_version from hieronymus_meta"),
        4
    );
    for table in V5_TABLES {
        assert!(
            !table_names(&connection).contains(&table.to_string()),
            "{table}"
        );
    }
    assert_eq!(scalar(&connection, "select count(*) from term_rules"), 2);
}

#[test]
fn v5_rejects_partial_schema_and_startup_missing_authority_columns() {
    let mut connection = v4_authority_fixture();
    connection
        .execute_batch("create table authority_state(series_id integer);")
        .unwrap();
    let tx = connection.transaction().unwrap();
    assert!(apply_steps(&tx, 4, 5).is_err());
    tx.rollback().unwrap();
    assert_eq!(scalar(&connection, "pragma user_version"), 4);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let connection = hieronymus::db::open_migrated(&path).unwrap();
    assert_eq!(scalar(&connection, "pragma user_version"), 5);
    connection
        .execute_batch("alter table authority_state drop column revision;")
        .unwrap();
    assert!(hieronymus::db::verify_current_rust_schema(&path).is_err());
}

#[test]
fn v5_fresh_and_v4_upgraded_schemas_are_identical() {
    let root = tempfile::tempdir().unwrap();
    let fresh = hieronymus::db::open_migrated(&root.path().join("hieronymus.sqlite")).unwrap();
    let mut upgraded = v4_authority_fixture();
    let tx = upgraded.transaction().unwrap();
    apply_steps(&tx, 4, 5).unwrap();
    tx.commit().unwrap();
    fn objects(connection: &Connection) -> Vec<(String, String)> {
        connection.prepare("select name,sql from sqlite_master where sql is not null and name not like 'sqlite_%' order by name").unwrap()
            .query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap()
    }
    assert_eq!(objects(&fresh), objects(&upgraded));
    for table in V5_TABLES {
        assert!(table_names(&fresh).contains(&table.to_string()), "{table}");
    }
}

#[test]
fn v5_series_initialization_preserves_revision_and_rolls_back_with_series() {
    use hieronymus::registry::Registry;
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series("one", "One", "ja", "en", None)
        .unwrap();
    let connection = open(&config.database_path());
    assert_eq!(
        scalar(&connection, "select revision from authority_state"),
        0
    );
    connection
        .execute_batch("update authority_state set revision=7;")
        .unwrap();
    registry
        .create_series("one", "Renamed", "ja", "en", None)
        .unwrap();
    assert_eq!(
        scalar(&connection, "select revision from authority_state"),
        7
    );
    connection.execute_batch("create trigger refuse_authority before insert on authority_state begin select raise(abort,'authority unavailable'); end;").unwrap();
    assert!(
        registry
            .create_series("two", "Two", "ja", "en", None)
            .is_err()
    );
    assert_eq!(scalar(&connection, "select count(*) from series"), 1);
}

#[test]
fn v5_constraints_preserve_history_and_reject_forged_legacy_authority() {
    let mut c = v4_authority_fixture();
    let tx = c.transaction().unwrap();
    apply_steps(&tx, 4, 5).unwrap();
    tx.commit().unwrap();
    c.execute_batch(
        "insert into origin_receipts values('agent-origin','agent','client',1,'event','text','{}','hash','now');
         insert into applicabilities(id,series_id,scope_predicates_json,metadata_state) values(10,1,'[]','unspecified');
         insert into decision_records values('decision',1,'agent-origin','agent',0,1,'{}','{}','applied','now');
         insert into provider_recovery_state values('default','unconfigured','now','now');
         insert into consolidation_jobs(decision_id,state,attempts,provider_slot_id,created_at,updated_at) values('decision','pending',0,'default','now','now');
         insert into consolidation_results(result_id,job_decision_id,generation,state,created_at,updated_at) values('result','decision',0,'reserved','now','now');
         insert into memory_claims(id,series_id,text,revision,status,applicability_id,created_at,updated_at) values(1,1,'claim',0,'current',10,'now','now');
         insert into claim_bindings(claim_id,crystal_id) values(1,1);"
    ).unwrap();
    for sql in [
        "insert into applicabilities(series_id,scope_predicates_json,metadata_state) values(null,'[]','legacy_global')",
        "update applicabilities set scope_predicates_json='[\"changed\"]' where id=1",
        "update rule_authority set origin_id='agent-origin' where rule_id=1",
        "update rule_authority set decision_id='decision' where rule_id=1",
        "insert into applicabilities(series_id,scope_predicates_json,metadata_state) values(null,'[]','unspecified')",
        "insert into applicabilities(series_id,scope_predicates_json,metadata_state) values(1,'broken','resolved')",
        "update authority_state set revision=-1",
        "insert into claim_bindings(claim_id,crystal_id,facet_id) values(1,1,1)",
        "insert into claim_bindings(claim_id) values(1)",
        "insert into claim_bindings(claim_id,crystal_id) values(1,1)",
        "insert into knowledge_gates(applicability_id,viewpoint_kind) values(10,'character')",
        "insert into knowledge_gates(applicability_id,viewpoint_kind,viewpoint_concept_id) values(10,'narrator',1)",
        "update consolidation_results set state='prepared'",
        "update consolidation_results set state='complete',canonical_output='{}',expected_revision=1,origin_id='agent-origin'",
        "insert into rule_exclusions(rule_id,applicability_id) values(1,10)",
        "insert into rule_exclusions(rule_id,applicability_id,decision_id,consolidation_result_id) values(1,10,'decision','result')",
        "insert into claim_derivations values(1,1,'result')",
        "delete from origin_receipts where id='agent-origin'",
        "delete from term_rules where id=1",
        "delete from crystals where id=1",
    ] {
        assert!(
            c.execute_batch(sql).is_err(),
            "accepted invalid authority write: {sql}"
        );
    }
    c.execute_batch("update consolidation_results set state='prepared',canonical_output='{}',expected_revision=1,origin_id='agent-origin'; update consolidation_results set state='complete',completion_receipt='{}';").unwrap();
    assert_eq!(
        scalar(&c, "select count(*) from pragma_foreign_key_check"),
        0
    );
}

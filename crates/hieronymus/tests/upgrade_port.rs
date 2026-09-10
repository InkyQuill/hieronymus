//! Write-side upgrade protocol (database-upgrade design, 2026-08-31; ADR
//! 0010/0011; data-root config migration design): staged config conversion,
//! the backup set, the cutover journal, the one transaction, atomic
//! promotion, resume, recovery, and failure injection at every protocol
//! step. Sentinel secrets prove reports and receipts never leak key
//! material.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION;
use hieronymus::migrate::{MigrateError, REFUSAL_DAEMON_ACTIVE, run_dry_run_in};
use hieronymus::ownership::RootOwnership;
use hieronymus::upgrade::{
    CutoverJournal, InjectionPoint, StagedFileRecord, UpgradeOptions, daemon_start_blocker,
    read_cutover_journal, run_recovery, run_upgrade,
};
use rusqlite::Connection;

const GLOBAL_SQL: &str = include_str!("../migrations/global.sql");

const PROVIDER_SENTINEL: &str = "sk-sentinel-upgrade-key-9b2c";
const MEMORY_SENTINEL: &str = "сентинель-память-кристалл";

/// Every protocol step boundary that aborts the run like a crashed process,
/// in protocol order. `InjectionPoint::VerificationFailed` is deliberately
/// absent: it takes the real `verification-failed` refusal path rather than
/// returning `MigrateError::Injected`, so it has its own test
/// (`rust_upgrade.rs::a_failed_verification_refuses_the_upgrade_and_rolls_back`).
const ALL_INJECTION_POINTS: &[InjectionPoint] = &[
    InjectionPoint::AfterPreflight,
    InjectionPoint::AfterStaging,
    InjectionPoint::AfterBackup,
    InjectionPoint::AfterJournalPrepared,
    InjectionPoint::BeforeCommit,
    InjectionPoint::AfterCommit,
    InjectionPoint::AfterDatabaseCommitted,
    InjectionPoint::MidPromotion,
    InjectionPoint::AfterReceipt,
    InjectionPoint::AfterComplete,
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn write_legacy_fixture(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let path = root.join("hieronymus.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(GLOBAL_SQL).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
        .execute(
            "insert into series(slug, title, default_source_language,
                                default_target_language, created_at, updated_at)
             values ('demo', 'demo', 'ja', 'en', '2026-01-01T00:00:00+00:00',
                     '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into strict_terms(series_slug, source_language, target_language,
                                      category, source_text, canonical_translation,
                                      status, notes, created_at, updated_at)
             values ('demo', 'ja', 'en', 'name', 'センス', 'sense', 'approved', '',
                     '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into strict_terms_fts(rowid, source_text, canonical_translation, notes)
             values (1, 'センス', 'sense', '')",
            [],
        )
        .unwrap();
    connection.close().unwrap();
    path
}

const DREAM_CONF_LEGACY: &str = "# keep this leading comment intact
[dreaming]
enabled = false
schedule_interval_minutes = 45

# legacy provider payload below the workflows
[providers.openai]
name = \"Openai\"
type = \"openai\"
url = \"https://api.openai.example/v1\"
api_key = \"sk-sentinel-upgrade-key-9b2c\"
timeout_seconds = 12

[workflows.terminology_candidates]
provider = \"openai\"
model = \"gpt-demo\"
enabled = true
max_records_per_pass = 20
";

fn write_legacy_configs(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("dream.conf"), DREAM_CONF_LEGACY).unwrap();
    // The legacy payload carries an api key, so the file must be user-only
    // for the credential-permission preflight to accept it.
    std::fs::set_permissions(
        root.join("dream.conf"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::write(
        root.join("ingest.conf"),
        // Byte-exact with the canonical current-format render, so the typed
        // round-trip leaves the file unstaged and byte-identical.
        "[short_memory]\nwarning_sentence_count = 5\nrejection_sentence_count = 25\n\
         warning_symbol_count = 0\nrejection_symbol_count = 0\n\n\
         [learn]\nmax_block_chars = 900\n",
    )
    .unwrap();
    std::fs::write(root.join("release.conf"), "[updates]\nchannel = \"dev\"\n").unwrap();
    std::fs::write(
        root.join("llmcache.tmp"),
        "{\"providers\": {\"openai\": {\"models\": []}}}",
    )
    .unwrap();
}

fn config(root: &Path) -> HieronymusConfig {
    HieronymusConfig::new(root)
}

/// A pristine copy of the settled fixture in a fresh root.
fn fresh_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    write_legacy_configs(root.path());
    root
}

fn query_scalar(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap()
}

fn open(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection
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
            } else if path.file_name().and_then(|name| name.to_str())
                != Some(hieronymus::ownership::OWNER_LOCK_FILE)
            {
                // The shared ownership lock is test infrastructure, not data:
                // it is created on every `run_upgrade` and left in place
                // (its inode is never unlinked), so exclude it from the
                // "nothing changed" digest.
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

fn journal_state(root: &Path) -> Option<String> {
    read_cutover_journal(&config(root))
        .unwrap()
        .map(|journal| journal.state)
}

fn write_journal_file(root: &Path, state: &str) {
    let journal = CutoverJournal {
        journal_version: 1,
        state: state.to_string(),
        source_state: "python-schema".to_string(),
        target_schema_version: SUPPORTED_RUST_SCHEMA_VERSION,
        staging_dir: ".migrate-staging".to_string(),
        backup_dir: "backups/pre-upgrade-test".to_string(),
        staged: Vec::<StagedFileRecord>::new(),
        backup_database_sha256: "0".repeat(64),
        backup_configs: BTreeMap::new(),
        updated_at: "2026-09-04T00:00:00+00:00".to_string(),
    };
    std::fs::write(
        root.join("cutover.json"),
        serde_json::to_string_pretty(&journal).unwrap(),
    )
    .unwrap();
}

/// The converted-rule count the settled fixture must produce.
fn expected_converted() -> i64 {
    1
}

// ---------------------------------------------------------------------------
// Happy path: the full cutover
// ---------------------------------------------------------------------------

#[test]
fn upgrade_completes_the_full_cutover() {
    let root = fresh_fixture();
    let original_ingest = std::fs::read_to_string(root.path().join("ingest.conf")).unwrap();
    let original_release = std::fs::read_to_string(root.path().join("release.conf")).unwrap();

    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete", "{report:?}");
    assert!(!report.resumed);
    assert_eq!(report.journal_state, "complete");
    let conversion = report.conversion.as_ref().unwrap();
    assert_eq!(conversion.converted, expected_converted() as u64);
    let verification = report.verification.as_ref().unwrap();
    assert_eq!(verification.foreign_key_violations, 0);
    assert_eq!(verification.integrity, "ok");
    assert!(verification.row_accounting_ok);
    assert!(verification.fts_equivalent);
    assert!(verification.domain_invariants_ok);
    // The durable semantic rebuild job was created in the same transaction.
    let job = report.semantic_job.as_ref().unwrap();
    assert!(job.job_id.starts_with("rebuild:upgrade-"), "{job:?}");
    assert_eq!(job.status, "queued");

    // Database side: converted rules, ledger, FTS, schema version, job row.
    let connection = open(&root.path().join("hieronymus.sqlite"));
    assert_eq!(
        query_scalar(&connection, "pragma user_version"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        expected_converted()
    );
    assert!(query_scalar(&connection, "select count(*) from term_migration_ledger") > 0);
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from semantic_jobs where status = 'queued'"
        ),
        1
    );
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from semantic_generations where status = 'building'"
        ),
        1
    );
    drop(connection);

    // Journal: complete, with staged and backup checksums recorded.
    let journal = read_cutover_journal(&config(root.path())).unwrap().unwrap();
    assert_eq!(journal.state, "complete");
    assert_eq!(journal.source_state, "python-schema");
    assert_eq!(journal.target_schema_version, SUPPORTED_RUST_SCHEMA_VERSION);
    assert!(journal.backup_database_sha256.len() == 64);
    assert!(
        journal
            .staged
            .iter()
            .any(|file| file.path == "provider.conf" && file.user_only)
    );

    // Backup set: database plus every original config, immutable.
    let sets = backup_sets(root.path());
    assert_eq!(sets.len(), 1, "{sets:?}");
    assert!(sets[0].join("hieronymus.sqlite").exists());
    assert!(sets[0].join("dream.conf").exists());
    let backed_up_dream = std::fs::read_to_string(sets[0].join("dream.conf")).unwrap();
    assert!(
        backed_up_dream.contains("[providers.openai]"),
        "backup keeps originals"
    );

    // Receipt: versions and checksums recorded next to the backup.
    let receipt_path = sets[0].join("receipt.json");
    assert_eq!(report.receipt_path.as_deref(), Some(receipt_path.as_path()));
    let receipt = std::fs::read_to_string(&receipt_path).unwrap();
    assert!(
        receipt.contains(&format!(
            "\"target_schema_version\": {SUPPORTED_RUST_SCHEMA_VERSION}"
        )),
        "{receipt}"
    );
    assert!(receipt.contains("backup_database_sha256"), "{receipt}");
    assert!(receipt.contains("migration_report_checksum"), "{receipt}");

    // Config promotion: staged current-format files are live now.
    let dream = std::fs::read_to_string(root.path().join("dream.conf")).unwrap();
    assert!(!dream.contains("[providers.openai]"), "{dream}");
    assert!(
        dream.contains("keep this leading comment intact"),
        "{dream}"
    );
    assert!(dream.contains("schedule_interval_minutes = 45"), "{dream}");
    let provider_conf = std::fs::read_to_string(root.path().join("provider.conf")).unwrap();
    assert!(
        provider_conf.contains(PROVIDER_SENTINEL),
        "exact key preserved"
    );
    assert!(provider_conf.contains("url = \"https://api.openai.example/v1\""));
    // The converted workflow still resolves against the staged catalog.
    assert!(dream.contains("provider = \"openai\""));
    // Unchanged authoritative files remain byte-identical.
    assert_eq!(
        std::fs::read_to_string(root.path().join("ingest.conf")).unwrap(),
        original_ingest
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("release.conf")).unwrap(),
        original_release
    );
    // Staging is cleaned up; the derived cache was invalidated.
    assert!(!root.path().join(".migrate-staging").exists());
    assert!(!root.path().join("llmcache.tmp").exists());
    // Ownership is released (the OS lock, not the inode): another owner can
    // take it now.
    assert!(RootOwnership::acquire(&config(root.path()), "test").is_ok());
    // The daemon may start again.
    assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);

    // Rerunning a complete cutover does nothing.
    let rerun = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(rerun.outcome.as_str(), "already-complete");
    assert_eq!(backup_sets(root.path()).len(), 1, "no second backup");
}

#[test]
fn already_complete_reconstructs_a_missing_receipt_from_the_journal() {
    // Astra receipt/finalization follow-up: the receipt is now written before
    // the terminal `complete` transition, but an older `complete` journal that
    // LACKS a receipt (the previous step order crashed between them) must be
    // finalized idempotently from the journal's stored checksums, never
    // accepted as-is.
    let root = fresh_fixture();
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    let receipt = report.receipt_path.unwrap();
    assert!(receipt.exists());
    let original = std::fs::read_to_string(&receipt).unwrap();

    // Simulate the crash window: a `complete` journal with no receipt.
    std::fs::remove_file(&receipt).unwrap();
    assert_eq!(journal_state(root.path()).as_deref(), Some("complete"));

    let rerun = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(rerun.outcome.as_str(), "already-complete");
    assert_eq!(rerun.receipt_path.as_deref(), Some(receipt.as_path()));
    assert!(receipt.exists(), "the receipt was reconstructed");

    let rebuilt = std::fs::read_to_string(&receipt).unwrap();
    assert!(
        rebuilt.contains(&format!(
            "\"target_schema_version\": {SUPPORTED_RUST_SCHEMA_VERSION}"
        )),
        "{rebuilt}"
    );
    assert!(rebuilt.contains("backup_database_sha256"), "{rebuilt}");
    assert!(rebuilt.contains("migration_report_checksum"), "{rebuilt}");
    // Same durable checksums as the original receipt.
    for field in ["backup_database_sha256", "migration_report_checksum"] {
        let value = |text: &str| {
            text.lines()
                .find(|line| line.contains(field))
                .map(|line| line.to_string())
                .unwrap()
        };
        assert_eq!(value(&original), value(&rebuilt), "{field}");
    }
    // No key material leaked into the reconstruction.
    assert!(!rebuilt.contains(PROVIDER_SENTINEL), "{rebuilt}");

    // Reconstruction is idempotent.
    let third = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(third.outcome.as_str(), "already-complete");
    assert_eq!(
        std::fs::read_to_string(&receipt).unwrap(),
        rebuilt,
        "a present receipt is left untouched"
    );
}

#[test]
fn a_resume_after_the_receipt_write_never_rewrites_the_receipt() {
    // The receipt is written before the journal's `complete` transition. A
    // crash in that window leaves `config_promotion_required`; the resume must
    // finish the journal WITHOUT overwriting the good receipt with a fresh
    // `completed_at` and possibly-different derived fields.
    let root = fresh_fixture();
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::AfterReceipt),
    };
    run_upgrade(&config(root.path()), false, &options).unwrap_err();
    assert_eq!(
        journal_state(root.path()).as_deref(),
        Some("config_promotion_required")
    );
    let sets = backup_sets(root.path());
    let receipt = sets[0].join("receipt.json");
    let original = std::fs::read(&receipt).unwrap();

    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
    assert!(report.resumed);
    assert_eq!(journal_state(root.path()).as_deref(), Some("complete"));
    assert_eq!(
        std::fs::read(&receipt).unwrap(),
        original,
        "the resume rewrote a receipt that was already durable"
    );
}

#[test]
fn upgrade_refuses_when_the_daemon_is_active() {
    let root = fresh_fixture();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), true, &UpgradeOptions::default()).unwrap_err();
    assert!(error.to_string().contains(REFUSAL_DAEMON_ACTIVE), "{error}");
    assert_eq!(file_tree_digest(root.path()), before, "data root changed");
}

// ---------------------------------------------------------------------------
// Fail-closed before mutation (config design §Preflight)
// ---------------------------------------------------------------------------

#[test]
fn upgrade_fails_closed_on_invalid_config_before_any_mutation() {
    let root = fresh_fixture();
    std::fs::write(root.path().join("dream.conf"), "not [valid toml").unwrap();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(matches!(error, MigrateError::ConfigInvalid(_)), "{error}");
    assert_eq!(file_tree_digest(root.path()), before, "mutation happened");
    assert_eq!(journal_state(root.path()), None);
}

#[test]
fn upgrade_fails_closed_on_unsafe_credential_permissions() {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    // A valid catalog carrying a key, but group/other readable.
    let provider_conf = root.path().join("provider.conf");
    std::fs::write(
        &provider_conf,
        "[openai]\nname = \"Openai\"\ntype = \"openai\"\nurl = \"https://api.openai.example/v1\"\nkey = \"sk-sentinel-upgrade-key-9b2c\"\ntimeout_seconds = 12\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider_conf, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(
        matches!(error, MigrateError::UnsafeCredentialPermissions(_)),
        "{error}"
    );
    assert_eq!(file_tree_digest(root.path()), before);
}

#[test]
fn upgrade_fails_closed_on_provider_collision() {
    let root = fresh_fixture();
    // Existing profile with the same id but a different endpoint.
    let provider_conf = root.path().join("provider.conf");
    std::fs::write(
        &provider_conf,
        "[openai]\nname = \"Openai\"\ntype = \"openai\"\nurl = \"https://other.example/v1\"\nkey = \"x\"\ntimeout_seconds = 30\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider_conf, std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(matches!(error, MigrateError::ConfigInvalid(_)), "{error}");
    assert!(error.to_string().contains("overwrite"), "{error}");
    assert_eq!(file_tree_digest(root.path()), before);
}

#[test]
fn upgrade_fails_closed_on_unresolvable_enabled_workflow() {
    let root = fresh_fixture();
    std::fs::write(
        root.path().join("dream.conf"),
        "[workflows.terminology_candidates]\nprovider = \"ghost\"\nmodel = \"m\"\nenabled = true\n",
    )
    .unwrap();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(
        matches!(error, MigrateError::WorkflowUnresolved(_)),
        "{error}"
    );
    assert_eq!(file_tree_digest(root.path()), before);
}

#[test]
fn upgrade_fails_closed_on_unknown_release_channel() {
    let root = fresh_fixture();
    std::fs::write(
        root.path().join("release.conf"),
        "[updates]\nchannel = \"beta\"\n",
    )
    .unwrap();
    let before = file_tree_digest(root.path());
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(matches!(error, MigrateError::ConfigInvalid(_)), "{error}");
    assert_eq!(file_tree_digest(root.path()), before);
}

// ---------------------------------------------------------------------------
// Failure injection at every protocol step
// ---------------------------------------------------------------------------

#[test]
fn failure_injection_leaves_only_safe_end_states_and_resume_completes() {
    for point in ALL_INJECTION_POINTS {
        let root = fresh_fixture();
        let options = UpgradeOptions {
            injection: Some(*point),
        };
        let result = run_upgrade(&config(root.path()), false, &options);
        assert!(
            matches!(&result, Err(MigrateError::Injected(injected)) if injected == point),
            "{point:?}: {result:?}"
        );

        match point {
            InjectionPoint::AfterComplete => {
                // A crash after the receipt is a complete cutover.
                assert_eq!(journal_state(root.path()).as_deref(), Some("complete"));
                assert_eq!(
                    daemon_start_blocker(&config(root.path())).unwrap(),
                    None,
                    "complete cutover must not block the daemon"
                );
                continue;
            }
            _ => {
                // Every earlier crash leaves a root the daemon refuses:
                // either the journal gate is shut, or the database is still
                // in its original non-Rust state (never a startable middle
                // state with mismatched database/config versions).
                let blocker = daemon_start_blocker(&config(root.path())).unwrap();
                let database =
                    hieronymus::db::classify_database(&root.path().join("hieronymus.sqlite"));
                let daemon_startable = blocker.is_none()
                    && matches!(database, hieronymus::db::DatabaseState::RustSchema { .. });
                assert!(
                    !daemon_startable,
                    "{point:?}: daemon could start in the middle state"
                );
            }
        }

        // Before-commit injections leave the original database intact.
        if matches!(
            point,
            InjectionPoint::AfterPreflight
                | InjectionPoint::AfterStaging
                | InjectionPoint::AfterBackup
                | InjectionPoint::AfterJournalPrepared
                | InjectionPoint::BeforeCommit
        ) {
            let connection = open(&root.path().join("hieronymus.sqlite"));
            assert_eq!(
                query_scalar(
                    &connection,
                    "select count(*) from sqlite_master where type = 'table'
                     and name = 'term_rules'"
                ),
                0,
                "{point:?}: target schema leaked past the rollback"
            );
            drop(connection);
        }

        // Rerunning without injection always finishes the cutover.
        let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
            .unwrap_or_else(|error| panic!("{point:?}: resume failed: {error}"));
        assert_eq!(report.outcome.as_str(), "complete", "{point:?}");
        // Only the preflight boundary leaves no durable trace of the earlier
        // attempt, so every other resume continues an interrupted cutover.
        let expect_resumed = *point != InjectionPoint::AfterPreflight;
        assert_eq!(report.resumed, expect_resumed, "{point:?}");

        // Final state is a fully promoted cutover with the daemon unblocked.
        let connection = open(&root.path().join("hieronymus.sqlite"));
        assert_eq!(
            query_scalar(&connection, "select count(*) from term_rules"),
            expected_converted(),
            "{point:?}"
        );
        drop(connection);
        assert_eq!(
            journal_state(root.path()).as_deref(),
            Some("complete"),
            "{point:?}"
        );
        assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);
        let dream = std::fs::read_to_string(root.path().join("dream.conf")).unwrap();
        assert!(!dream.contains("[providers.openai]"), "{point:?}: {dream}");
        assert!(!root.path().join(".migrate-staging").exists(), "{point:?}");
        assert!(
            RootOwnership::acquire(&config(root.path()), "test").is_ok(),
            "{point:?}: ownership must be released after a resumed cutover"
        );
    }
}

#[test]
fn injected_commit_never_reruns_the_converters_on_resume() {
    let root = fresh_fixture();
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::AfterCommit),
    };
    run_upgrade(&config(root.path()), false, &options).unwrap_err();

    // The database is committed: rules exist and the ledger is complete.
    let connection = open(&root.path().join("hieronymus.sqlite"));
    let committed_rules = query_scalar(&connection, "select count(*) from term_rules");
    assert_eq!(committed_rules, expected_converted());
    drop(connection);

    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
    assert!(report.resumed);
    // Resume promotes config only: no second conversion pass ran.
    let connection = open(&root.path().join("hieronymus.sqlite"));
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        committed_rules
    );
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from term_migration_ledger where outcome = 'converted'"
        ),
        committed_rules
    );
    drop(connection);
}

#[test]
fn resume_verifies_staged_checksums_before_promoting() {
    let root = fresh_fixture();
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::AfterCommit),
    };
    run_upgrade(&config(root.path()), false, &options).unwrap_err();

    // Keep the verified staging bytes, then corrupt the staged file behind
    // the journal's back.
    let staged = root.path().join(".migrate-staging").join("dream.conf");
    let verified = std::fs::read_to_string(&staged).unwrap();
    std::fs::write(&staged, "[dreaming]\nenabled = false\n").unwrap();

    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(
        matches!(error, MigrateError::StagedChecksumMismatch { .. }),
        "{error}"
    );
    // Nothing was promoted: the live file still carries the legacy block.
    let dream = std::fs::read_to_string(root.path().join("dream.conf")).unwrap();
    assert!(dream.contains("[providers.openai]"));

    // Restoring the verified staged bytes lets the resume complete.
    std::fs::write(&staged, verified).unwrap();
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
}

// ---------------------------------------------------------------------------
// Daemon gate over journal states
// ---------------------------------------------------------------------------

#[test]
fn daemon_gate_refuses_every_unfinished_journal_state() {
    for state in [
        "prepared",
        "database_committed",
        "config_promotion_required",
    ] {
        let root = fresh_fixture();
        write_journal_file(root.path(), state);
        let blocker = daemon_start_blocker(&config(root.path())).unwrap();
        assert_eq!(blocker.as_deref(), Some(state), "state {state}");
    }
    // An absent journal and a complete journal never block.
    let root = fresh_fixture();
    assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);
    write_journal_file(root.path(), "complete");
    assert_eq!(daemon_start_blocker(&config(root.path())).unwrap(), None);
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

#[test]
fn recovery_rebuilds_a_new_database_from_the_immutable_backup() {
    let root = fresh_fixture();
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    let sets = backup_sets(root.path());
    assert_eq!(sets.len(), 1);
    let backup_digest = file_tree_digest(&sets[0]);

    // The live database is destroyed (the scenario recovery exists for).
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
    // The rebuilt database is a verified Rust database with converted rules.
    let connection = open(&root.path().join("hieronymus.sqlite"));
    assert_eq!(
        query_scalar(&connection, "pragma user_version"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from term_rules"),
        expected_converted()
    );
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from sqlite_master where type = 'table'
             and name = 'term_migration_ledger'"
        ),
        1,
        "recovery imports through the current converter"
    );
    drop(connection);

    // The original backup was never touched or deleted.
    assert_eq!(file_tree_digest(&sets[0]), backup_digest, "backup mutated");
    assert!(backup_sets(root.path()).contains(&sets[0]));
    // The replaced live database was preserved, not deleted.
    assert!(backup_sets(root.path()).len() >= 2);
    // Live config files were not regressed to the pre-upgrade originals.
    let dream = std::fs::read_to_string(root.path().join("dream.conf")).unwrap();
    assert!(!dream.contains("[providers.openai]"), "{dream}");

    // Recovery is repeatable; it never deletes the last verified backup.
    let second = run_recovery(&config(root.path()), false)
        .map_err(|error| error.to_string())
        .unwrap();
    assert!(second.recovered_from.exists());
    assert!(backup_sets(root.path()).contains(&sets[0]));
}

#[test]
fn recovery_refuses_a_mid_cutover_state_and_a_missing_backup() {
    let root = fresh_fixture();
    // No backup at all: nothing to recover from.
    let error = run_recovery(&config(root.path()), false).unwrap_err();
    assert!(matches!(error, MigrateError::BackupMissing), "{error}");

    // A mid-cutover state must be resumed, not recovered over.
    let root = fresh_fixture();
    let options = UpgradeOptions {
        injection: Some(InjectionPoint::AfterCommit),
    };
    run_upgrade(&config(root.path()), false, &options).unwrap_err();
    let error = run_recovery(&config(root.path()), false).unwrap_err();
    assert!(matches!(error, MigrateError::RecoveryBlocked(_)), "{error}");
}

#[test]
fn recovery_refuses_an_active_daemon_before_touching_anything() {
    let root = fresh_fixture();
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    std::fs::write(root.path().join("hieronymus.sqlite"), b"corrupted").unwrap();
    let before = file_tree_digest(root.path());

    let error = run_recovery(&config(root.path()), true).unwrap_err();
    assert!(error.to_string().contains(REFUSAL_DAEMON_ACTIVE), "{error}");
    // Nothing was replaced or moved: the serving daemon keeps its database.
    assert_eq!(file_tree_digest(root.path()), before);
}

#[test]
fn recovery_secures_the_live_database_with_its_wal_sidecars() {
    let root = fresh_fixture();
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();

    // Simulate a WAL-mode live database: a `-wal` sidecar next to the main
    // file. Recovery must preserve it beside the secured main file, and the
    // new database must never inherit a stale sidecar.
    std::fs::write(
        root.path().join("hieronymus.sqlite-wal"),
        b"simulated wal frames",
    )
    .unwrap();
    std::fs::write(root.path().join("hieronymus.sqlite"), b"corrupted").unwrap();

    let report = run_recovery(&config(root.path()), false)
        .map_err(|error| error.to_string())
        .unwrap();

    let replacement = &report.replacement_backup_dir;
    assert!(
        replacement.join("hieronymus.sqlite").exists(),
        "replaced live db preserved"
    );
    assert_eq!(
        std::fs::read(replacement.join("hieronymus.sqlite-wal")).unwrap(),
        b"simulated wal frames",
        "the live WAL sidecar must be preserved with the replaced db"
    );
    // The rebuilt database lives alone: no stale sidecar next to it.
    assert!(!root.path().join("hieronymus.sqlite-wal").exists());
    assert!(!root.path().join("hieronymus.sqlite-shm").exists());
    // And the live database file itself is present and converted (the
    // promotion is a single rename, never a remove-then-rename window).
    let connection = open(&root.path().join("hieronymus.sqlite"));
    assert_eq!(
        query_scalar(&connection, "pragma user_version"),
        SUPPORTED_RUST_SCHEMA_VERSION
    );
    drop(connection);
}

// ---------------------------------------------------------------------------
// Data-root ownership guard (shared OS lock; see tests/ownership.rs for the
// primitive's own coverage, including the SIGKILL-release case)
// ---------------------------------------------------------------------------

#[test]
fn upgrade_refuses_a_root_another_owner_holds() {
    let root = fresh_fixture();
    let held = RootOwnership::acquire(&config(root.path()), "daemon").unwrap();
    let before = file_tree_digest(root.path());

    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap_err();
    assert!(matches!(error, MigrateError::RootOwnership(_)), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("daemon"),
        "diagnostic names the owner: {message}"
    );
    // The upgrade touched nothing.
    assert_eq!(file_tree_digest(root.path()), before);

    // Releasing the guard lets the upgrade proceed.
    drop(held);
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
}

#[test]
fn recovery_refuses_a_root_another_owner_holds() {
    let root = fresh_fixture();
    // A completed cutover so recovery gets past the journal gate.
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap();
    let held = RootOwnership::acquire(&config(root.path()), "daemon").unwrap();

    let error = run_recovery(&config(root.path()), false).unwrap_err();
    assert!(matches!(error, MigrateError::RootOwnership(_)), "{error}");
    drop(held);
}

// ---------------------------------------------------------------------------
// Sentinel secrets
// ---------------------------------------------------------------------------

#[test]
fn sentinels_never_reach_reports_journals_receipts_or_errors() {
    let root = fresh_fixture();
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();

    // The success surfaces are clean; the memory sentinel never participates
    // at all, and the provider sentinel only ever lives in provider.conf.
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains(PROVIDER_SENTINEL), "{rendered}");
    assert!(!rendered.contains(MEMORY_SENTINEL), "{rendered}");
    let journal_bytes = std::fs::read_to_string(root.path().join("cutover.json")).unwrap();
    assert!(
        !journal_bytes.contains(PROVIDER_SENTINEL),
        "{journal_bytes}"
    );
    assert!(!journal_bytes.contains(MEMORY_SENTINEL), "{journal_bytes}");
    let sets = backup_sets(root.path());
    let receipt = std::fs::read_to_string(sets[0].join("receipt.json")).unwrap();
    assert!(!receipt.contains(PROVIDER_SENTINEL), "{receipt}");
    // The error text of a failed run carries no key material either.
    let root = fresh_fixture();
    let provider_conf = root.path().join("provider.conf");
    std::fs::write(
        &provider_conf,
        format!("[openai]\nname = \"O\"\ntype = \"openai\"\nurl = \"https://x.example\"\nkey = \"{PROVIDER_SENTINEL}\"\ntimeout_seconds = 5\n[openai.broken]\nnope = true\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider_conf, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .unwrap_err()
        .to_string();
    assert!(!error.contains(PROVIDER_SENTINEL), "{error}");

    // Dry-run remains clean too.
    let work = tempfile::tempdir().unwrap();
    let dry = run_dry_run_in(&config(root.path()), false, work.path()).unwrap();
    let dry_rendered = serde_json::to_string(&dry).unwrap();
    assert!(!dry_rendered.contains(PROVIDER_SENTINEL));
}

// ---------------------------------------------------------------------------
// Config staging details
// ---------------------------------------------------------------------------

#[test]
fn staging_preserves_existing_provider_entries_and_formats() {
    let root = fresh_fixture();
    // An existing, unrelated profile that must survive the conversion.
    std::fs::write(
        root.path().join("provider.conf"),
        "# my hand-maintained catalog\n[ollama]\nname = \"Local\"\ntype = \"ollama\"\nurl = \"http://127.0.0.1:11434\"\nkey = \"\"\ntimeout_seconds = 60\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(root.path().join("provider.conf")).unwrap();

    run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();

    let after = std::fs::read_to_string(root.path().join("provider.conf")).unwrap();
    assert!(after.contains("[ollama]"), "{after}");
    assert!(after.contains("my hand-maintained catalog"), "{after}");
    assert!(after.contains("[openai]"), "{after}");
    assert!(after.contains(PROVIDER_SENTINEL));
    assert_ne!(before, after);
    // Secret-bearing staged file lands with user-only permissions.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(root.path().join("provider.conf"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "provider.conf must be user-only: {mode:o}");
}

#[test]
fn a_root_without_legacy_config_still_cutovers() {
    let root = tempfile::tempdir().unwrap();
    write_legacy_fixture(root.path());
    let report = run_upgrade(&config(root.path()), false, &UpgradeOptions::default())
        .map_err(|error| error.to_string())
        .unwrap();
    assert_eq!(report.outcome.as_str(), "complete");
    assert!(report.semantic_job.is_some());
    assert_eq!(journal_state(root.path()).as_deref(), Some("complete"));
}

#[test]
fn python_cutover_backfills_converted_rules_before_v5_commit() {
    let root = fresh_fixture();
    run_upgrade(&config(root.path()), false, &UpgradeOptions::default()).unwrap();
    let connection = open(&root.path().join("hieronymus.sqlite"));
    assert_eq!(query_scalar(&connection, "pragma user_version"), 5);
    assert_eq!(
        query_scalar(&connection, "select count(*) from rule_authority"),
        expected_converted()
    );
    assert_eq!(
        query_scalar(
            &connection,
            "select count(*) from rule_authority where authority='explicit_user' and legacy_protected=1"
        ),
        expected_converted()
    );
    assert_eq!(
        query_scalar(&connection, "select count(*) from pragma_foreign_key_check"),
        0
    );
}

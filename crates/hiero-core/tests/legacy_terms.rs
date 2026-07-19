use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use fs4::FileExt;
use hiero_core::db::{DbError, convert_legacy_strict_terms, migrate};
use sqlx::{
    AssertSqlSafe, Executor, Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::TempDir;
use tokio::sync::Barrier;

static TEST_MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

async fn pre_drop_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("fixture pool should connect");
    TEST_MIGRATOR
        .run_to(4, &pool)
        .await
        .expect("pre-conversion migrations should install");
    pool.execute(sqlx::raw_sql(LEGACY_SCHEMA))
        .await
        .expect("legacy fixture schema should install");
    pool.execute("INSERT INTO series(id, slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES (1, 'book', 'Book', 'ja', 'en', '2026-07-18T10:00:00Z', '2026-07-18T10:00:00Z')")
        .await
        .expect("fixture series should insert");
    pool
}

async fn file_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .pragma("recursive_triggers", "ON");
    SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .expect("file fixture pool should connect")
}

fn migration_sidecar(directory: &Path) -> PathBuf {
    std::fs::read_dir(directory)
        .expect("fixture directory should read")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".hieronymus-migrate-"))
        })
        .expect("persistent migration lock sidecar should exist")
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child-process marker should appear promptly");
}

fn spawn_lock_helper(mode: &str, database: &Path, marker: &Path) -> Child {
    Command::new(std::env::current_exe().expect("current test executable should resolve"))
        .args(["--exact", "migration_lock_process_helper", "--nocapture"])
        .env("HIERONYMUS_LOCK_HELPER_MODE", mode)
        .env("HIERONYMUS_LOCK_HELPER_DATABASE", database)
        .env("HIERONYMUS_LOCK_HELPER_MARKER", marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("migration-lock helper process should start")
}

async fn run_concurrent_migrations(pools: &[SqlitePool]) {
    let barrier = Arc::new(Barrier::new(13));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..12 {
        let pool = pools[index % pools.len()].clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            migrate(&pool).await
        });
    }
    barrier.wait().await;
    while let Some(result) = tasks.join_next().await {
        result
            .expect("migration task should not panic")
            .expect("every concurrent migration should succeed");
    }
}

async fn insert_term(pool: &SqlitePool, id: i64, status: &str, source: &str, target: &str) {
    sqlx::query("INSERT INTO strict_terms(id, series_slug, source_language, target_language, category, source_text, canonical_translation, status, notes, created_at, updated_at) VALUES (?, 'book', 'ja', 'en', 'correction', ?, ?, ?, ?, '2026-07-18T11:00:00Z', '2026-07-18T12:00:00Z')")
        .bind(id)
        .bind(source)
        .bind(target)
        .bind(status)
        .bind(format!("note-{id}"))
        .execute(pool)
        .await
        .expect("legacy term should insert");
}

async fn object_exists(pool: &SqlitePool, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?)")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("schema should be readable")
        != 0
}

#[tokio::test]
async fn conversion_maps_structured_rows_reuses_equivalent_targets_and_drops_legacy_objects() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 10, "approved", "攻撃力上昇", "ATK Up").await;
    insert_term(&pool, 11, "inactive", "防御力上昇", "DEF Up").await;
    insert_term(&pool, 12, "active", "速度上昇", "Speed Up").await;
    sqlx::query("INSERT INTO strict_term_tags(term_id, tag) VALUES (10, 'zeta'), (10, 'shared')")
        .execute(&pool)
        .await
        .expect("tags should insert");
    sqlx::query("INSERT INTO strict_term_aliases(id, term_id, language, text, kind, case_sensitive) VALUES (1, 10, 'en', 'shared', 'approved_variant', 1), (2, 10, 'en', 'Attack Up', 'approved_variant', 1)")
        .execute(&pool).await.expect("aliases should insert");

    let existing_id = sqlx::query("INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, status, created_at, updated_at) VALUES ('rule', '速度上昇 is translated as Speed Up', '速度上昇', 'series', 'series:book', 'book', 'ja', 'en', '[]', 0.8, 0.95, 'user_rule', 'correction', 'note-12', 'active', '2026-07-18T11:00:00Z', '2026-07-18T12:00:00Z')")
        .execute(&pool).await.expect("equivalent crystal should insert").last_insert_rowid();
    sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('strict_terms', '12', 'crystals', ?)")
        .bind(existing_id).execute(&pool).await.expect("existing ledger should insert");
    sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('strict_terms', '12', 'concepts', 999)")
        .execute(&pool).await.expect("a different legacy target ledger should coexist");

    let report = convert_legacy_strict_terms(&pool)
        .await
        .expect("structured conversion should succeed");

    assert_eq!(report.source_rows, 3);
    assert_eq!(report.converted_rows, 2);
    assert_eq!(report.existing_rows, 1);
    assert!(report.dropped);
    for name in [
        "strict_terms",
        "strict_term_tags",
        "strict_term_aliases",
        "strict_terms_fts",
    ] {
        assert!(
            !object_exists(&pool, name).await,
            "{name} should be dropped"
        );
    }
    let shadow_artifacts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE name LIKE 'strict_terms_fts_%'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy FTS shadow count should read");
    assert_eq!(shadow_artifacts, 0);

    let row = sqlx::query("SELECT crystals.text, crystals.title, crystals.scope_type, crystals.scope_key, crystals.series_slug, crystals.source_language, crystals.target_language, crystals.tags_json, crystals.strength, crystals.confidence, crystals.source_credibility, crystals.rule_intent, crystals.soft_origin, crystals.status, crystals.created_at, crystals.updated_at FROM crystals JOIN migration_ledger ON target_id = crystals.id WHERE source_table = 'strict_terms' AND source_id = '10' AND target_table = 'crystals'")
        .fetch_one(&pool).await.expect("converted crystal should be traceable");
    assert_eq!(
        row.get::<String, _>("text"),
        "攻撃力上昇 is translated as ATK Up"
    );
    assert_eq!(row.get::<String, _>("title"), "攻撃力上昇");
    assert_eq!(row.get::<String, _>("scope_type"), "series");
    assert_eq!(row.get::<String, _>("scope_key"), "series:book");
    assert_eq!(row.get::<String, _>("series_slug"), "book");
    assert_eq!(row.get::<String, _>("source_language"), "ja");
    assert_eq!(row.get::<String, _>("target_language"), "en");
    assert_eq!(
        row.get::<String, _>("tags_json"),
        r#"["Attack Up","shared","zeta"]"#
    );
    assert_eq!(row.get::<f64, _>("strength"), 0.8);
    assert_eq!(row.get::<f64, _>("confidence"), 0.95);
    assert_eq!(row.get::<String, _>("source_credibility"), "user_rule");
    assert_eq!(row.get::<String, _>("rule_intent"), "correction");
    assert_eq!(row.get::<String, _>("soft_origin"), "note-10");
    assert_eq!(row.get::<String, _>("status"), "active");
    assert_eq!(
        row.get::<String, _>("created_at"),
        "2026-07-18T11:00:00+00:00"
    );
    assert_eq!(
        row.get::<String, _>("updated_at"),
        "2026-07-18T12:00:00+00:00"
    );

    let tags: Vec<String> = sqlx::query_scalar("SELECT tag FROM crystal_semantic_tags JOIN migration_ledger ON target_id = crystal_id WHERE source_table = 'strict_terms' AND source_id = '10' AND target_table = 'crystals' ORDER BY tag")
        .fetch_all(&pool).await.expect("semantic tags should be readable");
    assert_eq!(tags, ["Attack Up", "shared", "zeta"]);
    let inactive_status: String = sqlx::query_scalar("SELECT status FROM crystals JOIN migration_ledger ON target_id = crystals.id WHERE source_table = 'strict_terms' AND source_id = '11' AND target_table = 'crystals'")
        .fetch_one(&pool).await.expect("inactive term should convert");
    assert_eq!(inactive_status, "archived");

    let second = convert_legacy_strict_terms(&pool)
        .await
        .expect("second conversion should be a no-op");
    assert_eq!(second.source_rows, 0);
    assert_eq!(second.converted_rows, 0);
    assert_eq!(second.existing_rows, 0);
    assert!(!second.dropped);
}

#[tokio::test]
async fn conflicting_ledger_target_rolls_back_and_preserves_every_legacy_object() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 20, "approved", "炎", "Fire").await;
    let wrong = sqlx::query("INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, status, created_at, updated_at) VALUES ('rule', 'wrong', 'wrong', 'series', 'series:book', 'book', 'ja', 'en', '[]', 0.8, 0.95, 'user_rule', 'correction', 'active', '2026-07-18T11:00:00Z', '2026-07-18T12:00:00Z')")
        .execute(&pool).await.expect("conflict target should insert").last_insert_rowid();
    sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('strict_terms', '20', 'crystals', ?)")
        .bind(wrong).execute(&pool).await.expect("conflict ledger should insert");

    let error = convert_legacy_strict_terms(&pool)
        .await
        .expect_err("mismatched target must fail");
    assert!(matches!(error, DbError::LegacyTermConflict { .. }));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM strict_terms")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(object_exists(&pool, "strict_terms_fts").await);
}

#[tokio::test]
async fn every_converter_owned_field_and_tag_attribute_must_match_for_existing_targets() {
    let cases = [
        (
            "is_inferred",
            "UPDATE crystals SET is_inferred = 1 WHERE id = ?",
        ),
        (
            "malformed_penalty",
            "UPDATE crystals SET malformed_penalty = 0.1 WHERE id = ?",
        ),
        (
            "supersedes_crystal_id",
            "UPDATE crystals SET supersedes_crystal_id = id WHERE id = ?",
        ),
        (
            "created_cycle",
            "UPDATE crystals SET created_cycle = 1 WHERE id = ?",
        ),
        (
            "last_activated_cycle",
            "UPDATE crystals SET last_activated_cycle = 0 WHERE id = ?",
        ),
        (
            "last_reinforced_cycle",
            "UPDATE crystals SET last_reinforced_cycle = 0 WHERE id = ?",
        ),
        (
            "tag confidence",
            "UPDATE crystal_semantic_tags SET confidence = 0.5 WHERE crystal_id = ?",
        ),
        (
            "tag timestamp",
            "UPDATE crystal_semantic_tags SET created_at = '2026-07-18T13:00:00Z' WHERE crystal_id = ?",
        ),
    ];

    for (label, mutation) in cases {
        let pool = pre_drop_pool().await;
        insert_term(&pool, 70, "approved", "雷", "Thunder").await;
        sqlx::query("INSERT INTO strict_term_tags(term_id, tag) VALUES (70, 'element')")
            .execute(&pool)
            .await
            .expect("tag should insert");
        let target_id = sqlx::query("INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at) VALUES ('rule', '雷 is translated as Thunder', '雷', 'series', 'series:book', 'book', 'ja', 'en', '[\"element\"]', 0.8, 0.95, 'user_rule', 'correction', 'note-70', 0, 0.0, NULL, 'active', 0, NULL, NULL, '2026-07-18T11:00:00Z', '2026-07-18T12:00:00Z')")
            .execute(&pool).await.expect("equivalent crystal should insert").last_insert_rowid();
        sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id, tag, confidence, created_at) VALUES (?, 'element', 0.95, '2026-07-18T11:00:00Z')")
            .bind(target_id).execute(&pool).await.expect("equivalent tag should insert");
        sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('strict_terms', '70', 'crystals', ?)")
            .bind(target_id).execute(&pool).await.expect("ledger should insert");
        sqlx::query(AssertSqlSafe(mutation.to_owned()))
            .bind(target_id)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("{label} mutation should apply: {error}"));

        let error = match migrate(&pool).await {
            Ok(()) => panic!("{label} drift unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(
            matches!(error, DbError::LegacyTermConflict { .. }),
            "{label} returned {error:?}"
        );
        assert!(object_exists(&pool, "strict_terms").await, "{label}");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM strict_terms")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1,
            "{label}"
        );
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, [1, 2, 3, 4], "{label}");
    }
}

#[tokio::test]
async fn invalid_alias_decode_rolls_back_without_partial_targets_or_ledger() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 30, "approved", "氷", "Ice").await;
    pool.execute("PRAGMA ignore_check_constraints = ON")
        .await
        .unwrap();
    sqlx::query("INSERT INTO strict_term_aliases(id, term_id, language, text, kind, case_sensitive) VALUES (1, 30, 'en', 'Frost', 'approved_variant', 2)")
        .execute(&pool).await.expect("invalid fixture row should insert");

    convert_legacy_strict_terms(&pool)
        .await
        .expect_err("invalid alias boolean must fail");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM migration_ledger")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(object_exists(&pool, "strict_terms").await);
}

#[tokio::test]
async fn injected_ledger_count_mismatch_rolls_back_every_conversion_write() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 40, "approved", "風", "Wind").await;
    pool.execute("CREATE TRIGGER ignore_strict_ledger BEFORE INSERT ON migration_ledger WHEN NEW.source_table = 'strict_terms' BEGIN SELECT RAISE(IGNORE); END")
        .await.expect("fault trigger should install");

    let error = convert_legacy_strict_terms(&pool)
        .await
        .expect_err("ledger mismatch must fail");
    assert!(matches!(error, DbError::LegacyTermCountMismatch { .. }));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM strict_terms")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(object_exists(&pool, "strict_terms_fts").await);
}

#[tokio::test]
async fn migrate_applies_conversion_before_recording_drop_migration() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 50, "approved", "光", "Light").await;

    migrate(&pool)
        .await
        .expect("migration orchestration should converge");

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("migration history should be readable");
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(!object_exists(&pool, "strict_terms").await);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM migration_ledger WHERE source_table = 'strict_terms' AND source_id = '50' AND target_table = 'crystals'").fetch_one(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn failed_conversion_never_records_drop_migration() {
    let pool = pre_drop_pool().await;
    insert_term(&pool, 60, "approved", "闇", "Dark").await;
    pool.execute("PRAGMA ignore_check_constraints = ON")
        .await
        .unwrap();
    sqlx::query("INSERT INTO strict_term_aliases(id, term_id, language, text, kind, case_sensitive) VALUES (1, 60, 'en', 'Shadow', 'approved_variant', 2)")
        .execute(&pool).await.expect("invalid fixture row should insert");

    migrate(&pool)
        .await
        .expect_err("invalid conversion must stop migration 5");

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("migration history should be readable");
    assert_eq!(versions, [1, 2, 3, 4]);
    assert!(object_exists(&pool, "strict_terms").await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_fresh_file_migrations_are_serialized_across_pools() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("fresh.sqlite");
    let first = file_pool(&path).await;
    let second = file_pool(&path).await;

    run_concurrent_migrations(&[first.clone(), second]).await;

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&first)
            .await
            .expect("migration history should read");
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&first)
            .await
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn concurrent_python_file_migrations_convert_once_across_pools() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("legacy.sqlite");
    let first = file_pool(&path).await;
    first
        .execute(sqlx::raw_sql(include_str!(
            "../../../src/hieronymus/migrations/global.sql"
        )))
        .await
        .expect("Python schema should install");
    first.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series(id, slug, title, default_source_language, default_target_language, created_at, updated_at)
        VALUES (1, 'book', 'Book', 'ja', 'en', '2026-07-18T10:00:00Z', '2026-07-18T10:00:00Z');
        INSERT INTO strict_terms(id, series_slug, source_language, target_language, category, source_text, canonical_translation, status, notes, created_at, updated_at)
        VALUES (80, 'book', 'ja', 'en', 'correction', '星', 'Star', 'approved', 'stellar', '2026-07-18T11:00:00Z', '2026-07-18T12:00:00Z');
        INSERT INTO strict_term_tags(term_id, tag) VALUES (80, 'astral');
        INSERT INTO strict_terms_fts(rowid, source_text, canonical_translation, notes)
        VALUES (80, '星', 'Star', 'stellar');
        "#,
    )).await.expect("Python rows should install");
    let second = file_pool(&path).await;

    run_concurrent_migrations(&[first.clone(), second]).await;

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&first)
            .await
            .expect("migration history should read");
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM migration_ledger WHERE source_table = 'strict_terms' AND source_id = '80' AND target_table = 'crystals'",
        )
        .fetch_one(&first)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals WHERE crystal_type = 'rule'")
            .fetch_one(&first)
            .await
            .unwrap(),
        1
    );
    assert!(!object_exists(&first, "strict_terms").await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&first)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&first)
            .await
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn failed_file_migration_releases_protocol_lock_for_retry() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("retry.sqlite");
    let first = file_pool(&path).await;
    TEST_MIGRATOR
        .run_to(4, &first)
        .await
        .expect("pre-conversion migrations should install");
    first
        .execute(sqlx::raw_sql(LEGACY_SCHEMA))
        .await
        .expect("legacy schema should install");
    first.execute("INSERT INTO series(id, slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES (1, 'book', 'Book', 'ja', 'en', '2026-07-18T10:00:00Z', '2026-07-18T10:00:00Z')")
        .await.unwrap();
    insert_term(&first, 90, "invented", "月", "Moon").await;
    let second = file_pool(&path).await;

    migrate(&first)
        .await
        .expect_err("mid-protocol validation failure should escape");
    sqlx::query("UPDATE strict_terms SET status = 'approved' WHERE id = 90")
        .execute(&second)
        .await
        .expect("invalid fixture should be repairable");
    migrate(&second)
        .await
        .expect("a second pool should acquire the released lock and finish");

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&second)
            .await
            .unwrap();
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals WHERE crystal_type = 'rule'")
            .fetch_one(&second)
            .await
            .unwrap(),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
async fn file_migration_sidecar_is_private_persistent_and_never_follows_a_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("protected.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let mode = std::fs::metadata(&lock_path)
        .expect("lock sidecar metadata should read")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    std::fs::remove_file(&lock_path).expect("released fixture lock should be removable");
    let victim = directory.path().join("victim");
    std::fs::write(&victim, b"untouched").expect("victim should be created");
    symlink(&victim, &lock_path).expect("symlink fixture should be created");

    let error = migrate(&pool)
        .await
        .expect_err("migration lock must reject a sidecar symlink");
    assert!(matches!(error, DbError::MigrationLockIo { .. }));
    assert_eq!(std::fs::read(&victim).unwrap(), b"untouched");
}

#[cfg(unix)]
#[tokio::test]
async fn hard_linked_database_alias_is_rejected_before_any_migration() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("hard-linked.sqlite");
    let pool = file_pool(&path).await;
    let alias = directory.path().join("database-alias.sqlite");
    std::fs::hard_link(&path, &alias).expect("database hard-link fixture should be created");

    let error = migrate(&pool)
        .await
        .expect_err("hard-linked database aliases must be rejected");

    assert!(matches!(
        error,
        DbError::UnsupportedDatabaseAlias { links: 2 }
    ));
    let migration_table_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = '_sqlx_migrations'")
            .fetch_one(&pool)
            .await
            .expect("schema should remain readable");
    assert_eq!(migration_table_count, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_sidecar_path_while_contended_is_rejected_after_acquisition() {
    use std::os::unix::fs::OpenOptionsExt;

    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("replacement.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("sidecar should open");
    held.lock_exclusive()
        .expect("fixture should hold sidecar lock");
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move { migrate(&waiting_pool).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let displaced = directory.path().join("displaced.lock");
    std::fs::rename(&lock_path, &displaced).expect("locked inode should be displaced");
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&lock_path)
        .expect("replacement sidecar should be created");
    held.unlock().expect("fixture lock should release");

    let error = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("waiter should stop promptly")
        .expect("waiter should not panic")
        .expect_err("path replacement must invalidate the acquired lock");
    assert!(matches!(error, DbError::MigrationLockIo { .. }));
}

#[cfg(unix)]
#[tokio::test]
async fn database_hard_link_created_while_contended_is_rejected_before_migration() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("late-hard-link.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("sidecar should open");
    held.lock_exclusive()
        .expect("fixture should hold sidecar lock");
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move { migrate(&waiting_pool).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::hard_link(&path, directory.path().join("late-alias.sqlite"))
        .expect("late hard-link fixture should be created");
    held.unlock().expect("fixture lock should release");

    let error = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("waiter should stop promptly")
        .expect("waiter should not panic")
        .expect_err("hard link created while waiting must invalidate acquisition");
    assert!(matches!(
        error,
        DbError::UnsupportedDatabaseAlias { links: 2 }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn sidecar_hard_link_created_while_contended_is_rejected_after_acquisition() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("late-sidecar-link.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("sidecar should open");
    held.lock_exclusive()
        .expect("fixture should hold sidecar lock");
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move { migrate(&waiting_pool).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::hard_link(&lock_path, directory.path().join("sidecar-alias.lock"))
        .expect("late sidecar hard-link fixture should be created");
    held.unlock().expect("fixture lock should release");

    let error = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("waiter should stop promptly")
        .expect("waiter should not panic")
        .expect_err("sidecar hard link created while waiting must invalidate acquisition");
    assert!(matches!(error, DbError::MigrationLockIo { .. }));
}

#[cfg(unix)]
#[test]
fn cancelled_contended_migrations_leave_no_blocking_worker_or_ghost_waiter() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("cancel.sqlite");
        let pool = file_pool(&path).await;
        migrate(&pool)
            .await
            .expect("initial migration should succeed");
        let lock_path = migration_sidecar(directory.path());
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("sidecar should open");
        held.lock_exclusive()
            .expect("fixture should hold sidecar lock");

        for _ in 0..4 {
            let waiting_pool = pool.clone();
            let waiter = tokio::spawn(async move { migrate(&waiting_pool).await });
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!waiter.is_finished(), "contended migration must still wait");
            waiter.abort();
            let _ = waiter.await;
            tokio::time::timeout(
                Duration::from_millis(250),
                tokio::task::spawn_blocking(|| ()),
            )
            .await
            .expect("aborting migration must not leave a blocked worker")
            .expect("blocking-pool probe should not panic");
        }

        held.unlock().expect("fixture lock should release");
        tokio::time::timeout(Duration::from_secs(2), migrate(&pool))
            .await
            .expect("next caller should not wait behind a ghost waiter")
            .expect("next migration should succeed");
    });
}

#[tokio::test]
async fn advisory_lock_excludes_a_real_child_process_until_parent_release() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("process.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("sidecar should open");
    held.lock_exclusive()
        .expect("parent should hold sidecar lock");
    let completed = directory.path().join("child-completed");
    let mut child = spawn_lock_helper("migrate", &path, &completed);
    let child_started = completed.with_extension("started");
    wait_for_file(&child_started).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !completed.exists(),
        "child must remain excluded while parent holds the lock"
    );
    held.unlock().expect("parent lock should release");
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || child.wait()),
    )
    .await
    .expect("child should exit promptly after parent release")
    .expect("child wait worker should not panic")
    .expect("child should be waitable");
    assert!(status.success());
    assert!(
        completed.exists(),
        "child should migrate after parent release"
    );
}

#[tokio::test]
async fn child_process_death_releases_advisory_lock_for_parent() {
    let directory = TempDir::new().expect("temporary directory should be created");
    let path = directory.path().join("process-death.sqlite");
    let pool = file_pool(&path).await;
    migrate(&pool)
        .await
        .expect("initial migration should succeed");
    let lock_path = migration_sidecar(directory.path());
    let marker = directory.path().join("child-held");
    let mut child = spawn_lock_helper("hold", &path, &marker);
    wait_for_file(&marker).await;
    child.kill().expect("child should be terminated");
    child.wait().expect("terminated child should be reaped");

    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("sidecar should open after child death");
    probe
        .try_lock_exclusive()
        .expect("child process death must release the OS lock");
    probe.unlock().expect("probe lock should release");
}

#[test]
fn migration_lock_process_helper() {
    let Ok(mode) = std::env::var("HIERONYMUS_LOCK_HELPER_MODE") else {
        return;
    };
    let database = PathBuf::from(
        std::env::var_os("HIERONYMUS_LOCK_HELPER_DATABASE")
            .expect("helper database path must be configured"),
    );
    let marker = PathBuf::from(
        std::env::var_os("HIERONYMUS_LOCK_HELPER_MARKER")
            .expect("helper marker path must be configured"),
    );
    match mode.as_str() {
        "migrate" => {
            std::fs::write(marker.with_extension("started"), b"started")
                .expect("helper start marker should write");
            let runtime = tokio::runtime::Runtime::new().expect("helper runtime should build");
            runtime.block_on(async {
                let pool = file_pool(&database).await;
                migrate(&pool)
                    .await
                    .expect("child migration should succeed");
            });
            std::fs::write(marker, b"completed").expect("helper completion marker should write");
        }
        "hold" => {
            let lock_path = migration_sidecar(
                database
                    .parent()
                    .expect("helper database should have a parent directory"),
            );
            let file = File::options()
                .read(true)
                .write(true)
                .open(lock_path)
                .expect("helper sidecar should open");
            file.lock_exclusive()
                .expect("helper should hold sidecar lock");
            std::fs::write(marker, b"held").expect("helper held marker should write");
            std::thread::sleep(Duration::from_secs(30));
        }
        other => panic!("unsupported migration-lock helper mode: {other}"),
    }
}

const LEGACY_SCHEMA: &str = r#"
CREATE TABLE strict_terms (
  id INTEGER PRIMARY KEY, series_slug TEXT NOT NULL REFERENCES series(slug),
  source_language TEXT NOT NULL, target_language TEXT NOT NULL, category TEXT NOT NULL,
  source_text TEXT NOT NULL, canonical_translation TEXT NOT NULL, status TEXT NOT NULL,
  notes TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE strict_term_tags (
  term_id INTEGER NOT NULL REFERENCES strict_terms(id) ON DELETE CASCADE,
  tag TEXT NOT NULL, PRIMARY KEY(term_id, tag)
) STRICT;
CREATE TABLE strict_term_aliases (
  id INTEGER PRIMARY KEY, term_id INTEGER NOT NULL REFERENCES strict_terms(id) ON DELETE CASCADE,
  language TEXT NOT NULL, text TEXT NOT NULL, kind TEXT NOT NULL,
  case_sensitive INTEGER NOT NULL DEFAULT 1 CHECK(case_sensitive IN (0, 1))
) STRICT;
CREATE VIRTUAL TABLE strict_terms_fts USING fts5(
  source_text, canonical_translation, notes, content='strict_terms', content_rowid='id'
);
"#;

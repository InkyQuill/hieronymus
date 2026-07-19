use std::str::FromStr;

use hiero_core::db::{DbError, convert_legacy_strict_terms, migrate};
use sqlx::{
    Executor, Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

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
    assert_eq!(versions, [1, 2, 3, 4, 5]);
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

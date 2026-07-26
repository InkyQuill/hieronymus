use std::str::FromStr;

use hiero_core::db::{connect_url, migrate};
use sqlx::{
    AssertSqlSafe, Executor, SqlSafeStr, SqlitePool,
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

const INITIAL_SCHEMA: &str = include_str!("../../../migrations/0001_initial_schema.sql");
const COMPOUND_INDEXES: &str = include_str!("../../../migrations/0003_compound_indexes.sql");
const SEMANTIC_INDEX_STATE: &str =
    include_str!("../../../migrations/0004_semantic_index_state.sql");

const FTS_TABLES: &[(&str, &str, &[&str])] = &[
    ("crystals_fts", "crystals", &["title", "text"]),
    ("short_term_memories_fts", "short_term_memories", &["text"]),
    (
        "concepts_fts",
        "concepts",
        &["canonical_name", "description"],
    ),
    ("concept_facet_fts", "concept_facets", &["value"]),
    (
        "rag_chunks_fts",
        "rag_chunks",
        &["text", "display_text", "location"],
    ),
];

async fn raw_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("memory URL should parse")
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("FTS fixture pool should connect")
}

fn old_migrator() -> Migrator {
    Migrator::with_migrations(vec![
        migration(1, "initial schema", INITIAL_SCHEMA),
        migration(3, "compound indexes", COMPOUND_INDEXES),
        migration(4, "semantic index state", SEMANTIC_INDEX_STATE),
    ])
}

fn migration(version: i64, description: &str, sql: &'static str) -> Migration {
    Migration::new(
        version,
        description.to_owned().into(),
        MigrationType::Simple,
        AssertSqlSafe(sql).into_sql_str(),
        false,
    )
}

async fn pre_fts_pool() -> SqlitePool {
    let pool = raw_pool().await;
    old_migrator()
        .run(&pool)
        .await
        .expect("old [1,3,4] history should install");
    pool
}

async fn seed_parents(pool: &SqlitePool) {
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series (
          id, slug, title, default_source_language, default_target_language, created_at, updated_at
        ) VALUES (1, 'series', 'Series', 'en', 'ru', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO task_sessions (
          id, series_slug, source_language, target_language, task_type, status,
          created_at, last_activity_at
        ) VALUES (1, 'series', 'en', 'ru', 'translation', 'active',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO rag_sources (
          id, series_slug, source_ref, source_type, content_type, checksum, created_at, updated_at
        ) VALUES (1, 'series', 'source-1', 'text', 'text/plain', 'checksum-1',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        "#,
    ))
    .await
    .expect("parent fixtures should insert");
}

async fn seed_preexisting_content(pool: &SqlitePool) {
    seed_parents(pool).await;
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO crystals (
          id, crystal_type, title, text, scope_type, strength, confidence, status,
          created_at, updated_at
        ) VALUES (1, 'lesson', 'crystalpretitle', 'crystalpretext', 'global', 0.5, 0.5,
          'active', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO short_term_memories (
          id, session_id, source_role, kind, text, created_at
        ) VALUES (1, 1, 'agent', 'note', 'memorypretext', '2026-07-19T00:00:00Z');
        INSERT INTO concepts (
          id, canonical_name, description, created_at, updated_at
        ) VALUES (1, 'conceptprename', 'conceptpredescription',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO concept_facets (
          id, concept_id, facet_type, value, created_at, updated_at
        ) VALUES (1, 1, 'name', 'facetprevalue',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO rag_chunks (
          id, source_id, series_slug, chunk_kind, text, display_text, location, created_at
        ) VALUES (1, 1, 'series', 'paragraph', 'ragpretext', 'ragpredisplay', 'ragprelocation',
          '2026-07-19T00:00:00Z');
        "#,
    ))
    .await
    .expect("preexisting content fixtures should insert");
}

async fn matching_rowids(pool: &SqlitePool, fts_table: &str, token: &str) -> Vec<i64> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT rowid FROM {fts_table} WHERE {fts_table} MATCH ? ORDER BY rowid"
    )))
    .bind(token)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|error| panic!("{fts_table} should be searchable: {error}"))
}

async fn assert_match(pool: &SqlitePool, fts_table: &str, token: &str, rowid: i64) {
    assert_eq!(matching_rowids(pool, fts_table, token).await, [rowid]);
}

async fn assert_no_match(pool: &SqlitePool, fts_table: &str, token: &str) {
    assert!(matching_rowids(pool, fts_table, token).await.is_empty());
}

async fn shadow_snapshot(pool: &SqlitePool, fts_table: &str) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(AssertSqlSafe(format!(
        "SELECT id, block FROM {fts_table}_data ORDER BY id"
    )))
    .fetch_all(pool)
    .await
    .unwrap_or_else(|error| panic!("{fts_table} shadow data should be readable: {error}"))
}

async fn assert_fts_integrity(pool: &SqlitePool) {
    for (fts_table, content_table, _) in FTS_TABLES {
        let quoted = format!("\"{}\"", fts_table.replace('"', "\"\""));
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO {quoted}({quoted}, rank) VALUES ('integrity-check', 1)"
        )))
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("{fts_table} integrity-check failed: {error}"));

        let orphan_rowids: Vec<i64> = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT id FROM {fts_table}_docsize EXCEPT SELECT id FROM {content_table}"
        )))
        .fetch_all(pool)
        .await
        .unwrap_or_else(|error| panic!("{fts_table} orphan query should succeed: {error}"));
        assert!(
            orphan_rowids.is_empty(),
            "{fts_table} has orphan index rowids {orphan_rowids:?}"
        );
        let missing_rowids: Vec<i64> = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT id FROM {content_table} EXCEPT SELECT id FROM {fts_table}_docsize"
        )))
        .fetch_all(pool)
        .await
        .unwrap_or_else(|error| panic!("{fts_table} missing-row query should succeed: {error}"));
        assert!(
            missing_rowids.is_empty(),
            "{fts_table} is missing base rowids {missing_rowids:?}"
        );
    }

    let foreign_key_violations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(pool)
            .await
            .expect("foreign_key_check should run");
    assert_eq!(foreign_key_violations, 0);
}

fn compact_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[tokio::test]
async fn migration_creates_exact_external_content_tables_and_narrow_triggers() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("fresh database should migrate");

    let virtual_tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND lower(sql) LIKE 'create virtual table%using fts5%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("virtual table names should read");
    assert_eq!(
        virtual_tables,
        [
            "concept_facet_fts",
            "concepts_fts",
            "crystals_fts",
            "rag_chunks_fts",
            "short_term_memories_fts",
        ]
    );

    for (fts_table, content_table, columns) in FTS_TABLES {
        let table_sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
                .bind(fts_table)
                .fetch_one(&pool)
                .await
                .expect("FTS table SQL should read");
        let normalized = compact_sql(&table_sql);
        let expected_columns = columns.join(",");
        assert!(
            normalized.contains(&format!(
                "usingfts5({expected_columns},content='{content_table}',content_rowid='id')"
            )),
            "unexpected {fts_table} SQL: {table_sql}"
        );

        let trigger_base = if *content_table == "concept_facets" {
            "concept_facets"
        } else {
            content_table
        };
        for suffix in ["ai", "ad", "au"] {
            let name = format!("{trigger_base}_{suffix}");
            let trigger_sql: String = sqlx::query_scalar(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?",
            )
            .bind(&name)
            .fetch_one(&pool)
            .await
            .expect("trigger should inspect");
            let trigger_sql = compact_sql(&trigger_sql);
            let values = columns
                .iter()
                .map(|column| format!("new.{column}"))
                .collect::<Vec<_>>()
                .join(",");
            let old_values = columns
                .iter()
                .map(|column| format!("old.{column}"))
                .collect::<Vec<_>>()
                .join(",");
            let insert =
                format!("insertinto{fts_table}(rowid,{expected_columns})values(new.id,{values})");
            let delete = format!(
                "insertinto{fts_table}({fts_table},rowid,{expected_columns})values('delete',old.id,{old_values})"
            );
            match suffix {
                "ai" => assert!(trigger_sql.contains(&insert), "wrong {name}: {trigger_sql}"),
                "ad" => assert!(trigger_sql.contains(&delete), "wrong {name}: {trigger_sql}"),
                "au" => {
                    assert!(trigger_sql.contains(&delete), "wrong {name}: {trigger_sql}");
                    assert!(trigger_sql.contains(&insert), "wrong {name}: {trigger_sql}");
                }
                _ => unreachable!(),
            }
        }

        let update_name = format!("{trigger_base}_au");
        let update_sql: String = sqlx::query_scalar(
            "SELECT lower(sql) FROM sqlite_schema WHERE type = 'trigger' AND name = ?",
        )
        .bind(&update_name)
        .fetch_one(&pool)
        .await
        .expect("update trigger SQL should read");
        assert!(
            update_sql.contains(&format!(
                "after update of {} on {content_table}",
                std::iter::once("id")
                    .chain(columns.iter().copied())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            "{update_name} must have the exact UPDATE OF list: {update_sql}"
        );
    }

    let trigger_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type = 'trigger' ORDER BY name")
            .fetch_all(&pool)
            .await
            .expect("trigger names should read");
    assert_eq!(trigger_names.len(), 15);
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn upgrade_rebuilds_every_preexisting_row_and_retires_legacy_strict_fts() {
    let pool = pre_fts_pool().await;
    seed_preexisting_content(&pool).await;
    pool.execute(sqlx::raw_sql(
        r#"
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
        INSERT INTO strict_terms(
          id, series_slug, source_language, target_language, category, source_text,
          canonical_translation, status, notes, created_at, updated_at
        ) VALUES (41, 'series', 'en', 'ru', 'correction', 'legacyterm', 'legacyvalue',
          'approved', 'legacy note', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO strict_terms_fts(strict_terms_fts) VALUES ('rebuild');
        "#,
    ))
    .await
    .expect("legacy strict-term FTS fixture should install");
    migrate(&pool)
        .await
        .expect("future migrator should fill version 2");

    for (table, token) in [
        ("crystals_fts", "crystalpretitle"),
        ("short_term_memories_fts", "memorypretext"),
        ("concepts_fts", "conceptprename"),
        ("concept_facet_fts", "facetprevalue"),
        ("rag_chunks_fts", "ragpretext"),
    ] {
        assert_match(&pool, table, token, 1).await;
    }
    let converted_id: i64 = sqlx::query_scalar(
        "SELECT target_id FROM migration_ledger WHERE source_table = 'strict_terms' AND source_id = '41' AND target_table = 'crystals'",
    )
    .fetch_one(&pool)
    .await
    .expect("converted rule should be traceable");
    assert_match(&pool, "crystals_fts", "legacyterm", converted_id).await;
    let legacy_objects: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name LIKE 'strict_term%'")
            .fetch_one(&pool)
            .await
            .expect("legacy object count should read");
    assert_eq!(legacy_objects, 0);

    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("migration history should read");
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn crystals_fts_tracks_each_text_change_ignores_metadata_and_deletes() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    sqlx::query(
        r#"INSERT INTO crystals (
          id, crystal_type, title, text, scope_type, strength, confidence, status, created_at, updated_at
        ) VALUES (2, 'lesson', 'crystaltitleold', 'crystaltextold', 'global', 0.5, 0.5,
          'active', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')"#,
    )
    .execute(&pool)
    .await
    .expect("crystal should insert");
    assert_match(&pool, "crystals_fts", "crystaltitleold", 2).await;
    assert_match(&pool, "crystals_fts", "crystaltextold", 2).await;
    assert_fts_integrity(&pool).await;

    sqlx::query("UPDATE crystals SET title = 'crystaltitlenew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("title should update");
    assert_no_match(&pool, "crystals_fts", "crystaltitleold").await;
    assert_match(&pool, "crystals_fts", "crystaltitlenew", 2).await;
    assert_fts_integrity(&pool).await;
    sqlx::query("UPDATE crystals SET text = 'crystaltextnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("text should update");
    assert_no_match(&pool, "crystals_fts", "crystaltextold").await;
    assert_match(&pool, "crystals_fts", "crystaltextnew", 2).await;
    assert_fts_integrity(&pool).await;

    let before = shadow_snapshot(&pool, "crystals_fts").await;
    sqlx::query("UPDATE crystals SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("metadata should update");
    assert_eq!(shadow_snapshot(&pool, "crystals_fts").await, before);
    assert_fts_integrity(&pool).await;

    sqlx::query("DELETE FROM crystals WHERE id = 2")
        .execute(&pool)
        .await
        .expect("crystal should delete");
    assert_no_match(&pool, "crystals_fts", "crystaltitlenew").await;
    assert_no_match(&pool, "crystals_fts", "crystaltextnew").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn short_term_memory_fts_tracks_text_ignores_metadata_and_handles_direct_and_cascade_delete()
{
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    seed_parents(&pool).await;
    for (id, text) in [(2, "memorydirectold"), (3, "memorycascadeold")] {
        sqlx::query(
            "INSERT INTO short_term_memories (id, session_id, source_role, kind, text, created_at) VALUES (?, 1, 'agent', 'note', ?, '2026-07-19T00:00:00Z')",
        )
        .bind(id)
        .bind(text)
        .execute(&pool)
        .await
        .expect("memory should insert");
    }
    assert_fts_integrity(&pool).await;
    sqlx::query("UPDATE short_term_memories SET text = 'memorydirectnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory text should update");
    assert_no_match(&pool, "short_term_memories_fts", "memorydirectold").await;
    assert_match(&pool, "short_term_memories_fts", "memorydirectnew", 2).await;
    assert_fts_integrity(&pool).await;

    let before = shadow_snapshot(&pool, "short_term_memories_fts").await;
    sqlx::query("UPDATE short_term_memories SET source_ref = 'ref' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory metadata should update");
    assert_eq!(
        shadow_snapshot(&pool, "short_term_memories_fts").await,
        before
    );
    assert_fts_integrity(&pool).await;

    sqlx::query("DELETE FROM short_term_memories WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory should delete directly");
    assert_no_match(&pool, "short_term_memories_fts", "memorydirectnew").await;
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM task_sessions WHERE id = 1")
        .execute(&pool)
        .await
        .expect("session should cascade");
    assert_no_match(&pool, "short_term_memories_fts", "memorycascadeold").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn concepts_fts_tracks_each_text_change_ignores_metadata_and_deletes() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    sqlx::query(
        "INSERT INTO concepts (id, canonical_name, description, created_at, updated_at) VALUES (2, 'conceptnameold', 'conceptdescriptionold', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("concept should insert");
    assert_fts_integrity(&pool).await;
    sqlx::query("UPDATE concepts SET canonical_name = 'conceptnamenew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept name should update");
    assert_no_match(&pool, "concepts_fts", "conceptnameold").await;
    assert_match(&pool, "concepts_fts", "conceptnamenew", 2).await;
    assert_fts_integrity(&pool).await;
    sqlx::query("UPDATE concepts SET description = 'conceptdescriptionnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept description should update");
    assert_no_match(&pool, "concepts_fts", "conceptdescriptionold").await;
    assert_match(&pool, "concepts_fts", "conceptdescriptionnew", 2).await;
    assert_fts_integrity(&pool).await;

    let before = shadow_snapshot(&pool, "concepts_fts").await;
    sqlx::query("UPDATE concepts SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept metadata should update");
    assert_eq!(shadow_snapshot(&pool, "concepts_fts").await, before);
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM concepts WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept should delete");
    assert_no_match(&pool, "concepts_fts", "conceptnamenew").await;
    assert_no_match(&pool, "concepts_fts", "conceptdescriptionnew").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn concept_facet_fts_tracks_text_ignores_metadata_and_handles_direct_and_cascade_delete() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    sqlx::query(
        "INSERT INTO concepts (id, canonical_name, created_at, updated_at) VALUES (2, 'facet-parent', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("concept should insert");
    for (id, value) in [(2, "facetdirectold"), (3, "facetcascadeold")] {
        sqlx::query(
            "INSERT INTO concept_facets (id, concept_id, facet_type, value, created_at, updated_at) VALUES (?, 2, 'name', ?, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
        )
        .bind(id)
        .bind(value)
        .execute(&pool)
        .await
        .expect("facet should insert");
    }
    assert_fts_integrity(&pool).await;
    sqlx::query("UPDATE concept_facets SET value = 'facetdirectnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet value should update");
    assert_no_match(&pool, "concept_facet_fts", "facetdirectold").await;
    assert_match(&pool, "concept_facet_fts", "facetdirectnew", 2).await;
    assert_fts_integrity(&pool).await;

    let before = shadow_snapshot(&pool, "concept_facet_fts").await;
    sqlx::query("UPDATE concept_facets SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet metadata should update");
    assert_eq!(shadow_snapshot(&pool, "concept_facet_fts").await, before);
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM concept_facets WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet should delete directly");
    assert_no_match(&pool, "concept_facet_fts", "facetdirectnew").await;
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM concepts WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept should cascade to facets");
    assert_no_match(&pool, "concept_facet_fts", "facetcascadeold").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn rag_fts_tracks_each_text_change_ignores_metadata_and_handles_direct_and_cascade_delete() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    seed_parents(&pool).await;
    for (id, suffix) in [(2, "direct"), (3, "cascade")] {
        sqlx::query(
            "INSERT INTO rag_chunks (id, source_id, series_slug, chunk_kind, text, display_text, location, created_at) VALUES (?, 1, 'series', 'paragraph', ?, ?, ?, '2026-07-19T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("ragtext{suffix}old"))
        .bind(format!("ragdisplay{suffix}old"))
        .bind(format!("raglocation{suffix}old"))
        .execute(&pool)
        .await
        .expect("RAG chunk should insert");
    }
    assert_fts_integrity(&pool).await;
    for (column, old, new) in [
        ("text", "ragtextdirectold", "ragtextdirectnew"),
        ("display_text", "ragdisplaydirectold", "ragdisplaydirectnew"),
        ("location", "raglocationdirectold", "raglocationdirectnew"),
    ] {
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE rag_chunks SET {column} = ? WHERE id = 2"
        )))
        .bind(new)
        .execute(&pool)
        .await
        .expect("RAG text column should update");
        assert_no_match(&pool, "rag_chunks_fts", old).await;
        assert_match(&pool, "rag_chunks_fts", new, 2).await;
        assert_fts_integrity(&pool).await;
    }

    let before = shadow_snapshot(&pool, "rag_chunks_fts").await;
    sqlx::query("UPDATE rag_chunks SET metadata_json = '{\"changed\":true}' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("RAG metadata should update");
    assert_eq!(shadow_snapshot(&pool, "rag_chunks_fts").await, before);
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM rag_chunks WHERE id = 2")
        .execute(&pool)
        .await
        .expect("RAG chunk should delete directly");
    for token in [
        "ragtextdirectnew",
        "ragdisplaydirectnew",
        "raglocationdirectnew",
    ] {
        assert_no_match(&pool, "rag_chunks_fts", token).await;
    }
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM rag_sources WHERE id = 1")
        .execute(&pool)
        .await
        .expect("RAG source should cascade to chunks");
    assert_no_match(&pool, "rag_chunks_fts", "ragtextcascadeold").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn every_fts_index_tracks_primary_key_updates() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    seed_preexisting_content(&pool).await;
    sqlx::query(
        "INSERT INTO concepts (id, canonical_name, description, created_at, updated_at) VALUES (2, 'conceptidentity', 'identitydescription', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("independent concept should insert");
    assert_fts_integrity(&pool).await;

    for (sql, fts_table, token, old_id, new_id) in [
        (
            "UPDATE crystals SET id = 11 WHERE id = 1",
            "crystals_fts",
            "crystalpretitle",
            1,
            11,
        ),
        (
            "UPDATE short_term_memories SET id = 11 WHERE id = 1",
            "short_term_memories_fts",
            "memorypretext",
            1,
            11,
        ),
        (
            "UPDATE concepts SET id = 12 WHERE id = 2",
            "concepts_fts",
            "conceptidentity",
            2,
            12,
        ),
        (
            "UPDATE concept_facets SET id = 11 WHERE id = 1",
            "concept_facet_fts",
            "facetprevalue",
            1,
            11,
        ),
        (
            "UPDATE rag_chunks SET id = 11 WHERE id = 1",
            "rag_chunks_fts",
            "ragpretext",
            1,
            11,
        ),
    ] {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("identity update failed: {error}"));
        let rowids = matching_rowids(&pool, fts_table, token).await;
        assert!(!rowids.contains(&old_id), "{fts_table} retained old rowid");
        assert_eq!(rowids, [new_id]);
        assert_fts_integrity(&pool).await;
    }
}

#[tokio::test]
async fn insert_or_replace_replaces_old_terms_in_every_fts_index() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    seed_preexisting_content(&pool).await;
    sqlx::query(
        "INSERT INTO concepts (id, canonical_name, description, created_at, updated_at) VALUES (2, 'replaceconceptold', 'replaceconceptdescriptionold', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("independent concept should insert");
    assert_fts_integrity(&pool).await;

    let replacements = [
        (
            r#"INSERT OR REPLACE INTO crystals (
              id, crystal_type, title, text, scope_type, strength, confidence, status, created_at, updated_at
            ) VALUES (1, 'lesson', 'crystalreplacenew', 'crystalreplacebodynew', 'global', 0.5, 0.5,
              'active', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')"#,
            "crystals_fts",
            "crystalpretitle",
            "crystalreplacenew",
            1,
        ),
        (
            "INSERT OR REPLACE INTO short_term_memories (id, session_id, source_role, kind, text, created_at) VALUES (1, 1, 'agent', 'note', 'memoryreplacenew', '2026-07-19T00:00:00Z')",
            "short_term_memories_fts",
            "memorypretext",
            "memoryreplacenew",
            1,
        ),
        (
            "INSERT OR REPLACE INTO concepts (id, canonical_name, description, created_at, updated_at) VALUES (2, 'replaceconceptnew', 'replaceconceptdescriptionnew', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
            "concepts_fts",
            "replaceconceptold",
            "replaceconceptnew",
            2,
        ),
        (
            "INSERT OR REPLACE INTO concept_facets (id, concept_id, facet_type, value, created_at, updated_at) VALUES (1, 1, 'name', 'facetreplacenew', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
            "concept_facet_fts",
            "facetprevalue",
            "facetreplacenew",
            1,
        ),
        (
            "INSERT OR REPLACE INTO rag_chunks (id, source_id, series_slug, chunk_kind, text, display_text, location, created_at) VALUES (1, 1, 'series', 'paragraph', 'ragreplacenew', 'ragdisplayreplacenew', 'raglocationreplacenew', '2026-07-19T00:00:00Z')",
            "rag_chunks_fts",
            "ragpretext",
            "ragreplacenew",
            1,
        ),
    ];

    for (sql, fts_table, old_token, new_token, rowid) in replacements {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("REPLACE failed for {fts_table}: {error}"));
        assert_no_match(&pool, fts_table, old_token).await;
        assert_match(&pool, fts_table, new_token, rowid).await;
        assert_fts_integrity(&pool).await;
    }
}

#[tokio::test]
async fn rag_fts_survives_source_and_multiple_path_series_cascades() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("schema should migrate");
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series (
          id, slug, title, default_source_language, default_target_language, created_at, updated_at
        ) VALUES (7, 'cascade-series', 'Cascade', 'en', 'ru',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO rag_sources (
          id, series_slug, source_ref, source_type, content_type, checksum, created_at, updated_at
        ) VALUES (7, 'cascade-series', 'source-7', 'text', 'text/plain', 'checksum-7',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO rag_chunks (
          id, source_id, series_slug, chunk_kind, text, display_text, location, created_at
        ) VALUES (7, 7, 'cascade-series', 'paragraph', 'ragsourcecascade', 'display', 'location',
          '2026-07-19T00:00:00Z');
        "#,
    ))
    .await
    .expect("first cascade fixture should insert");
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM rag_sources WHERE id = 7")
        .execute(&pool)
        .await
        .expect("source cascade should delete chunk");
    assert_no_match(&pool, "rag_chunks_fts", "ragsourcecascade").await;
    assert_fts_integrity(&pool).await;

    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO rag_sources (
          id, series_slug, source_ref, source_type, content_type, checksum, created_at, updated_at
        ) VALUES (8, 'cascade-series', 'source-8', 'text', 'text/plain', 'checksum-8',
          '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO rag_chunks (
          id, source_id, series_slug, chunk_kind, text, display_text, location, created_at
        ) VALUES (8, 8, 'cascade-series', 'paragraph', 'ragseriescascade', 'display', 'location',
          '2026-07-19T00:00:00Z');
        "#,
    ))
    .await
    .expect("series cascade fixture should insert");
    assert_fts_integrity(&pool).await;
    sqlx::query("DELETE FROM series WHERE id = 7")
        .execute(&pool)
        .await
        .expect("series cascade should follow both RAG foreign-key paths");
    assert_no_match(&pool, "rag_chunks_fts", "ragseriescascade").await;
    assert_fts_integrity(&pool).await;
}

#[tokio::test]
async fn fresh_history_contains_all_ten_ordered_migrations_and_is_idempotent() {
    let pool = connect_url("sqlite::memory:")
        .await
        .expect("fresh database should migrate");
    migrate(&pool)
        .await
        .expect("migration rerun should be safe");
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("migration history should read");
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

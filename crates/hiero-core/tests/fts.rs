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

async fn assert_no_fts_orphans(pool: &SqlitePool) {
    for (fts_table, content_table, _) in FTS_TABLES {
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
                columns.join(", ")
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
}

#[tokio::test]
async fn upgrade_rebuilds_every_preexisting_row_and_preserves_legacy_strict_fts() {
    let pool = pre_fts_pool().await;
    seed_preexisting_content(&pool).await;
    pool.execute(sqlx::raw_sql(
        r#"
        CREATE VIRTUAL TABLE strict_terms_fts USING fts5(source_text, canonical_translation);
        INSERT INTO strict_terms_fts(rowid, source_text, canonical_translation)
        VALUES (41, 'legacyterm', 'legacyvalue');
        "#,
    ))
    .await
    .expect("legacy strict-term FTS fixture should install");
    let legacy_sql_before: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'strict_terms_fts'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy FTS SQL should read");

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
    assert_match(&pool, "strict_terms_fts", "legacyterm", 41).await;
    let legacy_sql_after: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'strict_terms_fts'",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy FTS SQL should remain readable");
    assert_eq!(legacy_sql_after, legacy_sql_before);

    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("migration history should read");
    assert_eq!(versions, [1, 2, 3, 4]);
    assert_no_fts_orphans(&pool).await;
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

    sqlx::query("UPDATE crystals SET title = 'crystaltitlenew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("title should update");
    assert_no_match(&pool, "crystals_fts", "crystaltitleold").await;
    assert_match(&pool, "crystals_fts", "crystaltitlenew", 2).await;
    sqlx::query("UPDATE crystals SET text = 'crystaltextnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("text should update");
    assert_no_match(&pool, "crystals_fts", "crystaltextold").await;
    assert_match(&pool, "crystals_fts", "crystaltextnew", 2).await;

    let before = shadow_snapshot(&pool, "crystals_fts").await;
    sqlx::query("UPDATE crystals SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("metadata should update");
    assert_eq!(shadow_snapshot(&pool, "crystals_fts").await, before);

    sqlx::query("DELETE FROM crystals WHERE id = 2")
        .execute(&pool)
        .await
        .expect("crystal should delete");
    assert_no_match(&pool, "crystals_fts", "crystaltitlenew").await;
    assert_no_match(&pool, "crystals_fts", "crystaltextnew").await;
    assert_no_fts_orphans(&pool).await;
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
    sqlx::query("UPDATE short_term_memories SET text = 'memorydirectnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory text should update");
    assert_no_match(&pool, "short_term_memories_fts", "memorydirectold").await;
    assert_match(&pool, "short_term_memories_fts", "memorydirectnew", 2).await;

    let before = shadow_snapshot(&pool, "short_term_memories_fts").await;
    sqlx::query("UPDATE short_term_memories SET source_ref = 'ref' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory metadata should update");
    assert_eq!(
        shadow_snapshot(&pool, "short_term_memories_fts").await,
        before
    );

    sqlx::query("DELETE FROM short_term_memories WHERE id = 2")
        .execute(&pool)
        .await
        .expect("memory should delete directly");
    assert_no_match(&pool, "short_term_memories_fts", "memorydirectnew").await;
    sqlx::query("DELETE FROM task_sessions WHERE id = 1")
        .execute(&pool)
        .await
        .expect("session should cascade");
    assert_no_match(&pool, "short_term_memories_fts", "memorycascadeold").await;
    assert_no_fts_orphans(&pool).await;
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
    sqlx::query("UPDATE concepts SET canonical_name = 'conceptnamenew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept name should update");
    assert_no_match(&pool, "concepts_fts", "conceptnameold").await;
    assert_match(&pool, "concepts_fts", "conceptnamenew", 2).await;
    sqlx::query("UPDATE concepts SET description = 'conceptdescriptionnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept description should update");
    assert_no_match(&pool, "concepts_fts", "conceptdescriptionold").await;
    assert_match(&pool, "concepts_fts", "conceptdescriptionnew", 2).await;

    let before = shadow_snapshot(&pool, "concepts_fts").await;
    sqlx::query("UPDATE concepts SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept metadata should update");
    assert_eq!(shadow_snapshot(&pool, "concepts_fts").await, before);
    sqlx::query("DELETE FROM concepts WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept should delete");
    assert_no_match(&pool, "concepts_fts", "conceptnamenew").await;
    assert_no_match(&pool, "concepts_fts", "conceptdescriptionnew").await;
    assert_no_fts_orphans(&pool).await;
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
    sqlx::query("UPDATE concept_facets SET value = 'facetdirectnew' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet value should update");
    assert_no_match(&pool, "concept_facet_fts", "facetdirectold").await;
    assert_match(&pool, "concept_facet_fts", "facetdirectnew", 2).await;

    let before = shadow_snapshot(&pool, "concept_facet_fts").await;
    sqlx::query("UPDATE concept_facets SET confidence = 0.7 WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet metadata should update");
    assert_eq!(shadow_snapshot(&pool, "concept_facet_fts").await, before);
    sqlx::query("DELETE FROM concept_facets WHERE id = 2")
        .execute(&pool)
        .await
        .expect("facet should delete directly");
    assert_no_match(&pool, "concept_facet_fts", "facetdirectnew").await;
    sqlx::query("DELETE FROM concepts WHERE id = 2")
        .execute(&pool)
        .await
        .expect("concept should cascade to facets");
    assert_no_match(&pool, "concept_facet_fts", "facetcascadeold").await;
    assert_no_fts_orphans(&pool).await;
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
    }

    let before = shadow_snapshot(&pool, "rag_chunks_fts").await;
    sqlx::query("UPDATE rag_chunks SET metadata_json = '{\"changed\":true}' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("RAG metadata should update");
    assert_eq!(shadow_snapshot(&pool, "rag_chunks_fts").await, before);
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
    sqlx::query("DELETE FROM rag_sources WHERE id = 1")
        .execute(&pool)
        .await
        .expect("RAG source should cascade to chunks");
    assert_no_match(&pool, "rag_chunks_fts", "ragtextcascadeold").await;
    assert_no_fts_orphans(&pool).await;
}

#[tokio::test]
async fn fresh_history_contains_all_four_ordered_migrations_and_is_idempotent() {
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
    assert_eq!(versions, [1, 2, 3, 4]);
}

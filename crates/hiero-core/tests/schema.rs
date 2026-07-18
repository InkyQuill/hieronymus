use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use hiero_core::db::{connect_url, migrate};
use sqlx::{Executor, Row, SqlitePool};

const REQUIRED_TABLES: &[&str] = &[
    "audit_log",
    "concept_facet_language_tags",
    "concept_facet_semantic_tags",
    "concept_facet_story_scopes",
    "concept_facets",
    "concept_proposals",
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
    "migration_ledger",
    "rag_chunk_language_tags",
    "rag_chunk_semantic_tags",
    "rag_chunk_story_scopes",
    "rag_chunks",
    "rag_sources",
    "semantic_chunk_state",
    "semantic_index_jobs",
    "series",
    "series_language_tags",
    "short_term_memories",
    "short_term_memory_language_tags",
    "short_term_memory_semantic_tags",
    "short_term_memory_story_scopes",
    "task_session_language_tags",
    "task_session_semantic_tags",
    "task_session_story_scopes",
    "task_sessions",
];

const ABSENT_OBJECTS: &[&str] = &[
    "concept_facet_fts",
    "concepts_fts",
    "crystals_fts",
    "rag_chunks_fts",
    "short_term_memories_fts",
    "strict_term_aliases",
    "strict_term_tags",
    "strict_terms",
    "strict_terms_fts",
];

async fn migrated_pool() -> SqlitePool {
    connect_url("sqlite::memory:")
        .await
        .expect("fresh database should connect and migrate")
}

async fn schema_names(pool: &SqlitePool, object_type: &str) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_schema WHERE type = ? AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'",
    )
    .bind(object_type)
    .fetch_all(pool)
    .await
    .expect("sqlite_schema should be readable")
    .into_iter()
    .collect()
}

async fn exact_schema_manifest(pool: &SqlitePool) -> String {
    let mut output = String::new();
    for table in REQUIRED_TABLES {
        let sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("table SQL should be readable");
        output.push_str(&format!("TABLE {table}\nSQL {}\n", sql.replace('\n', " ")));

        for row in sqlx::query(
            "SELECT cid, name, type, [notnull], dflt_value, pk FROM pragma_table_info(?) ORDER BY cid",
        )
        .bind(table)
        .fetch_all(pool)
        .await
        .expect("table_info should be readable")
        {
            output.push_str(&format!(
                "COLUMN {}|{}|{}|{}|{}|{}\n",
                row.get::<i64, _>("cid"),
                row.get::<String, _>("name"),
                row.get::<String, _>("type"),
                row.get::<i64, _>("notnull"),
                row.try_get::<Option<String>, _>("dflt_value")
                    .expect("default value should decode")
                    .unwrap_or_else(|| "NULL".to_owned()),
                row.get::<i64, _>("pk"),
            ));
        }

        for row in sqlx::query(
            "SELECT id, seq, [table], [from], [to], on_update, on_delete, [match] FROM pragma_foreign_key_list(?) ORDER BY id, seq",
        )
        .bind(table)
        .fetch_all(pool)
        .await
        .expect("foreign_key_list should be readable")
        {
            output.push_str(&format!(
                "FK {}|{}|{}|{}|{}|{}|{}|{}\n",
                row.get::<i64, _>("id"),
                row.get::<i64, _>("seq"),
                row.get::<String, _>("table"),
                row.get::<String, _>("from"),
                row.get::<String, _>("to"),
                row.get::<String, _>("on_update"),
                row.get::<String, _>("on_delete"),
                row.get::<String, _>("match"),
            ));
        }

        for index in sqlx::query(
            "SELECT seq, name, [unique], origin, partial FROM pragma_index_list(?) ORDER BY name",
        )
        .bind(table)
        .fetch_all(pool)
        .await
        .expect("index_list should be readable")
        {
            let name: String = index.get("name");
            let columns = sqlx::query_scalar::<_, String>(
                "SELECT name FROM pragma_index_info(?) ORDER BY seqno",
            )
            .bind(&name)
            .fetch_all(pool)
            .await
            .expect("index_info should be readable")
            .join(",");
            output.push_str(&format!(
                "INDEX {}|{}|{}|{}|{}|{}\n",
                index.get::<i64, _>("seq"),
                name,
                index.get::<i64, _>("unique"),
                index.get::<String, _>("origin"),
                index.get::<i64, _>("partial"),
                columns,
            ));
        }
    }
    output
}

#[tokio::test]
async fn exact_schema_manifest_matches_every_column_constraint_foreign_key_and_index() {
    let pool = migrated_pool().await;
    let actual = exact_schema_manifest(&pool).await;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/schema_manifest.txt"
    );
    if std::env::var_os("UPDATE_SCHEMA_MANIFEST").is_some() {
        std::fs::write(path, &actual).expect("schema manifest should update");
    }
    let expected = std::fs::read_to_string(path).expect("schema manifest should be readable");
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn fresh_schema_contains_every_authoritative_table_and_no_owned_fts_yet() {
    let pool = migrated_pool().await;
    let actual = schema_names(&pool, "table").await;
    let expected = REQUIRED_TABLES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    for name in ABSENT_OBJECTS {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = ?")
            .bind(name)
            .fetch_one(&pool)
            .await
            .expect("absence query should succeed");
        assert_eq!(count, 0, "{name} belongs to another task or is retired");
    }
    assert!(schema_names(&pool, "trigger").await.is_empty());
}

#[tokio::test]
async fn every_ordinary_table_uses_only_strict_compatible_declared_types() {
    let pool = migrated_pool().await;

    for table in REQUIRED_TABLES {
        let create_sql: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(&pool)
                .await
                .expect("required table should have CREATE SQL");
        assert!(
            create_sql.trim_end().ends_with("STRICT"),
            "{table} must be STRICT"
        );

        let columns = sqlx::query("SELECT name, type FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(&pool)
            .await
            .expect("table_info should succeed");
        assert!(!columns.is_empty(), "{table} must declare columns");
        for column in columns {
            let declared_type: String = column.get("type");
            assert!(
                matches!(
                    declared_type.as_str(),
                    "INTEGER" | "REAL" | "TEXT" | "BLOB" | "ANY"
                ),
                "{table}.{} declares unsupported STRICT type {declared_type}",
                column.get::<String, _>("name")
            );
        }
    }
}

#[tokio::test]
async fn concepts_scope_columns_exist_without_a_series_foreign_key() {
    let pool = migrated_pool().await;
    let columns = sqlx::query("PRAGMA table_info(concepts)")
        .fetch_all(&pool)
        .await
        .expect("concept columns should be readable")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<BTreeSet<_>>();
    let foreign_tables = sqlx::query("PRAGMA foreign_key_list(concepts)")
        .fetch_all(&pool)
        .await
        .expect("concept foreign keys should be readable")
        .into_iter()
        .map(|row| row.get::<String, _>("table"))
        .collect::<BTreeSet<_>>();

    assert!(columns.contains("scope_type"));
    assert!(columns.contains("scope_key"));
    assert!(!columns.contains("series_slug"));
    assert!(!foreign_tables.contains("series"));
}

#[tokio::test]
async fn rag_chunks_has_the_exact_compound_source_and_series_foreign_key() {
    let pool = migrated_pool().await;
    let rows = sqlx::query("PRAGMA foreign_key_list(rag_chunks)")
        .fetch_all(&pool)
        .await
        .expect("RAG foreign keys should be readable");
    let compound = rows
        .iter()
        .filter(|row| row.get::<String, _>("table") == "rag_sources")
        .map(|row| {
            (
                row.get::<i64, _>("id"),
                row.get::<i64, _>("seq"),
                row.get::<String, _>("from"),
                row.get::<String, _>("to"),
                row.get::<String, _>("on_delete"),
            )
        })
        .collect::<Vec<_>>();

    assert!(compound.windows(2).any(|pair| {
        pair[0].0 == pair[1].0
            && pair[0].1 == 0
            && pair[1].1 == 1
            && pair[0].2 == "source_id"
            && pair[0].3 == "id"
            && pair[1].2 == "series_slug"
            && pair[1].3 == "series_slug"
            && pair[0].4 == "CASCADE"
            && pair[1].4 == "CASCADE"
    }));
}

#[tokio::test]
async fn explicit_indexes_have_the_exact_declared_columns() {
    let pool = migrated_pool().await;
    let expected = [
        (
            "crystals",
            "idx_crystals_maintenance",
            vec![
                "id",
                "created_cycle",
                "last_activated_cycle",
                "last_reinforced_cycle",
            ],
        ),
        (
            "rag_chunks",
            "rag_chunks_series_slug_idx",
            vec!["series_slug"],
        ),
        ("rag_chunks", "rag_chunks_source_id_idx", vec!["source_id"]),
    ];

    for (table, index, columns) in expected {
        let indexes = sqlx::query("SELECT name, [unique] FROM pragma_index_list(?)")
            .bind(table)
            .fetch_all(&pool)
            .await
            .expect("index_list should succeed");
        let row = indexes
            .iter()
            .find(|row| row.get::<String, _>("name") == index)
            .expect("required explicit index should exist");
        assert_eq!(row.get::<i64, _>("unique"), 0);

        let actual =
            sqlx::query_scalar::<_, String>("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                .bind(index)
                .fetch_all(&pool)
                .await
                .expect("index_info should succeed");
        assert_eq!(actual, columns);
    }
}

#[tokio::test]
async fn schema_enforces_boolean_score_status_and_scope_checks() {
    let pool = migrated_pool().await;
    let now = "2026-07-19T00:00:00Z";

    sqlx::query(
        "INSERT INTO concepts (canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES ('valid', 'global', '', 'candidate', 0.5, ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("valid constrained concept should insert");

    for statement in [
        "INSERT INTO concepts (canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES ('bad scope', 'series', '', 'candidate', 0.5, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
        "INSERT INTO concepts (canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES ('bad status', 'global', '', 'invented', 0.5, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
        "INSERT INTO concepts (canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES ('bad score', 'global', '', 'candidate', 1.1, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z')",
        "INSERT INTO memory_events (event_type, source_role, applied, created_at) VALUES ('test', 'system', 2, '2026-07-19T00:00:00Z')",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err("invalid constrained value should be rejected");
    }
}

#[tokio::test]
async fn every_closed_label_boolean_and_unit_range_check_rejects_invalid_storage() {
    let pool = migrated_pool().await;
    pool.execute(sqlx::raw_sql(
        r#"
        INSERT INTO series VALUES (1, 'checks', 'Checks', 'en', 'ru', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO task_sessions(id, series_slug, source_language, target_language, task_type, status, created_at, last_activity_at)
        VALUES (1, 'checks', 'en', 'ru', 'translation', 'active', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO crystals(id, crystal_type, text, scope_type, strength, confidence, status, created_at, updated_at)
        VALUES (1, 'observation', 'check', 'global', 0.5, 0.5, 'active', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO crystal_activations(id, crystal_id, session_id, recall_query, rank, score, outcome, created_at)
        VALUES (1, 1, 1, 'check', 1, 0.5, 'useful', '2026-07-19T00:00:00Z');
        INSERT INTO dream_runs(id, cycle_id, status, provider, created_at)
        VALUES (1, 1, 'running', 'check', '2026-07-19T00:00:00Z');
        INSERT INTO concepts(id, canonical_name, status, confidence, created_at, updated_at)
        VALUES (1, 'Check', 'candidate', 0.5, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO concept_facets(id, concept_id, facet_type, value, confidence, is_canonical, created_at, updated_at)
        VALUES (1, 1, 'name', 'Check', 0.5, 1, '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO concept_semantic_tags VALUES (1, 'check', 0.5, '2026-07-19T00:00:00Z');
        INSERT INTO concept_proposals(id, dream_run_id, source_language, target_language, concept_text, source_form, canonical_rendering, status, created_at, updated_at)
        VALUES (1, 1, 'en', 'ru', 'Check', 'Check', 'Проверка', 'pending', '2026-07-19T00:00:00Z', '2026-07-19T00:00:00Z');
        INSERT INTO crystal_concepts VALUES (1, 1, 'mentions', 0.5, '2026-07-19T00:00:00Z');
        INSERT INTO crystal_story_scopes VALUES (1, 'chapter:1', 0.5, '2026-07-19T00:00:00Z');
        INSERT INTO crystal_semantic_tags VALUES (1, 'check', 0.5, '2026-07-19T00:00:00Z');
        INSERT INTO memory_events(id, event_type, source_role, applied, created_at)
        VALUES (1, 'check', 'system', 1, '2026-07-19T00:00:00Z');
        INSERT INTO dream_phase_runs(id, dream_run_id, phase, provider_profile, provider_type, model, status, created_at)
        VALUES (1, 1, 'check', 'check', 'check', 'check', 'running', '2026-07-19T00:00:00Z');
        INSERT INTO semantic_index_jobs(id, status, created_at) VALUES (1, 'pending', '2026-07-19T00:00:00Z');
        "#,
    )).await.expect("valid CHECK fixtures should insert");

    for statement in [
        "UPDATE task_sessions SET status = 'invalid' WHERE id = 1",
        "UPDATE crystals SET crystal_type = 'invalid' WHERE id = 1",
        "UPDATE crystals SET strength = -0.1 WHERE id = 1",
        "UPDATE crystals SET confidence = 1.1 WHERE id = 1",
        "UPDATE crystals SET is_inferred = 2 WHERE id = 1",
        "UPDATE crystals SET malformed_penalty = -0.1 WHERE id = 1",
        "UPDATE crystals SET status = 'invalid' WHERE id = 1",
        "UPDATE crystal_activations SET outcome = 'invalid' WHERE id = 1",
        "UPDATE dream_runs SET status = 'invalid' WHERE id = 1",
        "UPDATE concepts SET status = 'invalid' WHERE id = 1",
        "UPDATE concepts SET confidence = 1.1 WHERE id = 1",
        "UPDATE concepts SET scope_type = 'series', scope_key = '' WHERE id = 1",
        "UPDATE concept_facets SET facet_type = 'invalid' WHERE id = 1",
        "UPDATE concept_facets SET confidence = -0.1 WHERE id = 1",
        "UPDATE concept_facets SET is_canonical = 2 WHERE id = 1",
        "UPDATE concept_semantic_tags SET confidence = 1.1 WHERE concept_id = 1",
        "UPDATE concept_proposals SET status = 'invalid' WHERE id = 1",
        "UPDATE crystal_concepts SET confidence = -0.1 WHERE crystal_id = 1",
        "UPDATE crystal_story_scopes SET confidence = 1.1 WHERE crystal_id = 1",
        "UPDATE crystal_semantic_tags SET confidence = -0.1 WHERE crystal_id = 1",
        "UPDATE memory_events SET applied = 2 WHERE id = 1",
        "UPDATE dream_phase_runs SET status = 'invalid' WHERE id = 1",
        "UPDATE semantic_index_jobs SET status = 'invalid' WHERE id = 1",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err("every declared CHECK must reject invalid storage");
    }
}

#[tokio::test]
async fn default_and_explicit_timestamps_roundtrip_as_rfc3339_utc() {
    let pool = migrated_pool().await;
    let explicit = "2026-07-19T03:04:05.678Z";

    let series_id = sqlx::query(
        "INSERT INTO series (slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES ('schema', 'Schema', 'en', 'ru', ?, ?) RETURNING id",
    )
    .bind(explicit)
    .bind(explicit)
    .fetch_one(&pool)
    .await
    .expect("series should insert")
    .get::<i64, _>("id");
    sqlx::query("INSERT INTO series_language_tags (series_id, language_tag) VALUES (?, 'en')")
        .bind(series_id)
        .execute(&pool)
        .await
        .expect("language tag should use timestamp default");
    sqlx::query(
        "INSERT INTO migration_ledger (source_table, source_id, target_table, target_id) VALUES ('old', '1', 'new', 1)",
    )
    .execute(&pool)
    .await
    .expect("ledger should use timestamp default");

    let values = sqlx::query_scalar::<_, String>(
        "SELECT created_at FROM series UNION ALL SELECT created_at FROM series_language_tags UNION ALL SELECT created_at FROM migration_ledger",
    )
    .fetch_all(&pool)
    .await
    .expect("timestamps should be readable");
    for value in values {
        let parsed = DateTime::parse_from_rfc3339(&value).expect("timestamp must be RFC 3339");
        assert_eq!(parsed.offset().local_minus_utc(), 0);
        let utc: DateTime<Utc> = parsed.into();
        assert_eq!(*utc.offset(), Utc);
    }
}

#[tokio::test]
async fn maintenance_query_uses_the_named_compound_index() {
    let pool = migrated_pool().await;
    let now = "2026-07-19T00:00:00Z";
    for id in 1..=200_i64 {
        sqlx::query(
            "INSERT INTO crystals (id, crystal_type, text, scope_type, strength, confidence, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at) VALUES (?, ?, ?, 'global', 0.5, 0.5, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(if id % 7 == 0 { "rule" } else { "observation" })
        .bind(format!("crystal {id}"))
        .bind(if id % 5 == 0 { "archived" } else { "active" })
        .bind(id % 19)
        .bind(id % 13)
        .bind(id % 17)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("selective maintenance fixture should insert");
    }

    let details = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id, crystal_type, strength, confidence, status FROM crystals WHERE status IN ('active', 'candidate') AND id > ? AND created_cycle != ? AND coalesce(last_activated_cycle, -1) != ? AND coalesce(last_reinforced_cycle, -1) != ? AND NOT (crystal_type = 'rule' AND status = 'active') ORDER BY id LIMIT ?",
    )
    .bind(50_i64)
    .bind(10_i64)
    .bind(10_i64)
    .bind(10_i64)
    .bind(20_i64)
    .fetch_all(&pool)
    .await
    .expect("maintenance plan should compile")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join("\n");

    assert!(
        details.contains("idx_crystals_maintenance"),
        "unexpected query plan:\n{details}"
    );
    assert!(
        !details.contains("USE TEMP B-TREE"),
        "unexpected sort:\n{details}"
    );
    assert!(
        !details.contains("SCAN crystals"),
        "unbounded scan:\n{details}"
    );
}

#[tokio::test]
async fn migration_versions_preserve_the_intentional_fts_gap_and_are_idempotent() {
    let pool = migrated_pool().await;
    migrate(&pool)
        .await
        .expect("re-running embedded migrations should be idempotent");
    let versions = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("migration versions should be readable");

    assert_eq!(versions, vec![1, 3, 4]);
}

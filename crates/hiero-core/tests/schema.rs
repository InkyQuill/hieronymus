use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use hiero_core::db::{
    ConceptFacetRecord, ConceptProposalRecord, ConceptProposalStatus, ConceptRecord, ConceptStatus,
    CrystalActivationRecord, CrystalLinkRecord, CrystalRecord, CrystalStatus, CrystalType,
    DreamRunRecord, DreamRunStatus, MemoryEventRecord, RagChunkRecord, RecallOutcome, SeriesRecord,
    ShortTermMemoryRecord, TaskSessionRecord, TaskSessionStatus, connect_url, migrate,
};
use sqlx::{AssertSqlSafe, Executor, Row, SqlitePool};

mod models {
    use super::*;
    use serde_json::Value;
    use std::str::FromStr;

    const UPDATED: &str = "2026-07-18T13:34:56Z";

    const MODEL_FIXTURE_MANIFEST: &[(&str, &str)] = &[
        (
            "SeriesRecord",
            "INSERT INTO series VALUES (1, 'book', 'Book', 'en', 'ru', '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "TaskSessionRecord",
            "INSERT INTO task_sessions VALUES (1, 'book', 'en', 'ru', 'translation', '1', '2', 'active', NULL, '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z', NULL)",
        ),
        (
            "CrystalRecord",
            "INSERT INTO crystals VALUES (1, 'rule', 'Use X', 'Rule', 'series', 'book', 'book', 'en', 'ru', '[\"tag\"]', 0.8, 0.9, 'user_explicit', 'term_rule', NULL, 1, 0.0, NULL, 'active', 3, NULL, 4, '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "ShortTermMemoryRecord",
            "INSERT INTO short_term_memories VALUES (1, 1, 'translator', 'observation', 'Text', 'chapter.md:1', '{not-json', NULL, NULL, NULL, 1, '2026-07-18T12:34:56.789012Z', NULL)",
        ),
        (
            "CrystalActivationRecord",
            "INSERT INTO crystal_activations VALUES (1, 1, 1, 'query', 2, 0.75, 'fts', NULL, NULL, '2026-07-18T12:34:56.789012Z')",
        ),
        (
            "CrystalLinkRecord",
            "INSERT INTO crystal_links VALUES (1, 1, 'supports')",
        ),
        (
            "DreamRunRecord",
            "INSERT INTO dream_runs VALUES (1, 8, 'running', 'deterministic', 4, 1, 2, '', '2026-07-18T12:34:56.789012Z', NULL)",
        ),
        (
            "ConceptRecord",
            "INSERT INTO concepts VALUES (1, 'Name', 'Description', 'global', '', 'candidate', 0.4, NULL, '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "ConceptFacetRecord",
            "INSERT INTO concept_facets VALUES (1, 1, 'ru', 'rendering', 'Имя', 1, 0.8, 0, NULL, '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "ConceptProposalRecord",
            "INSERT INTO concept_proposals VALUES (1, 1, 'book', 'en', 'ru', 'name', 'Name', 'Имя', '[\"Имя\"]', '{broken', 'why', 'pending', '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "MemoryEventRecord",
            "INSERT INTO memory_events VALUES (1, NULL, NULL, 'reinforce', 'translator', 'used', 0.1, 0.2, 0, NULL, '2026-07-18T12:34:56.789012Z')",
        ),
        (
            "RagSourceDependency",
            "INSERT INTO rag_sources VALUES (1, 'book', 'glossary.md', 'markdown', 'text/markdown', 'abc', '{}', '2026-07-18T12:34:56.789012Z', '2026-07-18T13:34:56Z')",
        ),
        (
            "RagChunkRecord",
            "INSERT INTO rag_chunks VALUES (1, 1, 'book', 'paragraph', 'Source', 'Display', 'line:1', '{malformed', '2026-07-18T12:34:56.789012Z')",
        ),
    ];

    async fn seeded_pool() -> SqlitePool {
        let pool = migrated_pool().await;
        for (model, statement) in MODEL_FIXTURE_MANIFEST {
            sqlx::raw_sql(*statement)
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("{model} fixture should insert: {error}"));
        }
        pool
    }

    macro_rules! assert_row {
        ($name:ident, $record:ty, $table:literal, $assertion:expr) => {
            #[tokio::test]
            async fn $name() {
                let pool = seeded_pool().await;
                let row = sqlx::query_as::<_, $record>(concat!("SELECT * FROM ", $table))
                    .fetch_one(&pool)
                    .await
                    .expect(concat!($table, " should decode"));
                ($assertion)(row);
            }
        };
    }

    assert_row!(
        series_record_decodes,
        SeriesRecord,
        "series",
        |row: SeriesRecord| {
            assert_eq!(row.slug, "book");
            assert_eq!(
                row.created_at.to_rfc3339(),
                "2026-07-18T12:34:56.789012+00:00"
            );
        }
    );
    assert_row!(
        task_session_record_decodes,
        TaskSessionRecord,
        "task_sessions",
        |row: TaskSessionRecord| {
            assert_eq!(row.status, "active");
            assert!(row.cycle_id.is_none() && row.completed_at.is_none());
        }
    );
    assert_row!(
        short_term_memory_record_decodes,
        ShortTermMemoryRecord,
        "short_term_memories",
        |row: ShortTermMemoryRecord| {
            assert_eq!(row.metadata_json, "{not-json");
            assert_eq!(row.source_crystal_id, Some(1));
            assert!(
                row.source_credibility.is_none()
                    && row.rule_intent.is_none()
                    && row.soft_origin.is_none()
                    && row.archived_at.is_none()
            );
        }
    );
    assert_row!(
        crystal_record_decodes,
        CrystalRecord,
        "crystals",
        |row: CrystalRecord| {
            assert_eq!(row.tags_json, "[\"tag\"]");
            assert!(row.is_inferred);
            assert!(
                row.soft_origin.is_none()
                    && row.supersedes_crystal_id.is_none()
                    && row.last_activated_cycle.is_none()
            );
            assert_eq!(row.last_reinforced_cycle, Some(4));
        }
    );
    assert_row!(
        crystal_activation_record_decodes,
        CrystalActivationRecord,
        "crystal_activations",
        |row: CrystalActivationRecord| {
            assert!(row.outcome.is_none() && row.cycle_id.is_none());
        }
    );
    assert_row!(
        crystal_link_record_decodes,
        CrystalLinkRecord,
        "crystal_links",
        |row: CrystalLinkRecord| {
            assert_eq!(row.link_type, "supports");
        }
    );
    assert_row!(
        concept_record_decodes,
        ConceptRecord,
        "concepts",
        |row: ConceptRecord| {
            assert!(row.merged_into_concept_id.is_none());
        }
    );
    assert_row!(
        concept_facet_record_decodes,
        ConceptFacetRecord,
        "concept_facets",
        |row: ConceptFacetRecord| {
            assert!(!row.is_canonical);
            assert_eq!(row.source_crystal_id, Some(1));
            assert!(row.superseded_at.is_none());
        }
    );
    assert_row!(
        concept_proposal_record_decodes,
        ConceptProposalRecord,
        "concept_proposals",
        |row: ConceptProposalRecord| {
            assert_eq!(row.forbidden_variants_json, "{broken");
            assert_eq!(row.dream_run_id, Some(1));
        }
    );
    assert_row!(
        memory_event_record_decodes,
        MemoryEventRecord,
        "memory_events",
        |row: MemoryEventRecord| {
            assert!(!row.applied);
            assert!(row.crystal_id.is_none() && row.session_id.is_none() && row.cycle_id.is_none());
        }
    );
    assert_row!(
        rag_chunk_record_decodes,
        RagChunkRecord,
        "rag_chunks",
        |row: RagChunkRecord| {
            assert_eq!(row.metadata_json, "{malformed");
        }
    );
    assert_row!(
        dream_run_record_decodes,
        DreamRunRecord,
        "dream_runs",
        |row: DreamRunRecord| {
            assert!(row.completed_at.is_none());
        }
    );

    #[tokio::test]
    async fn nullable_timestamps_decode_when_present() {
        let pool = seeded_pool().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "UPDATE task_sessions SET completed_at = '{UPDATED}'; UPDATE short_term_memories SET archived_at = '{UPDATED}'; UPDATE concept_facets SET superseded_at = '{UPDATED}'; UPDATE dream_runs SET completed_at = '{UPDATED}';"
        )))
        .execute(&pool)
        .await
        .expect("nullable timestamps should update");
        let session: TaskSessionRecord = sqlx::query_as("SELECT * FROM task_sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let memory: ShortTermMemoryRecord = sqlx::query_as("SELECT * FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap();
        let facet: ConceptFacetRecord = sqlx::query_as("SELECT * FROM concept_facets")
            .fetch_one(&pool)
            .await
            .unwrap();
        let dream: DreamRunRecord = sqlx::query_as("SELECT * FROM dream_runs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            session.completed_at.is_some()
                && memory.archived_at.is_some()
                && facet.superseded_at.is_some()
                && dream.completed_at.is_some()
        );
    }

    #[tokio::test]
    async fn optional_text_number_and_foreign_key_fields_decode_in_both_states() {
        let pool = seeded_pool().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            r#"
            UPDATE task_sessions SET cycle_id = 8, completed_at = '{UPDATED}';
            UPDATE short_term_memories SET source_credibility = 'thought', rule_intent = 'term_rule', soft_origin = 'dream', source_crystal_id = NULL, archived_at = '{UPDATED}';
            UPDATE crystals SET soft_origin = 'dream', supersedes_crystal_id = 1, last_activated_cycle = 7, last_reinforced_cycle = NULL;
            UPDATE crystal_activations SET outcome = 'useful', cycle_id = 8;
            UPDATE concepts SET merged_into_concept_id = 1;
            UPDATE concept_facets SET source_crystal_id = NULL, superseded_at = '{UPDATED}';
            UPDATE concept_proposals SET dream_run_id = NULL;
            UPDATE memory_events SET crystal_id = 1, session_id = 1, cycle_id = 8;
            UPDATE dream_runs SET completed_at = '{UPDATED}';
            "#
        )))
        .execute(&pool)
        .await
        .expect("optional values should update");

        let session: TaskSessionRecord = sqlx::query_as("SELECT * FROM task_sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let memory: ShortTermMemoryRecord = sqlx::query_as("SELECT * FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap();
        let crystal: CrystalRecord = sqlx::query_as("SELECT * FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap();
        let activation: CrystalActivationRecord =
            sqlx::query_as("SELECT * FROM crystal_activations")
                .fetch_one(&pool)
                .await
                .unwrap();
        let concept: ConceptRecord = sqlx::query_as("SELECT * FROM concepts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let facet: ConceptFacetRecord = sqlx::query_as("SELECT * FROM concept_facets")
            .fetch_one(&pool)
            .await
            .unwrap();
        let proposal: ConceptProposalRecord = sqlx::query_as("SELECT * FROM concept_proposals")
            .fetch_one(&pool)
            .await
            .unwrap();
        let event: MemoryEventRecord = sqlx::query_as("SELECT * FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(session.cycle_id, Some(8));
        assert_eq!(memory.source_credibility.as_deref(), Some("thought"));
        assert_eq!(memory.rule_intent.as_deref(), Some("term_rule"));
        assert_eq!(memory.soft_origin.as_deref(), Some("dream"));
        assert!(memory.source_crystal_id.is_none());
        assert_eq!(crystal.soft_origin.as_deref(), Some("dream"));
        assert_eq!(crystal.supersedes_crystal_id, Some(1));
        assert_eq!(crystal.last_activated_cycle, Some(7));
        assert!(crystal.last_reinforced_cycle.is_none());
        assert_eq!(activation.outcome.as_deref(), Some("useful"));
        assert_eq!(activation.cycle_id, Some(8));
        assert_eq!(concept.merged_into_concept_id, Some(1));
        assert!(facet.source_crystal_id.is_none());
        assert!(proposal.dream_run_id.is_none());
        assert_eq!(
            (event.crystal_id, event.session_id, event.cycle_id),
            (Some(1), Some(1), Some(8))
        );
    }

    #[tokio::test]
    async fn integer_booleans_accept_zero_and_one_but_reject_other_values() {
        let pool = seeded_pool().await;
        let false_event: MemoryEventRecord = sqlx::query_as("SELECT id, crystal_id, session_id, event_type, source_role, evidence, strength_delta, confidence_delta, 0 AS applied, cycle_id, created_at FROM memory_events").fetch_one(&pool).await.unwrap();
        let true_event: MemoryEventRecord = sqlx::query_as("SELECT id, crystal_id, session_id, event_type, source_role, evidence, strength_delta, confidence_delta, 1 AS applied, cycle_id, created_at FROM memory_events").fetch_one(&pool).await.unwrap();
        assert!(!false_event.applied && true_event.applied);
        let error = sqlx::query_as::<_, MemoryEventRecord>("SELECT id, crystal_id, session_id, event_type, source_role, evidence, strength_delta, confidence_delta, 2 AS applied, cycle_id, created_at FROM memory_events").fetch_one(&pool).await.expect_err("invalid boolean must not decode");
        assert!(error.to_string().contains("applied") && error.to_string().contains("0 or 1"));

        let crystal_error = sqlx::query_as::<_, CrystalRecord>(
            "SELECT id, crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, soft_origin, -1 AS is_inferred, malformed_penalty, supersedes_crystal_id, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at FROM crystals",
        )
        .fetch_one(&pool)
        .await
        .expect_err("invalid crystal boolean must not decode");
        assert!(crystal_error.to_string().contains("is_inferred"));

        let facet_error = sqlx::query_as::<_, ConceptFacetRecord>(
            "SELECT id, concept_id, language, facet_type, value, source_crystal_id, confidence, 3 AS is_canonical, superseded_at, created_at, updated_at FROM concept_facets",
        )
        .fetch_one(&pool)
        .await
        .expect_err("invalid facet boolean must not decode");
        assert!(facet_error.to_string().contains("is_canonical"));
    }

    #[tokio::test]
    async fn integer_booleans_reject_null_and_non_integer_storage_classes() {
        let pool = seeded_pool().await;
        for (expression, expected_type) in [
            ("NULL", "NULL"),
            ("'0'", "TEXT"),
            ("'1'", "TEXT"),
            ("0.0", "REAL"),
            ("X'30'", "BLOB"),
        ] {
            let query = format!(
                "SELECT id, crystal_id, session_id, event_type, source_role, evidence, strength_delta, confidence_delta, {expression} AS applied, cycle_id, created_at FROM memory_events"
            );
            let error = sqlx::query_as::<_, MemoryEventRecord>(AssertSqlSafe(query))
                .fetch_one(&pool)
                .await
                .expect_err("only non-null INTEGER booleans may decode");
            let message = error.to_string();
            assert!(
                message.contains("applied"),
                "missing column context: {message}"
            );
            assert!(
                message.contains(expected_type),
                "missing {expected_type} storage context: {message}"
            );
        }
    }

    #[tokio::test]
    async fn malformed_timestamp_reports_the_column_context() {
        let pool = seeded_pool().await;
        let error = sqlx::query_as::<_, SeriesRecord>("SELECT id, slug, title, default_source_language, default_target_language, 'yesterday' AS created_at, updated_at FROM series").fetch_one(&pool).await.expect_err("malformed timestamp must fail");
        assert!(error.to_string().contains("created_at"));

        let optional_error = sqlx::query_as::<_, TaskSessionRecord>(
            "SELECT id, series_slug, source_language, target_language, task_type, volume, chapter, status, cycle_id, created_at, last_activity_at, 'not-a-time' AS completed_at FROM task_sessions",
        )
        .fetch_one(&pool)
        .await
        .expect_err("malformed optional timestamp must fail when present");
        assert!(optional_error.to_string().contains("completed_at"));
    }

    #[test]
    fn closed_enums_accept_all_known_labels_and_reject_unknown_labels() {
        assert_eq!(
            CrystalType::from_str("lesson").unwrap(),
            CrystalType::Lesson
        );
        assert_eq!(CrystalType::from_str("rule").unwrap(), CrystalType::Rule);
        assert_eq!(
            CrystalType::from_str("thought").unwrap(),
            CrystalType::Thought
        );
        assert_eq!(
            CrystalType::from_str("observation").unwrap(),
            CrystalType::Observation
        );
        assert_eq!(
            CrystalType::from_str("concept_note").unwrap(),
            CrystalType::ConceptNote
        );
        assert_eq!(
            CrystalType::from_str("concept").unwrap(),
            CrystalType::Concept
        );
        assert_eq!(
            CrystalType::from_str("erudition").unwrap(),
            CrystalType::Erudition
        );
        for label in ["active", "completed", "dreamed"] {
            TaskSessionStatus::from_str(label).unwrap();
        }
        for label in ["active", "candidate", "archived", "rejected", "superseded"] {
            CrystalStatus::from_str(label).unwrap();
        }
        for label in ["candidate", "established", "archived", "merged"] {
            ConceptStatus::from_str(label).unwrap();
        }
        for label in ["pending", "approved", "rejected"] {
            ConceptProposalStatus::from_str(label).unwrap();
        }
        for label in ["running", "completed", "failed", "skipped"] {
            DreamRunStatus::from_str(label).unwrap();
        }
        for label in ["useful", "miss"] {
            RecallOutcome::from_str(label).unwrap();
        }
        for (error, discriminator) in [
            (
                CrystalType::from_str("future").unwrap_err().to_string(),
                "crystal type",
            ),
            (
                TaskSessionStatus::from_str("future")
                    .unwrap_err()
                    .to_string(),
                "task session status",
            ),
            (
                CrystalStatus::from_str("future").unwrap_err().to_string(),
                "crystal status",
            ),
            (
                ConceptStatus::from_str("future").unwrap_err().to_string(),
                "concept status",
            ),
            (
                ConceptProposalStatus::from_str("future")
                    .unwrap_err()
                    .to_string(),
                "concept proposal status",
            ),
            (
                DreamRunStatus::from_str("future").unwrap_err().to_string(),
                "dream run status",
            ),
            (
                RecallOutcome::from_str("future").unwrap_err().to_string(),
                "recall outcome",
            ),
        ] {
            assert!(error.contains("future"), "missing rejected label: {error}");
            assert!(
                error.contains(discriminator),
                "missing discriminator context: {error}"
            );
        }
    }

    #[tokio::test]
    async fn closed_enums_decode_from_sql_and_reject_unknown_labels() {
        let pool = seeded_pool().await;
        let value: CrystalType = sqlx::query_scalar("SELECT crystal_type FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(value, CrystalType::Rule);
        let error = sqlx::query_scalar::<_, CrystalType>("SELECT 'future'")
            .fetch_one(&pool)
            .await
            .expect_err("unknown SQL label must fail");
        assert!(error.to_string().contains("future"));
    }

    #[tokio::test]
    async fn serde_exposes_public_records_as_raw_storage_contracts() {
        let pool = seeded_pool().await;
        let memory: ShortTermMemoryRecord = sqlx::query_as("SELECT * FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap();
        let json = serde_json::to_value(memory).expect("public record should serialize");
        assert_eq!(json["metadata_json"], Value::String("{not-json".to_owned()));
        assert_eq!(json["source_crystal_id"], Value::from(1));
        assert_eq!(json["archived_at"], Value::Null);
        assert_eq!(
            serde_json::to_value(CrystalType::ConceptNote).unwrap(),
            "concept_note"
        );

        let crystal: CrystalRecord = sqlx::query_as("SELECT * FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap();
        let proposal: ConceptProposalRecord = sqlx::query_as("SELECT * FROM concept_proposals")
            .fetch_one(&pool)
            .await
            .unwrap();
        let chunk: RagChunkRecord = sqlx::query_as("SELECT * FROM rag_chunks")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(crystal.tags_json, "[\"tag\"]");
        assert_eq!(proposal.approved_variants_json, "[\"Имя\"]");
        assert_eq!(proposal.forbidden_variants_json, "{broken");
        assert_eq!(chunk.metadata_json, "{malformed");
    }
}

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
    "semantic_batch_claims",
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
    "strict_term_aliases",
    "strict_term_tags",
    "strict_terms",
    "strict_terms_fts",
];

const FTS_TABLES: &[&str] = &[
    "concept_facet_fts",
    "concepts_fts",
    "crystals_fts",
    "rag_chunks_fts",
    "short_term_memories_fts",
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
async fn fresh_schema_has_the_exact_authoritative_and_fts_object_set() {
    let pool = migrated_pool().await;
    let actual = schema_names(&pool, "table").await;
    let mut expected = REQUIRED_TABLES
        .iter()
        .map(|table| (*table).to_owned())
        .collect::<BTreeSet<_>>();
    for fts_table in FTS_TABLES {
        expected.insert((*fts_table).to_owned());
        for suffix in ["data", "idx", "docsize", "config"] {
            expected.insert(format!("{fts_table}_{suffix}"));
        }
    }
    assert_eq!(
        actual, expected,
        "unexpected or missing schema table object"
    );

    for name in ABSENT_OBJECTS {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE name = ?")
            .bind(name)
            .fetch_one(&pool)
            .await
            .expect("absence query should succeed");
        assert_eq!(count, 0, "{name} belongs to another task or is retired");
    }
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
            "semantic_batch_claims",
            "semantic_batch_claims_job_token_idx",
            vec!["job_id", "claim_token"],
        ),
        (
            "crystal_links",
            "idx_crystal_links_target",
            vec!["target_crystal_id", "source_crystal_id", "link_type"],
        ),
        ("crystals", "idx_crystals_maintenance", vec!["id"]),
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
async fn maintenance_query_enforces_staleness_boundary_and_uses_partial_cursor_index() {
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

    for (id, crystal_type, status, created, activated, reinforced) in [
        (
            201_i64,
            "observation",
            "active",
            1_i64,
            Some(2_i64),
            Some(5_i64),
        ),
        (202, "observation", "active", 1, Some(2), Some(6)),
        (203, "observation", "active", 1, Some(2), Some(9)),
        (204, "observation", "active", 1, Some(2), None),
        (205, "observation", "active", 10, Some(2), Some(5)),
        (206, "observation", "active", 1, Some(10), Some(5)),
        (207, "rule", "active", 1, Some(2), Some(5)),
        (208, "rule", "candidate", 1, Some(2), Some(5)),
    ] {
        sqlx::query(
            "INSERT INTO crystals (id, crystal_type, text, scope_type, strength, confidence, status, created_cycle, last_activated_cycle, last_reinforced_cycle, created_at, updated_at) VALUES (?, ?, ?, 'global', 0.5, 0.5, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(crystal_type)
        .bind(format!("boundary {id}"))
        .bind(status)
        .bind(created)
        .bind(activated)
        .bind(reinforced)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("maintenance boundary fixture should insert");
    }

    const QUERY: &str = "SELECT id, crystal_type, strength, confidence, status FROM crystals WHERE status IN ('active', 'candidate') AND id > ? AND created_cycle != ? AND coalesce(last_activated_cycle, -1) != ? AND coalesce(last_reinforced_cycle, 0) < ? AND NOT (crystal_type = 'rule' AND status = 'active') ORDER BY id LIMIT ?";
    let selected: Vec<i64> = sqlx::query_scalar(QUERY)
        .bind(200_i64)
        .bind(10_i64)
        .bind(10_i64)
        .bind(6_i64)
        .bind(20_i64)
        .fetch_all(&pool)
        .await
        .expect("maintenance candidates should select");
    assert_eq!(selected, vec![201, 204, 208]);

    let details = sqlx::query(AssertSqlSafe(format!("EXPLAIN QUERY PLAN {QUERY}")))
        .bind(50_i64)
        .bind(10_i64)
        .bind(10_i64)
        .bind(6_i64)
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

#[test]
fn maintenance_proposal_names_the_configurable_staleness_threshold() {
    let proposal =
        include_str!("../../../docs/rust-migration-proposal/004-dreaming-and-llm-integration.md");
    assert!(
        proposal.contains("coalesce(last_reinforced_cycle, 0) < :stale_before_cycle"),
        "proposal 004 must freeze the reinforcement staleness boundary"
    );
    assert!(
        proposal.contains("partial cursor/range index"),
        "proposal 004 must describe the index accurately"
    );
}

#[tokio::test]
async fn migration_versions_include_fts_and_are_idempotent() {
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

    assert_eq!(versions, vec![1, 2, 3, 4, 5, 6, 7]);
}

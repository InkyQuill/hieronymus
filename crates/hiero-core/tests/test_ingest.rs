use std::fs;

use hiero_core::{
    db::connect_url,
    domain::{ShortMemoryLimits, TermProposal, Termbase, TranslationContext, WorkspaceStore},
    ingest::{
        IngestConfig, IngestError, IngestionService, LearnInput, LearnLimits, ReadInput,
        extract_terms, split_blocks,
    },
    registry::SeriesRegistry,
};
use tempfile::tempdir;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:ingest-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

async fn session(pool: &sqlx::SqlitePool, context: &TranslationContext, task_type: &str) -> i64 {
    SeriesRegistry::new(pool)
        .create(
            &context.series_slug,
            "Only Sense Online",
            &context.source_language,
            &context.target_language,
        )
        .await
        .unwrap();
    WorkspaceStore::new(pool)
        .start_session(context, task_type, "1", "2")
        .await
        .unwrap()
        .id
}

#[test]
fn config_defaults_and_plaintext_round_trip_match_python_contract() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ingest.conf");
    let defaults = IngestConfig::load(&path).unwrap();
    assert_eq!(defaults, IngestConfig::default());
    assert_eq!(defaults.short_memory, ShortMemoryLimits::DEFAULT);
    assert_eq!(defaults.learn.max_block_chars, 1_200);

    let configured = IngestConfig {
        short_memory: ShortMemoryLimits {
            warning_sentence_count: 5,
            rejection_sentence_count: 12,
            warning_symbol_count: 1_000,
            rejection_symbol_count: 3_000,
        },
        learn: LearnLimits {
            max_block_chars: 900,
        },
    };
    configured.save(&path).unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("warning_sentence_count = 5"));
    assert!(raw.contains("max_block_chars = 900"));
    assert_eq!(IngestConfig::load(&path).unwrap(), configured);
}

#[test]
fn config_rejects_unknown_non_integer_minimum_and_threshold_errors() {
    let cases = [
        (
            "[unknown]\nvalue = 1\n",
            "unknown ingest config setting: unknown",
        ),
        (
            "[short_memory]\nextra = 1\n",
            "unknown ingest config setting: short_memory.extra",
        ),
        (
            "[learn]\nmax_block_chars = true\n",
            "learn.max_block_chars must be an integer",
        ),
        (
            "[short_memory]\nwarning_sentence_count = 0\n",
            "short_memory.warning_sentence_count must be at least 1",
        ),
        (
            "[short_memory]\nwarning_sentence_count = -1\n",
            "short_memory.warning_sentence_count must be at least 1",
        ),
        (
            "[learn]\nmax_block_chars = -1\n",
            "learn.max_block_chars must be at least 1",
        ),
        (
            "[short_memory]\nwarning_sentence_count = 30\nrejection_sentence_count = 6\n",
            "short_memory.rejection_sentence_count must be greater than or equal to short_memory.warning_sentence_count",
        ),
        (
            "[short_memory]\nwarning_symbol_count = 100\nrejection_symbol_count = 50\n",
            "short_memory.rejection_symbol_count must be greater than or equal to short_memory.warning_symbol_count",
        ),
    ];
    for (raw, expected) in cases {
        let directory = tempdir().unwrap();
        let path = directory.path().join("ingest.conf");
        fs::write(&path, raw).unwrap();
        assert_eq!(IngestConfig::load(&path).unwrap_err().to_string(), expected);
    }
}

#[cfg(unix)]
#[test]
fn config_load_and_save_refuse_symbolic_link_destinations() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let target = directory.path().join("target.conf");
    let link = directory.path().join("ingest.conf");
    let broken = directory.path().join("broken.conf");
    fs::write(&target, "[learn]\nmax_block_chars = 321\n").unwrap();
    symlink(&target, &link).unwrap();
    symlink(directory.path().join("missing.conf"), &broken).unwrap();

    assert!(matches!(
        IngestConfig::load(&link),
        Err(IngestError::UnsafePath { .. })
    ));
    assert!(matches!(
        IngestConfig::default().save(&link),
        Err(IngestError::UnsafePath { .. })
    ));
    assert!(matches!(
        IngestConfig::load(&broken),
        Err(IngestError::UnsafePath { .. })
    ));
    assert_eq!(
        fs::read_to_string(target).unwrap(),
        "[learn]\nmax_block_chars = 321\n"
    );
}

#[test]
fn splitting_and_term_extraction_preserve_order_and_bound_unicode_text() {
    let blocks = split_blocks(
        "First sentence is short. Second sentence is also short. Third sentence ends it.",
        60,
    )
    .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].text,
        "First sentence is short. Second sentence is also short."
    );
    assert_eq!(blocks[1].text, "Third sentence ends it.");

    let unbroken = split_blocks(&"界".repeat(137), 40).unwrap();
    assert_eq!(
        unbroken
            .iter()
            .map(|block| block.text.chars().count())
            .collect::<Vec<_>>(),
        vec![40, 40, 40, 17]
    );
    assert_eq!(
        extract_terms("Gantz met Soumen, Gantz, AB and O'Reilly_2."),
        vec!["Gantz", "Soumen", "O'Reilly_2"]
    );
}

#[tokio::test]
async fn learn_uses_configured_limits_and_writes_one_atomic_ordered_batch() {
    let pool = pool().await;
    let context = TranslationContext::new("oso", "ja", "en");
    let session_id = session(&pool, &context, "learning").await;
    let service = IngestionService::new(
        &pool,
        IngestConfig {
            short_memory: ShortMemoryLimits {
                warning_sentence_count: 6,
                rejection_sentence_count: 30,
                warning_symbol_count: 0,
                rejection_symbol_count: 40,
            },
            learn: LearnLimits {
                max_block_chars: 40,
            },
        },
    )
    .unwrap();

    let result = service
        .learn(
            session_id,
            LearnInput {
                text: "Gantz avoids heavy armor.\n\nGantz relies on speed.".into(),
                source_role: "mentor".into(),
                source_ref: Some("audit:chapter-1".into()),
                kind: "learned_block".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.memory_ids, vec![1, 2]);
    let memories = WorkspaceStore::new(&pool)
        .list_short_term(session_id)
        .await
        .unwrap();
    assert_eq!(
        memories
            .iter()
            .map(|memory| (
                memory.source_role.as_str(),
                memory.kind.as_str(),
                memory.source_ref.as_str(),
                memory.text.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "mentor",
                "learned_block",
                "audit:chapter-1",
                "Gantz avoids heavy armor."
            ),
            (
                "mentor",
                "learned_block",
                "audit:chapter-1",
                "Gantz relies on speed."
            ),
        ]
    );
    assert_eq!(memories[0].metadata["ingestion_mode"], "learn");
    assert_eq!(memories[0].metadata["block_index"], 1);
    assert_eq!(memories[0].metadata["block_count"], 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "learn ingestion must stay in short-term memory"
    );

    sqlx::query(
        "CREATE TRIGGER reject_bad_ingest BEFORE INSERT ON short_term_memories WHEN new.text = 'Bad.' BEGIN SELECT RAISE(ABORT, 'reject bad ingest'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let error = service
        .learn(
            session_id,
            LearnInput {
                text: "Good.\n\nBad.".into(),
                source_role: "mentor".into(),
                source_ref: None,
                kind: "learned_block".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reject bad ingest"));
    assert_eq!(
        WorkspaceStore::new(&pool)
            .list_short_term(session_id)
            .await
            .unwrap()
            .len(),
        2,
        "a rejected block must roll back the entire batch"
    );
}

#[tokio::test]
async fn read_uses_complete_stored_context_and_optionally_stores_original_observation() {
    let pool = pool().await;
    let context = TranslationContext::new("oso", "ja", "en").with_metadata(
        &["ja".into(), "en".into()],
        &["volume:1".into(), "chapter:2".into()],
        &["combat".into()],
        &[],
    );
    let session_id = session(&pool, &context, "reading").await;
    let termbase = Termbase::new(&pool, context.clone());
    let rule_id = termbase
        .propose(TermProposal {
            series_slug: "oso".into(),
            source_language: "ja".into(),
            target_language: "en".into(),
            category: "name".into(),
            source_text: "攻撃力上昇".into(),
            canonical_translation: "Attack Increase".into(),
            tags: vec![],
        })
        .await
        .unwrap();
    termbase.approve_term(rule_id).await.unwrap();
    let service = IngestionService::new(&pool, IngestConfig::default()).unwrap();

    let result = service
        .read(
            session_id,
            ReadInput {
                text: "Gantz studies 攻撃力上昇.".into(),
                source_ref: Some("notes:gantz".into()),
                store_observation: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.candidate_terms, vec!["Gantz"]);
    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].crystal_id, rule_id);
    assert_eq!(result.findings[0].kind, "canonical_missing");

    let memories = WorkspaceStore::new(&pool)
        .list_short_term(session_id)
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].source_role, "observation");
    assert_eq!(memories[0].kind, "read_observation");
    assert_eq!(memories[0].text, "Gantz studies 攻撃力上昇.");
    assert_eq!(memories[0].source_ref, "notes:gantz");
    assert_eq!(memories[0].metadata["candidate_terms"][0], "Gantz");
    assert_eq!(memories[0].language_tags, context.language_tags);
    assert_eq!(memories[0].story_scopes, context.story_scopes);
    assert_eq!(memories[0].semantic_tags, context.semantic_tags);
}

#[tokio::test]
async fn read_rejects_unknown_inactive_and_mismatched_sessions_without_writing() {
    let pool = pool().await;
    let context = TranslationContext::new("oso", "ja", "en");
    let active = session(&pool, &context, "reading").await;
    let inactive = WorkspaceStore::new(&pool)
        .start_session(&context, "reading", "", "")
        .await
        .unwrap()
        .id;
    WorkspaceStore::new(&pool)
        .complete_session(inactive)
        .await
        .unwrap();
    let service = IngestionService::new(&pool, IngestConfig::default()).unwrap();
    let input = || ReadInput {
        text: "Gantz".into(),
        source_ref: None,
        store_observation: true,
    };

    assert!(matches!(
        service.read(999, input()).await,
        Err(IngestError::Workspace(_))
    ));
    assert!(matches!(
        service.read(inactive, input()).await,
        Err(IngestError::SessionInactive { .. })
    ));
    sqlx::query("UPDATE task_sessions SET source_language = 'de' WHERE id = ?")
        .bind(active)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        service.read(active, input()).await,
        Err(IngestError::SessionContext { .. })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

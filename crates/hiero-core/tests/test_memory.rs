use std::sync::Arc;

use chrono::{Duration, Utc};
use hiero_core::{
    db::connect_url,
    domain::{
        AddMemoryInput, AddMemoryResult, ShortMemoryLimits, TranslationContext, WorkspaceError,
        WorkspaceStore, complete_stale_sessions,
    },
    registry::SeriesRegistry,
};
use serde_json::json;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:memory-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

async fn context(pool: &sqlx::SqlitePool) -> TranslationContext {
    SeriesRegistry::new(pool)
        .create("oso", "Only Sense Online", "ja", "ru")
        .await
        .expect("series should exist");
    TranslationContext::new("oso", "ja", "ru").with_metadata(
        &["ja".into(), "ru".into(), "en".into(), "ja".into()],
        &["volume:1".into(), "chapter:2".into(), "volume:1".into()],
        &["ui".into(), "term".into(), "ui".into()],
        &[],
    )
}

fn memory(text: &str) -> AddMemoryInput {
    AddMemoryInput {
        source_role: "user".into(),
        kind: "note".into(),
        text: text.into(),
        source_ref: "v1c2".into(),
        metadata: json!({"importance": 4}),
        ..AddMemoryInput::default()
    }
}

fn assert_add_result_contract(result: &AddMemoryResult) {
    assert!(result.memory.id > 0);
}

#[tokio::test]
async fn sentence_limits_cover_below_equal_above_warning_and_rejection_boundaries() {
    let pool = pool().await;
    let context = context(&pool).await;
    let limits = ShortMemoryLimits {
        warning_sentence_count: 2,
        rejection_sentence_count: 4,
        warning_symbol_count: 0,
        rejection_symbol_count: 0,
    };
    let store = WorkspaceStore::with_limits(&pool, limits).expect("valid limits should configure");
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();

    let below = store
        .add_short_term(session.id, memory("One."))
        .await
        .unwrap();
    assert_add_result_contract(&below);
    assert!(below.warnings.is_empty());
    let equal_warning = store
        .add_short_term(session.id, memory("One. Two."))
        .await
        .unwrap();
    assert!(equal_warning.warnings.is_empty());
    let above_warning = store
        .add_short_term(session.id, memory("One. Two. Three."))
        .await
        .unwrap();
    assert_eq!(
        above_warning.warnings,
        ["short-term memory is large; prefer 1-6 sentences"]
    );
    let equal_rejection = store
        .add_short_term(session.id, memory("One. Two. Three. Four."))
        .await
        .unwrap();
    assert_eq!(equal_rejection.warnings.len(), 1);
    let sentence_error = store
        .add_short_term(session.id, memory("One. Two. Three. Four. Five."))
        .await
        .unwrap_err();
    assert!(matches!(
        sentence_error,
        WorkspaceError::Validation { field: "text", .. }
    ));
}

#[tokio::test]
async fn symbol_limits_use_unicode_character_counts_and_exact_boundaries() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::with_limits(
        &pool,
        ShortMemoryLimits {
            warning_sentence_count: 10,
            rejection_sentence_count: 20,
            warning_symbol_count: 8,
            rejection_symbol_count: 16,
        },
    )
    .unwrap();
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();

    assert!(
        store
            .add_short_term(session.id, memory("1234567"))
            .await
            .unwrap()
            .warnings
            .is_empty()
    );
    assert!(
        store
            .add_short_term(session.id, memory("12345678"))
            .await
            .unwrap()
            .warnings
            .is_empty()
    );
    let above_warning = store
        .add_short_term(session.id, memory("123456789"))
        .await
        .unwrap();
    assert_eq!(
        above_warning.warnings,
        ["short-term memory is large; prefer <= 8 symbols"]
    );
    let equal_rejection = store
        .add_short_term(session.id, memory("1234567890123456"))
        .await
        .unwrap();
    assert_eq!(equal_rejection.warnings.len(), 1);
    let symbol_error = store
        .add_short_term(session.id, memory("abcdefghijklmnopq"))
        .await
        .unwrap_err();
    assert!(symbol_error.to_string().contains("exceeds 16 symbols"));
    let unicode = WorkspaceStore::with_limits(
        &pool,
        ShortMemoryLimits {
            warning_sentence_count: 10,
            rejection_sentence_count: 20,
            warning_symbol_count: 2,
            rejection_symbol_count: 4,
        },
    )
    .unwrap()
    .add_short_term(session.id, memory("я界🙂"))
    .await
    .unwrap();
    assert_eq!(unicode.memory.metadata["symbol_count"], 3);
    assert_eq!(
        unicode.warnings,
        ["short-term memory is large; prefer <= 2 symbols"]
    );
}

#[tokio::test]
async fn combined_warnings_are_ordered_and_batch_rejection_writes_nothing() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::with_limits(
        &pool,
        ShortMemoryLimits {
            warning_sentence_count: 2,
            rejection_sentence_count: 4,
            warning_symbol_count: 8,
            rejection_symbol_count: 32,
        },
    )
    .unwrap();
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let combined = store
        .add_short_term(session.id, memory("One. Two. Three."))
        .await
        .unwrap();
    assert_eq!(
        combined.warnings,
        [
            "short-term memory is large; prefer 1-6 sentences",
            "short-term memory is large; prefer <= 8 symbols",
        ]
    );
    assert_eq!(
        combined.memory.metadata["validation_warning"],
        "short-term memory is large; prefer 1-6 sentences; short-term memory is large; prefer <= 8 symbols"
    );

    let ordered = store
        .add_short_term_batch(session.id, &[memory("Brief."), memory("One. Two. Three.")])
        .await
        .unwrap();
    assert_eq!(ordered[0].memory.text, "Brief.");
    assert!(ordered[0].warnings.is_empty());
    assert_eq!(ordered[1].memory.text, "One. Two. Three.");
    assert_eq!(ordered[1].warnings.len(), 2);

    let before = store.list_short_term(session.id).await.unwrap().len();
    let error = store
        .add_short_term_batch(
            session.id,
            &[memory("Valid."), memory("One. Two. Three. Four. Five.")],
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("too large"));
    assert_eq!(
        store.list_short_term(session.id).await.unwrap().len(),
        before
    );
}

#[tokio::test]
async fn default_limits_match_python_and_disabled_symbol_thresholds() {
    assert_eq!(
        ShortMemoryLimits::default(),
        ShortMemoryLimits {
            warning_sentence_count: 6,
            rejection_sentence_count: 30,
            warning_symbol_count: 0,
            rejection_symbol_count: 0,
        }
    );
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    assert!(
        store
            .add_short_term(session.id, memory("One. Two. Three. Four. Five. Six."))
            .await
            .unwrap()
            .warnings
            .is_empty()
    );
    assert_eq!(
        store
            .add_short_term(
                session.id,
                memory("One. Two. Three. Four. Five. Six. Seven.")
            )
            .await
            .unwrap()
            .warnings
            .len(),
        1
    );
    store
        .add_short_term(session.id, memory(&"界".repeat(10_000)))
        .await
        .expect("zero symbol limits are disabled");
    store
        .add_short_term(session.id, memory(&"One. ".repeat(30)))
        .await
        .expect("equal rejection boundary is accepted");
    assert!(
        store
            .add_short_term(session.id, memory(&"One. ".repeat(31)))
            .await
            .unwrap_err()
            .to_string()
            .contains("too large")
    );
}

#[tokio::test]
async fn invalid_limit_relationships_are_rejected_before_store_use() {
    let pool = pool().await;
    for limits in [
        ShortMemoryLimits {
            warning_sentence_count: 0,
            ..ShortMemoryLimits::default()
        },
        ShortMemoryLimits {
            rejection_sentence_count: 0,
            ..ShortMemoryLimits::default()
        },
        ShortMemoryLimits {
            warning_sentence_count: 7,
            rejection_sentence_count: 6,
            ..ShortMemoryLimits::default()
        },
        ShortMemoryLimits {
            warning_symbol_count: 10,
            rejection_symbol_count: 9,
            ..ShortMemoryLimits::default()
        },
    ] {
        assert!(WorkspaceStore::with_limits(&pool, limits).is_err());
    }
}

#[tokio::test]
async fn session_round_trips_context_and_ordered_typed_metadata() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);

    let session = store
        .start_session(&context, "translate", "1", "2")
        .await
        .expect("valid session should start");
    let loaded = store
        .get_session(session.id)
        .await
        .expect("session should load");

    assert_eq!(loaded.record, session.record);
    assert_eq!(loaded.language_tags, ["ja", "ru", "en"]);
    assert_eq!(loaded.story_scopes, ["volume:1", "chapter:2"]);
    assert_eq!(loaded.semantic_tags, ["ui", "term"]);
    assert_eq!(loaded.status, "active");
    assert_eq!(loaded.created_at, loaded.last_activity_at);
}

#[tokio::test]
async fn session_rejects_unknown_series_language_mismatch_and_incoherent_tags() {
    let pool = pool().await;
    let valid = context(&pool).await;
    let store = WorkspaceStore::new(&pool);

    let mut unknown = valid.clone();
    unknown.series_slug = "missing".into();
    unknown.scope_key = "series:missing".into();
    let error = store
        .start_session(&unknown, "translate", "", "")
        .await
        .expect_err("unknown series should fail");
    assert!(matches!(
        error,
        WorkspaceError::Validation {
            field: "series_slug",
            ..
        }
    ));

    let mut mismatch = valid.clone();
    mismatch.target_language = "en".into();
    mismatch.language_tags.push("en".into());
    let error = store
        .start_session(&mismatch, "translate", "", "")
        .await
        .expect_err("series language mismatch should fail");
    assert!(matches!(
        error,
        WorkspaceError::Validation {
            field: "languages",
            ..
        }
    ));

    let mut tags = valid;
    tags.language_tags = vec!["ja".into()];
    let error = store
        .start_session(&tags, "translate", "", "")
        .await
        .expect_err("partial canonical tags should fail");
    assert!(matches!(
        error,
        WorkspaceError::Validation {
            field: "language_tags",
            ..
        }
    ));
}

#[tokio::test]
async fn complete_session_is_strict_idempotent_and_rejects_unknown_ids() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "1", "2")
        .await
        .unwrap();

    assert!(store.complete_session(session.id).await.unwrap());
    assert!(!store.complete_session(session.id).await.unwrap());
    assert_eq!(
        store.get_session(session.id).await.unwrap().status,
        "completed"
    );
    assert!(matches!(
        store.complete_session(999_999).await.unwrap_err(),
        WorkspaceError::SessionNotFound { .. }
    ));
}

#[tokio::test]
async fn inactive_completion_uses_strict_cutoff_and_is_concurrency_idempotent() {
    let pool = Arc::new(pool().await);
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let stale = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let equal = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let fresh = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let cutoff = Utc::now() - Duration::minutes(30);
    for (id, timestamp) in [
        (stale.id, cutoff - Duration::seconds(1)),
        (equal.id, cutoff),
        (fresh.id, cutoff + Duration::seconds(1)),
    ] {
        sqlx::query("UPDATE task_sessions SET last_activity_at = ? WHERE id = ?")
            .bind(timestamp)
            .bind(id)
            .execute(&*pool)
            .await
            .unwrap();
    }

    let first_pool = Arc::clone(&pool);
    let second_pool = Arc::clone(&pool);
    let first = tokio::spawn(async move { complete_stale_sessions(&first_pool, cutoff).await });
    let second = tokio::spawn(async move { complete_stale_sessions(&second_pool, cutoff).await });
    let mut results = vec![
        first.await.unwrap().unwrap(),
        second.await.unwrap().unwrap(),
    ];
    results.sort_by_key(Vec::len);

    assert_eq!(results, [Vec::<i64>::new(), vec![stale.id]]);
    assert_eq!(
        WorkspaceStore::new(&pool)
            .get_session(equal.id)
            .await
            .unwrap()
            .status,
        "active"
    );
    assert_eq!(
        WorkspaceStore::new(&pool)
            .get_session(fresh.id)
            .await
            .unwrap()
            .status,
        "active"
    );
}

#[tokio::test]
async fn inactive_completion_updates_more_than_conventional_sqlite_variable_limit_at_once() {
    let pool = pool().await;
    let context = context(&pool).await;
    let cutoff = Utc::now() - Duration::minutes(30);
    let stale_at = cutoff - Duration::seconds(1);
    sqlx::query(
        "WITH RECURSIVE seq(value) AS (VALUES(1) UNION ALL SELECT value + 1 FROM seq WHERE value < 1200) INSERT INTO task_sessions(series_slug, source_language, target_language, task_type, volume, chapter, status, created_at, last_activity_at) SELECT 'oso', 'ja', 'ru', 'translate', '', '', 'active', ?, ? FROM seq",
    )
    .bind(stale_at)
    .bind(stale_at)
    .execute(&pool)
    .await
    .unwrap();
    let equal = WorkspaceStore::new(&pool)
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let fresh = WorkspaceStore::new(&pool)
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    sqlx::query("UPDATE task_sessions SET last_activity_at = ? WHERE id = ?")
        .bind(cutoff)
        .bind(equal.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE task_sessions SET last_activity_at = ? WHERE id = ?")
        .bind(cutoff + Duration::seconds(1))
        .bind(fresh.id)
        .execute(&pool)
        .await
        .unwrap();

    let completed = WorkspaceStore::new(&pool)
        .complete_inactive(cutoff)
        .await
        .unwrap();

    assert_eq!(completed.len(), 1200);
    assert_eq!(completed.first(), Some(&1));
    assert_eq!(completed.last(), Some(&1200));
    let completion_timestamps: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT completed_at) FROM task_sessions WHERE id <= 1200",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(completion_timestamps, 1);
    assert_eq!(
        WorkspaceStore::new(&pool)
            .get_session(equal.id)
            .await
            .unwrap()
            .status,
        "active"
    );
    assert_eq!(
        WorkspaceStore::new(&pool)
            .get_session(fresh.id)
            .await
            .unwrap()
            .status,
        "active"
    );
}

#[tokio::test]
async fn memory_add_list_search_and_archive_hydrate_all_public_metadata() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "1", "2")
        .await
        .unwrap();
    let mut input = memory("Keep inventory labels concise.");
    input.language_tags = vec![" RU ".into(), "en".into(), "ru".into()];
    input.story_scopes = vec!["chapter:2".into(), " scene:boss ".into()];
    input.semantic_tags = vec![" term ".into(), "ui".into(), "term".into()];
    input.source_credibility = "user_rule".into();
    input.rule_intent = " terminology ".into();
    input.soft_origin = Some(" inline-correction ".into());
    let before = session.last_activity_at;

    let added = store.add_short_term(session.id, input).await.unwrap();
    let id = added.memory.id;
    let listed = store.list_short_term(session.id).await.unwrap();
    let found = store
        .search_short_term(session.id, "inventory OR labels", 10)
        .await
        .unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(found.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(found[0], listed[0]);
    assert_eq!(listed[0].language_tags, ["ru", "en"]);
    assert_eq!(listed[0].story_scopes, ["chapter:2", "scene:boss"]);
    assert_eq!(listed[0].semantic_tags, ["term", "ui"]);
    assert_eq!(listed[0].source_credibility.as_deref(), Some("user_rule"));
    assert_eq!(listed[0].rule_intent.as_deref(), Some("terminology"));
    assert_eq!(listed[0].soft_origin.as_deref(), Some("inline-correction"));
    assert_eq!(listed[0].metadata["importance"], 4);
    assert_eq!(listed[0].metadata["sentence_count"], 1);
    assert_eq!(listed[0].metadata["symbol_count"], 30);
    assert!(
        store
            .get_session(session.id)
            .await
            .unwrap()
            .last_activity_at
            > before
    );

    store.archive(id).await.unwrap();
    store
        .archive(id)
        .await
        .expect("archive should be idempotent");
    assert!(store.list_short_term(session.id).await.unwrap().is_empty());
    assert!(
        store
            .search_short_term(session.id, "inventory", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn memory_input_validation_is_typed_and_precedes_any_write() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let cases = [
        (
            "source_role",
            AddMemoryInput {
                source_role: " ".into(),
                ..memory("valid")
            },
        ),
        (
            "kind",
            AddMemoryInput {
                kind: String::new(),
                ..memory("valid")
            },
        ),
        ("text", memory("   ")),
        (
            "source_credibility",
            AddMemoryInput {
                source_credibility: "invented".into(),
                ..memory("valid")
            },
        ),
    ];
    for (field, input) in cases {
        let error = store.add_short_term(session.id, input).await.unwrap_err();
        assert!(
            matches!(error, WorkspaceError::Validation { field: actual, .. } if actual == field)
        );
    }
    let error = store
        .add_short_term(
            session.id,
            AddMemoryInput {
                metadata: json!([]),
                ..memory("valid")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::MetadataObject));
    assert!(store.list_short_term(session.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn batch_is_atomic_preserves_input_order_accepts_duplicates_and_reuses_pool_after_failure() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let duplicate = memory("Same text is a valid separate observation.");
    let ids = store
        .add_short_term_batch(session.id, &[duplicate.clone(), duplicate])
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids[0].memory.id < ids[1].memory.id);

    sqlx::query(
        "CREATE TRIGGER reject_exploding_tag BEFORE INSERT ON short_term_memory_semantic_tags WHEN new.semantic_tag = 'explode' BEGIN SELECT RAISE(ABORT, 'injected side-table failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = store.list_short_term(session.id).await.unwrap().len();
    let error = store
        .add_short_term_batch(
            session.id,
            &[
                memory("This first row must roll back."),
                AddMemoryInput {
                    semantic_tags: vec!["explode".into()],
                    ..memory("This second row triggers failure.")
                },
            ],
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Database { .. }));
    assert_eq!(
        store.list_short_term(session.id).await.unwrap().len(),
        before
    );
    assert!(
        store
            .search_short_term(session.id, "first row", 10)
            .await
            .unwrap()
            .is_empty(),
        "the trigger-owned FTS row must roll back with the failed batch"
    );
    sqlx::query("DROP TRIGGER reject_exploding_tag")
        .execute(&pool)
        .await
        .unwrap();
    store
        .add_short_term(session.id, memory("Pool remains reusable."))
        .await
        .expect("rollback must finish before connection reuse");
}

#[tokio::test]
async fn cancelling_a_writer_waiting_for_begin_immediate_keeps_pool_reusable() {
    let pool = Arc::new(pool().await);
    let context = context(&pool).await;
    let session = WorkspaceStore::new(&pool)
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let first = pool.acquire().await.unwrap();
    let second = pool.acquire().await.unwrap();
    drop(first);
    drop(second);
    let lock = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("test should hold the writer lock");
    let idle_before_spawn = pool.num_idle();
    assert!(
        idle_before_spawn >= 1,
        "a second prewarmed connection must be idle"
    );
    let task_pool = Arc::clone(&pool);
    let session_id = session.id;
    let task = tokio::spawn(async move {
        WorkspaceStore::new(&task_pool)
            .add_short_term(session_id, memory("Cancelled while waiting."))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if pool.num_idle() < idle_before_spawn {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer must check out the second connection and reach the contended boundary");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    lock.rollback().await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if pool.num_idle() > idle_before_spawn {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("holder and cancelled writer connections must return to the pool");
    assert!(
        WorkspaceStore::new(&pool)
            .list_short_term(session_id)
            .await
            .unwrap()
            .is_empty()
    );

    WorkspaceStore::new(&pool)
        .add_short_term(
            session_id,
            memory("Pool remains healthy after cancellation."),
        )
        .await
        .expect("cancelled begin must not strand a pooled connection");
}

#[tokio::test]
async fn batch_rejects_empty_over_limit_and_inactive_or_missing_sessions() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();

    assert!(matches!(
        store
            .add_short_term_batch(session.id, &[])
            .await
            .unwrap_err(),
        WorkspaceError::Validation { field: "items", .. }
    ));
    let oversized = (0..501)
        .map(|index| memory(&format!("Memory {index}.")))
        .collect::<Vec<_>>();
    assert!(matches!(
        store
            .add_short_term_batch(session.id, &oversized)
            .await
            .unwrap_err(),
        WorkspaceError::Validation { field: "items", .. }
    ));
    store.complete_session(session.id).await.unwrap();
    assert!(matches!(
        store
            .add_short_term(session.id, memory("No longer active."))
            .await
            .unwrap_err(),
        WorkspaceError::SessionInactive { .. }
    ));
    assert!(matches!(
        store
            .add_short_term(999_999, memory("Unknown."))
            .await
            .unwrap_err(),
        WorkspaceError::SessionNotFound { .. }
    ));
}

#[tokio::test]
async fn concurrent_batches_are_serialized_without_lost_writes() {
    let pool = Arc::new(pool().await);
    let context = context(&pool).await;
    let session = WorkspaceStore::new(&pool)
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let first_pool = Arc::clone(&pool);
    let second_pool = Arc::clone(&pool);
    let session_id = session.id;
    let first = tokio::spawn(async move {
        WorkspaceStore::new(&first_pool)
            .add_short_term_batch(session_id, &[memory("A1."), memory("A2.")])
            .await
    });
    let second = tokio::spawn(async move {
        WorkspaceStore::new(&second_pool)
            .add_short_term_batch(session_id, &[memory("B1."), memory("B2.")])
            .await
    });
    let first_ids = first.await.unwrap().unwrap();
    let second_ids = second.await.unwrap().unwrap();
    let rows = WorkspaceStore::new(&pool)
        .list_short_term(session_id)
        .await
        .unwrap();

    assert!(first_ids[0].memory.id < first_ids[1].memory.id);
    assert!(second_ids[0].memory.id < second_ids[1].memory.id);
    assert_eq!(rows.len(), 4);
}

#[tokio::test]
async fn fts_is_trigger_owned_tracks_base_updates_and_deletes_and_escapes_plain_queries() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let id = store
        .add_short_term(session.id, memory("Original inventory wording."))
        .await
        .unwrap()
        .memory
        .id;

    assert_eq!(
        store
            .search_short_term(session.id, "inventory OR (wording)", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query(
        "UPDATE short_term_memories SET text = 'Replacement glossary wording.' WHERE id = ?",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        store
            .search_short_term(session.id, "inventory", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .search_short_term(session.id, "glossary", usize::MAX)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("DELETE FROM short_term_memories WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store
            .search_short_term(session.id, "glossary", 10)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query(
        "INSERT INTO short_term_memories_fts(short_term_memories_fts) VALUES ('integrity-check')",
    )
    .execute(&pool)
    .await
    .expect("trigger-maintained FTS should pass its integrity check");

    let source = include_str!("../src/domain/workspace.rs");
    assert!(!source.contains("INSERT INTO short_term_memories_fts"));
    assert!(!source.contains("DELETE FROM short_term_memories_fts"));
}

#[tokio::test]
async fn hydration_chunks_beyond_sqlite_variable_limit_without_n_plus_one() {
    let pool = pool().await;
    let context = context(&pool).await;
    let store = WorkspaceStore::new(&pool);
    let session = store
        .start_session(&context, "translate", "", "")
        .await
        .unwrap();
    let items = (0..500)
        .map(|index| AddMemoryInput {
            semantic_tags: vec![format!("tag-{index}")],
            ..memory(&format!("Bulk memory {index}."))
        })
        .collect::<Vec<_>>();
    store
        .add_short_term_batch(session.id, &items)
        .await
        .unwrap();
    let extra = store
        .add_short_term(
            session.id,
            AddMemoryInput {
                semantic_tags: vec!["tag-extra".into()],
                ..memory("Bulk extra.")
            },
        )
        .await
        .unwrap()
        .memory
        .id;

    let rows = store.list_short_term(session.id).await.unwrap();
    assert_eq!(rows.len(), 501);
    assert_eq!(rows.last().unwrap().id, extra);
    assert_eq!(rows.last().unwrap().semantic_tags, ["tag-extra"]);
}

#[tokio::test]
async fn list_and_search_reject_unknown_sessions_even_for_empty_queries() {
    let pool = pool().await;
    let store = WorkspaceStore::new(&pool);
    assert!(matches!(
        store.list_short_term(42).await.unwrap_err(),
        WorkspaceError::SessionNotFound { id: 42 }
    ));
    assert!(matches!(
        store.search_short_term(42, "", 10).await.unwrap_err(),
        WorkspaceError::SessionNotFound { id: 42 }
    ));
}

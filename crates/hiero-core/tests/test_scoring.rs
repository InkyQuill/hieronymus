use hiero_core::{
    db::{CrystalRecord, connect_url},
    domain::{
        AddCrystalInput, FeedbackError, FeedbackEvent, FeedbackStore, IMMEDIATE_EVENT_DELTAS,
        PASSIVE_EVENT_DELTAS, ScoreDelta, TranslationContext, WorkspaceStore, apply_score_delta,
    },
    registry::SeriesRegistry,
};
use sqlx::Row;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:scoring-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

async fn fixture(pool: &sqlx::SqlitePool) -> (i64, i64) {
    SeriesRegistry::new(pool)
        .create("oso", "Only Sense Online", "ja", "ru")
        .await
        .unwrap();
    let context = TranslationContext::new("oso", "ja", "ru");
    let session = WorkspaceStore::new(pool)
        .start_session(&context, "translate", "1", "2")
        .await
        .unwrap();
    let crystal = hiero_core::domain::CrystalStore::new(pool)
        .add(AddCrystalInput {
            crystal_type: "lesson".into(),
            title: "UI wording".into(),
            text: "Use concise Russian inventory labels.".into(),
            scope_type: "series".into(),
            scope_key: "series:oso".into(),
            series_slug: "oso".into(),
            source_language: "ja".into(),
            target_language: "ru".into(),
            strength: 0.4,
            confidence: 0.5,
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    (session.id, crystal)
}

async fn activation(pool: &sqlx::SqlitePool, session_id: i64, crystal_id: i64) {
    sqlx::query("INSERT INTO crystal_activations(crystal_id, session_id, recall_query, rank, score, created_at) VALUES (?, ?, 'inventory', 1, 0.8, '2026-07-19T00:00:00Z')")
        .bind(crystal_id)
        .bind(session_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn record(pool: &sqlx::SqlitePool, id: i64) -> CrystalRecord {
    sqlx::query_as("SELECT * FROM crystals WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[test]
fn pure_scoring_clamps_archives_and_preserves_archived_status() {
    let mut crystal = sample_record();
    crystal.strength = 0.95;
    crystal.confidence = 0.01;
    assert_eq!(
        apply_score_delta(
            &crystal,
            ScoreDelta {
                strength: 0.2,
                confidence: -0.5,
            },
        ),
        (1.0, 0.0, "archived".into())
    );
    crystal.status = "archived".into();
    assert_eq!(
        apply_score_delta(
            &crystal,
            ScoreDelta {
                strength: 0.1,
                confidence: 0.1,
            },
        )
        .2,
        "archived"
    );
}

#[test]
fn rule_intent_dampens_negative_deltas_by_credibility() {
    let mut ordinary = sample_record();
    ordinary.source_credibility = "user_rule".into();
    let mut rule = ordinary.clone();
    rule.rule_intent = "terminology".into();
    let delta = ScoreDelta {
        strength: -0.2,
        confidence: -0.2,
    };
    assert_eq!(
        apply_score_delta(&ordinary, delta),
        (0.3, 0.3, "active".into())
    );
    assert_eq!(
        apply_score_delta(&rule, delta),
        (0.405, 0.405, "active".into())
    );
}

#[test]
fn non_finite_deltas_are_deliberately_ignored() {
    let crystal = sample_record();
    assert_eq!(
        apply_score_delta(
            &crystal,
            ScoreDelta {
                strength: f64::NAN,
                confidence: f64::INFINITY,
            },
        ),
        (0.5, 0.5, "active".into())
    );
}

#[test]
fn event_delta_matrices_are_complete_and_exact() {
    let immediate = [
        ("confirmed_by_user", (0.15, 0.20)),
        ("contradicted_by_user", (-0.20, -0.25)),
        ("deleted_by_user", (-0.50, -0.35)),
        ("recalled_miss", (-0.05, -0.03)),
        ("recalled_useful", (0.06, 0.04)),
    ];
    assert_eq!(IMMEDIATE_EVENT_DELTAS.len(), immediate.len());
    for (label, delta) in immediate {
        assert_eq!(IMMEDIATE_EVENT_DELTAS.get(label), Some(&delta));
    }
    let passive = [
        ("caused_correction", (-0.10, -0.12)),
        ("cited", (0.03, 0.02)),
        ("passed_review", (0.07, 0.05)),
        ("recalled_again", (0.02, 0.0)),
        ("superseded", (-0.12, -0.05)),
        ("used_in_translation", (0.05, 0.02)),
    ];
    assert_eq!(PASSIVE_EVENT_DELTAS.len(), passive.len());
    for (label, delta) in passive {
        assert_eq!(PASSIVE_EVENT_DELTAS.get(label), Some(&delta));
    }
}

#[tokio::test]
async fn immediate_and_passive_events_share_one_audit_boundary() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    let store = FeedbackStore::new(&pool);
    let immediate = store
        .record(FeedbackEvent {
            crystal_id,
            event_type: "confirmed_by_user".into(),
            source_role: "editorial-review".into(),
            evidence: Some("accepted".into()),
            session_id: None,
        })
        .await
        .unwrap();
    let passive = store
        .record(FeedbackEvent {
            crystal_id,
            event_type: "recalled_again".into(),
            source_role: "system".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .unwrap();
    let crystal = record(&pool, crystal_id).await;
    assert_eq!((crystal.strength, crystal.confidence), (0.55, 0.7));
    let rows = sqlx::query("SELECT id, event_type, source_role, evidence, strength_delta, confidence_delta, applied FROM memory_events ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64, _>("id"), immediate);
    assert!(rows[0].get::<bool, _>("applied"));
    assert_eq!(rows[0].get::<String, _>("event_type"), "confirmed_by_user");
    assert_eq!(rows[0].get::<String, _>("source_role"), "editorial-review");
    assert_eq!(rows[0].get::<String, _>("evidence"), "accepted");
    assert_eq!(rows[1].get::<i64, _>("id"), passive);
    assert!(!rows[1].get::<bool, _>("applied"));
    assert_eq!(rows[1].get::<String, _>("evidence"), "");
}

#[tokio::test]
async fn recall_outcome_is_idempotent_and_conflicts_atomically() {
    let pool = pool().await;
    let (session_id, crystal_id) = fixture(&pool).await;
    activation(&pool, session_id, crystal_id).await;
    let store = FeedbackStore::new(&pool);
    store
        .record_recall_outcome(session_id, &[crystal_id, crystal_id], &[])
        .await
        .unwrap();
    store
        .record_recall_outcome(session_id, &[crystal_id], &[])
        .await
        .unwrap();
    assert_eq!(
        (record(&pool, crystal_id).await.strength * 100.0).round() / 100.0,
        0.46
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events WHERE event_type = 'recalled_useful'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let error = store
        .record_recall_outcome(session_id, &[], &[crystal_id])
        .await
        .expect_err("opposite outcome must conflict");
    assert!(matches!(error, FeedbackError::OutcomeConflict { .. }));
    assert_eq!(record(&pool, crystal_id).await.confidence, 0.54);
}

#[tokio::test]
async fn recall_outcomes_require_matching_activations_and_disjoint_lists() {
    let pool = pool().await;
    let (session_id, crystal_id) = fixture(&pool).await;
    let store = FeedbackStore::new(&pool);
    assert!(matches!(
        store
            .record_recall_outcome(session_id, &[crystal_id], &[])
            .await,
        Err(FeedbackError::ActivationNotFound { .. })
    ));
    activation(&pool, session_id, crystal_id).await;
    assert!(matches!(
        store
            .record_recall_outcome(session_id, &[crystal_id], &[crystal_id])
            .await,
        Err(FeedbackError::OutcomeOverlap { .. })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_duplicate_outcomes_apply_once() {
    let pool = pool().await;
    let (session_id, crystal_id) = fixture(&pool).await;
    activation(&pool, session_id, crystal_id).await;
    let left_pool = pool.clone();
    let right_pool = pool.clone();
    let (left, right) = tokio::join!(
        async move {
            FeedbackStore::new(&left_pool)
                .record_recall_outcome(session_id, &[crystal_id], &[])
                .await
        },
        async move {
            FeedbackStore::new(&right_pool)
                .record_recall_outcome(session_id, &[crystal_id], &[])
                .await
        },
    );
    left.unwrap();
    right.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events WHERE event_type = 'recalled_useful'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn multiple_activations_share_one_crystal_outcome_delta() {
    let pool = pool().await;
    let (session_id, crystal_id) = fixture(&pool).await;
    activation(&pool, session_id, crystal_id).await;
    activation(&pool, session_id, crystal_id).await;
    FeedbackStore::new(&pool)
        .record_recall_outcome(session_id, &[], &[crystal_id])
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystal_activations WHERE outcome = 'miss'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events WHERE event_type = 'recalled_miss'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let crystal = record(&pool, crystal_id).await;
    assert!((crystal.strength - 0.35).abs() < f64::EPSILON);
    assert!((crystal.confidence - 0.47).abs() < f64::EPSILON);
}

#[tokio::test]
async fn feedback_preserves_candidate_and_archived_lifecycle_states() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    sqlx::query("UPDATE crystals SET status = 'candidate' WHERE id = ?")
        .bind(crystal_id)
        .execute(&pool)
        .await
        .unwrap();
    let store = FeedbackStore::new(&pool);
    store
        .record(FeedbackEvent {
            crystal_id,
            event_type: "confirmed_by_user".into(),
            source_role: "user".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(record(&pool, crystal_id).await.status, "candidate");
    sqlx::query("UPDATE crystals SET status = 'archived' WHERE id = ?")
        .bind(crystal_id)
        .execute(&pool)
        .await
        .unwrap();
    store
        .record(FeedbackEvent {
            crystal_id,
            event_type: "confirmed_by_user".into(),
            source_role: "user".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(record(&pool, crystal_id).await.status, "archived");
}

#[tokio::test]
async fn delete_threshold_archives_and_score_only_update_keeps_fts_integrity() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    FeedbackStore::new(&pool)
        .record(FeedbackEvent {
            crystal_id,
            event_type: "deleted_by_user".into(),
            source_role: "user".into(),
            evidence: Some("removed".into()),
            session_id: None,
        })
        .await
        .unwrap();
    let crystal = record(&pool, crystal_id).await;
    assert_eq!(crystal.strength, 0.0);
    assert!((crystal.confidence - 0.15).abs() < f64::EPSILON);
    assert_eq!(crystal.status, "archived");
    sqlx::query("INSERT INTO crystals_fts(crystals_fts) VALUES ('integrity-check')")
        .execute(&pool)
        .await
        .expect("trigger-owned FTS shadow should remain valid");
}

#[tokio::test]
async fn cross_context_feedback_rolls_back_without_audit_or_score_change() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    SeriesRegistry::new(&pool)
        .create("beta", "Beta", "ja", "ru")
        .await
        .unwrap();
    let other = WorkspaceStore::new(&pool)
        .start_session(
            &TranslationContext::new("beta", "ja", "ru"),
            "translate",
            "",
            "",
        )
        .await
        .unwrap();
    let result = FeedbackStore::new(&pool)
        .record(FeedbackEvent {
            crystal_id,
            event_type: "confirmed_by_user".into(),
            source_role: "user".into(),
            evidence: None,
            session_id: Some(other.id),
        })
        .await;
    assert!(matches!(
        result,
        Err(FeedbackError::SessionContext {
            field: "series_slug",
            ..
        })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        (
            record(&pool, crystal_id).await.strength,
            record(&pool, crystal_id).await.confidence
        ),
        (0.4, 0.5)
    );
}

#[tokio::test]
async fn database_fault_rolls_back_audit_and_pool_remains_reusable() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    sqlx::query("CREATE TRIGGER reject_score_update BEFORE UPDATE OF strength ON crystals BEGIN SELECT RAISE(ABORT, 'injected score fault'); END")
        .execute(&pool).await.unwrap();
    let event = || FeedbackEvent {
        crystal_id,
        event_type: "confirmed_by_user".into(),
        source_role: "user".into(),
        evidence: None,
        session_id: None,
    };
    assert!(matches!(
        FeedbackStore::new(&pool).record(event()).await,
        Err(FeedbackError::Database { .. })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER reject_score_update")
        .execute(&pool)
        .await
        .unwrap();
    FeedbackStore::new(&pool)
        .record(event())
        .await
        .expect("rolled-back connection should be reusable");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn cancelling_a_waiting_feedback_write_keeps_pool_reusable() {
    let pool = pool().await;
    let (_, crystal_id) = fixture(&pool).await;
    let blocker = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let waiting_pool = pool.clone();
    let writer = tokio::spawn(async move {
        FeedbackStore::new(&waiting_pool)
            .record(FeedbackEvent {
                crystal_id,
                event_type: "confirmed_by_user".into(),
                source_role: "user".into(),
                evidence: None,
                session_id: None,
            })
            .await
    });
    tokio::task::yield_now().await;
    writer.abort();
    let _ = writer.await;
    blocker.rollback().await.unwrap();
    FeedbackStore::new(&pool)
        .record(FeedbackEvent {
            crystal_id,
            event_type: "confirmed_by_user".into(),
            source_role: "user".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .expect("pool should remain reusable after cancellation");
}

#[tokio::test]
async fn a_late_outcome_conflict_rolls_back_earlier_crystal_changes() {
    let pool = pool().await;
    let (session_id, first_id) = fixture(&pool).await;
    let second_id = hiero_core::domain::CrystalStore::new(&pool)
        .add(AddCrystalInput {
            crystal_type: "lesson".into(),
            title: "Second".into(),
            text: "Second activated memory.".into(),
            scope_type: "series".into(),
            scope_key: "series:oso".into(),
            series_slug: "oso".into(),
            source_language: "ja".into(),
            target_language: "ru".into(),
            strength: 0.4,
            confidence: 0.5,
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    activation(&pool, session_id, first_id).await;
    activation(&pool, session_id, second_id).await;
    sqlx::query("UPDATE crystal_activations SET outcome = 'miss' WHERE crystal_id = ?")
        .bind(second_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        FeedbackStore::new(&pool)
            .record_recall_outcome(session_id, &[first_id, second_id], &[])
            .await,
        Err(FeedbackError::OutcomeConflict { crystal_id, .. }) if crystal_id == second_id
    ));
    assert_eq!(record(&pool, first_id).await.strength, 0.4);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let first_outcome = sqlx::query_scalar::<_, Option<String>>(
        "SELECT outcome FROM crystal_activations WHERE crystal_id = ?",
    )
    .bind(first_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_outcome, None);
}

fn sample_record() -> CrystalRecord {
    let now = chrono::Utc::now();
    CrystalRecord {
        id: 1,
        crystal_type: "lesson".into(),
        text: "text".into(),
        title: "title".into(),
        scope_type: "global".into(),
        scope_key: String::new(),
        series_slug: String::new(),
        source_language: String::new(),
        target_language: String::new(),
        tags_json: "[]".into(),
        strength: 0.5,
        confidence: 0.5,
        source_credibility: "observation".into(),
        rule_intent: String::new(),
        soft_origin: None,
        is_inferred: false,
        malformed_penalty: 0.0,
        supersedes_crystal_id: None,
        status: "active".into(),
        created_cycle: 0,
        last_activated_cycle: None,
        last_reinforced_cycle: None,
        created_at: now,
        updated_at: now,
    }
}

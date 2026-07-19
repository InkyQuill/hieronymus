use hiero_core::{
    db::connect_url,
    domain::{
        AddCrystalInput, AddMemoryInput, CrystalStore, MemorySource, TranslationContext,
        WorkspaceStore,
    },
    recall::{RecallError, RecallService},
    registry::SeriesRegistry,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:recall-{}?mode=memory&cache=shared",
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
        &["ja".into(), "ru".into()],
        &["volume:5".into()],
        &["inventory-ui".into()],
        &[],
    )
}

fn crystal(text: &str) -> AddCrystalInput {
    AddCrystalInput {
        crystal_type: "lesson".into(),
        title: "Inventory wording".into(),
        text: text.into(),
        scope_type: "series".into(),
        scope_key: "series:oso".into(),
        series_slug: "oso".into(),
        source_language: "ja".into(),
        target_language: "ru".into(),
        ..AddCrystalInput::default()
    }
}

async fn session(pool: &sqlx::SqlitePool, ctx: &TranslationContext) -> i64 {
    WorkspaceStore::new(pool)
        .start_session(ctx, "translate", "5", "1")
        .await
        .expect("session should start")
        .id
}

#[tokio::test]
async fn rule_lane_is_boosted_before_limit_but_is_not_mandatory() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);

    let mut ordinary = crystal("Render inventory glyph consistently.");
    ordinary.strength = 0.8;
    ordinary.confidence = 0.8;
    ordinary.source_credibility = "observation".into();
    let ordinary_id = store.add(ordinary).await.unwrap();
    for index in 0..12 {
        let mut decoy = crystal(&format!(
            "Render inventory glyph consistently in decoy {index}."
        ));
        decoy.strength = 0.8;
        decoy.confidence = 0.8;
        store.add(decoy).await.unwrap();
    }
    let mut boosted_rule = crystal("Render inventory glyph consistently.");
    boosted_rule.strength = 0.0;
    boosted_rule.confidence = 0.0;
    boosted_rule.source_credibility = "user_rule".into();
    boosted_rule.rule_intent = "terminology".into();
    let rule_id = store.add(boosted_rule).await.unwrap();

    let result = RecallService::new(&pool)
        .recall(session_id, &ctx, "inventory glyph", 1)
        .await
        .unwrap();
    assert_eq!(result[0].id, rule_id, "boost must apply before truncation");
    assert_ne!(result[0].id, ordinary_id);

    let mut strong = crystal("Render inventory menu deterministically.");
    strong.strength = 1.0;
    strong.confidence = 1.0;
    strong.source_credibility = "expert".into();
    let strong_id = store.add(strong).await.unwrap();
    let mut weak_rule = crystal("Render inventory menu deterministically.");
    weak_rule.strength = 0.0;
    weak_rule.confidence = 0.0;
    weak_rule.source_credibility = "rumor".into();
    weak_rule.rule_intent = "style".into();
    let weak_rule_id = store.add(weak_rule).await.unwrap();

    let result = RecallService::new(&pool)
        .recall(session_id, &ctx, "inventory menu deterministically", 1)
        .await
        .unwrap();
    assert_eq!(result[0].id, strong_id);
    assert_ne!(result[0].id, weak_rule_id, "rule intent is not mandatory");
}

#[tokio::test]
async fn source_credibility_and_context_metadata_boost_ranking() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let mut ordinary = crystal("Guarded inventory crafting phrase.");
    ordinary.source_credibility = "rumor".into();
    let ordinary_id = store.add(ordinary).await.unwrap();
    let mut credible = crystal("Guarded inventory crafting phrase.");
    credible.source_credibility = "expert".into();
    credible.story_scopes = vec!["volume:5".into()];
    credible.semantic_tags = vec!["inventory-ui".into()];
    let credible_id = store.add(credible).await.unwrap();

    let results = RecallService::new(&pool)
        .recall(session_id, &ctx, "guarded inventory crafting", 2)
        .await
        .unwrap();
    assert_eq!(
        results.iter().map(|item| item.id).collect::<Vec<_>>(),
        [credible_id, ordinary_id]
    );
    assert!(results[0].score > results[1].score);
}

#[tokio::test]
async fn graph_expansion_is_one_hop_context_filtered_and_cross_lane_deduplicated() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let mut source = crystal("Inventory anchor phrase.");
    source.strength = 1.0;
    source.confidence = 1.0;
    source.source_credibility = "expert".into();
    let source_id = store.add(source).await.unwrap();
    let neighbor_id = store
        .add(crystal("Associative wording reached only through a link."))
        .await
        .unwrap();
    let duplicate_id = store
        .add(crystal("Inventory anchor phrase duplicate lane."))
        .await
        .unwrap();
    let second_hop_id = store
        .add(crystal("Second hop must never appear."))
        .await
        .unwrap();
    let mut other_series = crystal("Other-series linked wording.");
    other_series.series_slug = "other".into();
    other_series.scope_key = "series:other".into();
    let other_id = store.add(other_series).await.unwrap();
    store.link(source_id, neighbor_id, "related").await.unwrap();
    store
        .link(source_id, duplicate_id, "related")
        .await
        .unwrap();
    store
        .link(neighbor_id, second_hop_id, "related")
        .await
        .unwrap();
    store.link(source_id, other_id, "related").await.unwrap();

    let results = RecallService::new(&pool)
        .recall(session_id, &ctx, "inventory anchor phrase", 10)
        .await
        .unwrap();
    let ids = results.iter().map(|item| item.id).collect::<Vec<_>>();
    assert_eq!(ids.iter().filter(|&&id| id == duplicate_id).count(), 1);
    assert!(ids.contains(&neighbor_id));
    assert!(ids.contains(&source_id));
    assert!(!ids.contains(&second_hop_id));
    assert!(!ids.contains(&other_id));
}

#[tokio::test]
async fn ranking_uses_stable_id_tie_breaking() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let first = store.add(crystal("Stable inventory tie.")).await.unwrap();
    let second = store.add(crystal("Stable inventory tie.")).await.unwrap();

    let first_run = RecallService::new(&pool)
        .recall(session_id, &ctx, "stable inventory tie", 2)
        .await
        .unwrap();
    let second_run = RecallService::new(&pool)
        .recall(session_id, &ctx, "stable inventory tie", 2)
        .await
        .unwrap();
    let expected = vec![first, second];
    assert_eq!(
        first_run.iter().map(|item| item.id).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        second_run.iter().map(|item| item.id).collect::<Vec<_>>(),
        expected
    );
}

#[tokio::test]
async fn recall_creates_one_working_copy_and_logs_each_returned_activation() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let first = store
        .add(crystal("Compact inventory label one."))
        .await
        .unwrap();
    let second = store
        .add(crystal("Compact inventory label two."))
        .await
        .unwrap();
    let service = RecallService::new(&pool);

    let first_results = service
        .recall(session_id, &ctx, "compact inventory label", 2)
        .await
        .unwrap();
    assert!(
        first_results
            .iter()
            .all(|result| result.source == MemorySource::LongTerm)
    );
    assert_eq!(
        first_results
            .iter()
            .map(|result| result.rank)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let activation_count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_activations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(activation_count, 2);
    let copy_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM short_term_memories WHERE source_crystal_id IN (?, ?)",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(copy_count, 2);

    service
        .recall(session_id, &ctx, "compact inventory label", 2)
        .await
        .unwrap();
    let copy_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM short_term_memories WHERE source_crystal_id IN (?, ?)",
    )
    .bind(first)
    .bind(second)
    .fetch_one(&pool)
    .await
    .unwrap();
    let repeat_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events WHERE event_type = 'recalled_again'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let activation_count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_activations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((copy_count, repeat_events, activation_count), (2, 2, 4));
}

#[tokio::test]
async fn side_effect_failure_rolls_back_every_working_copy_event_and_activation_for_retry() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let first = store
        .add(crystal("Atomic recall boundary one."))
        .await
        .unwrap();
    let second = store
        .add(crystal("Atomic recall boundary two."))
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_second_activation BEFORE INSERT ON crystal_activations WHEN NEW.rank = 2 BEGIN SELECT RAISE(ABORT, 'injected activation failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    RecallService::new(&pool)
        .recall(session_id, &ctx, "atomic recall boundary", 2)
        .await
        .expect_err("the injected final activation failure must abort recall");
    let counts: (i64, i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM crystal_activations")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!(
        counts,
        (0, 0, 0),
        "no partial recall side effect may commit"
    );

    sqlx::query("DROP TRIGGER fail_second_activation")
        .execute(&pool)
        .await
        .unwrap();
    let retry = RecallService::new(&pool)
        .recall(session_id, &ctx, "atomic recall boundary", 2)
        .await
        .unwrap();
    assert_eq!(
        retry.iter().map(|item| item.id).collect::<Vec<_>>(),
        [first, second]
    );
    let counts: (i64, i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM crystal_activations")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!(
        counts,
        (2, 0, 2),
        "retry must behave like the first attempt"
    );

    sqlx::query("DELETE FROM crystal_activations")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_repeat_activation BEFORE INSERT ON crystal_activations WHEN NEW.rank = 2 BEGIN SELECT RAISE(ABORT, 'injected repeat failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    RecallService::new(&pool)
        .recall(session_id, &ctx, "atomic recall boundary", 2)
        .await
        .expect_err("repeat events must share the failing activation transaction");
    let counts: (i64, i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM short_term_memories")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM memory_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM crystal_activations")
            .fetch_one(&pool)
            .await
            .unwrap(),
    );
    assert_eq!(counts, (2, 0, 0), "repeat events must also roll back");

    sqlx::query("DROP TRIGGER fail_repeat_activation")
        .execute(&pool)
        .await
        .unwrap();
    RecallService::new(&pool)
        .recall(session_id, &ctx, "atomic recall boundary", 2)
        .await
        .unwrap();
    let repeat_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events WHERE event_type = 'recalled_again'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        repeat_events, 2,
        "successful retry records each repeat once"
    );
}

#[tokio::test]
async fn invisible_low_id_links_cannot_starve_a_visible_neighbor_before_limit() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let store = CrystalStore::new(&pool);
    let mut invisible_ids = Vec::new();
    for index in 0..13 {
        let mut invisible = crystal(&format!("Invisible other-series neighbor {index}."));
        invisible.series_slug = "other".into();
        invisible.scope_key = "series:other".into();
        invisible_ids.push(store.add(invisible).await.unwrap());
    }
    let mut source = crystal("Unique starvation anchor phrase.");
    source.strength = 1.0;
    source.confidence = 1.0;
    source.source_credibility = "expert".into();
    let source_id = store.add(source).await.unwrap();
    let visible_id = store
        .add(crystal(
            "Visible associative neighbor without lexical overlap.",
        ))
        .await
        .unwrap();
    for invisible_id in invisible_ids {
        store
            .link(source_id, invisible_id, "related")
            .await
            .unwrap();
    }
    store.link(source_id, visible_id, "related").await.unwrap();

    let results = RecallService::new(&pool)
        .recall(session_id, &ctx, "unique starvation anchor phrase", 2)
        .await
        .unwrap();
    assert_eq!(
        results.iter().map(|item| item.id).collect::<Vec<_>>(),
        [source_id, visible_id]
    );
}

#[tokio::test]
async fn reverse_link_lookup_has_a_target_first_index() {
    let pool = pool().await;
    let indexes = sqlx::query("PRAGMA index_list(crystal_links)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();
    assert!(
        indexes
            .iter()
            .any(|name| name == "idx_crystal_links_target")
    );

    let plan = sqlx::query("EXPLAIN QUERY PLAN SELECT source_crystal_id FROM crystal_links WHERE target_crystal_id = ? ORDER BY source_crystal_id, link_type LIMIT ?")
        .bind(1_i64)
        .bind(10_i64)
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("idx_crystal_links_target"),
        "query plan: {plan}"
    );
}

#[tokio::test]
async fn short_term_lane_is_ranked_without_duplicating_a_returned_working_copy() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    let workspace = WorkspaceStore::new(&pool);
    let note = workspace
        .add_short_term(
            session_id,
            AddMemoryInput {
                text: "Remember compact inventory captions.".into(),
                metadata: json!({"origin": "chapter-note"}),
                ..AddMemoryInput::default()
            },
        )
        .await
        .unwrap()
        .memory;
    let crystal_id = CrystalStore::new(&pool)
        .add(crystal("Compact inventory captions use nouns."))
        .await
        .unwrap();

    let first = RecallService::new(&pool)
        .recall(session_id, &ctx, "compact inventory captions", 5)
        .await
        .unwrap();
    assert!(
        first
            .iter()
            .any(|item| item.source == MemorySource::ShortTerm && item.id == note.id)
    );
    assert!(
        first
            .iter()
            .any(|item| item.source == MemorySource::LongTerm && item.id == crystal_id)
    );
    let second = RecallService::new(&pool)
        .recall(session_id, &ctx, "compact inventory captions", 5)
        .await
        .unwrap();
    let source_copy_id: i64 =
        sqlx::query("SELECT id FROM short_term_memories WHERE source_crystal_id = ?")
            .bind(crystal_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("id");
    assert!(
        !second
            .iter()
            .any(|item| item.source == MemorySource::ShortTerm && item.id == source_copy_id)
    );
}

#[tokio::test]
async fn recall_rejects_inactive_or_context_mismatched_sessions() {
    let pool = pool().await;
    let ctx = context(&pool).await;
    let session_id = session(&pool, &ctx).await;
    assert!(
        RecallService::new(&pool)
            .recall(session_id, &ctx, "", 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        RecallService::new(&pool)
            .recall(session_id, &ctx, "inventory", 0)
            .await
            .unwrap_err(),
        RecallError::Validation { field: "limit", .. }
    ));
    let mismatched = TranslationContext::new("oso", "en", "ru");
    let error = RecallService::new(&pool)
        .recall(session_id, &mismatched, "inventory", 5)
        .await
        .expect_err("session context must match");
    assert!(matches!(error, RecallError::SessionContext { .. }));

    WorkspaceStore::new(&pool)
        .complete_session(session_id)
        .await
        .unwrap();
    let error = RecallService::new(&pool)
        .recall(session_id, &ctx, "inventory", 5)
        .await
        .expect_err("inactive session must be rejected");
    assert!(matches!(error, RecallError::SessionInactive { .. }));
}

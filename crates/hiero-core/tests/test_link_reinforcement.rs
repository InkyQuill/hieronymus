use chrono::Utc;
use hiero_core::{
    db::{CrystalActivationRecord, connect_url},
    domain::{AddCrystalInput, CrystalStore},
    dreaming::{
        COMBINATION_TEXT_SIMILARITY_THRESHOLD, DecayManager, DecayScope, DreamPhase, LinkOutcome,
        LinkReinforcer, text_similarity, useful_pairs,
    },
};
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:link-reinforcement-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .unwrap()
}

fn crystal(text: &str, credibility: &str, strength: f64) -> AddCrystalInput {
    AddCrystalInput {
        crystal_type: "lesson".into(),
        text: text.into(),
        source_credibility: credibility.into(),
        strength,
        ..AddCrystalInput::default()
    }
}

fn activation(id: i64, crystal_id: i64, session_id: i64) -> CrystalActivationRecord {
    CrystalActivationRecord {
        id,
        crystal_id,
        session_id,
        recall_query: "q".into(),
        rank: id,
        score: 1.0,
        reason: String::new(),
        outcome: Some("useful".into()),
        cycle_id: Some(9),
        created_at: Utc::now(),
    }
}

#[test]
fn text_similarity_uses_the_same_unicode_token_metric_and_strict_boundary() {
    assert_eq!(text_similarity("Straße label", "STRASSE LABEL"), 1.0);
    assert_eq!(
        text_similarity("one two three four five", "one two three four changed"),
        COMBINATION_TEXT_SIMILARITY_THRESHOLD
    );
    assert!(
        text_similarity(
            "one two three four five six",
            "one two three four five changed"
        ) > COMBINATION_TEXT_SIMILARITY_THRESHOLD
    );
}

#[test]
fn useful_pairs_are_canonical_deduplicated_and_lexicographic() {
    let rows = vec![
        activation(1, 3, 1),
        activation(2, 1, 1),
        activation(3, 3, 1),
        activation(4, 2, 1),
    ];
    assert_eq!(useful_pairs(&rows), [(1, 2), (1, 3), (2, 3)]);
}

#[tokio::test]
async fn link_reinforcer_combines_only_one_pair_and_preserves_audit_links() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let survivor = store
        .add(crystal("same memory text", "expert", 0.7))
        .await
        .unwrap();
    let absorbed = store
        .add(crystal("same memory text", "observation", 0.9))
        .await
        .unwrap();
    let third = store
        .add(crystal("same memory text", "observation", 0.8))
        .await
        .unwrap();
    store.link(absorbed, third, "supports").await.unwrap();

    let outcomes = LinkReinforcer::new(9)
        .run(
            &pool,
            vec![
                activation(1, survivor, 1),
                activation(2, absorbed, 1),
                activation(3, third, 1),
            ],
        )
        .await
        .unwrap();

    assert!(outcomes.contains(&LinkOutcome::Combined {
        survivor_id: survivor,
        absorbed_id: absorbed,
    }));
    assert_eq!(store.get(absorbed).await.unwrap().status, "superseded");
    assert_eq!(store.get(third).await.unwrap().status, "active");
    let combined = sqlx::query("SELECT evidence,strength_delta,confidence_delta,applied,cycle_id FROM memory_events WHERE crystal_id=? AND event_type='combined_into'")
        .bind(absorbed).fetch_one(&pool).await.unwrap();
    assert_eq!(combined.get::<String, _>("evidence"), survivor.to_string());
    assert_eq!(combined.get::<f64, _>("strength_delta"), 0.0);
    assert_eq!(combined.get::<f64, _>("confidence_delta"), 0.0);
    assert!(combined.get::<bool, _>("applied"));
    assert_eq!(combined.get::<i64, _>("cycle_id"), 9);
    let original_link: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_links WHERE source_crystal_id=? AND target_crystal_id=? AND link_type='supports'")
        .bind(absorbed).bind(third).fetch_one(&pool).await.unwrap();
    let copied_link: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_links WHERE source_crystal_id=? AND target_crystal_id=? AND link_type='supports'")
        .bind(survivor).bind(third).fetch_one(&pool).await.unwrap();
    assert_eq!((original_link, copied_link), (1, 1));
}

#[tokio::test]
async fn combination_keeps_the_stronger_duplicate_concept_link() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let survivor = store
        .add(crystal("survivor wording", "expert", 0.7))
        .await
        .unwrap();
    let absorbed = store
        .add(crystal("absorbed wording", "observation", 0.9))
        .await
        .unwrap();
    let now = Utc::now();
    let concept_id = sqlx::query("INSERT INTO concepts(canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at) VALUES ('Shared','','global','','established',0.8,?,?)")
        .bind(now).bind(now).execute(&pool).await.unwrap().last_insert_rowid();
    for (crystal_id, confidence) in [(survivor, 0.2), (absorbed, 0.9)] {
        sqlx::query("INSERT INTO crystal_concepts(crystal_id,concept_id,link_type,confidence,created_at) VALUES (?,?,'mentions',?,?)")
            .bind(crystal_id).bind(concept_id).bind(confidence).bind(now).execute(&pool).await.unwrap();
    }
    LinkReinforcer::new(10)
        .run(
            &pool,
            vec![activation(1, survivor, 1), activation(2, absorbed, 1)],
        )
        .await
        .unwrap();
    let confidence: f64 = sqlx::query_scalar("SELECT confidence FROM crystal_concepts WHERE crystal_id=? AND concept_id=? AND link_type='mentions'")
        .bind(survivor).bind(concept_id).fetch_one(&pool).await.unwrap();
    assert_eq!(confidence, 0.9);
}

#[tokio::test]
async fn decay_is_bounded_stable_dampened_and_uses_the_maintenance_index() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let protected = store
        .add(crystal("protected", "observation", 0.5))
        .await
        .unwrap();
    let ordinary = store
        .add(crystal("ordinary", "observation", 0.19))
        .await
        .unwrap();
    let mut rule = crystal("rule", "user_rule", 0.19);
    rule.crystal_type = "rule".into();
    rule.rule_intent = "correction".into();
    rule.confidence = 0.005;
    let rule = store.add(rule).await.unwrap();
    let scope = DecayScope {
        after_id: 0,
        current_cycle: 7,
        stale_before_cycle: 7,
        recalled_ids: vec![protected],
        linked_ids: vec![],
        limit: 200,
    };
    let plan = DecayManager::explain(&pool, &scope).await.unwrap();
    assert!(
        plan.iter()
            .any(|line| line.contains("idx_crystals_maintenance"))
    );
    assert!(!plan.iter().any(|line| line.contains("TEMP B-TREE")));
    assert!(!plan.iter().any(|line| line.contains("SCAN crystals")));

    let decayed = DecayManager.run(&pool, scope).await.unwrap();
    assert_eq!(decayed, [ordinary, rule]);
    assert_eq!(store.get(ordinary).await.unwrap().strength, 0.16);
    let dampened = store.get(rule).await.unwrap();
    assert!((dampened.strength - 0.17425).abs() < 1e-9);
    assert_eq!(dampened.status, "archived");
    assert_eq!(store.get(protected).await.unwrap().strength, 0.5);
}

#[tokio::test]
async fn decay_pages_by_exclusive_id_with_a_hard_two_hundred_row_cap() {
    let pool = pool().await;
    let now = Utc::now();
    for index in 0..203 {
        sqlx::query("INSERT INTO crystals(crystal_type,text,scope_type,strength,confidence,status,created_cycle,created_at,updated_at) VALUES ('lesson',?,'global',0.5,0.5,'active',0,?,?)")
            .bind(format!("candidate {index}")).bind(now).bind(now).execute(&pool).await.unwrap();
    }
    let scope = DecayScope {
        after_id: 0,
        current_cycle: 5,
        stale_before_cycle: 5,
        recalled_ids: vec![],
        linked_ids: vec![],
        limit: 500,
    };
    let first = DecayManager.run(&pool, scope.clone()).await.unwrap();
    assert_eq!(first.len(), 200);
    assert!(first.windows(2).all(|pair| pair[0] < pair[1]));
    let second = DecayManager
        .run(
            &pool,
            DecayScope {
                after_id: *first.last().unwrap(),
                ..scope
            },
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 3);
    assert!(second[0] > first[199]);
}

#[tokio::test]
async fn cancelling_a_waiting_decay_result_changes_nothing_and_reuses_the_pool() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let id = store
        .add(crystal("cancelled decay", "observation", 0.5))
        .await
        .unwrap();
    let blocker = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let cancelled = tokio::time::timeout(
        Duration::from_millis(30),
        DecayManager.run(
            &pool,
            DecayScope {
                after_id: 0,
                current_cycle: 8,
                stale_before_cycle: 8,
                recalled_ids: vec![],
                linked_ids: vec![],
                limit: 200,
            },
        ),
    )
    .await;
    assert!(cancelled.is_err());
    blocker.rollback().await.unwrap();
    assert_eq!(store.get(id).await.unwrap().strength, 0.5);
    sqlx::query("UPDATE crystals SET title='pool reusable' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
}

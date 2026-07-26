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
    activation_at(id, crystal_id, session_id, 9)
}

fn activation_at(
    id: i64,
    crystal_id: i64,
    session_id: i64,
    cycle_id: i64,
) -> CrystalActivationRecord {
    CrystalActivationRecord {
        id,
        crystal_id,
        session_id,
        recall_query: "q".into(),
        rank: id,
        score: 1.0,
        reason: String::new(),
        outcome: Some("useful".into()),
        cycle_id: Some(cycle_id),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn link_reinforcer_ignores_useful_activations_from_other_cycles() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let left = store
        .add(crystal("same text", "expert", 0.7))
        .await
        .unwrap();
    let right = store
        .add(crystal("same text", "observation", 0.7))
        .await
        .unwrap();

    let outcomes = LinkReinforcer::new(10)
        .run(
            &pool,
            vec![activation_at(1, left, 1, 9), activation_at(2, right, 1, 9)],
        )
        .await
        .unwrap();

    assert!(outcomes.is_empty());
    let link_count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(link_count, 0);
    assert_eq!(store.get(left).await.unwrap().status, "active");
    assert_eq!(store.get(right).await.unwrap().status, "active");
}

#[tokio::test]
async fn one_crystal_budget_never_permits_a_two_participant_link_or_combination() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let left = store
        .add(crystal("same text", "expert", 0.7))
        .await
        .unwrap();
    let right = store
        .add(crystal("same text", "observation", 0.7))
        .await
        .unwrap();

    let outcomes = LinkReinforcer::new(9)
        .with_affected_crystals(1, [])
        .run(&pool, vec![activation(1, left, 1), activation(2, right, 1)])
        .await
        .unwrap();

    assert!(outcomes.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystal_links")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(store.get(left).await.unwrap().status, "active");
    assert_eq!(store.get(right).await.unwrap().status, "active");
}

#[tokio::test]
async fn link_reinforcer_durably_limits_each_crystal_to_one_combination_per_cycle() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let survivor = store
        .add(crystal("same text", "expert", 0.7))
        .await
        .unwrap();
    let first_absorbed = store
        .add(crystal("same text", "observation", 0.7))
        .await
        .unwrap();
    let later_candidate = store
        .add(crystal("same text", "observation", 0.6))
        .await
        .unwrap();
    let reinforcer = LinkReinforcer::new(9);

    let first = reinforcer
        .run(
            &pool,
            vec![activation(1, survivor, 1), activation(2, first_absorbed, 1)],
        )
        .await
        .unwrap();
    assert!(matches!(first.as_slice(), [LinkOutcome::Combined { .. }]));

    let second = reinforcer
        .run(
            &pool,
            vec![
                activation(3, survivor, 2),
                activation(4, later_candidate, 2),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        second,
        [LinkOutcome::Strengthened {
            source_id: survivor,
            target_id: later_candidate,
        }]
    );
    assert_eq!(store.get(later_candidate).await.unwrap().status, "active");
    let combinations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events WHERE event_type='combined_into' AND cycle_id=9",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(combinations, 1);
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
    LinkReinforcer::new(9)
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
async fn decay_retry_in_the_same_cycle_changes_each_crystal_at_most_once() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let id = store
        .add(crystal("retry once", "observation", 0.5))
        .await
        .unwrap();
    let scope = DecayScope {
        after_id: 0,
        current_cycle: 8,
        stale_before_cycle: 8,
        recalled_ids: vec![],
        linked_ids: vec![],
        limit: 200,
    };

    assert_eq!(DecayManager.run(&pool, scope.clone()).await.unwrap(), [id]);
    assert!(DecayManager.run(&pool, scope).await.unwrap().is_empty());
    assert_eq!(store.get(id).await.unwrap().strength, 0.47);
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events
         WHERE crystal_id=? AND event_type='cycle_decay' AND cycle_id=8",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 1);
}

#[tokio::test]
async fn decay_executes_every_protection_and_staleness_boundary() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let recalled = store
        .add(crystal("recalled", "observation", 0.5))
        .await
        .unwrap();
    let linked = store
        .add(crystal("linked", "observation", 0.5))
        .await
        .unwrap();
    let created_current = store
        .add(crystal("created current", "observation", 0.5))
        .await
        .unwrap();
    let activated_current = store
        .add(crystal("activated current", "observation", 0.5))
        .await
        .unwrap();
    let at_cutoff = store
        .add(crystal("at cutoff", "observation", 0.5))
        .await
        .unwrap();
    let before_cutoff = store
        .add(crystal("before cutoff", "observation", 0.5))
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET created_cycle=10 WHERE id=?")
        .bind(created_current)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET last_activated_cycle=10 WHERE id=?")
        .bind(activated_current)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET last_reinforced_cycle=10 WHERE id=?")
        .bind(at_cutoff)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET last_reinforced_cycle=9 WHERE id=?")
        .bind(before_cutoff)
        .execute(&pool)
        .await
        .unwrap();

    let decayed = DecayManager
        .run(
            &pool,
            DecayScope {
                after_id: 0,
                current_cycle: 10,
                stale_before_cycle: 10,
                recalled_ids: vec![recalled],
                linked_ids: vec![linked],
                limit: 200,
            },
        )
        .await
        .unwrap();

    assert_eq!(decayed, [before_cutoff]);
    for protected in [
        recalled,
        linked,
        created_current,
        activated_current,
        at_cutoff,
    ] {
        assert_eq!(store.get(protected).await.unwrap().strength, 0.5);
    }
    assert_eq!(store.get(before_cutoff).await.unwrap().strength, 0.47);
}

#[tokio::test]
async fn cancelling_in_flight_decay_preserves_prior_commit_and_retry_converges() {
    let directory = tempfile::tempdir().unwrap();
    let pool = connect_url(&format!(
        "sqlite://{}",
        directory.path().join("decay-cancellation.sqlite").display()
    ))
    .await
    .unwrap();
    let store = CrystalStore::new(&pool);
    let first = store
        .add(crystal("committed decay", "observation", 0.5))
        .await
        .unwrap();
    let second = store
        .add(crystal("in-flight decay", "observation", 0.5))
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER slow_second_decay
         BEFORE UPDATE OF strength ON crystals
         WHEN OLD.id={second} AND NEW.strength < OLD.strength
         BEGIN
           SELECT sum(value) FROM (
             WITH RECURSIVE counter(value) AS (
               VALUES(0)
               UNION ALL
               SELECT value + 1 FROM counter WHERE value < 2000000
             )
             SELECT value FROM counter
           );
         END"
    )))
    .execute(&pool)
    .await
    .unwrap();
    let scope = DecayScope {
        after_id: 0,
        current_cycle: 8,
        stale_before_cycle: 8,
        recalled_ids: vec![],
        linked_ids: vec![],
        limit: 200,
    };
    let run_pool = pool.clone();
    let run_scope = scope.clone();
    let task = tokio::spawn(async move { DecayManager.run(&run_pool, run_scope).await });

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let committed: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM memory_events
                 WHERE crystal_id=? AND event_type='cycle_decay' AND cycle_id=8",
            )
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
            if committed == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the first decay result must commit");

    let mut observer = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut *observer)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *observer)
            .await
            .is_ok()
        {
            sqlx::query("ROLLBACK")
                .execute(&mut *observer)
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the second decay result must hold the write transaction");

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(observer);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let first_strength = store.get(first).await.unwrap().strength;
            let second_strength = store.get(second).await.unwrap().strength;
            if first_strength == 0.47 && second_strength == 0.5 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the in-flight transaction must roll back");
    sqlx::query("DROP TRIGGER slow_second_decay")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(DecayManager.run(&pool, scope).await.unwrap(), [second]);
    assert_eq!(store.get(first).await.unwrap().strength, 0.47);
    assert_eq!(store.get(second).await.unwrap().strength, 0.47);
    let first_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events
         WHERE crystal_id=? AND event_type='cycle_decay' AND cycle_id=8",
    )
    .bind(first)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_events, 1);
    sqlx::query("UPDATE crystals SET title='pool reusable' WHERE id=?")
        .bind(second)
        .execute(&pool)
        .await
        .unwrap();
}

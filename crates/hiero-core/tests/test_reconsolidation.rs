use chrono::Utc;
use hiero_core::{
    db::{ConceptRecord, CrystalActivationRecord, MemoryEventRecord, connect_url, migrate},
    domain::{AddCrystalInput, CrystalStore, FeedbackEvent, FeedbackStore, WorkspaceStore},
    dreaming::{
        Consolidator, DreamPhase, LinkReinforcer, ReconsolidationDecision, ReconsolidationOutcome,
        Reconsolidator, ReinforcementManager, diff_ratio, reconsolidation_decision,
    },
};
use sqlx::Row;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:reconsolidation-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .unwrap()
}

async fn concept(
    pool: &sqlx::SqlitePool,
    name: &str,
    scope_key: &str,
    status: &str,
    confidence: f64,
) -> ConceptRecord {
    sqlx::query_as(
        "INSERT INTO concepts(canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at)
         VALUES (?,'','series',?,?,?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP) RETURNING *",
    )
    .bind(name)
    .bind(scope_key)
    .bind(status)
    .bind(confidence)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn consolidator_persists_stable_casefolded_pending_merge_proposals_idempotently() {
    let pool = pool().await;
    migrate(&pool).await.unwrap();
    let target = concept(&pool, " STRASSE ", "series:oso", "established", 0.7).await;
    let source_low = concept(&pool, "Straße", "series:oso", "candidate", 0.9).await;
    let source_high = concept(&pool, "strasse", "series:oso", "established", 0.6).await;
    let other_scope = concept(&pool, "STRASSE", "series:other", "established", 1.0).await;
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO dream_runs(cycle_id,status,provider,created_at) VALUES (1,'running','deterministic',CURRENT_TIMESTAMP) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let input = vec![
        source_low.clone(),
        other_scope,
        target.clone(),
        source_high.clone(),
    ];

    let first = Consolidator::new(run_id, 10)
        .run(&pool, input.clone())
        .await
        .unwrap();
    let second = Consolidator::new(run_id, 10)
        .run(&pool, input)
        .await
        .unwrap();

    assert_eq!(
        first
            .iter()
            .map(|proposal| (proposal.source_concept_id, proposal.target_concept_id))
            .collect::<Vec<_>>(),
        [(source_low.id, target.id), (source_high.id, target.id)]
    );
    assert!(second.is_empty());
    let rows: Vec<(i64, i64, String, String)> = sqlx::query_as(
        "SELECT source_concept_id,target_concept_id,rationale,status
         FROM concept_merge_proposals ORDER BY source_concept_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.1 == target.id
        && row.2 == "Canonical name and scope are exact duplicates."
        && row.3 == "pending"));
}

fn input(text: &str) -> AddCrystalInput {
    AddCrystalInput {
        crystal_type: "lesson".into(),
        text: text.into(),
        title: "Inventory".into(),
        scope_type: "series".into(),
        scope_key: "series:oso".into(),
        series_slug: "oso".into(),
        source_language: "ja".into(),
        target_language: "ru".into(),
        tags: vec!["ui".into()],
        language_tags: vec!["ja".into()],
        semantic_tags: vec!["inventory".into()],
        ..AddCrystalInput::default()
    }
}

async fn working_copy(
    pool: &sqlx::SqlitePool,
    crystal_id: i64,
    text: &str,
) -> hiero_core::domain::ShortTermMemory {
    let now = Utc::now();
    sqlx::query("INSERT OR IGNORE INTO series(slug,title,default_source_language,default_target_language,created_at,updated_at) VALUES ('oso','OSO','ja','ru',?,?)")
        .bind(now).bind(now).execute(pool).await.unwrap();
    let session_id = sqlx::query("INSERT INTO task_sessions(series_slug,source_language,target_language,task_type,status,created_at,last_activity_at) VALUES ('oso','ja','ru','translate','completed',?,?)")
        .bind(now).bind(now).execute(pool).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO short_term_memories(session_id,source_role,kind,text,source_ref,metadata_json,source_credibility,rule_intent,source_crystal_id,created_at) VALUES (?,'agent','working_copy',?,'','{}','observation','',?,?)")
        .bind(session_id).bind(text).bind(crystal_id).bind(now).execute(pool).await.unwrap();
    WorkspaceStore::new(pool)
        .list_short_term(session_id)
        .await
        .unwrap()
        .pop()
        .unwrap()
}

#[test]
fn unicode_word_diff_casefolds_and_uses_longer_token_count() {
    assert_eq!(diff_ratio("Straße label", "STRASSE LABEL"), 0.0);
    assert_eq!(
        diff_ratio("one two changed four", "one two three four"),
        0.25
    );
    assert_eq!(diff_ratio("", ""), 0.0);
    assert_eq!(diff_ratio("memory", ""), 1.0);
}

#[test]
fn reconsolidation_threshold_is_exclusive_for_in_place_reinforcement() {
    assert_eq!(
        reconsolidation_decision("one two changed four five", "one two three four five", 0.20),
        ReconsolidationDecision::Supersede
    );
    assert_eq!(
        reconsolidation_decision("one two three four five", "one two three four five", 0.20),
        ReconsolidationDecision::ReinforceInPlace
    );
}

#[tokio::test]
async fn reconsolidator_reinforces_and_archives_one_working_copy_atomically() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let crystal_id = store.add(input("one two three four five")).await.unwrap();
    let memory = working_copy(&pool, crystal_id, "one two three four five").await;
    let crystal = store.get(crystal_id).await.unwrap();

    let outcomes = Reconsolidator::new(0.20, 7)
        .run(&pool, vec![(memory.clone(), crystal)])
        .await
        .unwrap();

    assert_eq!(
        outcomes,
        [ReconsolidationOutcome::ReinforcedInPlace { crystal_id }]
    );
    let updated = store.get(crystal_id).await.unwrap();
    assert_eq!(updated.strength, 0.52);
    assert_eq!(updated.confidence, 0.5);
    assert_eq!(updated.last_reinforced_cycle, Some(7));
    let row = sqlx::query("SELECT event_type,strength_delta,confidence_delta,applied,cycle_id FROM memory_events WHERE crystal_id=?")
        .bind(crystal_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        row.get::<String, _>("event_type"),
        "reconsolidated_in_place"
    );
    assert_eq!(row.get::<f64, _>("strength_delta"), 0.02);
    assert_eq!(row.get::<f64, _>("confidence_delta"), 0.0);
    assert!(row.get::<bool, _>("applied"));
    assert_eq!(row.get::<i64, _>("cycle_id"), 7);
    assert!(
        WorkspaceStore::new(&pool)
            .list_short_term(memory.session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn reconsolidator_supersedes_with_inherited_metadata_and_concepts() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let old_id = store.add(input("one two three four five")).await.unwrap();
    let concept_id = sqlx::query("INSERT INTO concepts(canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at) VALUES ('Inventory','','series','series:oso','established',0.8,?,?)")
        .bind(Utc::now()).bind(Utc::now()).execute(&pool).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO crystal_concepts(crystal_id,concept_id,link_type,confidence,created_at) VALUES (?,?,'mentions',0.9,?)")
        .bind(old_id).bind(concept_id).bind(Utc::now()).execute(&pool).await.unwrap();
    let memory = working_copy(&pool, old_id, "completely revised inventory wording").await;
    let old = store.get(old_id).await.unwrap();
    let old_tags_json = old.tags_json.clone();

    let outcomes = Reconsolidator::new(0.20, 11)
        .run(&pool, vec![(memory.clone(), old)])
        .await
        .unwrap();
    let ReconsolidationOutcome::Superseded {
        old_crystal_id,
        new_crystal_id,
    } = outcomes[0]
    else {
        panic!("expected supersession");
    };
    assert_eq!(old_crystal_id, old_id);
    let new = store.get(new_crystal_id).await.unwrap();
    assert_eq!(new.text, "completely revised inventory wording");
    assert_eq!(new.supersedes_crystal_id, Some(old_id));
    assert_eq!(new.created_cycle, 11);
    assert_eq!(new.tags_json, old_tags_json);
    assert_eq!(new.language_tags, ["ja"]);
    assert_eq!(new.semantic_tags, ["inventory"]);
    assert_eq!(new.concept_ids, [concept_id]);
    assert_eq!(store.get(old_id).await.unwrap().status, "superseded");
    assert!(
        WorkspaceStore::new(&pool)
            .list_short_term(memory.session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn passive_reinforcement_applies_once_and_marks_the_cycle() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let crystal_id = store.add(input("repeat recall")).await.unwrap();
    let event_id = FeedbackStore::new(&pool)
        .record(FeedbackEvent {
            crystal_id,
            event_type: "recalled_again".into(),
            source_role: "recall".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .unwrap();
    let event: MemoryEventRecord = sqlx::query_as("SELECT * FROM memory_events WHERE id=?")
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let manager = ReinforcementManager::new(13);
    assert_eq!(
        manager.run(&pool, vec![event.clone()]).await.unwrap(),
        [crystal_id]
    );
    assert!(manager.run(&pool, vec![event]).await.unwrap().is_empty());
    let reinforced = store.get(crystal_id).await.unwrap();
    assert_eq!(reinforced.strength, 0.52);
    assert_eq!(reinforced.last_reinforced_cycle, Some(13));
    let applied: (bool, i64) =
        sqlx::query_as("SELECT applied,cycle_id FROM memory_events WHERE id=?")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(applied, (true, 13));
}

#[tokio::test]
async fn passive_reinforcement_rereads_the_canonical_event_delta() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let crystal_id = store.add(input("canonical delta")).await.unwrap();
    let event_id = FeedbackStore::new(&pool)
        .record(FeedbackEvent {
            crystal_id,
            event_type: "recalled_again".into(),
            source_role: "recall".into(),
            evidence: None,
            session_id: None,
        })
        .await
        .unwrap();
    let mut event: MemoryEventRecord = sqlx::query_as("SELECT * FROM memory_events WHERE id=?")
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    event.strength_delta = 0.48;
    event.event_type = "not_the_stored_event".into();
    event.applied = true;
    event.crystal_id = None;

    ReinforcementManager::new(14)
        .run(&pool, vec![event])
        .await
        .unwrap();
    assert_eq!(store.get(crystal_id).await.unwrap().strength, 0.52);
}

#[tokio::test]
async fn reconsolidator_rereads_the_working_copy_after_write_lock_acquisition() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let old_id = store.add(input("one two three four five")).await.unwrap();
    let stale_memory = working_copy(&pool, old_id, "one two three four five").await;
    sqlx::query("UPDATE short_term_memories SET text='completely revised wording' WHERE id=?")
        .bind(stale_memory.id)
        .execute(&pool)
        .await
        .unwrap();

    let outcomes = Reconsolidator::new(0.20, 15)
        .run(
            &pool,
            vec![(stale_memory, store.get(old_id).await.unwrap())],
        )
        .await
        .unwrap();
    let ReconsolidationOutcome::Superseded { new_crystal_id, .. } = outcomes[0] else {
        panic!("current working-copy text must drive the decision");
    };
    assert_eq!(
        store.get(new_crystal_id).await.unwrap().text,
        "completely revised wording"
    );
}

#[tokio::test]
async fn reconsolidator_rejects_source_linked_non_working_copy_from_locked_reread() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let source_id = store.add(input("one two three")).await.unwrap();
    let memory = working_copy(&pool, source_id, "one two three").await;
    sqlx::query("UPDATE short_term_memories SET kind='note' WHERE id=?")
        .bind(memory.id)
        .execute(&pool)
        .await
        .unwrap();

    let error = Reconsolidator::new(0.20, 16)
        .run(
            &pool,
            vec![(memory.clone(), store.get(source_id).await.unwrap())],
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        hiero_core::dreaming::DreamPhaseError::InvalidInput(
            "source-linked memory is not a working copy"
        )
    ));
    assert_eq!(store.get(source_id).await.unwrap().strength, 0.5);
    let archived_at: Option<String> =
        sqlx::query_scalar("SELECT archived_at FROM short_term_memories WHERE id=?")
            .bind(memory.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(archived_at.is_none());
}

#[tokio::test]
async fn reconsolidator_rejects_a_source_that_is_no_longer_active() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let old_id = store.add(input("stale source")).await.unwrap();
    let memory = working_copy(&pool, old_id, "stale source").await;
    let stale_snapshot = store.get(old_id).await.unwrap();
    store.archive(old_id).await.unwrap();

    assert!(
        Reconsolidator::new(0.20, 16)
            .run(&pool, vec![(memory.clone(), stale_snapshot)])
            .await
            .is_err()
    );
    assert_eq!(store.get(old_id).await.unwrap().strength, 0.5);
    assert_eq!(
        WorkspaceStore::new(&pool)
            .list_short_term(memory.session_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn supersession_failure_rolls_back_new_row_status_and_archive() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let old_id = store.add(input("old source wording")).await.unwrap();
    let concept_id = sqlx::query("INSERT INTO concepts(canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at) VALUES ('Rollback','','series','series:oso','established',0.8,?,?)")
        .bind(Utc::now()).bind(Utc::now()).execute(&pool).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO crystal_concepts(crystal_id,concept_id,link_type,confidence,created_at) VALUES (?,?,'mentions',0.9,?)")
        .bind(old_id).bind(concept_id).bind(Utc::now()).execute(&pool).await.unwrap();
    let memory = working_copy(&pool, old_id, "new and very different wording").await;
    let old = store.get(old_id).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER fail_inherited_concept BEFORE INSERT ON crystal_concepts WHEN NEW.crystal_id <> {old_id} BEGIN SELECT RAISE(ABORT,'inherit failure'); END"
    )))
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        Reconsolidator::new(0.20, 17)
            .run(&pool, vec![(memory.clone(), old)])
            .await
            .is_err()
    );
    assert_eq!(store.get(old_id).await.unwrap().status, "active");
    let crystal_count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(crystal_count, 1);
    assert_eq!(
        WorkspaceStore::new(&pool)
            .list_short_term(memory.session_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn reconsolidation_retry_converges_after_an_earlier_supersession_commits() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let first_id = store.add(input("first old wording")).await.unwrap();
    let second_id = store.add(input("second old wording")).await.unwrap();
    let first_memory = working_copy(&pool, first_id, "first completely revised wording").await;
    let second_memory = working_copy(&pool, second_id, "second completely revised wording").await;
    let first_source = store.get(first_id).await.unwrap();
    let second_source = store.get(second_id).await.unwrap();
    let original_input = vec![
        (first_memory.clone(), first_source.clone()),
        (second_memory.clone(), second_source.clone()),
    ];
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER fail_second_supersession
         BEFORE UPDATE OF status ON crystals
         WHEN OLD.id={second_id} AND NEW.status='superseded'
         BEGIN SELECT RAISE(ABORT,'second supersession failure'); END"
    )))
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        Reconsolidator::new(0.20, 18)
            .run(&pool, original_input.clone())
            .await
            .is_err()
    );
    let first_replacement: i64 =
        sqlx::query_scalar("SELECT id FROM crystals WHERE supersedes_crystal_id=?")
            .bind(first_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(store.get(first_id).await.unwrap().status, "superseded");
    assert_eq!(store.get(second_id).await.unwrap().status, "active");
    assert!(
        WorkspaceStore::new(&pool)
            .list_short_term(first_memory.session_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        WorkspaceStore::new(&pool)
            .list_short_term(second_memory.session_id)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("DROP TRIGGER fail_second_supersession")
        .execute(&pool)
        .await
        .unwrap();

    let outcomes = Reconsolidator::new(0.20, 18)
        .run(&pool, original_input)
        .await
        .unwrap();

    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes[0],
        ReconsolidationOutcome::Superseded {
            old_crystal_id: first_id,
            new_crystal_id: first_replacement,
        }
    );
    assert!(matches!(
        outcomes[1],
        ReconsolidationOutcome::Superseded {
            old_crystal_id,
            ..
        } if old_crystal_id == second_id
    ));
    let replacement_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM crystals WHERE supersedes_crystal_id IN (?,?)")
            .bind(first_id)
            .bind(second_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(replacement_count, 2);
}

#[tokio::test]
async fn in_place_reconsolidation_retry_survives_later_same_cycle_combination() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let source_id = store.add(input("stable wording")).await.unwrap();
    let memory = working_copy(&pool, source_id, "stable wording").await;
    let source_snapshot = store.get(source_id).await.unwrap();
    let retry_input = vec![(memory, source_snapshot)];
    let reconsolidator = Reconsolidator::new(0.20, 19);
    assert_eq!(
        reconsolidator
            .run(&pool, retry_input.clone())
            .await
            .unwrap(),
        [ReconsolidationOutcome::ReinforcedInPlace {
            crystal_id: source_id,
        }]
    );
    let mut partner_input = input("stable wording");
    partner_input.source_credibility = "expert".into();
    let partner_id = store.add(partner_input).await.unwrap();
    let now = Utc::now();
    let activation = |id, crystal_id| CrystalActivationRecord {
        id,
        crystal_id,
        session_id: 1,
        recall_query: "q".into(),
        rank: id,
        score: 1.0,
        reason: String::new(),
        outcome: Some("useful".into()),
        cycle_id: Some(19),
        created_at: now,
    };
    LinkReinforcer::new(19)
        .run(
            &pool,
            vec![activation(1, source_id), activation(2, partner_id)],
        )
        .await
        .unwrap();
    assert_eq!(store.get(source_id).await.unwrap().status, "superseded");

    assert_eq!(
        reconsolidator.run(&pool, retry_input).await.unwrap(),
        [ReconsolidationOutcome::ReinforcedInPlace {
            crystal_id: source_id,
        }]
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memory_events
         WHERE crystal_id=? AND event_type='reconsolidated_in_place' AND cycle_id=19",
    )
    .bind(source_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 1);
}

#[tokio::test]
async fn retry_discriminates_mixed_outcomes_for_two_working_copies_of_one_source() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let source_id = store.add(input("shared source wording")).await.unwrap();
    let equivalent = working_copy(&pool, source_id, "shared source wording").await;
    let revised = working_copy(
        &pool,
        source_id,
        "completely different revised source wording",
    )
    .await;
    let source_snapshot = store.get(source_id).await.unwrap();
    let original_input = vec![
        (equivalent.clone(), source_snapshot.clone()),
        (revised.clone(), source_snapshot),
    ];
    let reconsolidator = Reconsolidator::new(0.20, 20);

    let first = reconsolidator
        .run(&pool, original_input.clone())
        .await
        .unwrap();
    let ReconsolidationOutcome::Superseded { new_crystal_id, .. } = first[1] else {
        panic!("the revised working copy must supersede");
    };
    assert_eq!(
        first,
        [
            ReconsolidationOutcome::ReinforcedInPlace {
                crystal_id: source_id,
            },
            ReconsolidationOutcome::Superseded {
                old_crystal_id: source_id,
                new_crystal_id,
            },
        ]
    );

    assert_eq!(
        reconsolidator.run(&pool, original_input).await.unwrap(),
        first
    );
    let replacement_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM crystals WHERE supersedes_crystal_id=?")
            .bind(source_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(replacement_count, 1);
    let replacement_source: i64 =
        sqlx::query_scalar("SELECT short_term_memory_id FROM crystal_sources WHERE crystal_id=?")
            .bind(new_crystal_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(replacement_source, revised.id);
}

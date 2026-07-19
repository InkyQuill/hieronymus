use hiero_core::{
    db::connect_url,
    domain::{
        AddCrystalInput, ConceptStore, CreateConceptInput, CrystalStore, TermProposal, Termbase,
    },
};
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:termbase-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

fn proposal(source: &str, canonical: &str) -> TermProposal {
    TermProposal {
        series_slug: "oso".into(),
        source_language: "ja".into(),
        target_language: "en".into(),
        category: "name".into(),
        source_text: source.into(),
        canonical_translation: canonical.into(),
        tags: vec!["character".into(), "character".into()],
    }
}

#[tokio::test]
async fn proposed_terms_are_inert_until_atomic_idempotent_approval() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool);
    let id = termbase.propose(proposal("ガンツ", "Gantz")).await.unwrap();
    assert!(
        termbase
            .contract("ガンツ laughed")
            .await
            .unwrap()
            .is_empty()
    );
    termbase.approve_term(id).await.unwrap();
    termbase.approve_term(id).await.unwrap();
    let contract = termbase.contract("ガンツ laughed").await.unwrap();
    assert_eq!(contract.len(), 1);
    assert_eq!(contract[0].crystal_id, id);
    assert_eq!(contract[0].source_text, "ガンツ");
    assert_eq!(contract[0].canonical_translation, "Gantz");
    let concept_links: i64 =
        sqlx::query_scalar("SELECT count(*) FROM crystal_concepts WHERE crystal_id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(concept_links, 1);
}

#[tokio::test]
async fn contract_and_validation_use_only_active_linked_rule_intent_crystals() {
    let pool = pool().await;
    let concepts = ConceptStore::new(&pool);
    let crystals = CrystalStore::new(&pool);
    let concept = concepts
        .create(CreateConceptInput {
            canonical_name: "攻撃力上昇".into(),
            scope_type: "series".into(),
            scope_key: "series:oso".into(),
        })
        .await
        .unwrap();
    let mut input = AddCrystalInput {
        crystal_type: "observation".into(),
        text: "攻撃力上昇 is translated as Attack Increase, not Attack Boost.".into(),
        scope_type: "series".into(),
        scope_key: "series:oso".into(),
        series_slug: "oso".into(),
        source_language: "ja".into(),
        target_language: "en".into(),
        source_credibility: "user_rule".into(),
        rule_intent: "skill".into(),
        strength: 0.9,
        confidence: 0.9,
        ..AddCrystalInput::default()
    };
    let active = crystals.add(input.clone()).await.unwrap();
    concepts
        .link_crystal(active, concept.id, "defines", 0.95)
        .await
        .unwrap();
    input.text = "Hidden is translated as Secret.".into();
    input.status = "archived".into();
    let archived = crystals.add(input).await.unwrap();
    concepts
        .link_crystal(archived, concept.id, "defines", 0.95)
        .await
        .unwrap();

    let termbase = Termbase::new(&pool);
    let contract = termbase.contract("攻撃力上昇 and Hidden").await.unwrap();
    assert_eq!(
        contract
            .iter()
            .map(|term| term.crystal_id)
            .collect::<Vec<_>>(),
        [active]
    );
    let findings = termbase
        .validate("Attack Boost", Some("攻撃力上昇"), None)
        .await
        .unwrap();
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.kind.as_str())
            .collect::<Vec<_>>(),
        ["forbidden_variant_used", "canonical_missing"]
    );
}

#[tokio::test]
async fn ambiguous_concepts_are_not_guessed_and_longer_surfaces_win() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool);
    let first = termbase.propose(proposal("Sense", "Talent")).await.unwrap();
    let second = termbase
        .propose(proposal("Sense", "Meaning"))
        .await
        .unwrap();
    let longer = termbase
        .propose(proposal("High Sense", "High Talent"))
        .await
        .unwrap();
    termbase.approve_term(first).await.unwrap();
    termbase.approve_term(second).await.unwrap();
    termbase.approve_term(longer).await.unwrap();
    let contract = termbase.contract("High Sense and Sense").await.unwrap();
    assert_eq!(
        contract
            .iter()
            .map(|term| term.crystal_id)
            .collect::<Vec<_>>(),
        [longer]
    );
}

#[tokio::test]
async fn malformed_unknown_and_nonfinite_inputs_are_rejected_without_partial_writes() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool);
    for invalid in [
        proposal("", "ok"),
        proposal("ok", ""),
        proposal("A is translated as B", "C"),
    ] {
        assert!(termbase.propose(invalid).await.is_err());
    }
    assert!(termbase.approve_term(404).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn concurrent_and_cancelled_approval_is_serialized_and_rollback_safe() {
    use std::time::Duration;

    let pool = pool().await;
    let termbase = Termbase::new(&pool);
    let id = termbase.propose(proposal("ユン", "Yun")).await.unwrap();
    let first = termbase.approve_term(id);
    let second = termbase.approve_term(id);
    let (left, right) = tokio::join!(first, second);
    left.unwrap();
    right.unwrap();
    let concepts: i64 = sqlx::query_scalar("SELECT count(*) FROM concepts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(concepts, 1);

    let cancelled = termbase.propose(proposal("センス", "Sense")).await.unwrap();
    let blocker = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let timed_out =
        tokio::time::timeout(Duration::from_millis(30), termbase.approve_term(cancelled)).await;
    assert!(timed_out.is_err());
    blocker.rollback().await.unwrap();
    let partial_links: i64 =
        sqlx::query_scalar("SELECT count(*) FROM crystal_concepts WHERE crystal_id = ?")
            .bind(cancelled)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(partial_links, 0);
    termbase.approve_term(cancelled).await.unwrap();
}

#[tokio::test]
async fn contract_serialization_and_fts_integrity_are_stable() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool);
    let id = termbase.propose(proposal("ガンツ", "Gantz")).await.unwrap();
    termbase.approve_term(id).await.unwrap();
    let contract = termbase.contract("ガンツ").await.unwrap();
    assert_eq!(
        serde_json::to_value(&contract).unwrap(),
        serde_json::json!([{
            "crystal_id": id,
            "source_text": "ガンツ",
            "canonical_translation": "Gantz"
        }])
    );
    sqlx::query("INSERT INTO crystals_fts(crystals_fts, rank) VALUES('integrity-check', 1)")
        .execute(&pool)
        .await
        .unwrap();
}

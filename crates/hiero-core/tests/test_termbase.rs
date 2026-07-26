use hiero_core::{
    db::connect_url,
    domain::{
        AddCrystalInput, ConceptStore, CreateConceptInput, CrystalStore, TermProposal, Termbase,
        TermbaseError, TranslationContext,
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
        notes: String::new(),
    }
}

fn proposal_for(
    series: &str,
    source_language: &str,
    target_language: &str,
    source: &str,
    canonical: &str,
) -> TermProposal {
    TermProposal {
        series_slug: series.into(),
        source_language: source_language.into(),
        target_language: target_language.into(),
        category: "name".into(),
        source_text: source.into(),
        canonical_translation: canonical.into(),
        tags: vec![],
        notes: String::new(),
    }
}

fn context(series: &str, source: &str, target: &str) -> TranslationContext {
    TranslationContext::new(series, source, target)
}

#[tokio::test]
async fn candidate_context_uses_the_persisted_non_default_language_pair() {
    let pool = pool().await;
    let proposal_context = TranslationContext::new("oso", "ko", "de").with_metadata(
        &[],
        &["volume:3".into()],
        &[],
        &[],
    );
    let id = Termbase::new(&pool, proposal_context.clone())
        .propose(TermProposal {
            series_slug: "oso".into(),
            source_language: "ko".into(),
            target_language: "de".into(),
            category: "name".into(),
            source_text: "호로".into(),
            canonical_translation: "Holo".into(),
            tags: vec!["character".into()],
            notes: String::new(),
        })
        .await
        .unwrap();

    let candidate = Termbase::candidate_context(&pool, id).await.unwrap();
    assert_eq!(
        (
            candidate.series_slug.as_str(),
            candidate.source_language.as_str(),
            candidate.target_language.as_str(),
        ),
        ("oso", "ko", "de")
    );
    assert_eq!(candidate.semantic_tags, ["character"]);
}

#[tokio::test]
async fn proposed_terms_are_inert_until_atomic_idempotent_approval() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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

    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
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

#[tokio::test]
async fn context_blocks_cross_series_and_language_leakage_but_allows_global_fallback() {
    let pool = pool().await;
    let oso = Termbase::new(&pool, context("oso", "ja", "en"));
    let other = Termbase::new(&pool, context("other", "ja", "en"));
    let french = Termbase::new(&pool, context("oso", "en", "fr"));
    let oso_id = oso.propose(proposal("共通", "Shared")).await.unwrap();
    let other_id = other
        .propose(proposal_for("other", "ja", "en", "他", "Other"))
        .await
        .unwrap();
    let french_id = french
        .propose(proposal_for("oso", "en", "fr", "English", "Français"))
        .await
        .unwrap();
    oso.approve_term(oso_id).await.unwrap();
    other.approve_term(other_id).await.unwrap();
    french.approve_term(french_id).await.unwrap();

    let concepts = ConceptStore::new(&pool);
    let crystals = CrystalStore::new(&pool);
    let global_concept = concepts
        .create(CreateConceptInput {
            canonical_name: "Global".into(),
            scope_type: "global".into(),
            scope_key: String::new(),
        })
        .await
        .unwrap();
    let global_id = crystals
        .add(AddCrystalInput {
            crystal_type: "lesson".into(),
            text: "Global is translated as Universal.".into(),
            source_credibility: "user_rule".into(),
            rule_intent: "name".into(),
            strength: 0.9,
            confidence: 0.9,
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    concepts
        .link_crystal(global_id, global_concept.id, "defines", 0.95)
        .await
        .unwrap();

    let terms = oso.contract("共通 他 English Global").await.unwrap();
    assert_eq!(
        terms.iter().map(|term| term.crystal_id).collect::<Vec<_>>(),
        [oso_id, global_id]
    );
}

#[tokio::test]
async fn active_reapproval_rejects_archived_or_corrupted_graph_without_replacement() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
    let archived_rule = termbase.propose(proposal("ユン", "Yun")).await.unwrap();
    termbase.approve_term(archived_rule).await.unwrap();
    let archived_concept: i64 = sqlx::query_scalar(
        "SELECT concept_id FROM crystal_concepts WHERE crystal_id = ? AND link_type = 'defines'",
    )
    .bind(archived_rule)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE concepts SET status = 'archived' WHERE id = ?")
        .bind(archived_concept)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        termbase.approve_term(archived_rule).await,
        Err(TermbaseError::Conflict { .. })
    ));
    let concept_count: i64 = sqlx::query_scalar("SELECT count(*) FROM concepts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(concept_count, 1);

    let corrupt_rule = termbase.propose(proposal("センス", "Sense")).await.unwrap();
    termbase.approve_term(corrupt_rule).await.unwrap();
    sqlx::query("DELETE FROM migration_ledger WHERE source_table = 'term_proposals' AND source_id = ? AND target_table = 'concepts'")
        .bind(corrupt_rule.to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        termbase.approve_term(corrupt_rule).await,
        Err(TermbaseError::Conflict { .. })
    ));
    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM crystal_concepts WHERE crystal_id = ? AND link_type = 'defines'",
    )
    .bind(corrupt_rule)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(links, 1);
}

#[tokio::test]
async fn validation_reports_ambiguity_and_same_concept_conflicts_without_enforcement() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool, context("oso", "ja", "en"));
    let talent = termbase.propose(proposal("Sense", "Talent")).await.unwrap();
    let meaning = termbase
        .propose(proposal("Sense", "Meaning"))
        .await
        .unwrap();
    termbase.approve_term(talent).await.unwrap();
    termbase.approve_term(meaning).await.unwrap();
    let ambiguous = termbase
        .validate("unchanged", Some("Sense"), None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&ambiguous).unwrap(),
        serde_json::json!([{
            "crystal_id": talent.min(meaning),
            "kind": "ambiguous_source",
            "severity": "warning",
            "expected": "Meaning, Talent",
            "observed": "Sense",
            "detail": "source form maps to multiple active rule concepts; add context before enforcing a rendering"
        }])
    );

    let concept_id: i64 = sqlx::query_scalar(
        "SELECT concept_id FROM crystal_concepts WHERE crystal_id = ? AND link_type = 'defines'",
    )
    .bind(talent)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM crystal_concepts WHERE crystal_id = ?")
        .bind(meaning)
        .execute(&pool)
        .await
        .unwrap();
    ConceptStore::new(&pool)
        .link_crystal(meaning, concept_id, "defines", 0.95)
        .await
        .unwrap();
    let conflict = termbase
        .validate("unchanged", Some("Sense"), None)
        .await
        .unwrap();
    assert_eq!(conflict.len(), 1);
    assert_eq!(conflict[0].kind, "conflicting_active_rules");
    assert_eq!(conflict[0].expected, "Meaning, Talent");
    assert!(termbase.contract("Sense").await.unwrap().is_empty());
}

#[tokio::test]
async fn story_context_resolves_ambiguity_and_multilingual_facets_respect_direction() {
    let pool = pool().await;
    let base = Termbase::new(&pool, context("oso", "ja", "en"));
    let scoped = base.propose(proposal("Sense", "Talent")).await.unwrap();
    let unscoped = base.propose(proposal("Sense", "Meaning")).await.unwrap();
    base.approve_term(scoped).await.unwrap();
    base.approve_term(unscoped).await.unwrap();
    let scoped_concept: i64 = sqlx::query_scalar(
        "SELECT concept_id FROM crystal_concepts WHERE crystal_id = ? AND link_type = 'defines'",
    )
    .bind(scoped)
    .fetch_one(&pool)
    .await
    .unwrap();
    let source_facet: i64 = sqlx::query_scalar(
        "SELECT id FROM concept_facets WHERE concept_id = ? AND facet_type = 'name' ORDER BY id LIMIT 1",
    )
    .bind(scoped_concept)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO concept_facet_story_scopes(facet_id, story_scope) VALUES (?, 'volume:5')",
    )
    .bind(source_facet)
    .execute(&pool)
    .await
    .unwrap();
    let concepts = ConceptStore::new(&pool);
    concepts
        .add_facet(scoped_concept, "ja", "name", "センス")
        .await
        .unwrap();
    concepts
        .add_facet(scoped_concept, "en", "name", "EnglishAlias")
        .await
        .unwrap();
    concepts
        .add_facet(scoped_concept, "ru", "name", "ThirdAlias")
        .await
        .unwrap();
    concepts
        .add_facet(scoped_concept, "en", "rendering", "TargetOnly")
        .await
        .unwrap();

    let story = vec!["volume:5".to_owned()];
    let contextual = Termbase::new(
        &pool,
        context("oso", "ja", "en").with_metadata(&[], &story, &[], &[]),
    );
    let terms = contextual
        .contract("Sense センス ThirdAlias EnglishAlias TargetOnly")
        .await
        .unwrap();
    assert!(terms.iter().all(|term| term.crystal_id == scoped));
    assert_eq!(
        terms
            .iter()
            .map(|term| term.source_text.as_str())
            .collect::<Vec<_>>(),
        ["Sense", "ThirdAlias", "センス"]
    );

    let other_story = vec!["volume:6".to_owned()];
    let mismatched = Termbase::new(
        &pool,
        context("oso", "ja", "en").with_metadata(&[], &other_story, &[], &[]),
    );
    assert!(
        mismatched
            .contract("センス ThirdAlias")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn unicode_casefold_matches_python_without_normalizing_and_preserves_observed_slices() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool, context("oso", "de", "en"));

    let street = termbase
        .propose(proposal_for("oso", "de", "en", "Straße", "Street"))
        .await
        .unwrap();
    let sigma = termbase
        .propose(proposal_for("oso", "de", "en", "ΟΣ", "Sigma"))
        .await
        .unwrap();
    let dotted = termbase
        .propose(proposal_for("oso", "de", "en", "İ", "Dotted"))
        .await
        .unwrap();
    let dotless = termbase
        .propose(proposal_for("oso", "de", "en", "ı", "Dotless"))
        .await
        .unwrap();
    let composed = termbase
        .propose(proposal_for("oso", "de", "en", "é", "Composed"))
        .await
        .unwrap();
    let expansion = termbase
        .propose(proposal_for("oso", "de", "en", "ss", "DoubleS"))
        .await
        .unwrap();
    for id in [street, sigma, dotted, dotless, composed, expansion] {
        termbase.approve_term(id).await.unwrap();
    }

    let contract = termbase
        .contract("STRASSE ος i\u{307} ı e\u{301} aßb")
        .await
        .unwrap();
    let observed: Vec<&str> = contract
        .iter()
        .map(|term| term.source_text.as_str())
        .collect();
    assert!(observed.contains(&"STRASSE"));
    assert!(observed.contains(&"ος"));
    assert!(observed.contains(&"i\u{307}"));
    assert!(observed.contains(&"ı"));
    assert!(observed.contains(&"ß"));
    assert!(!observed.contains(&"e\u{301}"));
    assert_eq!(
        contract
            .iter()
            .find(|term| term.canonical_translation == "DoubleS")
            .unwrap()
            .source_text,
        "ß"
    );
    assert_eq!(termbase.contract("οσ").await.unwrap()[0].source_text, "οσ");
    assert!(termbase.contract("i").await.unwrap().is_empty());
}

#[tokio::test]
async fn unicode_casefold_drives_ambiguity_and_case_insensitive_validation() {
    let pool = pool().await;
    let termbase = Termbase::new(&pool, context("oso", "de", "en"));
    let first = termbase
        .propose(proposal_for("oso", "de", "en", "Straße", "Street"))
        .await
        .unwrap();
    let second = termbase
        .propose(proposal_for("oso", "de", "en", "STRASSE", "Road"))
        .await
        .unwrap();
    termbase.approve_term(first).await.unwrap();
    termbase.approve_term(second).await.unwrap();
    let findings = termbase
        .validate("unchanged", Some("sTrAsSe"), None)
        .await
        .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "ambiguous_source");
    assert_eq!(findings[0].observed, "sTrAsSe");

    let house = termbase
        .propose(proposal_for("oso", "de", "en", "Haus", "House"))
        .await
        .unwrap();
    termbase.approve_term(house).await.unwrap();
    sqlx::query(
        "UPDATE crystals SET text = 'Haus is translated as House, not Straße.' WHERE id = ?",
    )
    .bind(house)
    .execute(&pool)
    .await
    .unwrap();
    let findings = termbase
        .validate("HOUSE and STRASSE", Some("HAUS"), None)
        .await
        .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "forbidden_variant_used");
    assert_eq!(findings[0].observed, "STRASSE");
}

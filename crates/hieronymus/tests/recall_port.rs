//! Behavior ported from `tests/test_recall.py` and
//! `tests/test_combined_recall.py`: the FTS-lane recall merge with active
//! rule protection, context boosts, activations, and degraded short-term
//! mixing.

#[path = "support/current_story.rs"]
mod current_story;

use std::fs;

use hieronymus::concepts::ConceptStore;
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::rag::{RagError, RagImport, RagStore};
use hieronymus::recall::{RecallError, RecallHit, RecallService};
use hieronymus::registry::Registry;
use hieronymus::workspace::WorkspaceStore;

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    session_id: i64,
}

fn fixture(series_slug: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap();
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series(series_slug, series_slug, "ja", "en", None)
        .unwrap();
    current_story::register(&config, series_slug);
    let workspace = WorkspaceStore::open(&config).unwrap();
    let context = current_story::context(series_slug, "ja", "en", "translation");
    let session = workspace.start_session(&context).unwrap();
    Fixture {
        root,
        config,
        session_id: session.id,
    }
}

#[test]
fn recall_returns_crystal_and_short_term_hits() {
    let fixture = fixture("demo");
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    crystals
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(
                &fixture.config,
                "lesson",
                "The binding ritual requires chalk.",
            ),
        )
        .unwrap();
    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(
                &fixture.config,
                "note",
                "Binding chalk is mentioned in chapter notes.",
            ),
        )
        .unwrap();

    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "chalk binding", 10)
        .unwrap()
        .hits;

    assert!(!hits.is_empty());
    let sources: Vec<&str> = hits.iter().map(RecallHit::source).collect();
    assert!(sources.contains(&"long_term"));
    assert!(sources.contains(&"short_term"));
    // Long-term outranks the short-term base score when text matches strongly.
    assert_eq!(hits.iter().map(RecallHit::source).next(), Some("long_term"));
}

#[test]
fn recall_respects_limit_and_active_rule_protection() {
    let fixture = fixture("demo");
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let _active_rule = crystals
        .add_crystal(
            &context,
            "rule",
            &current_story::crystal(&fixture.config, "rule", "Always localize 魔法 as magic.")
                .confidence(0.9),
        )
        .unwrap();
    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(&fixture.config, "note", "magic note one"),
        )
        .unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(&fixture.config, "note", "magic note two"),
        )
        .unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(&fixture.config, "note", "magic note three"),
        )
        .unwrap();

    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "magic", 2)
        .unwrap()
        .hits;

    assert_eq!(hits.len(), 2);
    // The protected active rule occupies a slot ahead of pooled memories.
    assert!(hits.iter().any(|hit| match hit {
        RecallHit::LongTerm { crystal, .. } => {
            crystal.crystal_type == "rule" && crystal.status == "active"
        }
        _ => false,
    }));
}

#[test]
fn recall_records_activations_for_long_term_hits() {
    let fixture = fixture("demo");
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let crystal_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(
                &fixture.config,
                "lesson",
                "Activations are recorded on recall.",
            ),
        )
        .unwrap();

    let service = RecallService::open(&fixture.config).unwrap();
    service
        .recall(fixture.session_id, &context, "activations recorded", 10)
        .unwrap();

    let connection = rusqlite::Connection::open_with_flags(
        fixture.config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (count, query): (i64, String) = connection
        .query_row(
            "select count(*), max(recall_query) from crystal_activations
             where crystal_id = ?1",
            [crystal_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(query, "activations recorded");
}

#[test]
fn concept_match_boosts_linked_crystal_in_ranking() {
    let fixture = fixture("demo");
    let concepts = ConceptStore::open(&fixture.config).unwrap();
    let concept = concepts
        .create_concept("enchant", &Default::default())
        .unwrap();
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    // Identical texts produce identical FTS lane scores, so without the
    // concept boost the earlier crystal would win the id tie-break.
    let unlinked_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(&fixture.config, "lesson", "The enchant ritual needs chalk."),
        )
        .unwrap();
    let boosted_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal {
                concept_ids: vec![concept.id],
                ..current_story::crystal(
                    &fixture.config,
                    "lesson",
                    "The enchant ritual needs chalk.",
                )
            },
        )
        .unwrap();

    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "enchant", 10)
        .unwrap()
        .hits;
    let long_term_scores: Vec<(i64, f64)> = hits
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::LongTerm { crystal, score, .. } => Some((crystal.id, *score)),
            _ => None,
        })
        .collect();
    let score_of = |crystal_id: i64| {
        long_term_scores
            .iter()
            .find(|(id, _)| *id == crystal_id)
            .map(|(_, score)| *score)
    };

    let boosted_score = score_of(boosted_id).expect("boosted crystal recalled");
    let unlinked_score = score_of(unlinked_id).expect("unlinked crystal recalled");
    assert!(
        boosted_score > unlinked_score,
        "expected the concept-linked crystal to outrank its twin"
    );
    assert_close(boosted_score - unlinked_score, 0.15);
}

fn assert_close(value: f64, expected: f64) {
    assert!(
        (value - expected).abs() < 1e-9,
        "expected {expected}, got {value}"
    );
}

#[test]
fn recall_rejects_inactive_and_unknown_sessions() {
    let fixture = fixture("demo");
    current_story::capture_chunks(&fixture.config, "demo");
    let context = current_story::context("demo", "ja", "en", "translation");
    let service = RecallService::open(&fixture.config).unwrap();

    let error = service
        .recall(999999, &context, "anything", 10)
        .unwrap_err();
    assert!(matches!(error, RecallError::UnknownSession(999999)));

    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    let session = workspace
        .start_session(&current_story::context("demo", "ja", "en", "translation"))
        .unwrap();
    workspace.complete_session(session.id).unwrap();
    let error = service
        .recall(session.id, &context, "anything", 10)
        .unwrap_err();
    assert!(matches!(error, RecallError::InactiveSession));
}

#[test]
fn recall_data_root_stays_clean() {
    let fixture = fixture("demo");
    let _service = RecallService::open(&fixture.config).unwrap();
    let sqlite_files: Vec<_> = fs::read_dir(fixture.config.data_root())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sqlite"))
        .collect();
    assert_eq!(sqlite_files, vec!["hieronymus.sqlite".to_string()]);
}

#[test]
fn add_crystal_hydrates_concept_links() {
    let fixture = fixture("demo");
    let concepts = ConceptStore::open(&fixture.config).unwrap();
    let concept = concepts
        .create_concept("Linked Concept", &Default::default())
        .unwrap();
    let crystals = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let crystal_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal {
                concept_ids: vec![concept.id],
                ..current_story::crystal(&fixture.config, "lesson", "Concept-linked crystal.")
            },
        )
        .unwrap();

    let crystal = crystals.get(crystal_id).unwrap();
    assert_eq!(crystal.concept_ids, vec![concept.id]);
}

#[test]
fn recall_joins_rag_hits_into_merged_recall() {
    let fixture = fixture("demo");
    let rag_path = fixture.root.path().join("chapter.txt");
    fs::write(&rag_path, "Cooking Talent appears here.").unwrap();
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("demo", &rag_path, &RagImport::new())
        .unwrap();

    current_story::capture_chunks(&fixture.config, "demo");
    let context = current_story::context("demo", "ja", "en", "translation");
    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "Cooking Talent", 10)
        .unwrap()
        .hits;

    assert!(
        hits.iter()
            .any(|hit| matches!(hit, RecallHit::Rag { chunk, .. } if chunk.text == "Cooking Talent appears here."))
    );
    assert!(hits.len() <= 10);
}

#[test]
fn recall_keeps_rag_hits_series_isolated() {
    let fixture = fixture("demo");
    let registry = Registry::open(&fixture.config).unwrap();
    registry
        .create_series("other", "other", "ja", "en", None)
        .unwrap();
    let rag_path = fixture.root.path().join("chapter.txt");
    fs::write(&rag_path, "Cooking Talent appears here.").unwrap();
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("other", &rag_path, &RagImport::new())
        .unwrap();

    current_story::capture_chunks(&fixture.config, "demo");
    let context = current_story::context("demo", "ja", "en", "translation");
    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "Cooking Talent", 10)
        .unwrap()
        .hits;

    assert!(hits.iter().all(|hit| !matches!(hit, RecallHit::Rag { .. })));
}

#[test]
fn failed_conflicting_reimport_preserves_rag_recall() {
    let fixture = fixture("demo");
    let glossary_path = fixture.root.path().join("glossary.json");
    fs::write(&glossary_path, r#"[{"source": "Sense", "target": "Сенс"}]"#).unwrap();
    let rag = RagStore::open(&fixture.config).unwrap();
    rag.import_file("demo", &glossary_path, &RagImport::new())
        .unwrap();

    fs::write(&glossary_path, "{broken json").unwrap();
    let error = rag
        .import_file("demo", &glossary_path, &RagImport::new())
        .unwrap_err();
    assert!(matches!(error, RagError::InvalidSource(_)));

    current_story::capture_chunks(&fixture.config, "demo");
    let context = current_story::context("demo", "ja", "en", "translation");
    let service = RecallService::open(&fixture.config).unwrap();
    let hits = service
        .recall(fixture.session_id, &context, "Sense", 10)
        .unwrap()
        .hits;

    assert!(
        hits.iter()
            .any(|hit| matches!(hit, RecallHit::Rag { chunk, .. }
        if chunk.metadata.get("source") == Some(&serde_json::Value::String("Sense".to_string()))))
    );
}

fn invalidate_crystal(fixture: &Fixture, id: i64) {
    let db = hieronymus::db::open_migrated(&fixture.config.database_path()).unwrap();
    db.execute("update memory_claims set status='invalid',revision=revision+1 where id in(select claim_id from claim_bindings where crystal_id=?)",[id]).unwrap();
}

#[test]
fn invalid_fts_and_metadata_candidates_refill_without_assertion_leaks() {
    for metadata in [false, true] {
        let fixture = fixture("demo");
        let store = CrystalStore::open(&fixture.config).unwrap();
        let mut context = current_story::context("demo", "ja", "en", "translation");
        if metadata {
            context.semantic_tags = vec!["garden".into()];
        }
        let mut good =
            current_story::crystal(&fixture.config, "lesson", "secret valid garden memory");
        good.semantic_tags = vec!["garden".into()];
        let good_id = store.add_crystal(&context, "lesson", &good).unwrap();
        for _ in 0..60 {
            let mut bad =
                current_story::crystal(&fixture.config, "lesson", "secret obsolete assertion")
                    .strength(1.0);
            bad.semantic_tags = vec!["garden".into()];
            let id = store.add_crystal(&context, "lesson", &bad).unwrap();
            invalidate_crystal(&fixture, id);
        }
        let response = RecallService::open(&fixture.config)
            .unwrap()
            .recall(
                fixture.session_id,
                &context,
                if metadata { "unmatched" } else { "secret" },
                1,
            )
            .unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].item_id(), good_id);
        assert!(!response.candidate_exhausted);
        assert!(response.non_current.iter().all(|h| match h {
            RecallHit::LongTerm { crystal, .. } =>
                crystal.text.is_empty() && crystal.title.is_empty(),
            _ => false,
        }));
    }
}

#[test]
fn invalid_trigger_cannot_spread_and_valid_trigger_refills_invalid_neighbors() {
    let fixture = fixture("demo");
    let store = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let trigger = store
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(&fixture.config, "lesson", "secret trigger")
                .strength(1.0)
                .confidence(1.0),
        )
        .unwrap();
    let good = store
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(&fixture.config, "lesson", "valid neighboring memory"),
        )
        .unwrap();
    let db = hieronymus::db::open_migrated(&fixture.config.database_path()).unwrap();
    for _ in 0..30 {
        let bad = store
            .add_crystal(
                &context,
                "lesson",
                &current_story::crystal(&fixture.config, "lesson", "obsolete neighbor"),
            )
            .unwrap();
        invalidate_crystal(&fixture, bad);
        db.execute(
            "insert into crystal_links values(?1,?2,'related',1)",
            rusqlite::params![trigger, bad],
        )
        .unwrap();
    }
    db.execute(
        "insert into crystal_links values(?1,?2,'related',0.8)",
        rusqlite::params![trigger, good],
    )
    .unwrap();
    let service = RecallService::open(&fixture.config).unwrap();
    let response = service.recall_context(&context, "secret", 2).unwrap();
    assert!(response.hits.iter().any(|h| h.item_id() == good));
    invalidate_crystal(&fixture, trigger);
    let response = service.recall_context(&context, "secret", 2).unwrap();
    assert!(
        response.hits.is_empty(),
        "invalid trigger must not give its neighbor a rank"
    );
}

#[test]
fn metadata_lane_filters_tags_before_its_result_limit() {
    let fixture = fixture("demo");
    let store = CrystalStore::open(&fixture.config).unwrap();
    let mut context = current_story::context("demo", "ja", "en", "translation");
    context.semantic_tags = vec!["garden".into()];
    let mut wanted = current_story::crystal(&fixture.config, "lesson", "garden memory");
    wanted.semantic_tags = vec!["garden".into()];
    let wanted = store.add_crystal(&context, "lesson", &wanted).unwrap();
    for _ in 0..205 {
        store
            .add_crystal(
                &context,
                "lesson",
                &current_story::crystal(&fixture.config, "lesson", "unrelated memory"),
            )
            .unwrap();
    }
    let response = RecallService::open(&fixture.config)
        .unwrap()
        .recall_context(&context, "unmatched", 1)
        .unwrap();
    assert_eq!(
        response
            .hits
            .iter()
            .map(RecallHit::item_id)
            .collect::<Vec<_>>(),
        vec![wanted]
    );
}

#[test]
fn invalid_facets_do_not_boost_and_valid_facet_remains_eligible() {
    let fixture = fixture("demo");
    let store = CrystalStore::open(&fixture.config).unwrap();
    let concepts = ConceptStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let mut ids = vec![];
    let db = hieronymus::db::open_migrated(&fixture.config.database_path()).unwrap();
    for (name, invalid) in [("Old identity", true), ("Valid identity", false)] {
        let concept = concepts
            .create_concept(
                name,
                &hieronymus::concepts::NewConcept {
                    scope_type: "series".into(),
                    scope_key: "series:demo".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut claim = current_story::claim(&fixture.config, "demo", "secret alias");
        claim.concept_id = Some(concept.id);
        let facet = concepts
            .add_facet_with_claims(
                concept.id,
                "secret alias",
                &Default::default(),
                0.8,
                false,
                &[claim],
            )
            .unwrap();
        let id = store
            .add_crystal(
                &context,
                "lesson",
                &current_story::crystal(&fixture.config, "lesson", "secret alias memory"),
            )
            .unwrap();
        concepts
            .link_crystal(id, concept.id, "evidence", 0.8)
            .unwrap();
        if invalid {
            db.execute("update memory_claims set status='invalid' where id in(select claim_id from claim_bindings where facet_id=?)",[facet.id]).unwrap();
        }
        ids.push(id);
    }
    let response = RecallService::open(&fixture.config)
        .unwrap()
        .recall_context(&context, "secret alias", 2)
        .unwrap();
    assert_eq!(
        response.hits[0].item_id(),
        ids[1],
        "only the eligible facet contributes its boost"
    );
    assert_eq!(response.hits.len(), 2);
}

#[test]
fn qualified_recall_keeps_working_copy_bytes_and_claim_lineage() {
    let fixture = fixture("demo");
    let store = CrystalStore::open(&fixture.config).unwrap();
    let context = current_story::context("demo", "ja", "en", "translation");
    let id = store
        .add_crystal(
            &context,
            "lesson",
            &current_story::crystal(&fixture.config, "lesson", "secret assertion"),
        )
        .unwrap();
    let db = hieronymus::db::open_migrated(&fixture.config.database_path()).unwrap();
    db.execute("update memory_claims set status='qualified',qualification='Only suspected' where id in(select claim_id from claim_bindings where crystal_id=?)",[id]).unwrap();
    let service = RecallService::open(&fixture.config).unwrap();
    service
        .recall(fixture.session_id, &context, "secret", 1)
        .unwrap();
    let copied: String = db
        .query_row(
            "select text from short_term_memories where source_crystal_id=?",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(copied, "secret assertion");
    assert_eq!(
        db.query_row(
            "select count(distinct claim_id) from claim_bindings",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    service
        .recall(fixture.session_id, &context, "secret", 1)
        .unwrap();
    assert_eq!(
        db.query_row("select count(*) from short_term_memories", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from crystal_activations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn sessionless_short_term_refills_invalid_and_unknown_candidates() {
    let fixture = fixture("demo");
    let context = current_story::context("demo", "ja", "en", "translation");
    let store = WorkspaceStore::open(&fixture.config).unwrap();
    let bad = store
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(&fixture.config, "note", "secret obsolete"),
        )
        .unwrap();
    let good = store
        .add_short_term_memory(
            fixture.session_id,
            &current_story::memory(&fixture.config, "note", "secret valid retained"),
        )
        .unwrap();
    store
        .add_short_term_memory(
            fixture.session_id,
            &hieronymus::workspace::ShortTermMemoryInput::new("note", "secret unknown"),
        )
        .unwrap();
    let db = hieronymus::db::open_migrated(&fixture.config.database_path()).unwrap();
    db.execute("update memory_claims set status='invalid' where id in(select claim_id from claim_bindings where short_term_id=?)",[bad.id]).unwrap();
    let response = RecallService::open(&fixture.config)
        .unwrap()
        .recall_context(&context, "secret", 1)
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].item_id(), good.id);
    assert_eq!(response.hits[0].source(), "short_term");
    assert!(
        response
            .non_current
            .iter()
            .all(|hit| matches!(hit,RecallHit::ShortTerm {memory,..} if memory.text.is_empty()))
    );
    assert_eq!(
        db.query_row("select count(*) from crystal_activations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

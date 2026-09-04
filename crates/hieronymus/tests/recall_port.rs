//! Behavior ported from `tests/test_recall.py` and
//! `tests/test_combined_recall.py`: the FTS-lane recall merge with active
//! rule protection, context boosts, activations, and degraded short-term
//! mixing.

use std::fs;

use hieronymus::concepts::ConceptStore;
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::rag::{RagError, RagImport, RagStore};
use hieronymus::recall::{RecallError, RecallHit, RecallService};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

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
    let workspace = WorkspaceStore::open(&config).unwrap();
    let context = TranslationContext::new(series_slug, "ja", "en", "translation");
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal::new("lesson", "The binding ritual requires chalk."),
        )
        .unwrap();
    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &ShortTermMemoryInput::new("note", "Binding chalk is mentioned in chapter notes."),
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let _active_rule = crystals
        .add_crystal(
            &context,
            "rule",
            &NewCrystal::new("rule", "Always localize 魔法 as magic.").confidence(0.9),
        )
        .unwrap();
    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &ShortTermMemoryInput::new("note", "magic note one"),
        )
        .unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &ShortTermMemoryInput::new("note", "magic note two"),
        )
        .unwrap();
    workspace
        .add_short_term_memory(
            fixture.session_id,
            &ShortTermMemoryInput::new("note", "magic note three"),
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let crystal_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal::new("lesson", "Activations are recorded on recall."),
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    // Identical texts produce identical FTS lane scores, so without the
    // concept boost the earlier crystal would win the id tie-break.
    let unlinked_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal::new("lesson", "The enchant ritual needs chalk."),
        )
        .unwrap();
    let boosted_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal {
                concept_ids: vec![concept.id],
                ..NewCrystal::new("lesson", "The enchant ritual needs chalk.")
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let service = RecallService::open(&fixture.config).unwrap();

    let error = service
        .recall(999999, &context, "anything", 10)
        .unwrap_err();
    assert!(matches!(error, RecallError::UnknownSession(999999)));

    let workspace = WorkspaceStore::open(&fixture.config).unwrap();
    let session = workspace
        .start_session(&TranslationContext::new("demo", "ja", "en", "translation"))
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
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let crystal_id = crystals
        .add_crystal(
            &context,
            "lesson",
            &NewCrystal {
                concept_ids: vec![concept.id],
                ..NewCrystal::new("lesson", "Concept-linked crystal.")
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

    let context = TranslationContext::new("demo", "ja", "en", "translation");
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

    let context = TranslationContext::new("demo", "ja", "en", "translation");
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

    let context = TranslationContext::new("demo", "ja", "en", "translation");
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

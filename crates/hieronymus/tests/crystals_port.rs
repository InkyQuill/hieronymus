//! Behavior ported from `tests/test_crystals.py`: crystal insertion with
//! clamped scores, side-table hydration, series/language-bounded weighted
//! search, low-confidence maintenance ordering, and supersede rules.

use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;

fn context(series_slug: &str) -> TranslationContext {
    TranslationContext::new(series_slug, "ja", "en", "translation")
}

fn open_store(root: &tempfile::TempDir) -> CrystalStore {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    CrystalStore::open(&config).unwrap()
}

#[test]
fn add_and_search_series_crystal() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let crystal_id = store
        .add_crystal(
            &context("only-sense-online"),
            "lesson",
            &NewCrystal::new("lesson", "Sense means ability in this story."),
        )
        .unwrap();

    let hits = store
        .search(&context("only-sense-online"), "Sense ability", 10)
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, crystal_id);
    assert_eq!(hits[0].series_slug, "only-sense-online");
}

#[test]
fn scores_are_clamped_when_adding_crystal() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let crystal_id = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Clamped.")
                .strength(42.0)
                .confidence(-3.0),
        )
        .unwrap();

    let crystal = store.get(crystal_id).unwrap();
    assert_eq!(crystal.strength, 1.0);
    assert_eq!(crystal.confidence, 0.0);
}

#[test]
fn crystal_scalar_metadata_defaults_hydrate_from_store() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let crystal_id = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Defaults."),
        )
        .unwrap();

    let crystal = store.get(crystal_id).unwrap();
    assert_eq!(crystal.source_credibility, "observation");
    assert_eq!(crystal.rule_intent, "");
    assert_eq!(crystal.malformed_penalty, 0.0);
    assert!(crystal.supersedes_crystal_id.is_none());
    assert!(!crystal.is_inferred);
    assert!(crystal.soft_origin.is_empty());
    assert!(crystal.concept_ids.is_empty());
}

#[test]
fn add_crystal_stores_and_hydrates_side_table_fields() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let crystal_id = store
        .add_crystal(
            &context("demo"),
            "observation",
            &NewCrystal {
                language_tags: vec![" ja ".into(), "en".into()],
                story_scopes: vec!["chapter:2".into(), "volume:1".into()],
                semantic_tags: vec!["term".into(), "ui".into()],
                soft_origin: "inline-correction".into(),
                ..NewCrystal::new("observation", "Side tables hold typed metadata.")
            },
        )
        .unwrap();

    let crystal = store.get(crystal_id).unwrap();
    assert_eq!(crystal.language_tags, vec!["en", "ja"]);
    assert_eq!(crystal.story_scopes, vec!["chapter:2", "volume:1"]);
    assert_eq!(crystal.semantic_tags, vec!["term", "ui"]);
    assert_eq!(crystal.soft_origin, "inline-correction");
}

#[test]
fn add_crystal_falls_back_to_context_tags_for_legacy_tags_json() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let context = context("demo");
    let mut context = context;
    context.tags = vec!["legacy".into(), "tags".into()];
    let crystal_id = store
        .add_crystal(
            &context,
            "observation",
            &NewCrystal::new("observation", "Legacy tags."),
        )
        .unwrap();

    // The legacy tags_json column mirrors the context tags when no explicit
    // semantic tags are provided.
    let connection = rusqlite::Connection::open_with_flags(
        store.config().database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let tags_json: String = connection
        .query_row(
            "select tags_json from crystals where id = ?1",
            [crystal_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tags_json, r#"["legacy","tags"]"#);
}

#[test]
fn search_excludes_other_series_crystals() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    store
        .add_crystal(
            &context("other-series"),
            "lesson",
            &NewCrystal::new("lesson", "Needle text about binding."),
        )
        .unwrap();

    let hits = store
        .search(&context("demo"), "Needle binding", 10)
        .unwrap();
    assert!(hits.is_empty());
}

#[test]
fn search_includes_global_crystals() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let mut global_context = context("demo");
    global_context.series_slug = String::new();
    // Global scope requires an empty key; emulate via a global-scoped row.
    let crystal_id = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Scoped row."),
        )
        .unwrap();
    {
        let connection = rusqlite::Connection::open(store.config().database_path()).unwrap();
        connection
            .execute(
                "update crystals set scope_type = 'global', scope_key = '' where id = ?1",
                [crystal_id],
            )
            .unwrap();
    }

    let hits = store.search(&global_context, "Scoped row", 10).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn search_prefers_higher_strength_when_text_relevance_matches() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let weak = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Binding terminology repeats.").strength(0.1),
        )
        .unwrap();
    let strong = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Binding terminology repeats again.").strength(0.9),
        )
        .unwrap();

    let hits = store
        .search(&context("demo"), "Binding terminology", 10)
        .unwrap();
    let ids: Vec<i64> = hits.iter().map(|crystal| crystal.id).collect();
    assert!(ids.contains(&weak));
    assert_eq!(ids[0], strong);
}

#[test]
fn search_scored_exposes_weighted_scores_in_order() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Scored crystal one.").confidence(0.9),
        )
        .unwrap();
    store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Scored crystal two.").confidence(0.1),
        )
        .unwrap();

    let scored = store
        .search_scored(&context("demo"), "Scored crystal", 10)
        .unwrap();

    assert_eq!(scored.len(), 2);
    assert!(scored[0].1 >= scored[1].1);
}

#[test]
fn low_confidence_first_orders_candidates_and_excludes_active_rules() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let low = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Low.").confidence(0.1),
        )
        .unwrap();
    let high = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "High.").confidence(0.9),
        )
        .unwrap();
    let active_rule = store
        .add_crystal(
            &context("demo"),
            "rule",
            &NewCrystal::new("rule", "Active rule.").confidence(0.95),
        )
        .unwrap();

    let ordered = store
        .low_confidence_first(&[high, low, active_rule], 5)
        .unwrap();

    assert_eq!(ordered, vec![low, high]);
}

#[test]
fn invalid_type_status_and_empty_text_raise_value_error() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let error = store
        .add_crystal(
            &context("demo"),
            "made-up",
            &NewCrystal::new("made-up", "x"),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "unknown crystal_type: made-up");

    let error = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "x").with_type_and_status("bogus"),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "unknown status: bogus");

    let error = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "   "),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "text must not be empty");
}

#[test]
fn get_unknown_crystal_reports_identifier() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let error = store.get(654321).unwrap_err();

    assert_eq!(error.to_string(), "unknown crystal: 654321");
}

#[test]
fn supersede_marks_old_superseded_and_validates_shape() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let old = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "Old lesson."),
        )
        .unwrap();
    let new = store
        .add_crystal(
            &context("demo"),
            "lesson",
            &NewCrystal::new("lesson", "New lesson."),
        )
        .unwrap();

    store.supersede(old, new, "Replaced lesson.", 3).unwrap();

    assert_eq!(store.get(old).unwrap().status, "superseded");
    assert_eq!(store.get(new).unwrap().supersedes_crystal_id, Some(old));

    let other_series = store
        .add_crystal(
            &context("other-series"),
            "lesson",
            &NewCrystal::new("lesson", "Different shape."),
        )
        .unwrap();
    let error = store
        .supersede(new, other_series, "Mismatched series.", 4)
        .unwrap_err();
    assert!(error.to_string().contains("does not match"), "{error}");

    let error = store.supersede(new, new, "Self.", 5).unwrap_err();
    assert_eq!(error.to_string(), "crystal cannot supersede itself");
}

#[test]
fn supersede_rejects_active_rule_crystals_in_either_direction() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    // NewCrystal::new defaults to status "active", so this is the ADR 0011
    // protected shape: an active rule crystal.
    let rule = store
        .add_crystal(
            &context("demo"),
            "rule",
            &NewCrystal::new("rule", "Always translate X as Y."),
        )
        .unwrap();
    let replacement = store
        .add_crystal(
            &context("demo"),
            "rule",
            &NewCrystal::new("rule", "Newer rendering.").with_type_and_status("candidate"),
        )
        .unwrap();

    let error = store
        .supersede(rule, replacement, "Dream replace.", 1)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("crystal {rule} is an active rule and cannot be superseded here (ADR 0011)")
    );

    let error = store
        .supersede(replacement, rule, "Dream replace.", 2)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("crystal {rule} is an active rule and cannot be superseded here (ADR 0011)")
    );

    // The primitive refused before any row mutated.
    assert_eq!(store.get(rule).unwrap().status, "active");
    assert_eq!(store.get(replacement).unwrap().status, "candidate");
    assert!(
        store
            .get(replacement)
            .unwrap()
            .supersedes_crystal_id
            .is_none()
    );
}

// Small helper used by the status test above; kept local to the port file.
trait WithTypeAndStatus {
    fn with_type_and_status(self, status: &str) -> Self;
}

impl WithTypeAndStatus for NewCrystal {
    fn with_type_and_status(mut self, status: &str) -> Self {
        self.status = status.to_string();
        self
    }
}

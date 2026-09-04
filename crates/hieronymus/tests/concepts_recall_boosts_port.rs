//! Behavior ported from `ConceptStore.recall_boosts_for_crystals`
//! (`src/hieronymus/concepts.py:992`): recall-time ranking boosts for
//! crystals whose linked concepts match the query. The function has no
//! direct Python test file, so expected values are derived from the Python
//! implementation (`_CONCEPT_RECALL_TEXT_BOOST = 0.15`,
//! `_CONCEPT_RECALL_STORY_SCOPE_BOOST = 0.25`).

use hieronymus::concepts::{ConceptStore, FacetFields, NewConcept};
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::registry::Registry;

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    store: ConceptStore,
    crystals: CrystalStore,
    context: TranslationContext,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config)
        .unwrap()
        .create_series("demo", "demo", "ja", "en", None)
        .unwrap();
    let store = ConceptStore::open(&config).unwrap();
    let crystals = CrystalStore::open(&config).unwrap();
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    Fixture {
        root,
        config,
        store,
        crystals,
        context,
    }
}

fn crystal(fixture: &Fixture) -> i64 {
    fixture
        .crystals
        .add_crystal(
            &fixture.context,
            "lesson",
            &NewCrystal::new("lesson", "Linked memory prose."),
        )
        .unwrap()
}

fn concept(store: &ConceptStore, name: &str) -> i64 {
    store
        .create_concept(name, &NewConcept::default())
        .unwrap()
        .id
}

fn assert_close(value: f64, expected: f64) {
    assert!(
        (value - expected).abs() < 1e-9,
        "expected {expected}, got {value}"
    );
}

#[test]
fn empty_crystal_ids_or_blank_query_return_no_boosts() {
    let fixture = fixture();
    assert!(
        fixture
            .store
            .recall_boosts_for_crystals(&[], "enchant", &[])
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .store
            .recall_boosts_for_crystals(&[1], "   ", &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn canonical_name_match_boosts_linked_crystal_case_insensitively() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchant", &[])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn facet_value_match_boosts_linked_crystal() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant Rite");
    fixture
        .store
        .add_facet(concept_id, "Enchant", &Default::default(), 0.8, false)
        .unwrap();
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "ENCHANT", &[])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn fts_facet_match_boosts_without_exact_value_equality() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant Rite");
    fixture
        .store
        .add_facet(
            concept_id,
            "the enchantus ritual",
            &Default::default(),
            0.8,
            false,
        )
        .unwrap();
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchantus ritual", &[])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn duplicate_input_ids_are_deduplicated_before_boosting() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id, crystal_id], "enchant", &[])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn crystals_without_positive_boosts_are_filtered_from_the_map() {
    let fixture = fixture();
    let linked_id = crystal(&fixture);
    let unlinked_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .link_crystal(linked_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[linked_id, unlinked_id, 9_999], "enchant", &[])
        .unwrap();
    assert_eq!(boosts.len(), 1);
    assert_eq!(boosts.get(&linked_id), Some(&0.15));
}

#[test]
fn story_scoped_facet_accumulates_with_the_text_boost() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .add_facet(
            concept_id,
            "Enchant",
            &FacetFields {
                story_scopes: vec!["chapter:3".to_string()],
                ..Default::default()
            },
            0.8,
            false,
        )
        .unwrap();
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchant", &["chapter:3".to_string()])
        .unwrap();
    assert_close(*boosts.get(&crystal_id).unwrap(), 0.40);
}

#[test]
fn story_scope_join_requires_a_matching_context_scope() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .add_facet(
            concept_id,
            "Enchant",
            &FacetFields {
                story_scopes: vec!["chapter:3".to_string()],
                ..Default::default()
            },
            0.8,
            false,
        )
        .unwrap();
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchant", &["chapter:9".to_string()])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn story_scopes_without_any_matching_facet_keep_the_text_boost() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchant", &["chapter:3".to_string()])
        .unwrap();
    assert_eq!(boosts.get(&crystal_id), Some(&0.15));
}

#[test]
fn superseded_facets_do_not_boost() {
    let fixture = fixture();
    let crystal_id = crystal(&fixture);
    let concept_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .add_facet(concept_id, "Enchantus", &Default::default(), 0.8, false)
        .unwrap();
    fixture
        .store
        .link_crystal(crystal_id, concept_id, "evidence", 0.8)
        .unwrap();
    // Merging is the only public supersede path and would move the crystal
    // link with it, so supersede the facet directly to isolate the filter.
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update concept_facets
             set superseded_at = '2026-01-01T00:00:00Z'
             where value = 'Enchantus'",
            [],
        )
        .unwrap();

    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[crystal_id], "enchantus", &[])
        .unwrap();
    assert!(boosts.is_empty());
}

#[test]
fn archived_and_merged_concepts_do_not_boost() {
    let fixture = fixture();
    let archived_crystal = crystal(&fixture);
    let archived_id = concept(&fixture.store, "Enchant");
    fixture
        .store
        .link_crystal(archived_crystal, archived_id, "evidence", 0.8)
        .unwrap();
    fixture
        .store
        .archive_concept(archived_id, "obsolete")
        .unwrap();
    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[archived_crystal], "enchant", &[])
        .unwrap();
    assert!(boosts.is_empty());

    // A merged-away concept is excluded the same way (merge_concepts moves
    // the link to the target, so exercise the status filter through a raw
    // status flip like the Python `not in (archived, merged)` guard).
    let merged_crystal = crystal(&fixture);
    let merged_id = concept(&fixture.store, "Charm");
    fixture
        .store
        .link_crystal(merged_crystal, merged_id, "evidence", 0.8)
        .unwrap();
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update concepts set status = 'merged' where id = ?1",
            [merged_id],
        )
        .unwrap();
    let boosts = fixture
        .store
        .recall_boosts_for_crystals(&[merged_crystal], "charm", &[])
        .unwrap();
    assert!(boosts.is_empty());
}

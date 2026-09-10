//! Behavior ported from `tests/test_concept_lifecycle.py`: rename, merge, and
//! crystal-linking lifecycle rules.

use std::fs;

use hieronymus::concepts::ConceptError;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::registry::Registry;

fn open_store(root: &tempfile::TempDir) -> hieronymus::concepts::ConceptStore {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    hieronymus::concepts::ConceptStore::open(&config).unwrap()
}

fn open_registry(root: &tempfile::TempDir) -> Registry {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap()
}

/// Insert a minimal crystal row the way the Python lifecycle helper does.
fn insert_crystal(config: &HieronymusConfig, text: &str) -> i64 {
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute(
            "insert into crystals(
               crystal_type, text, scope_type, strength, confidence, status,
               created_at, updated_at
             )
             values ('lesson', ?1, 'global', 0.5, 0.5, 'active', '2026-06-10', '2026-06-10')",
            rusqlite::params![text],
        )
        .unwrap();
    connection.last_insert_rowid()
}

#[test]
fn rename_keeps_old_label_searchable_as_single_facet() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store.create_concept("Yun", &Default::default()).unwrap();
    store
        .add_facet(
            concept.id,
            "Yun",
            &hieronymus::concepts::FacetFields {
                facet_type: Some("alias".into()),
                ..Default::default()
            },
            0.2,
            false,
        )
        .unwrap();

    let renamed = store
        .rename_concept(concept.id, "Yun Talent", None)
        .unwrap();
    let facets = store.list_facets(concept.id).unwrap();

    assert_eq!(renamed.canonical_name, "Yun Talent");
    assert_eq!(
        facets.iter().filter(|facet| facet.value == "Yun").count(),
        1,
        "the pre-existing alias must not duplicate the former_label facet"
    );
}

#[test]
fn rename_old_label_is_searchable_through_store_api() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store.create_concept("Yun", &Default::default()).unwrap();

    let renamed = store
        .rename_concept(concept.id, "Yun Talent", None)
        .unwrap();

    assert_eq!(store.search("Yun", None, &[]).unwrap(), vec![renamed]);
}

#[test]
fn rename_rejects_empty_label_and_same_label_noop() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store.create_concept("Yun", &Default::default()).unwrap();

    let error = store.rename_concept(concept.id, "   ", None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "concept canonical_name must not be empty"
    );

    let same = store.rename_concept(concept.id, "Yun", None).unwrap();
    assert_eq!(same.canonical_name, "Yun");
}

#[test]
fn merge_moves_facets_links_and_tags_to_target() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let store = open_store(&root);
    let source = store.create_concept("Yun", &Default::default()).unwrap();
    let target = store
        .create_concept("Yun Talent", &Default::default())
        .unwrap();
    store
        .add_facet(source.id, "Yun variant", &Default::default(), 0.2, false)
        .unwrap();
    store
        .set_semantic_tags(source.id, &["talent".into()])
        .unwrap();
    let crystal_id = insert_crystal(&config, "Yun grows as a talent.");
    store
        .link_crystal(crystal_id, source.id, "mentions", 0.8)
        .unwrap();

    store
        .merge_concepts(source.id, target.id, "Consolidated.")
        .unwrap();

    let merged = store.get(source.id).unwrap();
    assert_eq!(merged.status, "merged");
    assert_eq!(merged.merged_into_concept_id, Some(target.id));

    let target_facets = store.list_facets(target.id).unwrap();
    let values: Vec<&str> = target_facets.iter().map(|f| f.value.as_str()).collect();
    assert!(values.contains(&"Yun variant"), "{values:?}");
    assert_eq!(
        store.concept_ids_for_crystal(crystal_id).unwrap(),
        vec![target.id]
    );
    assert_eq!(store.get(target.id).unwrap().tags, vec!["talent"]);
}

#[test]
fn merge_rejects_archived_and_merged_targets() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let source = store.create_concept("Yun", &Default::default()).unwrap();
    let archived_target = store
        .create_concept("Archived Target", &Default::default())
        .unwrap();
    store
        .archive_concept(archived_target.id, "Inactive target.")
        .unwrap();

    let error = store
        .merge_concepts(source.id, archived_target.id, "Should not merge.")
        .unwrap_err();
    assert_eq!(error.to_string(), "merge target concept must be active");

    let merged_target = store
        .create_concept("Merged Target", &Default::default())
        .unwrap();
    let another_source = store
        .create_concept("Another", &Default::default())
        .unwrap();
    store
        .merge_concepts(merged_target.id, another_source.id, "First merge.")
        .unwrap();
    let third = store.create_concept("Third", &Default::default()).unwrap();
    let error = store
        .merge_concepts(
            third.id,
            merged_target.id,
            "Should not merge into merged target.",
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "merge target concept must be active");
}

#[test]
fn merge_rejects_self_merge() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store.create_concept("Yun", &Default::default()).unwrap();

    let error = store
        .merge_concepts(concept.id, concept.id, "Self merge.")
        .unwrap_err();

    assert_eq!(error.to_string(), "source and target concepts must differ");
}

#[test]
fn link_crystal_rejects_inactive_concept_and_refreshes_status() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let store = open_store(&root);
    let concept = store.create_concept("Yun", &Default::default()).unwrap();
    store.archive_concept(concept.id, "Inactive.").unwrap();
    let crystal_id = insert_crystal(&config, "Orphan crystal.");

    let error = store
        .link_crystal(crystal_id, concept.id, "mentions", 0.8)
        .unwrap_err();
    assert_eq!(error.to_string(), "cannot link crystal to inactive concept");

    // Enough linked evidence plus confidence establishes an active concept.
    let active = store.create_concept("Sense", &Default::default()).unwrap();
    let crystal_a = insert_crystal(&config, "Crystal A");
    let crystal_b = insert_crystal(&config, "Crystal B");
    store
        .update_concept(active.id, None, None, Some(ESTABLISHED_CONFIDENCE))
        .unwrap();
    store
        .link_crystal(crystal_a, active.id, "mentions", 0.9)
        .unwrap();
    store
        .link_crystal(crystal_b, active.id, "mentions", 0.9)
        .unwrap();
    assert_eq!(store.get(active.id).unwrap().status, "established");
}

// The lifecycle tests rely on the registry only to keep the data root wired.
#[test]
fn registry_and_store_share_one_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let _registry = open_registry(&root);
    let store = open_store(&root);
    let concept = store.create_concept("Shared", &Default::default()).unwrap();
    assert!(config.database_path().exists());
    assert_eq!(concept.canonical_name, "Shared");
}

const ESTABLISHED_CONFIDENCE: f64 = 0.75;

// Silence unused warning when fs import is only used by future tests.
#[test]
fn data_root_directory_stays_clean_across_lifecycle() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let store = open_store(&root);
    let source = store.create_concept("Source", &Default::default()).unwrap();
    let target = store.create_concept("Target", &Default::default()).unwrap();
    store
        .merge_concepts(source.id, target.id, "Consolidated.")
        .unwrap();
    let entries: Vec<_> = fs::read_dir(config.data_root())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sqlite"))
        .collect();
    assert_eq!(entries.len(), 1, "exactly one database file expected");
}

// Re-exported error type keeps the failure surface explicit in tests.
#[test]
fn concept_error_is_displayable() {
    let error: ConceptError = ConceptError::Invalid("boom".into());
    assert_eq!(error.to_string(), "boom");
}

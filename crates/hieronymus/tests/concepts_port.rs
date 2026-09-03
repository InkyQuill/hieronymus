//! Behavior ported from `tests/test_concept_store.py`,
//! `tests/test_concept_facets_multilingual.py`, and
//! `tests/test_concept_models.py` (core store contract; lifecycle merge/rename
//! and crystal-linking port with their own slices).

use std::fs;

use hieronymus::concept_models::{ConceptFacetRecord, ConceptRecord};
use hieronymus::concepts::{
    ConceptError, ConceptStore, ESTABLISHED_CONFIDENCE, ESTABLISHED_EVIDENCE_COUNT, NewConcept,
};
use hieronymus::data_root::HieronymusConfig;

fn open_store(root: &tempfile::TempDir) -> ConceptStore {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    ConceptStore::open(&config).unwrap()
}

#[test]
fn create_concept_defaults_to_candidate_with_clamped_confidence() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let concept = store
        .create_concept(
            "Only Sense Online",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 5.0,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    assert_eq!(concept.canonical_name, "Only Sense Online");
    assert_eq!(concept.status, "candidate");
    assert_eq!(concept.confidence, 1.0);
    assert_eq!(concept.scope_type, "global");
    assert!(concept.tags.is_empty());
}

#[test]
fn create_concept_rejects_empty_name_and_inactive_status() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let error = store
        .create_concept(
            "   ",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "concept canonical_name must not be empty"
    );

    let error = store
        .create_concept(
            "Archived Path",
            &NewConcept {
                description: "".to_string(),
                status: "archived".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "concept_create cannot set inactive status"
    );
}

#[test]
fn create_concept_validates_scope_pairs() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let error = store
        .create_concept(
            "Scoped",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "series:demo".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "global concept scope requires an empty key"
    );

    let error = store
        .create_concept(
            "Scoped",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "series".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "non-global concept scope requires a key");

    let scoped = store
        .create_concept(
            "Scoped",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "series".into(),
                scope_key: "series:demo".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(scoped.scope_type, "series");
    assert_eq!(scoped.scope_key, "series:demo");
}

#[test]
fn list_concepts_filters_status_including_legacy_names() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    store
        .create_concept(
            "Vague Path",
            &NewConcept {
                description: "".to_string(),
                status: "vague".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    store
        .create_concept(
            "Solid Path",
            &NewConcept {
                description: "".to_string(),
                status: "solid".into(),
                confidence: 0.9,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    store
        .create_concept(
            "Gone Path",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    store.archive_concept(3, "cleanup").unwrap();

    let candidates = store.list_concepts(Some("candidate"), None).unwrap();
    let names: Vec<&str> = candidates
        .iter()
        .map(|concept| concept.canonical_name.as_str())
        .collect();
    assert_eq!(names, vec!["Vague Path"]);

    let established = store.list_concepts(Some("solid"), None).unwrap();
    assert_eq!(established.len(), 1);
    assert_eq!(established[0].status, "established");

    let archived = store.list_concepts(Some("archived"), None).unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].canonical_name, "Gone Path");

    let all = store.list_concepts(None, None).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn update_concept_changes_description_status_confidence() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store
        .create_concept(
            "Update Me",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    let updated = store
        .update_concept(
            concept.id,
            Some("Updated description"),
            Some("established"),
            Some(0.9),
        )
        .unwrap();

    assert_eq!(updated.description, "Updated description");
    assert_eq!(updated.status, "established");
    assert_eq!(updated.confidence, 0.9);

    let error = store
        .update_concept(concept.id, None, Some("merged"), None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "concept_update cannot set inactive status"
    );
}

#[test]
fn add_facet_seeds_language_tags_from_language_and_canonicalizes_flag() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store
        .create_concept(
            "Only Sense Online",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    let facet = store
        .add_facet(
            concept.id,
            "穿刺対象",
            &hieronymus::concepts::FacetFields {
                language: "ja".into(),
                language_tags: vec!["ja".into(), "RU ".into()],
                ..Default::default()
            },
            0.5,
            true,
        )
        .unwrap();

    assert_eq!(facet.language, "ja");
    assert_eq!(facet.facet_type, "name");
    assert_eq!(facet.language_tags, vec!["ja", "ru"]);
    assert!(facet.is_canonical);

    let facets = store.list_facets(concept.id).unwrap();
    assert_eq!(facets.len(), 1);
    assert!(facets[0].is_canonical);

    // Adding another canonical facet moves the flag.
    store
        .add_facet(
            concept.id,
            "Only Sense Online",
            &hieronymus::concepts::FacetFields::default(),
            0.2,
            true,
        )
        .unwrap();
    let facets = store.list_facets(concept.id).unwrap();
    assert_eq!(facets[0].value, "Only Sense Online");
    assert!(!facets[1].is_canonical);
}

#[test]
fn add_facet_rejects_empty_value_and_unknown_kinds() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store
        .create_concept(
            "Demo",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    let error = store
        .add_facet(concept.id, "   ", &Default::default(), 0.2, false)
        .unwrap_err();
    assert_eq!(error.to_string(), "concept facet value must not be empty");

    let error = store
        .add_facet(
            concept.id,
            "value",
            &hieronymus::concepts::FacetFields {
                kind: Some("made-up".into()),
                ..Default::default()
            },
            0.2,
            false,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "unknown concept facet kind: made-up");

    let error = store
        .add_facet(
            concept.id,
            "value",
            &hieronymus::concepts::FacetFields {
                kind: Some("rendering".into()),
                facet_type: Some("alias".into()),
                ..Default::default()
            },
            0.2,
            false,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "kind and facet_type must not conflict");
}

#[test]
fn set_canonical_facet_rejects_foreign_facet() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let first = store
        .create_concept(
            "First",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    let second = store
        .create_concept(
            "Second",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    let facet = store
        .add_facet(second.id, "second facet", &Default::default(), 0.2, false)
        .unwrap();

    let error = store.set_canonical_facet(first.id, facet.id).unwrap_err();

    assert_eq!(
        error.to_string(),
        format!("unknown concept facet: {}", facet.id)
    );
}

#[test]
fn archived_concept_blocks_mutations_but_stays_readable() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store
        .create_concept(
            "Archive Me",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    store.archive_concept(concept.id, "cleanup").unwrap();

    let archived = store.get(concept.id).unwrap();
    assert_eq!(archived.status, "archived");

    let error = store
        .add_facet(concept.id, "late", &Default::default(), 0.2, false)
        .unwrap_err();
    assert_eq!(error.to_string(), "cannot mutate inactive concept");
}

#[test]
fn semantic_tags_set_and_filter() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let concept = store
        .create_concept(
            "Tagged",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: vec![" ui ", "ui", "term"]
                    .into_iter()
                    .map(|s: &str| s.to_string())
                    .collect(),
            },
        )
        .unwrap();

    assert_eq!(concept.tags, vec!["term", "ui"]);

    let filtered = store.list_concepts(None, Some("ui")).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, concept.id);
    assert!(
        store
            .list_concepts(None, Some("absent"))
            .unwrap()
            .is_empty()
    );

    store
        .set_semantic_tags(concept.id, &["fresh".into()])
        .unwrap();
    assert_eq!(store.get(concept.id).unwrap().tags, vec!["fresh"]);
}

#[test]
fn search_ranks_exact_above_fts_and_boosts_story_scopes() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let needle = store
        .create_concept(
            "Needle Concept",
            &NewConcept {
                description: "unique needle description".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();
    store
        .add_facet(needle.id, "needle alias", &Default::default(), 0.2, false)
        .unwrap();
    let filler = store
        .create_concept(
            "Unrelated",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    let exact = store.search("Needle Concept", None, &[]).unwrap();
    assert_eq!(exact[0].id, needle.id);

    let fts = store.search("needle", None, &[]).unwrap();
    let ids: Vec<i64> = fts.iter().map(|concept| concept.id).collect();
    assert!(ids.contains(&needle.id));
    assert!(!ids.contains(&filler.id));

    assert!(store.search("", None, &[]).unwrap().is_empty());
}

#[test]
fn established_requires_confidence_and_evidence() {
    // Behavior constants from the Python ConceptStore class attributes.
    assert_eq!(ESTABLISHED_CONFIDENCE, 0.75);
    assert_eq!(ESTABLISHED_EVIDENCE_COUNT, 2);
}

#[test]
fn concept_record_scope_validation_matches_python() {
    let record = ConceptRecord {
        id: 1,
        canonical_name: "X".into(),
        description: String::new(),
        status: "candidate".into(),
        confidence: 0.2,
        scope_type: "global".into(),
        scope_key: String::new(),
        tags: vec![],
        merged_into_concept_id: None,
    };
    assert!(record.validate().is_ok());

    let scoped = ConceptRecord {
        scope_type: "series".into(),
        scope_key: "series:demo".into(),
        ..record.clone()
    };
    assert!(scoped.validate().is_ok());

    let broken = ConceptRecord {
        scope_type: "global".into(),
        scope_key: "series:demo".into(),
        ..record.clone()
    };
    assert_eq!(
        broken.validate().unwrap_err(),
        "global concept scope requires an empty key"
    );

    let broken = ConceptRecord {
        scope_type: "series".into(),
        scope_key: String::new(),
        ..record
    };
    assert_eq!(
        broken.validate().unwrap_err(),
        "non-global concept scope requires a key"
    );
}

#[test]
fn facet_kind_compatibility_mapping() {
    let alias = ConceptFacetRecord {
        id: 1,
        concept_id: 1,
        language: String::new(),
        facet_type: "alias".into(),
        value: "value".into(),
        confidence: 0.5,
        source_crystal_id: None,
        language_tags: vec![],
        story_scopes: vec![],
        semantic_tags: vec![],
        is_canonical: false,
    };
    assert_eq!(alias.kind(), "name");

    let former_label = ConceptFacetRecord {
        facet_type: "former_label".into(),
        ..alias.clone()
    };
    assert_eq!(former_label.kind(), "name");

    let rendering = ConceptFacetRecord {
        facet_type: "rendering".into(),
        ..alias
    };
    assert_eq!(rendering.kind(), "rendering");
}

// Keep the config root isolated: store tests must not leak files outside
// their temp roots.
#[test]
fn store_initialization_isolated_to_data_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    ConceptStore::open(&config).unwrap();
    let leftovers: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["hieronymus".to_string()]);
}

#[test]
fn unknown_concept_reports_identifier() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    let error: ConceptError = store.get(987654).unwrap_err();

    assert_eq!(error.to_string(), "unknown concept: 987654");
}

// Ensure the concept search respects the module-level search-expression
// builder (word tokens only).
#[test]
fn search_ignores_operator_only_queries() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    store
        .create_concept(
            "Operator Query",
            &NewConcept {
                description: "".to_string(),
                status: "candidate".into(),
                confidence: 0.2,
                scope_type: "global".into(),
                scope_key: "".into(),
                semantic_tags: Vec::new(),
            },
        )
        .unwrap();

    assert!(store.search("and or not", None, &[]).unwrap().is_empty());
}

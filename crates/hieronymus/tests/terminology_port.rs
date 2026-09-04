//! Behavior ported from `tests/test_termbase_contract.py` and
//! `tests/test_termbase_validate.py` (core deterministic contract; concept
//! context resolution ports with the recall slice).

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::registry::Registry;
use hieronymus::terminology::{ProposeFields, Source, Termbase};

fn open_termbase(root: &tempfile::TempDir) -> Termbase {
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap();
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    Termbase::open(&config, &context).unwrap()
}

#[test]
fn propose_creates_candidate_rule_with_forms() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);

    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: vec!["stab target".to_string()],
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(rule.status, "candidate");
    assert_eq!(rule.source_text, "穿刺対象");
    assert_eq!(rule.canonical_translation, "pierce target");
    assert_eq!(rule.forbidden_variants, vec!["stab target"]);
    assert_eq!(rule.revision, 1);
}

#[test]
fn propose_validates_rule_shape() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);

    let error = termbase
        .propose(
            "source",
            "canonical",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: vec!["bad1".into(), "bad2".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "rule crystals support at most one forbidden variant"
    );

    let error = termbase
        .propose(
            "source",
            "canonical",
            &ProposeFields {
                concept_id: None,
                approved_variants: vec!["different".to_string()],
                forbidden_variants: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "approved variants that differ from canonical rendering are unsupported"
    );

    let error = termbase
        .propose(
            "",
            "canonical",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "rule crystal text cannot round-trip parsed fields"
    );
}

#[test]
fn approve_activates_candidate_and_records_revision() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);
    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap();

    termbase
        .approve(rule.id, "pavel", "Approved via review.")
        .unwrap();

    let activated = termbase.get_rule(rule.id).unwrap();
    assert_eq!(activated.status, "active");

    let error = termbase
        .approve(rule.id, "pavel", "Double approval.")
        .unwrap_err();
    assert!(
        error.to_string().contains("only candidate rules"),
        "{error}"
    );
}

#[test]
fn contract_matches_active_source_surface_case_insensitively() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);
    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: vec!["stab target".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(rule.id, "pavel", "review").unwrap();

    let terms = termbase
        .contract("The 穿刺対象 scene starts here.")
        .unwrap();

    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].source_text, "穿刺対象");
    assert_eq!(terms[0].canonical_translation, "pierce target");
    assert_eq!(terms[0].forbidden_variants, vec!["stab target"]);
    assert_eq!(
        terms[0].notes,
        "穿刺対象 is translated as pierce target, not stab target."
    );
}

#[test]
fn contract_ignores_candidates_and_non_matching_text() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);
    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(termbase.contract("nothing relevant").unwrap().is_empty());

    termbase.approve(rule.id, "pavel", "review").unwrap();
    assert!(termbase.contract("nothing relevant").unwrap().is_empty());
}

#[test]
fn validate_reports_forbidden_and_missing_canonical() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);
    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: vec!["stab target".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(rule.id, "pavel", "review").unwrap();

    let findings = termbase
        .validate(
            "He stabbed with the stab target skill.",
            Source::Raw("穿刺対象 appears.".into()),
        )
        .unwrap();

    let kinds: Vec<(&str, &str)> = findings
        .iter()
        .map(|finding| (finding.kind.as_str(), finding.severity.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("forbidden_variant", "high"),
            ("missing_canonical", "medium")
        ]
    );
    assert_eq!(findings[0].observed, "stab target");

    let clean = termbase
        .validate(
            "The pierce target skill activated.",
            Source::Raw("穿刺対象 appears.".into()),
        )
        .unwrap();
    assert!(clean.is_empty());
}

#[test]
fn ambiguous_sources_warn_instead_of_enforcing() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap();
    let concept_store = hieronymus::concepts::ConceptStore::open(&config).unwrap();
    let concept_a = concept_store
        .create_concept("Yuni A", &Default::default())
        .unwrap();
    let concept_b = concept_store
        .create_concept("Yuni B", &Default::default())
        .unwrap();
    let termbase = Termbase::open(
        &config,
        &TranslationContext::new("demo", "ja", "en", "translation"),
    )
    .unwrap();
    // Two active rules with the same source surface but different concepts
    // are ambiguous: neither becomes enforceable, both surface a warning.
    let first = termbase
        .propose(
            "yuni",
            "yuni rendering",
            &ProposeFields {
                concept_id: Some(concept_a.id),
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(first.id, "pavel", "review").unwrap();
    let second = termbase
        .propose(
            "yuni",
            "other rendering",
            &ProposeFields {
                concept_id: Some(concept_b.id),
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(second.id, "pavel", "review").unwrap();

    let findings = termbase
        .validate("yuni appears here", Source::Raw("yuni".into()))
        .unwrap();

    assert!(
        findings
            .iter()
            .any(|finding| finding.kind == "ambiguous_source"),
        "{findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| finding.kind == "missing_canonical"),
        "{findings:?}"
    );
}

#[test]
fn context_tags_disambiguate_conflicting_source_surfaces() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    Registry::open(&config).unwrap();
    let concept_store = hieronymus::concepts::ConceptStore::open(&config).unwrap();
    let concept_a = concept_store
        .create_concept("Magic A", &Default::default())
        .unwrap();
    let concept_b = concept_store
        .create_concept("Magic B", &Default::default())
        .unwrap();
    let mut context = TranslationContext::new("demo", "ja", "en", "translation");
    context.semantic_tags = vec!["battle".into()];
    let termbase = Termbase::open(&config, &context).unwrap();

    // Same source surface, different concepts: "battle" tag resolves to B.
    let first = termbase
        .propose(
            "mahou",
            "magic a",
            &ProposeFields {
                concept_id: Some(concept_a.id),
                semantic_tags: vec!["daily".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(first.id, "pavel", "review").unwrap();
    let second = termbase
        .propose(
            "mahou",
            "magic b",
            &ProposeFields {
                concept_id: Some(concept_b.id),
                semantic_tags: vec!["battle".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    termbase.approve(second.id, "pavel", "review").unwrap();

    let mut battle_context = TranslationContext::new("demo", "ja", "en", "translation");
    battle_context.semantic_tags = vec!["battle".into()];
    let battle_termbase = Termbase::open(&config, &battle_context).unwrap();
    let terms = battle_termbase.contract("mahou appears").unwrap();
    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].canonical_translation, "magic b");

    // Without the tag the surface stays ambiguous: no contract term, only a
    // warning.
    let plain_termbase =
        Termbase::open(&config, &TranslationContext::new("demo", "ja", "en", "translation"))
            .unwrap();
    assert!(plain_termbase.contract("mahou appears").unwrap().is_empty());
    let findings = plain_termbase
        .validate("mahou appears", Source::Raw("mahou".into()))
        .unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding.kind == "ambiguous_source"),
        "{findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| finding.kind == "missing_canonical"),
        "{findings:?}"
    );
}

#[test]
fn propose_stores_context_metadata_side_tables() {
    let root = tempfile::tempdir().unwrap();
    let termbase = open_termbase(&root);
    let rule = termbase
        .propose(
            "穿刺対象",
            "pierce target",
            &ProposeFields {
                semantic_tags: vec!["term".into()],
                story_scopes: vec!["chapter:2".into()],
                language_tags: vec!["ru".into()],
                ..Default::default()
            },
        )
        .unwrap();

    let hydrated = termbase.get_rule(rule.id).unwrap();
    assert_eq!(hydrated.semantic_tags, vec!["term"]);
    assert_eq!(hydrated.story_scopes, vec!["chapter:2"]);
    assert_eq!(hydrated.language_tags, vec!["ru"]);
}

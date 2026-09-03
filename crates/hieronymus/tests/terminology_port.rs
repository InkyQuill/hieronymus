//! Behavior ported from `tests/test_termbase_contract.py` and
//! `tests/test_termbase_validate.py` (core deterministic contract; concept
//! context resolution ports with the recall slice).

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::registry::Registry;
use hieronymus::terminology::{Source, Termbase};

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
            None,
            "穿刺対象",
            "pierce target",
            &[],
            &["stab target".into()],
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
            None,
            "source",
            "canonical",
            &[],
            &["bad1".into(), "bad2".into()],
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "rule crystals support at most one forbidden variant"
    );

    let error = termbase
        .propose(None, "source", "canonical", &["different".to_string()], &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "approved variants that differ from canonical rendering are unsupported"
    );

    let error = termbase
        .propose(None, "", "canonical", &[], &[])
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
        .propose(None, "穿刺対象", "pierce target", &[], &[])
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
            None,
            "穿刺対象",
            "pierce target",
            &[],
            &["stab target".into()],
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
        .propose(None, "穿刺対象", "pierce target", &[], &[])
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
            None,
            "穿刺対象",
            "pierce target",
            &[],
            &["stab target".into()],
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
        .propose(Some(concept_a.id), "yuni", "yuni rendering", &[], &[])
        .unwrap();
    termbase.approve(first.id, "pavel", "review").unwrap();
    let second = termbase
        .propose(Some(concept_b.id), "yuni", "other rendering", &[], &[])
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

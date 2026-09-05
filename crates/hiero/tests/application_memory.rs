//! M2 regression: the application memory family performs real memory, RAG,
//! and feedback domain work, and every recall response carries the
//! deterministic terminology contract separately from the ranked results
//! (ADR 0011).
//!
//! Coverage: the step-1 recall DTO regression, the versioned
//! `compatibility/rust/recall-v2.json` expectation, contract serialization
//! with a store-seeded approved rule, batch atomic rejection, session
//! ownership and override mismatches, oversize input, unsupported RAG import
//! types, repeated-recall working-copy dedup with activation ids, legacy
//! memory_add/memory_search wrapper semantics, and default/null handling.

use std::fs;

use hiero::application::{AppError, Application};
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::terminology::{ProposeFields, Termbase};
use hieronymus::workspace::WorkspaceStore;
use serde_json::{Value, json};

const ACTOR: &str = "local-user";
const RECALL_V2_FIXTURE: &str = include_str!("../../../compatibility/rust/recall-v2.json");

fn test_application() -> (tempfile::TempDir, Application) {
    let root = tempfile::tempdir().unwrap();
    let application = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    (root, application)
}

fn create_series(app: &Application, slug: &str, source: &str, target: &str) {
    app.call(
        "hieronymus_series_create",
        &json!({"slug": slug, "title": "Book", "source_language": source, "target_language": target}),
        ACTOR,
    )
    .unwrap();
}

fn start_session(app: &Application, slug: &str) -> i64 {
    let session = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": slug}),
            ACTOR,
        )
        .unwrap();
    session["session_id"].as_i64().unwrap()
}

fn expect_domain(error: AppError, needle: &str) {
    match error {
        AppError::Domain(message) => assert!(
            message.contains(needle),
            "expected {needle:?} in domain diagnostic: {message}"
        ),
        other => panic!("expected a domain error, got: {other}"),
    }
}

fn expect_invalid(error: AppError, needle: &str) {
    match error {
        AppError::Invalid(message) => assert!(
            message.contains(needle),
            "expected {needle:?} in invalid-argument diagnostic: {message}"
        ),
        other => panic!("expected an invalid-argument error, got: {other}"),
    }
}

/// The sentence-capped oversize body (default rejection threshold: 30
/// sentences).
fn oversize_text() -> String {
    (0..31)
        .map(|index| format!("Sentence number {index} is long enough to count."))
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------------------ step 1 regression

#[test]
fn recall_exposes_contract_even_when_results_are_empty() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book"}),
        "local-user",
    )
    .unwrap();
    let session = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug":"book"}),
            "local-user",
        )
        .unwrap();
    let recall = app
        .call(
            "hieronymus_recall",
            // M1's session_start payload carries the id under `session_id`.
            &json!({"session_id":session["session_id"],"series_slug":"book","query":"absent","limit":1}),
            "local-user",
        )
        .unwrap();
    assert_eq!(recall["deterministic_contract"], json!([]));
    assert!(recall["results"].is_array());
    assert!(recall["recall_id"].is_string());
}

// ------------------------------------------------------------ ADR 0011 contract

/// Seed one approved rule through the deterministic termbase store (the rule
/// lifecycle tools themselves land in M3; the recall DTO must expose the
/// already-computed contract regardless).
fn seed_approved_rule(config: &HieronymusConfig, source: &str, canonical: &str) -> i64 {
    let context = TranslationContext::new("book", "ja", "ru", "translation");
    let termbase = Termbase::open(config, &context).unwrap();
    let rule = termbase
        .propose(
            source,
            canonical,
            &ProposeFields {
                concept_id: None,
                approved_variants: Vec::new(),
                forbidden_variants: Vec::new(),
                semantic_tags: Vec::new(),
                story_scopes: Vec::new(),
                language_tags: Vec::new(),
                notes: String::new(),
            },
        )
        .unwrap();
    termbase.approve(rule.id, "test", "seed").unwrap();
    rule.id
}

#[test]
fn recall_serializes_the_whole_contract_despite_limit_one() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "ru");
    let session_id = start_session(&app, "book");
    let config = HieronymusConfig::new(root.path());
    seed_approved_rule(&config, "猫", "кот");

    // The only advisory candidate is an unrelated short-term note, so with
    // limit=1 the ranked list cannot carry the rule — the contract section
    // still carries it whole, with the canonical rendering intact.
    app.call(
        "hieronymus_short_term_add",
        &json!({"session_id": session_id, "kind": "note", "text": "whisker观察 note"}),
        ACTOR,
    )
    .unwrap();

    let recall = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "book", "query": "猫", "limit": 1}),
            ACTOR,
        )
        .unwrap();

    let contract = recall["deterministic_contract"].as_array().unwrap();
    assert_eq!(contract.len(), 1, "{recall}");
    assert_eq!(contract[0]["source_text"], json!("猫"));
    assert_eq!(contract[0]["canonical_translation"], json!("кот"));
    assert_eq!(contract[0]["forbidden_variants"], json!([]));
    assert_eq!(contract[0]["category"], json!("rule"));
    assert!(recall["results"].as_array().unwrap().len() <= 1);
}

#[test]
fn recall_v2_fixture_pins_the_rust_dto() {
    let fixture: Value = serde_json::from_str(RECALL_V2_FIXTURE).unwrap();
    assert_eq!(fixture["adr"], json!("0011"));
    for expectation in fixture["expectations"].as_array().unwrap() {
        let (root, app) = test_application();
        create_series(&app, "book", "ja", "ru");
        assert_eq!(
            start_session(&app, "book"),
            1,
            "fixture arguments pin session 1"
        );
        let config = HieronymusConfig::new(root.path());
        if expectation["seed"] == json!("none") {
            // No seed.
        } else {
            seed_approved_rule(&config, "猫", "кот");
        }

        let recall = app
            .call("hieronymus_recall", &expectation["arguments"], ACTOR)
            .unwrap();
        let expected = &expectation["expected"];

        if expected["recall_id_is_string"] == json!(true) {
            assert!(recall["recall_id"].is_string());
        }
        if let Some(results) = expected["results"].as_array() {
            assert_eq!(recall["results"], json!(results), "{expectation}");
        }
        if let Some(warnings) = expected["warnings"].as_array() {
            assert_eq!(recall["warnings"], json!(warnings), "{expectation}");
        }
        // Contract rows are matched as subsets: stable fields must match
        // exactly; the storage id is deployment-order dependent.
        if let Some(expected_rows) = expected["deterministic_contract"].as_array() {
            let actual_rows = recall["deterministic_contract"].as_array().unwrap();
            assert_eq!(actual_rows.len(), expected_rows.len(), "{expectation}");
            for expected_row in expected_rows {
                let source_text = &expected_row["source_text"];
                let actual_row = actual_rows
                    .iter()
                    .find(|row| &row["source_text"] == source_text)
                    .unwrap_or_else(|| panic!("contract row {source_text} missing: {recall}"));
                for key in [
                    "canonical_translation",
                    "forbidden_variants",
                    "tags",
                    "notes",
                ] {
                    if let Some(expected_value) = expected_row.get(key) {
                        assert_eq!(
                            actual_row[key], *expected_value,
                            "contract field {key} mismatch: {recall}"
                        );
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------- session context rules

#[test]
fn recall_rejects_cross_series_and_mismatched_overrides() {
    let (_root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    create_series(&app, "other", "ja", "en");
    let session_id = start_session(&app, "book");

    // Cross-series session arguments.
    let error = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "other", "query": "x"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "session context mismatch");

    // Unknown series and unknown sessions.
    let error = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "ghost", "query": "x"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown series");
    let error = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": 424_242, "series_slug": "book", "query": "x"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown session");

    // Override mismatches name the field; matching overrides are accepted.
    for field in [
        "source_language",
        "target_language",
        "task_type",
        "volume",
        "chapter",
    ] {
        let error = app
            .call(
                "hieronymus_recall",
                &json!({"session_id": session_id, "series_slug": "book", "query": "x",
                        field: "definitely-not-the-stored-value"}),
                ACTOR,
            )
            .unwrap_err();
        expect_domain(error, &format!("session context mismatch: {field}"));
    }
    app.call(
        "hieronymus_recall",
        &json!({"session_id": session_id, "series_slug": "book", "query": "x",
                "source_language": "ja", "target_language": "en", "task_type": "translation",
                "volume": "", "chapter": ""}),
        ACTOR,
    )
    .unwrap();

    // limit below 1 is rejected before any ranking runs.
    let error = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "book", "query": "x", "limit": 0}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "limit must be at least 1");
}

// --------------------------------------------------------------- batch semantics

#[test]
fn short_term_add_batch_rejects_atomically() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    // One invalid item rejects the whole batch: nothing is written.
    let error = app
        .call(
            "hieronymus_short_term_add_batch",
            &json!({"session_id": session_id, "items": [
                {"kind": "note", "text": "first good note"},
                {"kind": "   ", "text": "blank kind rejects everything"},
                {"kind": "note", "text": "third good note"}
            ]}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "kind must not be empty");

    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    assert!(
        store
            .list_short_term_memories(session_id)
            .unwrap()
            .is_empty(),
        "an atomic rejection must leave no partial memories"
    );

    // The all-good batch lands in order, in one call.
    let batch = app
        .call(
            "hieronymus_short_term_add_batch",
            &json!({"session_id": session_id, "items": [
                {"kind": "note", "text": "first good note"},
                {"kind": "note", "text": "second good note"}
            ]}),
            ACTOR,
        )
        .unwrap();
    let ids = batch["memory_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(batch["count"], json!(2));

    // Empty and oversize batches are domain rejections with no writes.
    let error = app
        .call(
            "hieronymus_short_term_add_batch",
            &json!({"session_id": session_id, "items": []}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "items must not be empty");
    let flood: Vec<Value> = (0..501)
        .map(|index| json!({"kind": "note", "text": format!("flood note {index}")}))
        .collect();
    let error = app
        .call(
            "hieronymus_short_term_add_batch",
            &json!({"session_id": session_id, "items": flood}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "a batch may contain at most 500");
}

#[test]
fn oversize_short_term_input_is_rejected() {
    let (_root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    let error = app
        .call(
            "hieronymus_short_term_add",
            &json!({"session_id": session_id, "kind": "note", "text": oversize_text()}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "short-term memory is too large");

    // The legacy wrapper runs through the same thresholds.
    let error = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "note", "text": oversize_text()}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "short-term memory is too large");
}

// ------------------------------------------------------- session ownership

#[test]
fn short_term_add_rejects_unknown_and_inactive_sessions() {
    let (_root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    let error = app
        .call(
            "hieronymus_short_term_add",
            &json!({"session_id": 987_654, "kind": "note", "text": "orphan note"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown session");

    app.call(
        "hieronymus_session_complete",
        &json!({"session_id": session_id}),
        ACTOR,
    )
    .unwrap();
    let error = app
        .call(
            "hieronymus_short_term_add",
            &json!({"session_id": session_id, "kind": "note", "text": "late note"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "active session");

    let error = app
        .call(
            "hieronymus_feedback",
            &json!({"session_id": 987_654, "correction_text": "use кот"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown session");
}

// ------------------------------------------------------------- legacy wrappers

#[test]
fn memory_add_wraps_into_short_term_with_defaults() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");

    // Defaults: importance 3, empty source_ref, note kind for unknown kinds.
    let first = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "preference", "text": "keep the glossary nearby"}),
            ACTOR,
        )
        .unwrap();
    assert_eq!(first["storage"], json!("short_term"));
    let first_id = first["memory_id"].as_i64().unwrap();

    // Rule kinds map onto correction memories.
    let second = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "rule", "text": "always use кот for 猫",
                    "importance": 5, "source_ref": "ch 1"}),
            ACTOR,
        )
        .unwrap();

    let config = HieronymusConfig::new(root.path());
    let store = WorkspaceStore::open(&config).unwrap();

    // Verify through a fresh default-session lookup: exactly one active
    // default session now exists, holding both memories.
    let context = TranslationContext::new("book", "ja", "en", "translation");
    let session = store.active_default_session(&context).unwrap().unwrap();
    let memories = store.list_short_term_memories(session.id).unwrap();
    let ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
    assert_eq!(ids.len(), 2, "both wrappers share the default session");
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second["memory_id"].as_i64().unwrap()));

    let preference = memories.iter().find(|m| m.id == first_id).unwrap();
    assert_eq!(preference.kind, "note");
    assert_eq!(preference.source_role, "user");
    assert_eq!(
        preference.metadata.get("legacy_kind"),
        Some(&json!("preference"))
    );
    assert_eq!(preference.metadata.get("importance"), Some(&json!(3)));
    assert_eq!(preference.source_ref, "");

    let correction = memories
        .iter()
        .find(|m| m.id == second["memory_id"].as_i64().unwrap())
        .unwrap();
    assert_eq!(correction.kind, "correction");
    assert_eq!(correction.metadata.get("legacy_kind"), Some(&json!("rule")));
    assert_eq!(correction.metadata.get("importance"), Some(&json!(5)));
    assert_eq!(correction.source_ref, "ch 1");

    // Empty kinds are rejected exactly like the Python wrapper.
    let error = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "  ", "text": "nope"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "kind must not be empty");

    // Language overrides must match the registry defaults.
    let error = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "note", "text": "x", "source_language": "fr"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "does not match registry default");
}

#[test]
fn memory_search_returns_legacy_entries_with_and_without_a_session() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");

    // No active default session yet: the direct fallback path.
    let config = HieronymusConfig::new(root.path());
    let crystals = CrystalStore::open(&config).unwrap();
    crystals
        .add_crystal(
            &TranslationContext::new("book", "ja", "en", "translation"),
            "lesson",
            &NewCrystal::new("lesson", "The binding ritual requires chalk."),
        )
        .unwrap();
    let fallback = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "book", "query": "chalk binding"}),
            ACTOR,
        )
        .unwrap();
    let rows = fallback.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{fallback}");
    assert_eq!(rows[0]["kind"], json!("lesson"));
    // importance = round(strength * 5) over the stored crystal strength
    // (default strength 0.6 renders as 3).
    assert_eq!(rows[0]["importance"], json!(3));
    assert_eq!(rows[0]["source_ref"], json!(""));

    // Whitespace-only queries are empty results, not errors.
    let empty = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "book", "query": "   "}),
            ACTOR,
        )
        .unwrap();
    assert!(empty.as_array().unwrap().is_empty());

    // limit below 1 is a domain rejection.
    let error = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "book", "query": "chalk", "limit": 0}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "limit must be at least 1");

    // With a default session, search runs through recall and keeps the
    // legacy entry shape (legacy_kind wins over the stored kind).
    let memory = app
        .call(
            "hieronymus_memory_add",
            &json!({"series_slug": "book", "kind": "glossary", "text": "chalk dust observations",
                    "importance": 4}),
            ACTOR,
        )
        .unwrap();
    let searched = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "book", "query": "chalk dust"}),
            ACTOR,
        )
        .unwrap();
    let rows = searched.as_array().unwrap();
    assert!(
        rows.iter().any(|row| row["id"] == memory["memory_id"]
            && row["kind"] == json!("glossary")
            && row["importance"] == json!(4)),
        "{searched}"
    );

    // Unknown series are rejections; language overrides must match.
    let error = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "ghost", "query": "chalk"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unknown series");
    let error = app
        .call(
            "hieronymus_memory_search",
            &json!({"series_slug": "book", "query": "chalk", "target_language": "fr"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "does not match registry default");
}

// ---------------------------------------------------------------- feedback tool

#[test]
fn feedback_records_user_correction_memory() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    let payload = app
        .call(
            "hieronymus_feedback",
            &json!({"session_id": session_id, "correction_text": "use кот, not кошка"}),
            ACTOR,
        )
        .unwrap();
    let memory_id = payload["memory_id"].as_i64().unwrap();

    // This is the correction-text tool: one short-term correction memory on
    // the session — distinct from correlated recall-outcome feedback, which
    // addresses a recall_id by activation ids.
    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    let memories = store.list_short_term_memories(session_id).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id, memory_id);
    assert_eq!(memories[0].kind, "correction");
    assert_eq!(memories[0].source_role, "user");
    assert_eq!(memories[0].text, "use кот, not кошка");
}

// --------------------------------------------------------- short-term defaults

#[test]
fn short_term_add_applies_schema_defaults() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    let payload = app
        .call(
            "hieronymus_short_term_add",
            &json!({"session_id": session_id, "kind": "note", "text": "defaults check"}),
            ACTOR,
        )
        .unwrap();

    let store = WorkspaceStore::open(&HieronymusConfig::new(root.path())).unwrap();
    let memories = store.list_short_term_memories(session_id).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id, payload["memory_id"].as_i64().unwrap());
    assert_eq!(memories[0].source_role, "agent");
    assert_eq!(memories[0].source_credibility, "observation");
    assert_eq!(memories[0].rule_intent, "");
    assert_eq!(memories[0].soft_origin, "");
    assert_eq!(memories[0].language_tags, Vec::<String>::new());

    // Explicit typed metadata round-trips.
    app.call(
        "hieronymus_short_term_add",
        &json!({"session_id": session_id, "kind": "rule_intent", "text": "glossary discipline",
                "source_role": "user", "source_credibility": "user_stated",
                "rule_intent": "prefer the termbase", "language_tags": [" JA ", "en"],
                "story_scopes": ["chapter:2"], "semantic_tags": ["terminology"]}),
        ACTOR,
    )
    .unwrap();
    let memories = store.list_short_term_memories(session_id).unwrap();
    let typed = &memories[1];
    assert_eq!(typed.source_role, "user");
    assert_eq!(typed.source_credibility, "user_stated");
    assert_eq!(typed.rule_intent, "prefer the termbase");
    assert_eq!(typed.language_tags, vec!["en", "ja"]);
    assert_eq!(typed.story_scopes, vec!["chapter:2"]);
    assert_eq!(typed.semantic_tags, vec!["terminology"]);

    // Missing required fields are invalid arguments (no writes).
    let error = app
        .call(
            "hieronymus_short_term_add",
            &json!({"session_id": session_id, "text": "missing kind"}),
            ACTOR,
        )
        .unwrap_err();
    expect_invalid(error, "missing field `kind`");
    assert_eq!(store.list_short_term_memories(session_id).unwrap().len(), 2);
}

// ------------------------------------------------------------------- RAG tools

#[test]
fn rag_import_persists_chunks_and_rejects_unsupported_types() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");

    let source_path = root.path().join("chapter.md");
    fs::write(&source_path, "# One\n\nCooking Talent appears here.\n").unwrap();
    let payload = app
        .call(
            "hieronymus_rag_import",
            &json!({"series_slug": "book", "path": source_path.to_str().unwrap(),
                    "source_ref": "chapter-1", "language_tags": ["en"],
                    "story_scopes": ["chapter:1"], "semantic_tags": ["kitchen"]}),
            ACTOR,
        )
        .unwrap();
    assert_eq!(payload["series_slug"], json!("book"));
    assert_eq!(payload["source_ref"], json!("chapter-1"));
    assert_eq!(payload["skipped"], json!(false));
    assert!(payload["chunk_count"].as_u64().unwrap() >= 1, "{payload}");
    assert!(payload["source_id"].is_i64());
    assert_eq!(payload["normalized_format"], json!("markdown"));

    // Unsupported import types are clean tool errors; nothing is imported.
    let error = app
        .call(
            "hieronymus_rag_import",
            &json!({"series_slug": "book", "path": source_path.to_str().unwrap(),
                    "source_type": "bogus"}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "Unsupported RAG source type");

    // Missing files are clean tool errors too.
    let error = app
        .call(
            "hieronymus_rag_import",
            &json!({"series_slug": "book", "path": root.path().join("ghost.txt").to_str().unwrap()}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "RAG source not found");

    // Identical re-import is the checksum-aware refresh.
    let again = app
        .call(
            "hieronymus_rag_import",
            &json!({"series_slug": "book", "path": source_path.to_str().unwrap(),
                    "source_ref": "chapter-1"}),
            ACTOR,
        )
        .unwrap();
    assert_eq!(again["skipped"], json!(true));
}

#[test]
fn rag_search_returns_advisory_rows_for_one_series() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");

    let source_path = root.path().join("chapter.txt");
    fs::write(&source_path, "Cooking Talent appears here.\n").unwrap();
    app.call(
        "hieronymus_rag_import",
        &json!({"series_slug": "book", "path": source_path.to_str().unwrap()}),
        ACTOR,
    )
    .unwrap();

    let hits = app
        .call(
            "hieronymus_rag_search",
            &json!({"series_slug": "book", "query": "Cooking Talent"}),
            ACTOR,
        )
        .unwrap();
    let rows = hits.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{hits}");
    assert_eq!(rows[0]["source"], json!("rag"));
    assert_eq!(rows[0]["text"], json!("Cooking Talent appears here."));
    assert_eq!(rows[0]["rank_reason"], json!("rag project text match"));
    assert!(rows[0]["score"].is_number());

    // Series isolation and limit handling.
    let other = app
        .call(
            "hieronymus_rag_search",
            &json!({"series_slug": "ghost", "query": "Cooking Talent"}),
            ACTOR,
        )
        .unwrap();
    assert!(other.as_array().unwrap().is_empty());
    let error = app
        .call(
            "hieronymus_rag_search",
            &json!({"series_slug": "book", "query": "Cooking Talent", "limit": 0}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "limit must be at least 1");
}

// -------------------------------------------------- working copies + activations

#[test]
fn repeated_recall_deduplicates_working_copies_and_rotates_activation_ids() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "en");
    let session_id = start_session(&app, "book");

    let config = HieronymusConfig::new(root.path());
    let crystals = CrystalStore::open(&config).unwrap();
    let crystal_id = crystals
        .add_crystal(
            &TranslationContext::new("book", "ja", "en", "translation"),
            "lesson",
            &NewCrystal::new("lesson", "The binding ritual requires chalk."),
        )
        .unwrap();

    let first = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "book", "query": "binding ritual chalk"}),
            ACTOR,
        )
        .unwrap();
    let second = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "book", "query": "binding ritual chalk"}),
            ACTOR,
        )
        .unwrap();
    assert_ne!(first["recall_id"], second["recall_id"]);

    let activation_of = |recall: &Value| -> Vec<(i64, i64)> {
        recall["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["tier"] == json!("long_term") && row["id"] == json!(crystal_id))
            .map(|row| {
                (
                    row["activation_id"]
                        .as_i64()
                        .expect("activation id on long-term rows"),
                    row["rank"].as_i64().unwrap(),
                )
            })
            .collect()
    };
    let first_activations = activation_of(&first);
    let second_activations = activation_of(&second);
    assert_eq!(first_activations.len(), 1, "{first}");
    assert_eq!(second_activations.len(), 1, "{second}");
    assert_ne!(
        first_activations[0].0, second_activations[0].0,
        "each recall invocation writes its own activation row"
    );
    assert!(first_activations[0].0 > 0 && second_activations[0].0 > 0);

    // One working copy for the (session, crystal) pair despite two recalls;
    // the second recall recorded a recalled_again event instead.
    let connection = rusqlite::Connection::open_with_flags(
        config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let working_copies: i64 = connection
        .query_row(
            "select count(*) from short_term_memories
             where session_id = ?1 and source_crystal_id = ?2 and source_role = 'recall'",
            rusqlite::params![session_id, crystal_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(working_copies, 1);
    let activations: i64 = connection
        .query_row(
            "select count(*) from crystal_activations where crystal_id = ?1",
            [crystal_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(activations, 2);
}

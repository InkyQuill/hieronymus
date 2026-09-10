//! M4 regression: the concept/facet/crystal graph tools answer with real
//! graph data over the domain stores, preserving identity, canonical-facet
//! uniqueness, tag/scope side tables, and the trigger-maintained FTS
//! projection (ADR 0003 graph; the Python `mcp_server.py` wrappers are the
//! payload reference).
//!
//! Coverage: the step-1 rename regression, the full
//! create→facet→canonical→rename→merge→list/get lifecycle, facet type/kind
//! precedence, null-versus-omitted facet updates (Python pins every optional
//! field as null-as-unchanged), unknown ids, foreign facet ownership, invalid
//! confidence, archived/merged link behavior, the proposals DTO projection,
//! and transactional FTS projection of facet and concept updates.

use hiero::application::{AppError, Application};
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use serde_json::{Value, json};

const ACTOR: &str = "local-user";

fn test_application() -> (tempfile::TempDir, Application) {
    let root = tempfile::tempdir().unwrap();
    let application = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    (root, application)
}

fn config_of(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path())
}

fn connection(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(config_of(root).database_path()).unwrap()
}

fn call(app: &Application, tool: &str, arguments: Value) -> Value {
    app.call(tool, &arguments, ACTOR)
        .unwrap_or_else(|error| panic!("{tool} should succeed: {error}"))
}

fn call_error(app: &Application, tool: &str, arguments: Value) -> AppError {
    app.call(tool, &arguments, ACTOR)
        .expect_err(&format!("{tool} should fail"))
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

fn expect_invalid(error: AppError) {
    match error {
        AppError::Invalid(_) => {}
        other => panic!("expected an invalid-arguments error, got: {other}"),
    }
}

fn create_series(app: &Application, slug: &str) {
    call(
        app,
        "hieronymus_series_create",
        json!({"slug": slug, "title": "Book", "source_language": "ja", "target_language": "ru"}),
    );
}

fn create_concept(app: &Application, canonical_name: &str) -> Value {
    call(
        app,
        "hieronymus_concept_create",
        json!({"canonical_name": canonical_name}),
    )
}

fn seed_crystal(root: &tempfile::TempDir, text: &str) -> i64 {
    let context = TranslationContext::new("book", "ja", "ru", "translation");
    CrystalStore::open(&config_of(root))
        .unwrap()
        .add_crystal(
            &context,
            "observation",
            &NewCrystal::new("observation", text),
        )
        .unwrap()
}

// ------------------------------------------------------------ step 1 regression

#[test]
fn concept_rename_keeps_identity() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let concept = app
        .call(
            "hieronymus_concept_create",
            &json!({"canonical_name":"Cat"}),
            "local-user",
        )
        .unwrap();
    app.call(
        "hieronymus_concept_rename",
        &json!({"concept_id":concept["id"],"new_label":"House Cat"}),
        "local-user",
    )
    .unwrap();
    let after = app
        .call(
            "hieronymus_concept_get",
            &json!({"concept_id":concept["id"]}),
            "local-user",
        )
        .unwrap();
    assert_eq!(after["id"], concept["id"]);
    assert_eq!(after["canonical_name"], "House Cat");
}

// --------------------------------------------- create→facet→canonical→merge flow

#[test]
fn concept_lifecycle_create_facet_canonical_rename_merge_list_get() {
    let (_root, app) = test_application();

    // Create: the Python `_concept_payload` projection with defaults.
    let cat = create_concept(&app, "Cat");
    assert_eq!(
        cat,
        json!({
            "id": cat["id"],
            "canonical_name": "Cat",
            "description": "",
            "status": "candidate",
            "confidence": 0.2,
            "scope_type": "global",
            "scope_key": "",
            "semantic_tags": [],
            "merged_into_concept_id": null,
        })
    );
    let cat_id = cat["id"].as_i64().unwrap();

    // Facets: canonical flag moves through the set_canonical tool.
    let kot = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": cat_id, "value": "Кот", "language": "ru", "confidence": 0.8}),
    );
    assert_eq!(kot["facet_type"], json!("name"));
    assert_eq!(kot["kind"], json!("name"));
    assert_eq!(kot["language_tags"], json!(["ru"]));
    assert_eq!(kot["is_canonical"], json!(false));
    let named = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": cat_id, "value": "Cat", "is_canonical": true}),
    );
    assert_eq!(named["is_canonical"], json!(true));

    let facets = call(
        &app,
        "hieronymus_concept_facet_list",
        json!({"concept_id": cat_id}),
    );
    let facets = facets.as_array().unwrap();
    assert_eq!(facets.len(), 2, "{facets:?}");
    assert_eq!(
        facets[0]["value"],
        json!("Cat"),
        "canonical facet sorts first"
    );
    assert_eq!(facets[1]["value"], json!("Кот"));

    call(
        &app,
        "hieronymus_concept_facet_set_canonical",
        json!({"concept_id": cat_id, "facet_id": kot["id"]}),
    );
    let relisted = call(
        &app,
        "hieronymus_concept_facet_list",
        json!({"concept_id": cat_id}),
    );
    let relisted = relisted.as_array().unwrap();
    assert_eq!(relisted[0]["value"], json!("Кот"), "canonical moved");
    assert_eq!(relisted[0]["is_canonical"], json!(true));
    assert_eq!(relisted[1]["is_canonical"], json!(false));

    // Rename keeps identity; the old label survives as a facet value.
    let renamed = call(
        &app,
        "hieronymus_concept_rename",
        json!({"concept_id": cat_id, "new_label": "House Cat"}),
    );
    assert_eq!(renamed["id"], json!(cat_id));
    assert_eq!(renamed["canonical_name"], json!("House Cat"));

    // Merge Cat into a fresh concept: the source redirects, the facets and
    // tags move.
    call(
        &app,
        "hieronymus_concept_semantic_tags_set",
        json!({"concept_id": cat_id, "semantic_tags": ["animal", " pet "]}),
    );
    let feline = create_concept(&app, "Feline");
    let feline_id = feline["id"].as_i64().unwrap();
    let merged = call(
        &app,
        "hieronymus_concept_merge",
        json!({"source_concept_id": cat_id, "target_concept_id": feline_id}),
    );
    assert_eq!(merged["source"]["status"], json!("merged"));
    assert_eq!(merged["source"]["merged_into_concept_id"], json!(feline_id));
    assert_eq!(merged["target"]["id"], json!(feline_id));

    let target_facets = call(
        &app,
        "hieronymus_concept_facet_list",
        json!({"concept_id": feline_id}),
    );
    // The merge folds the source's canonical name into a former_label facet
    // on the target, then re-parents the unique facets.
    let values: Vec<&str> = target_facets
        .as_array()
        .unwrap()
        .iter()
        .map(|facet| facet["value"].as_str().unwrap())
        .collect();
    assert_eq!(values.len(), 3, "{target_facets}");
    assert!(
        values.contains(&"Кот") && values.contains(&"Cat") && values.contains(&"House Cat"),
        "{values:?}"
    );
    let target = call(
        &app,
        "hieronymus_concept_get",
        json!({"concept_id": feline_id}),
    );
    assert_eq!(target["semantic_tags"], json!(["animal", "pet"]));

    // list: status filter (including the legacy mapping), tag filter, and
    // the merged concept showing up under "merged".
    let candidates = call(
        &app,
        "hieronymus_concept_list",
        json!({"status": "candidate"}),
    );
    assert_eq!(
        candidates
            .as_array()
            .unwrap()
            .iter()
            .map(|concept| concept["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![feline_id]
    );
    let animals = call(
        &app,
        "hieronymus_concept_list",
        json!({"semantic_tag": "animal"}),
    );
    assert_eq!(animals.as_array().unwrap().len(), 1);
    assert_eq!(animals[0]["id"], json!(feline_id));
    let all = call(&app, "hieronymus_concept_list", json!({}));
    assert_eq!(all.as_array().unwrap().len(), 2);
}

// ------------------------------------------------- facet type/kind precedence

#[test]
fn facet_add_gives_explicit_facet_type_precedence_over_legacy_kind() {
    let (_root, app) = test_application();
    let concept_id = create_concept(&app, "Cat")["id"].as_i64().unwrap();

    let add = |arguments: Value| call(&app, "hieronymus_concept_facet_add", arguments);

    // Explicit facet_type replaces the default `name` kind; the public kind
    // of a compatibility facet still presents as `name`.
    let alias = add(json!({"concept_id": concept_id, "value": "V1", "facet_type": "alias"}));
    assert_eq!(alias["facet_type"], json!("alias"));
    assert_eq!(alias["kind"], json!("name"));

    // An explicitly chosen kind without facet_type is stored as-is.
    let description = add(json!({"concept_id": concept_id, "value": "V2", "kind": "description"}));
    assert_eq!(description["facet_type"], json!("description"));
    assert_eq!(description["kind"], json!("description"));

    // facet_type wins even against the default `name` kind...
    let rendering =
        add(json!({"concept_id": concept_id, "value": "V3", "facet_type": "rendering"}));
    assert_eq!(rendering["facet_type"], json!("rendering"));

    // ...and an agreeing explicit pair is accepted.
    let pair = add(
        json!({"concept_id": concept_id, "value": "V4", "kind": "rendering", "facet_type": "rendering"}),
    );
    assert_eq!(pair["facet_type"], json!("rendering"));

    // An explicit null kind falls back to `name` (Python kind=None).
    let null_kind = add(json!({"concept_id": concept_id, "value": "V5", "kind": null}));
    assert_eq!(null_kind["facet_type"], json!("name"));

    // A conflicting explicit pair is a deterministic rejection.
    let error = call_error(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "V6", "kind": "rendering", "facet_type": "alias"}),
    );
    expect_domain(error, "kind and facet_type must not conflict");

    // An unknown kind stays rejected.
    let error = call_error(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "V7", "kind": "bogus"}),
    );
    expect_domain(error, "unknown concept facet kind: bogus");
}

// --------------------------------------------- null-versus-omitted facet updates

#[test]
fn facet_update_pins_null_as_unchanged_for_every_field() {
    let (root, app) = test_application();
    create_series(&app, "book");
    let concept_id = create_concept(&app, "Cat")["id"].as_i64().unwrap();
    let crystal_id = seed_crystal(&root, "the cat observation");
    let facet = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({
            "concept_id": concept_id,
            "value": "Whiskers",
            "language": "en",
            "confidence": 0.9,
            "semantic_tags": ["a"],
            "story_scopes": ["s1"],
            "source_crystal_id": crystal_id,
        }),
    );
    let facet_id = facet["id"].as_i64().unwrap();

    let snapshot = || {
        call(
            &app,
            "hieronymus_concept_facet_list",
            json!({"concept_id": concept_id}),
        )[0]
        .clone()
    };

    // Omitting every field changes nothing.
    call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id}),
    );
    let unchanged = snapshot();
    assert_eq!(unchanged["value"], json!("Whiskers"));
    assert_eq!(unchanged["confidence"], json!(0.9));
    assert_eq!(unchanged["semantic_tags"], json!(["a"]));
    assert_eq!(unchanged["story_scopes"], json!(["s1"]));
    assert_eq!(unchanged["language_tags"], json!(["en"]));
    assert_eq!(unchanged["source_crystal_id"], json!(crystal_id));

    // The Python wrapper cannot distinguish an explicit null from an omitted
    // field, so EVERY optional field is null-as-unchanged: this call must be
    // a no-op, not a clear.
    call(
        &app,
        "hieronymus_concept_facet_update",
        json!({
            "facet_id": facet_id,
            "value": null,
            "language": null,
            "language_tags": null,
            "kind": null,
            "facet_type": null,
            "confidence": null,
            "source_crystal_id": null,
            "is_canonical": null,
            "story_scopes": null,
            "semantic_tags": null,
        }),
    );
    assert_eq!(
        snapshot(),
        unchanged,
        "explicit nulls must not clear anything"
    );

    // A present value replaces its field and only its field.
    let updated = call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "confidence": 0.4}),
    );
    assert_eq!(updated["confidence"], json!(0.4));
    assert_eq!(updated["value"], json!("Whiskers"), "omitted value stays");
    assert_eq!(updated["semantic_tags"], json!(["a"]), "omitted tags stay");

    // An empty list clears exactly its list field.
    let cleared = call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "semantic_tags": []}),
    );
    assert_eq!(cleared["semantic_tags"], json!([]));
    assert_eq!(
        cleared["story_scopes"],
        json!(["s1"]),
        "untouched list stays"
    );

    // Language alone recomputes the language-tag set (Python rule).
    let relanguaged = call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "language": "RU"}),
    );
    assert_eq!(relanguaged["language_tags"], json!(["ru"]));

    // Value updates are trimmed and projected into FTS (see the FTS test).
    let renamed_value = call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "value": "  Tiger  "}),
    );
    assert_eq!(renamed_value["value"], json!("Tiger"));

    // An explicitly empty value is rejected, unlike an omitted one.
    let error = call_error(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "value": "   "}),
    );
    expect_domain(error, "concept facet value must not be empty");
    assert_eq!(
        snapshot()["value"],
        json!("Tiger"),
        "the rejection is a no-op"
    );
}

// ---------------------------------------------------------------- unknown ids

#[test]
fn unknown_ids_reject_as_domain_errors() {
    let (_root, app) = test_application();
    for (tool, arguments) in [
        ("hieronymus_concept_get", json!({"concept_id": 999})),
        ("hieronymus_concept_update", json!({"concept_id": 999})),
        ("hieronymus_concept_archive", json!({"concept_id": 999})),
        (
            "hieronymus_concept_rename",
            json!({"concept_id": 999, "new_label": "X"}),
        ),
        (
            "hieronymus_concept_semantic_tags_set",
            json!({"concept_id": 999, "semantic_tags": []}),
        ),
        ("hieronymus_concept_facet_list", json!({"concept_id": 999})),
        (
            "hieronymus_concept_facet_add",
            json!({"concept_id": 999, "value": "V"}),
        ),
        (
            "hieronymus_concept_facet_set_canonical",
            json!({"concept_id": 999, "facet_id": 999}),
        ),
        (
            "hieronymus_concept_merge",
            json!({"source_concept_id": 999, "target_concept_id": 998}),
        ),
    ] {
        let error = call_error(&app, tool, arguments);
        expect_domain(error, "unknown concept");
    }
    let error = call_error(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": 999}),
    );
    expect_domain(error, "unknown concept facet: 999");
}

#[test]
fn foreign_facet_ownership_is_rejected() {
    let (_root, app) = test_application();
    let first = create_concept(&app, "Cat")["id"].as_i64().unwrap();
    let second = create_concept(&app, "Dog")["id"].as_i64().unwrap();
    let facet = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": first, "value": "Кот"}),
    );
    let foreign_facet_id = facet["id"].as_i64().unwrap();

    let error = call_error(
        &app,
        "hieronymus_concept_facet_set_canonical",
        json!({"concept_id": second, "facet_id": foreign_facet_id}),
    );
    expect_domain(error, &format!("unknown concept facet: {foreign_facet_id}"));

    // Nothing moved: the second concept still has no facets at all.
    let facets = call(
        &app,
        "hieronymus_concept_facet_list",
        json!({"concept_id": second}),
    );
    assert!(facets.as_array().unwrap().is_empty());
}

// -------------------------------------------------------- confidence handling

#[test]
fn confidence_rejects_type_errors_and_clamps_values() {
    let (_root, app) = test_application();
    let concept_id = create_concept(&app, "Cat")["id"].as_i64().unwrap();
    let facet = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "Кот"}),
    );
    let facet_id = facet["id"].as_i64().unwrap();

    let error = call_error(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "confidence": "high"}),
    );
    expect_invalid(error);

    // Out-of-range numeric confidence is clamped into [0, 1], not rejected.
    let clamped = call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "confidence": 5.0}),
    );
    assert_eq!(clamped["confidence"], json!(1.0));
}

// ------------------------------------------------ archived/merged link lifecycle

#[test]
fn crystal_links_respect_concept_lifecycle() {
    let (root, app) = test_application();
    create_series(&app, "book");
    let concept = call(
        &app,
        "hieronymus_concept_create",
        json!({"canonical_name": "Cat", "confidence": 0.8}),
    );
    let concept_id = concept["id"].as_i64().unwrap();

    // Each link is evidence; the payload echoes the linked crystal. Seeds go
    // through the store directly (the link tool takes existing crystals).
    let seed = |text: &str| {
        CrystalStore::open(&config_of(&root))
            .unwrap()
            .add_crystal(
                &TranslationContext::new("book", "ja", "ru", "translation"),
                "observation",
                &NewCrystal::new("observation", text),
            )
            .unwrap()
    };
    let first = seed("first observation");
    let linked = call(
        &app,
        "hieronymus_crystal_link_concept",
        json!({"crystal_id": first, "concept_id": concept_id, "confidence": 0.5}),
    );
    assert_eq!(linked["concept_ids"], json!([concept_id]));
    assert_eq!(
        call(
            &app,
            "hieronymus_concept_get",
            json!({"concept_id": concept_id})
        )["status"],
        json!("candidate")
    );

    // Two links plus confidence >= 0.75 establish the concept.
    let second = seed("second observation");
    call(
        &app,
        "hieronymus_crystal_link_concept",
        json!({"crystal_id": second, "concept_id": concept_id}),
    );
    assert_eq!(
        call(
            &app,
            "hieronymus_concept_get",
            json!({"concept_id": concept_id})
        )["status"],
        json!("established")
    );

    // Archived and merged concepts fail closed on new links.
    call(
        &app,
        "hieronymus_concept_archive",
        json!({"concept_id": concept_id, "reason": "done"}),
    );
    let third = seed("third observation");
    let error = call_error(
        &app,
        "hieronymus_crystal_link_concept",
        json!({"crystal_id": third, "concept_id": concept_id}),
    );
    expect_domain(error, "cannot link crystal to inactive concept");

    // A merged concept fails closed on new links too.
    let kitten = create_concept(&app, "Kitten");
    let kitten_id = kitten["id"].as_i64().unwrap();
    let feline = create_concept(&app, "Feline");
    call(
        &app,
        "hieronymus_concept_merge",
        json!({"source_concept_id": kitten_id, "target_concept_id": feline["id"]}),
    );
    let error = call_error(
        &app,
        "hieronymus_crystal_link_concept",
        json!({"crystal_id": third, "concept_id": kitten_id}),
    );
    expect_domain(error, "cannot link crystal to inactive concept");
}

// ------------------------------------------------------ crystal metadata tools

#[test]
fn crystal_metadata_setters_replace_and_reject_unknown_crystals() {
    let (root, app) = test_application();
    create_series(&app, "book");
    let crystal_id = seed_crystal(&root, "the cat observation");

    let scoped = call(
        &app,
        "hieronymus_crystal_story_scopes_set",
        json!({"crystal_id": crystal_id, "story_scopes": ["vol1", " vol2 "]}),
    );
    assert_eq!(scoped["story_scopes"], json!(["vol1", "vol2"]));
    let tagged = call(
        &app,
        "hieronymus_crystal_semantic_tags_set",
        json!({"crystal_id": crystal_id, "semantic_tags": ["cats", " cats "]}),
    );
    assert_eq!(tagged["semantic_tags"], json!(["cats"]));
    assert_eq!(tagged["text"], json!("the cat observation"));

    for tool in [
        "hieronymus_crystal_story_scopes_set",
        "hieronymus_crystal_semantic_tags_set",
    ] {
        let error = call_error(
            &app,
            tool,
            json!({"crystal_id": 999, "story_scopes": ["x"], "semantic_tags": ["x"]}),
        );
        expect_domain(error, "unknown crystal: 999");
    }
}

// ---------------------------------------------------------- list series filter

#[test]
fn concept_list_applies_series_scope_filters() {
    let (_root, app) = test_application();
    create_series(&app, "book");

    let global = create_concept(&app, "Global");
    let scoped = call(
        &app,
        "hieronymus_concept_create",
        json!({"canonical_name": "Booked", "series_slug": "book"}),
    );
    assert_eq!(scoped["scope_type"], json!("series"));
    assert_eq!(scoped["scope_key"], json!("series:book"));
    let scoped_id = scoped["id"].as_i64().unwrap();
    let global_id = global["id"].as_i64().unwrap();

    let all = call(&app, "hieronymus_concept_list", json!({}));
    assert_eq!(all.as_array().unwrap().len(), 2);

    // include_global defaults to true: both concepts come back.
    let with_global = call(
        &app,
        "hieronymus_concept_list",
        json!({"series_slug": "book"}),
    );
    let ids: Vec<i64> = with_global
        .as_array()
        .unwrap()
        .iter()
        .map(|concept| concept["id"].as_i64().unwrap())
        .collect();
    assert!(
        ids.contains(&scoped_id) && ids.contains(&global_id),
        "{ids:?}"
    );

    let scoped_only = call(
        &app,
        "hieronymus_concept_list",
        json!({"series_slug": "book", "include_global": false}),
    );
    assert_eq!(scoped_only.as_array().unwrap().len(), 1, "{scoped_only}");
    assert_eq!(scoped_only[0]["id"], json!(scoped_id));

    let error = call_error(
        &app,
        "hieronymus_concept_create",
        json!({"canonical_name": "X", "series_slug": "nope"}),
    );
    expect_domain(error, "unknown series: nope");

    // Create tool validation: an empty canonical_name is a domain rejection.
    let error = call_error(
        &app,
        "hieronymus_concept_create",
        json!({"canonical_name": "   "}),
    );
    expect_domain(error, "concept canonical_name must not be empty");

    // Status list filters validate the status name.
    let error = call_error(&app, "hieronymus_concept_list", json!({"status": "bogus"}));
    expect_domain(error, "unknown concept status: bogus");
}

// ------------------------------------------------- concept update/archive/merge guards

#[test]
fn concept_update_archive_and_merge_enforce_lifecycle_guards() {
    let (_root, app) = test_application();
    let concept = create_concept(&app, "Cat");
    let concept_id = concept["id"].as_i64().unwrap();

    // Mutable metadata update with the legacy "solid" status mapping.
    let updated = call(
        &app,
        "hieronymus_concept_update",
        json!({"concept_id": concept_id, "description": "A cat.", "status": "solid", "confidence": 0.9}),
    );
    assert_eq!(updated["description"], json!("A cat."));
    assert_eq!(updated["status"], json!("established"));
    assert_eq!(updated["confidence"], json!(0.9));

    // Omitted and explicit-null update fields are unchanged.
    let untouched = call(
        &app,
        "hieronymus_concept_update",
        json!({"concept_id": concept_id, "description": null, "status": null, "confidence": null}),
    );
    assert_eq!(untouched, updated);

    // The update path cannot set a terminal status.
    let error = call_error(
        &app,
        "hieronymus_concept_update",
        json!({"concept_id": concept_id, "status": "archived"}),
    );
    expect_domain(error, "concept_update cannot set inactive status");

    // Merge guards: identical ids and inactive targets.
    let other = create_concept(&app, "Feline");
    let other_id = other["id"].as_i64().unwrap();
    let error = call_error(
        &app,
        "hieronymus_concept_merge",
        json!({"source_concept_id": concept_id, "target_concept_id": concept_id}),
    );
    expect_domain(error, "source and target concepts must differ");

    // Tag replacement guards against archived concepts.
    call(
        &app,
        "hieronymus_concept_archive",
        json!({"concept_id": concept_id}),
    );
    let archived = call(
        &app,
        "hieronymus_concept_get",
        json!({"concept_id": concept_id}),
    );
    assert_eq!(archived["status"], json!("archived"));
    let error = call_error(
        &app,
        "hieronymus_concept_archive",
        json!({"concept_id": concept_id}),
    );
    expect_domain(error, "cannot mutate inactive concept");
    let error = call_error(
        &app,
        "hieronymus_concept_semantic_tags_set",
        json!({"concept_id": concept_id, "semantic_tags": ["x"]}),
    );
    expect_domain(error, "cannot mutate inactive concept");
    let error = call_error(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "V"}),
    );
    expect_domain(error, "cannot mutate inactive concept");

    // An inactive merge target is rejected; the source stays untouched.
    let error = call_error(
        &app,
        "hieronymus_concept_merge",
        json!({"source_concept_id": other_id, "target_concept_id": concept_id}),
    );
    expect_domain(error, "merge target concept must be active");
    assert_eq!(
        call(
            &app,
            "hieronymus_concept_get",
            json!({"concept_id": other_id})
        )["status"],
        json!("candidate")
    );

    // Rename guard: an empty label is rejected without touching the row.
    let error = call_error(
        &app,
        "hieronymus_concept_rename",
        json!({"concept_id": other_id, "new_label": "  "}),
    );
    expect_domain(error, "concept canonical_name must not be empty");
}

// --------------------------------------------------------- proposals projection

#[test]
fn concept_proposals_list_projects_pending_rows_as_safe_dtos() {
    let (root, app) = test_application();
    connection(&root)
        .execute_batch(
            "insert into strict_concept_proposals(
               series_slug, source_language, target_language, concept_text,
               source_form, canonical_rendering, approved_variants_json,
               forbidden_variants_json, rationale, status, created_at, updated_at
             )
             values ('book', 'ja', 'ru', 'Cat', 'Cat', 'Кот',
                     '[\"Котик\", \" Кот \"]', '[\"кошка\"]', 'dream evidence',
                     'pending', '2026-09-04T00:00:00+00:00', '2026-09-04T00:00:00+00:00'),
                    ('book', 'ja', 'ru', 'Dog', 'Dog', 'Пёс', '[]', '[]', '',
                     'approved', '2026-09-04T00:00:00+00:00', '2026-09-04T00:00:00+00:00');",
        )
        .unwrap();

    let proposals = call(&app, "hieronymus_concept_proposals_list", json!({}));
    let rows = proposals.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{proposals}");
    let row = &rows[0];
    let mut keys: Vec<&str> = row
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "approved_variants",
            "canonical_rendering",
            "concept_text",
            "forbidden_variants",
            "id",
            "rationale",
            "series_slug",
            "source_form",
            "source_language",
            "status",
            "target_language",
        ],
        "the DTO never leaks internal columns (dream_run_id, timestamps)"
    );
    assert_eq!(row["concept_text"], json!("Cat"));
    assert_eq!(row["canonical_rendering"], json!("Кот"));
    // Variants project exactly as stored — the DTO never rewrites data.
    assert_eq!(row["approved_variants"], json!(["Котик", " Кот "]));
    assert_eq!(row["forbidden_variants"], json!(["кошка"]));
    assert_eq!(row["status"], json!("pending"));

    // A malformed stored variant array is a loud domain rejection, never a
    // raw-JSON passthrough.
    connection(&root)
        .execute(
            "insert into strict_concept_proposals(
               series_slug, source_language, target_language, concept_text,
               source_form, canonical_rendering, approved_variants_json,
               forbidden_variants_json, rationale, status, created_at, updated_at
             )
             values ('book', 'ja', 'ru', 'Bird', 'Bird', 'Птица', 'not-json', '[]', '',
                     'pending', '2026-09-04T00:00:00+00:00', '2026-09-04T00:00:00+00:00')",
            [],
        )
        .unwrap();
    let error = call_error(&app, "hieronymus_concept_proposals_list", json!({}));
    expect_domain(error, "malformed proposal variants");
}

// ------------------------------------------------- transactional FTS projection

#[test]
fn facet_and_concept_writes_project_into_fts() {
    let (root, app) = test_application();
    let concept = create_concept(&app, "Cat");
    let concept_id = concept["id"].as_i64().unwrap();
    let fts_rows = |root: &tempfile::TempDir, table: &str, query: &str| -> i64 {
        connection(root)
            .query_row(
                &format!("select count(*) from {table} where {table} match ?1"),
                [query],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };

    assert_eq!(fts_rows(&root, "concepts_fts", "cat"), 1);
    let facet = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "Whiskers"}),
    );
    let facet_id = facet["id"].as_i64().unwrap();
    assert_eq!(fts_rows(&root, "concept_facet_fts", "whiskers"), 1);

    // A facet value update re-projects the FTS row (delete + insert).
    call(
        &app,
        "hieronymus_concept_facet_update",
        json!({"facet_id": facet_id, "value": "Tiger"}),
    );
    assert_eq!(fts_rows(&root, "concept_facet_fts", "whiskers"), 0);
    assert_eq!(fts_rows(&root, "concept_facet_fts", "tiger"), 1);

    // A rename re-projects the concept FTS row.
    call(
        &app,
        "hieronymus_concept_rename",
        json!({"concept_id": concept_id, "new_label": "Lion"}),
    );
    assert_eq!(fts_rows(&root, "concepts_fts", "cat"), 0);
    assert_eq!(fts_rows(&root, "concepts_fts", "lion"), 1);

    // Merging folds an identical target facet (the source FTS row is
    // deleted) while a unique facet survives the re-parent with its
    // original FTS rowid — the projection follows the row, transactionally.
    let fts_rowids = |root: &tempfile::TempDir, query: &str| -> Vec<i64> {
        let db = connection(root);
        let mut statement = db
            .prepare("select rowid from concept_facet_fts where concept_facet_fts match ?1")
            .unwrap();
        let rows = statement
            .query_map([query], |row| row.get::<_, i64>(0))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    };
    let saber = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": concept_id, "value": "Saber"}),
    );
    let saber_id = saber["id"].as_i64().unwrap();
    let target = create_concept(&app, "Feline");
    let target_id = target["id"].as_i64().unwrap();
    let twin = call(
        &app,
        "hieronymus_concept_facet_add",
        json!({"concept_id": target_id, "value": "Tiger"}),
    );
    let twin_id = twin["id"].as_i64().unwrap();
    call(
        &app,
        "hieronymus_concept_merge",
        json!({"source_concept_id": concept_id, "target_concept_id": target_id}),
    );
    assert_eq!(
        fts_rowids(&root, "tiger"),
        vec![twin_id],
        "the folded source facet's FTS row is gone; only the target twin survives"
    );
    assert_eq!(
        fts_rowids(&root, "saber"),
        vec![saber_id],
        "the re-parented facet keeps its FTS projection"
    );
}

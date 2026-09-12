//! Direction predicates travel through real public tools and durable sessions.
mod common;
use hiero::application::Application;
use hieronymus::{data_root::HieronymusConfig, workspace::WorkspaceStore};
use serde_json::{Value, json};

fn call(app: &Application, tool: &str, args: Value) -> Value {
    app.call(tool, &args, "agent")
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

#[test]
fn scopes_survive_session_restart_with_volume_and_chapter_seeds() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    let session = call(
        &app,
        "hieronymus_session_start",
        json!({"series_slug":"book", "volume":"I", "chapter":"1", "story_scopes":["cws:direction:ru-main", "cws:edition:en-first", "cws:direction:ru-main"]}),
    );
    drop(app);
    let app = Application::open(&config).unwrap();
    let stored = WorkspaceStore::open(app.config())
        .unwrap()
        .get_session(session["session_id"].as_i64().unwrap())
        .unwrap();
    assert_eq!(
        stored.context.story_scopes,
        [
            "chapter:1",
            "cws:direction:ru-main",
            "cws:edition:en-first",
            "volume:I"
        ]
    );
    assert_eq!(draft["source_language"], "en");
    assert_eq!(stored.context.source_language, "en");
    assert_eq!(stored.context.target_language, "ru");
}

#[test]
fn explicit_languages_are_normalized_registry_defaults_are_optional() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    common::authority::prepared(&app, root.path());
    let session = call(
        &app,
        "hieronymus_session_start",
        json!({"series_slug":"book", "source_language":" JA ", "target_language":" EN "}),
    );
    let stored = WorkspaceStore::open(app.config())
        .unwrap()
        .get_session(session["session_id"].as_i64().unwrap())
        .unwrap();
    assert_eq!(
        (
            stored.context.source_language.as_str(),
            stored.context.target_language.as_str()
        ),
        ("ja", "en")
    );
    let recalled = call(
        &app,
        "hieronymus_recall",
        json!({"series_slug":"book", "session_id":session["session_id"], "source_language":" JA ", "target_language":" EN ", "query":"absent"}),
    );
    assert_eq!(recalled["results"], json!([]));
    for value in ["", " \t"] {
        for key in ["source_language", "target_language"] {
            let mut args = json!({"series_slug":"book"});
            args[key] = json!(value);
            let error = app
                .call("hieronymus_session_start", &args, "agent")
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("must not be empty"),
                "{key}={value:?}: {error}"
            );
        }
    }
}

#[test]
fn conflicting_or_malformed_direction_predicates_are_rejected_everywhere() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    for scopes in [
        json!(["cws:direction:ru-main", "cws:direction:ru-literary"]),
        json!([""]),
        json!(["cws:direction:"]),
        json!(["cws:edition:../unsafe"]),
        json!([" cws:direction:ru-main"]),
    ] {
        for tool in [
            "hieronymus_session_start",
            "hieronymus_memory_add",
            "hieronymus_memory_search",
            "hieronymus_recall",
            "hieronymus_termbase_contract",
            "hieronymus_termbase_validate",
            "hieronymus_rag_search",
        ] {
            let args = json!({"series_slug":"book", "session_id":draft["session_id"], "story_scopes":scopes, "query":"Alex", "raw_text":"Alex", "translated_text":"А", "kind":"note", "text":"Alex"});
            let result = app.call(tool, &args, "agent");
            assert!(result.is_err(), "{tool} accepted {scopes}: {result:?}");
            assert!(
                result.unwrap_err().to_string().contains("story_scopes"),
                "{tool}"
            );
        }
    }
}

#[test]
fn advertised_schemas_expose_direction_scopes_and_rag_languages() {
    let registry = hiero::daemon::registry::McpRegistry::embedded();
    for name in [
        "hieronymus_session_start",
        "hieronymus_memory_add",
        "hieronymus_memory_search",
        "hieronymus_recall",
        "hieronymus_termbase_contract",
        "hieronymus_termbase_validate",
        "hieronymus_rag_search",
    ] {
        let tool = registry
            .list_tools()
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert!(
            tool.input_schema["properties"]["story_scopes"].is_object(),
            "{name}"
        );
        if name == "hieronymus_rag_search" {
            assert!(tool.input_schema["properties"]["source_language"].is_object());
            assert!(tool.input_schema["properties"]["target_language"].is_object());
        }
    }
}

fn semantic_lane(app: &Application) {
    use hieronymus::{
        semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider},
        semantic_jobs::{JobOutcome, RebuildConfig, RebuildInputs, SemanticJobStore},
        semantic_recall::{QueueOutcome, SemanticLane, queue_semantic_rebuild},
        semantic_store::SemanticSample,
        semantic_tokenizer::ModelTokenizer,
    };
    let mut provider = FakeEmbeddingProvider::new(384);
    let mut tokenizer = ModelTokenizer::from_bytes(include_bytes!(
        "../../hieronymus/tests/fixtures/minilm-tokenizer.json"
    ))
    .unwrap();
    let queued = queue_semantic_rebuild(app.config(), provider.identity()).unwrap();
    let (QueueOutcome::Enqueued(job) | QueueOutcome::AlreadyQueued(job)) = queued else {
        panic!("{queued:?}")
    };
    let outcome = SemanticJobStore::open(app.config())
        .unwrap()
        .run_rebuild(
            &job,
            RebuildInputs {
                provider: &mut provider,
                tokenizer: &mut tokenizer,
                sample: SemanticSample {
                    text: "Alex".into(),
                    series_slug: "book".into(),
                    token_ids: vec![101, 102],
                },
            },
            &RebuildConfig::default(),
        )
        .unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "{outcome:?}"
    );
    app.install_semantic_lane(SemanticLane::new(Box::new(provider), Box::new(tokenizer)))
        .unwrap();
    app.install_semantic_status(std::sync::Arc::new(|| {
        hieronymus::recall::SemanticAvailability::Ready
    }));
}

#[test]
fn public_reads_isolate_same_language_directions_and_multiple_target_languages() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let mut order = Value::Null;
    let mut concept = Value::Null;
    let mut selections = Vec::new();
    for (index, (direction, language, rendering)) in [
        ("ru-main", "ru", "А"),
        ("ru-literary", "ru", "Б"),
        ("en-main", "en", "В"),
    ]
    .into_iter()
    .enumerate()
    {
        let evidence_root = root.path().join(direction);
        std::fs::create_dir(&evidence_root).unwrap();
        let scopes = json!([format!("cws:direction:{direction}")]);
        let (mut draft, _) = common::authority::prepared_with(
            |tool, mut args| {
                if tool == "hieronymus_series_create" && index > 0 {
                    return Value::Null;
                }
                if tool == "hieronymus_order_register" && index > 0 {
                    return order.clone();
                }
                if tool == "hieronymus_concept_create" && index > 0 {
                    return concept.clone();
                }
                if tool == "hieronymus_session_start" {
                    args["story_scopes"] = scopes.clone();
                    args["target_language"] = json!(language);
                }
                if tool == "hieronymus_evidence_capture" {
                    args["binding"]["applicability"]["scope_predicates"] = scopes.clone();
                    args["binding"]["target_language"] = json!(language);
                    if args["kind"] == "aligned_rendering" {
                        let path = std::path::Path::new(args["snapshot"]["path"].as_str().unwrap());
                        let contents = format!("{rendering}\r\n\r\n{rendering}");
                        std::fs::write(path, &contents).unwrap();
                        args["snapshot"]["expected_hash"] =
                            json!(format!("{:x}", Sha256::digest(contents.as_bytes())));
                        args["selection"]["expected_text"] = json!(rendering);
                    }
                }
                if tool == "hieronymus_termbase_propose" {
                    args["canonical_translation"] = json!(rendering);
                    args["target_language"] = json!(language);
                }
                let result = call(&app, tool, args);
                if tool == "hieronymus_order_register" {
                    order = result.clone();
                }
                if tool == "hieronymus_concept_create" {
                    concept = result.clone();
                }
                result
            },
            &evidence_root,
        );
        draft["decision_id"] = json!(format!("12000000-0000-4000-8000-{:012}", index + 1));
        draft["expected_revision"] = json!(index + 1);
        draft["target_language"] = json!(language);
        draft["applicability"]["scope_predicates"] = scopes.clone();
        let applied = call(&app, "hieronymus_decide", draft.clone());
        assert!(applied.get("Applied").is_some(), "{applied}");
        let rule = draft["operation"]["Activate"]["candidate_id"].clone();
        let text = format!("Alex {direction} observation.");
        let claim = json!({"text":text,"concept_id":draft["concept_id"],"applicability":draft["applicability"]});
        let captured = call(
            &app,
            "hieronymus_short_term_add",
            json!({"session_id":draft["session_id"], "kind":"note", "text":text, "claims":[claim]}),
        );
        let file = evidence_root.join("rag.txt");
        std::fs::write(&file, &text).unwrap();
        call(
            &app,
            "hieronymus_rag_import",
            json!({"series_slug":"book", "path":file, "claims":{"0":[claim]}}),
        );
        selections.push((
            direction,
            language,
            rendering,
            draft,
            rule,
            captured["claims"][0]["claim_id"].clone(),
            text,
        ));
    }
    // Reopen before querying: all isolation must survive durable session reload.
    let reopened = Application::open(app.config()).unwrap();
    semantic_lane(&reopened);
    for (direction, language, rendering, draft, rule, claim_id, text) in &selections {
        let mut context = json!({"series_slug":"book", "source_language":"en", "target_language":language, "volume":"I", "chapter":"1", "story_timeline_id":order["timeline_id"], "story_scene_key":"a", "story_scopes":[format!("cws:direction:{direction}")], "raw_text":"Alex", "translated_text":rendering, "query":"Alex"});
        let contract = call(&reopened, "hieronymus_termbase_contract", context.clone());
        assert_eq!(
            contract["results"].as_array().unwrap().len(),
            1,
            "{direction}: {contract}"
        );
        assert_eq!(contract["results"][0]["id"], *rule);
        assert_eq!(
            call(&reopened, "hieronymus_termbase_validate", context.clone())["results"],
            json!([])
        );
        context["translated_text"] = json!("unrelated");
        let findings = call(&reopened, "hieronymus_termbase_validate", context.clone());
        assert_eq!(
            findings["results"].as_array().unwrap().len(),
            1,
            "{findings}"
        );
        assert_eq!(findings["results"][0]["term_id"], *rule);
        let rag = call(&reopened, "hieronymus_rag_search", context.clone());
        assert_eq!(
            rag["results"].as_array().unwrap().len(),
            1,
            "{direction}: {rag}"
        );
        assert_eq!(rag["results"][0]["text"], *text);
        context["session_id"] = draft["session_id"].clone();
        context.as_object_mut().unwrap().remove("story_scopes");
        let recalled = call(&reopened, "hieronymus_recall", context.clone());
        assert_eq!(
            recalled["deterministic_contract"].as_array().unwrap().len(),
            1,
            "{recalled}"
        );
        assert_eq!(recalled["deterministic_contract"][0]["id"], *rule);
        assert!(
            recalled["results"].as_array().unwrap().iter().any(|row| {
                row["claim_annotation"]["claims"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["claim_id"] == *claim_id)
            }),
            "{recalled}"
        );
        for row in recalled["results"].as_array().unwrap() {
            assert_eq!(row["text"], *text);
        }
        context["story_scopes"] = json!(["cws:direction:another"]);
        assert!(
            reopened
                .call("hieronymus_recall", &context, "agent")
                .unwrap_err()
                .to_string()
                .contains("conflicting")
        );
        context["story_scopes"] = json!([format!("cws:direction:{direction}")]);
        context["story_query_mode"] = json!("OmniscientResearch");
        let research = call(&reopened, "hieronymus_recall", context.clone());
        for row in research["results"].as_array().unwrap() {
            assert_eq!(row["text"], *text);
        }
        for row in research["non_current"].as_array().unwrap() {
            assert_ne!(row["claim_annotation"]["disposition"]["status"], "current");
        }
        call(
            &reopened,
            "hieronymus_session_complete",
            json!({"session_id":draft["session_id"]}),
        );
        context["story_query_mode"] = json!("Current");
        let search = call(&reopened, "hieronymus_memory_search", context);
        assert_eq!(
            search["results"].as_array().unwrap().len(),
            1,
            "{direction}: {search}"
        );
        assert_eq!(search["results"][0]["text"], *text);
    }
    // A Russian rule cannot apply in an English task even with its direction ID.
    let wrong = call(
        &reopened,
        "hieronymus_termbase_contract",
        json!({"series_slug":"book", "target_language":"en", "volume":"I", "chapter":"1", "story_scopes":["cws:direction:ru-main"], "raw_text":"Alex"}),
    );
    assert_eq!(wrong["results"], json!([]));
}

#[test]
fn implicit_default_session_selection_includes_exact_predicates() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    let mut ids = Vec::new();
    for direction in ["ru-main", "ru-literary", "ru-main"] {
        let session = call(
            &app,
            "hieronymus_session_start",
            json!({"series_slug":"book", "volume":"I", "chapter":"1", "story_scopes":[format!("cws:direction:{direction}")]}),
        );
        ids.push(session["session_id"].as_i64().unwrap());
    }
    let store = WorkspaceStore::open(app.config()).unwrap();
    let context = store.get_session(ids[1]).unwrap().context;
    assert_eq!(
        store.active_default_session(&context).unwrap().unwrap().id,
        ids[1]
    );
    let captured = call(
        &app,
        "hieronymus_memory_add",
        json!({"series_slug":"book", "volume":"I", "chapter":"1", "story_scopes":["cws:direction:ru-literary"], "kind":"note", "text":"Selected session"}),
    );
    assert!(
        store
            .list_short_term_memories(ids[1])
            .unwrap()
            .iter()
            .any(|m| json!(m.id) == captured["memory_id"])
    );
    assert!(store.list_short_term_memories(ids[2]).unwrap().is_empty());
    assert_ne!(draft["session_id"], json!(ids[0]));
}

#[test]
fn common_direction_and_edition_specific_claims_keep_distinct_applicability() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    for (name, scopes) in [
        ("common", json!([])),
        ("direction", json!(["cws:direction:ru-main"])),
        (
            "edition",
            json!(["cws:direction:ru-main", "cws:edition:en-first"]),
        ),
    ] {
        let text = format!("Alex {name} evidence.");
        let mut applicability = draft["applicability"].clone();
        applicability["scope_predicates"] = scopes;
        let path = root.path().join(format!("{name}.txt"));
        std::fs::write(&path, &text).unwrap();
        call(
            &app,
            "hieronymus_rag_import",
            json!({"series_slug":"book", "path":path, "claims":{"0":[{"text":text,"concept_id":draft["concept_id"],"applicability":applicability}]}}),
        );
    }
    semantic_lane(&app);
    for (scopes, mut expected) in [
        (
            json!(["cws:direction:ru-main"]),
            vec!["Alex common evidence.", "Alex direction evidence."],
        ),
        (
            json!(["cws:direction:ru-main", "cws:edition:en-first"]),
            vec![
                "Alex common evidence.",
                "Alex direction evidence.",
                "Alex edition evidence.",
            ],
        ),
        (
            json!(["cws:direction:ru-main", "cws:edition:another"]),
            vec!["Alex common evidence.", "Alex direction evidence."],
        ),
        (
            json!(["cws:direction:ru-literary"]),
            vec!["Alex common evidence."],
        ),
    ] {
        let result = call(
            &app,
            "hieronymus_rag_search",
            json!({"series_slug":"book", "query":"Alex", "volume":"I", "chapter":"1", "story_scopes":scopes}),
        );
        let mut actual: Vec<_> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["text"].as_str().unwrap())
            .collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected, "{result}");
    }
    let mut malformed = draft["applicability"].clone();
    malformed["scope_predicates"] = json!(["cws:direction:ru-main", "cws:direction:ru-literary"]);
    assert!(app.call("hieronymus_short_term_add", &json!({"session_id":draft["session_id"],"kind":"note","text":"Alex conflict", "claims":[{"text":"Alex conflict", "concept_id":draft["concept_id"], "applicability":malformed}]}), "agent").is_err());
}

#[test]
fn legacy_memory_add_persists_request_scope_provenance() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    common::authority::prepared(&app, root.path());
    let session = call(
        &app,
        "hieronymus_session_start",
        json!({"series_slug":"book", "story_scopes":["cws:direction:ru-main"]}),
    );
    let added = call(
        &app,
        "hieronymus_memory_add",
        json!({"series_slug":"book", "kind":"note", "text":"Scoped observation", "story_scopes":["cws:direction:ru-main"]}),
    );
    drop(app);
    let records = WorkspaceStore::open(&config)
        .unwrap()
        .list_short_term_memories(session["session_id"].as_i64().unwrap())
        .unwrap();
    let record = records
        .iter()
        .find(|record| json!(record.id) == added["memory_id"])
        .unwrap();
    assert_eq!(record.story_scopes, ["cws:direction:ru-main"]);
}

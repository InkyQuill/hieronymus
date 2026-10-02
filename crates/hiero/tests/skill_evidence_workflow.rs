mod common;
use hiero::{application::Application, daemon::McpRegistry};
use hieronymus::data_root::HieronymusConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn expand(template: &str, values: &BTreeMap<&str, Value>) -> Value {
    let mut text = template.to_string();
    for (key, value) in values {
        text = text.replace(key, &value.to_string());
    }
    serde_json::from_str(&text).unwrap()
}
// Check advertised object fields as well as exercising the actual public decoder.
fn object_fields(schema: &Value, object: &Value) {
    for name in schema["required"].as_array().unwrap() {
        assert!(
            object.get(name.as_str().unwrap()).is_some(),
            "missing {name}"
        );
    }
    for name in object.as_object().unwrap().keys() {
        assert!(
            schema["properties"].get(name).is_some(),
            "unadvertised {name}"
        );
    }
}
#[test]
fn documented_multilingual_capture_and_binding_use_observed_ids() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let call = |name: &str, args: Value| {
        app.call(name, &args, "agent")
            .unwrap_or_else(|error| panic!("{name}: {error}; {args}"))
    };
    let series = call(
        "hieronymus_series_create",
        json!({"slug":"evidence-example","title":"Example","source_language":"ja","target_language":"ru"}),
    );
    let concept = call(
        "hieronymus_concept_create",
        json!({"canonical_name":"犬","scope_type":"series","scope_key":"series:evidence-example"}),
    );
    let manifest = json!({"version":1,"series_id":series["id"],"timeline_name":"example","positions":[{"volume_key":"I","chapter_key":"1","scene_key":"a"}]}).to_string();
    let order_path = root.path().join("order.json");
    std::fs::write(&order_path, &manifest).unwrap();
    let order = call(
        "hieronymus_order_register",
        json!({"series_id":series["id"],"snapshot":{"kind":"file","path":order_path,"expected_hash":digest(&manifest)},"expected_revision":0}),
    );
    let scope = json!({"series_id":series["id"],"timeline_id":order["timeline_id"],"volume_key":"I","chapter_key":"1","scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Resolved","knowledge_gates":[{"viewpoint":"Narrator","known_from":null,"known_until":null}]});
    let source_path = root.path().join("source.txt");
    let target_path = root.path().join("target.txt");
    std::fs::write(&source_path, "猫と犬").unwrap();
    std::fs::write(&target_path, "Кот и пёс").unwrap();
    let mut values = BTreeMap::from([
        ("SERIES_ID", series["id"].clone()),
        ("CONCEPT_ID", concept["id"].clone()),
        ("POSITION_ID", order["positions"][0]["id"].clone()),
        ("APPLICABILITY", scope),
        ("SOURCE_PATH", json!(source_path)),
        ("TARGET_PATH", json!(target_path)),
        ("SOURCE_SHA256", json!(digest("猫と犬"))),
        ("TARGET_SHA256", json!(digest("Кот и пёс"))),
    ]);
    let files = hiero::agent_plugins::render(&config).unwrap();
    let resource = &files
        .iter()
        .find(|(p, _)| p.ends_with("codex/skills/hieronymus-learn/resources/evidence-capture.md"))
        .unwrap()
        .1;
    let templates: Vec<_> = resource
        .split("```json\n")
        .skip(1)
        .map(|s| s.split_once("\n```").unwrap().0)
        .collect();
    assert_eq!(templates.len(), 3);
    let registry = McpRegistry::embedded();
    let schema = &registry
        .list_tools()
        .iter()
        .find(|t| t.name == "hieronymus_evidence_capture")
        .unwrap()
        .input_schema;
    let source_args = expand(templates[0], &values);
    object_fields(schema, &source_args);
    object_fields(
        &schema["properties"]["selection"],
        &source_args["selection"],
    );
    object_fields(&schema["$defs"]["Binding"], &source_args["binding"]);
    let source = call("hieronymus_evidence_capture", source_args.clone());
    assert_eq!(source["selected_text"], "犬");
    assert_eq!(source["reference"]["span_start"], 6);
    assert_eq!(source["reference"]["span_end"], 9);
    assert_eq!(source["reference"]["content_hash"], digest("猫と犬"));
    values.insert("SOURCE_EVIDENCE_ID", source["reference"]["id"].clone());
    let target_args = expand(templates[1], &values);
    object_fields(schema, &target_args);
    object_fields(&schema["$defs"]["Binding"], &target_args["binding"]);
    let target = call("hieronymus_evidence_capture", target_args);
    assert_eq!(target["selected_text"], "пёс");
    assert_eq!(target["reference"]["span_start"], 10);
    assert_eq!(target["reference"]["span_end"], 16);
    assert_eq!(target["reference"]["content_hash"], digest("Кот и пёс"));
    assert_eq!(target["paragraph_start"], source["paragraph_start"]);
    assert_eq!(target["paragraph_end"], source["paragraph_end"]);
    for (start, end, expected) in [(2, 3, "犬"), (6, 8, "犬"), (6, 9, "猫")] {
        let mut invalid = source_args.clone();
        invalid["selection"] = json!({"start":start,"end":end,"expected_text":expected});
        assert!(
            app.call("hieronymus_evidence_capture", &invalid, "agent")
                .is_err()
        );
    }
    let mut selected_hash = source_args.clone();
    selected_hash["snapshot"]["expected_hash"] = json!(digest("犬"));
    assert!(
        app.call("hieronymus_evidence_capture", &selected_hash, "agent")
            .is_err()
    );
    std::fs::write(&source_path, "猫と犬\n").unwrap();
    assert!(
        app.call("hieronymus_evidence_capture", &source_args, "agent")
            .is_err()
    );
    let session = call(
        "hieronymus_session_start",
        json!({"series_slug":"evidence-example","source_language":"ja","target_language":"ru","story_timeline_id":order["timeline_id"],"story_scene_key":"a","volume":"I","chapter":"1"}),
    );
    let recalled = call(
        "hieronymus_recall",
        json!({"series_slug":"evidence-example","session_id":session["session_id"],"query":"犬"}),
    );
    values.insert("SESSION_ID", session["session_id"].clone());
    values.insert("HOST_SESSION_ID", json!("disposable-example-host"));
    values.insert("EXPECTED_REVISION", recalled["resulting_revision"].clone());
    values.insert("SOURCE_REFERENCE", source["reference"].clone());
    let binding = expand(templates[2], &values);
    let _daemon = common::start_daemon(root.path());
    assert_eq!(
        hiero::agent_prompt_delivery::bind_context(&config, &binding).unwrap()["bound"],
        true
    );
    let mut wrong = binding;
    wrong["selected_sources"] = json!([target["reference"]]);
    assert!(hiero::agent_prompt_delivery::bind_context(&config, &wrong).is_err());
    let completed = call(
        "hieronymus_session_complete",
        json!({"session_id":session["session_id"]}),
    );
    assert_eq!(completed["completed"], true);
}

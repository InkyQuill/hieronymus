//! Disposable public-operation fixture; no SQL order, evidence, receipt or rule staging.
use hiero::application::Application;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
fn call(app: &Application, name: &str, value: Value) -> Value {
    app.call(name, &value, "forged-user-label").unwrap()
}
fn snapshot(root: &Path, name: &str, text: &str) -> Value {
    let path = root.join(name);
    std::fs::write(&path, text).unwrap();
    json!({"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(text.as_bytes()))})
}
pub fn prepared(app: &Application, root: &Path) -> (Value, Value) {
    prepared_with(|name, value| call(app, name, value), root)
}
pub fn prepared_with(mut invoke: impl FnMut(&str, Value) -> Value, root: &Path) -> (Value, Value) {
    invoke(
        "hieronymus_series_create",
        json!({"slug":"book","title":"Book","source_language":"en","target_language":"ru"}),
    );
    let concept = invoke(
        "hieronymus_concept_create",
        json!({"canonical_name":"Alex","scope_type":"series","scope_key":"series:book"}),
    );
    let concept_id = concept["id"].as_i64().unwrap();
    let manifest=json!({"version":1,"series_id":1,"timeline_name":"story","positions":[{"volume_key":"I","chapter_key":"1","scene_key":"a"}]}).to_string();
    let order = invoke(
        "hieronymus_order_register",
        json!({"series_id":1,"snapshot":snapshot(root,"order.json",&manifest),"expected_revision":0}),
    );
    let scope = json!({"series_id":1,"timeline_id":order["timeline_id"],"volume_key":"I","chapter_key":"1","scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Resolved","knowledge_gates":[{"viewpoint":"All","known_from":null,"known_until":null}]});
    let session = invoke(
        "hieronymus_session_start",
        json!({"series_slug":"book","source_language":"en","target_language":"ru","volume":"I","chapter":"1","story_timeline_id":order["timeline_id"],"story_scene_key":"a"}),
    );
    let source = "Préface.\r\n\r\nAlex walks.\r\n\r\nAlex talks.";
    let target = "А\r\n\r\nА";
    let source_snapshot = snapshot(root, "source.txt", source);
    let target_snapshot = snapshot(root, "target.txt", target);
    let mut references = vec![];
    for (source_start, target_start) in [
        (source.find("Alex").unwrap(), 0),
        (source.rfind("Alex").unwrap(), 6),
    ] {
        let binding = json!({"concept_id":concept_id,"source_language":"en","target_language":"ru","applicability":scope,"position_id":order["positions"][0]["id"],"paragraph_start":0,"paragraph_end":0,"identity_anchor":true,"aligned_source_id":null,"rendering":null,"contradicts_rule":null,"conflict_kind":null});
        let captured = invoke(
            "hieronymus_evidence_capture",
            json!({"series_id":1,"snapshot":source_snapshot,"kind":"source_passage","selection":{"start":source_start,"end":source_start+4,"expected_text":"Alex"},"binding":binding}),
        );
        assert_eq!(captured["selected_text"], "Alex");
        let mut aligned = binding;
        aligned["aligned_source_id"] = captured["reference"]["id"].clone();
        let alignment = invoke(
            "hieronymus_evidence_capture",
            json!({"series_id":1,"snapshot":target_snapshot,"kind":"aligned_rendering","selection":{"start":target_start,"end":target_start+2,"expected_text":"А"},"binding":aligned}),
        );
        references.extend([
            captured["reference"].clone(),
            alignment["reference"].clone(),
        ]);
    }
    let candidate = invoke(
        "hieronymus_termbase_propose",
        json!({"series_slug":"book","category":"name","source_text":"Alex","canonical_translation":"А","concept_id":concept_id}),
    );
    let draft = json!({"version":1,"decision_id":"11000000-0000-4000-8000-000000000001","expected_revision":1,"evidence_refs":references,"series_id":1,"concept_id":concept_id,"source_language":"en","target_language":"ru","applicability":scope,"operation":{"Activate":{"candidate_id":candidate["id"],"candidate_revision":1}},"session_id":session["session_id"]});
    let event = json!({"version":1,"decision_id":"11000000-0000-4000-8000-000000000002","event_id":"test-event-1","expected_revision":2,"series_id":1,"session_id":session["session_id"],"source_language":"en","target_language":"ru","applicability":scope,"selected_sources":[references[0]],"selected_claims":[],"selected_rule":{"id":candidate["id"],"revision":2},"text":"translate this as Б","structured":null});
    (draft, event)
}

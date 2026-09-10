//! Public, typed setup shared by source and real installed multilingual runs.
//! Original corpus text/expected IDs remain in hybrid-relevance.json unchanged.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path};
fn snapshot(root: &Path, name: &str, text: &str) -> Value {
    let path = root.join(name);
    std::fs::write(&path, text).unwrap();
    json!({"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(text.as_bytes()))})
}
pub fn seed(
    mut tool: impl FnMut(&str, Value) -> Value,
    fixture: &Value,
    root: &Path,
    require_semantic_queue: bool,
) -> HashMap<String, Value> {
    std::fs::create_dir_all(root).unwrap();
    let mut contexts = HashMap::new();
    for series in fixture["series"].as_array().unwrap() {
        let created = tool("hieronymus_series_create", series.clone());
        let id = created["id"].clone();
        let slug = series["slug"].as_str().unwrap();
        let manifest=json!({"version":1,"series_id":id,"timeline_name":"baseline","positions":[{"volume_key":"I","chapter_key":"Opening","scene_key":"baseline"}]}).to_string();
        let order = tool(
            "hieronymus_order_register",
            json!({"series_id":id,"snapshot":snapshot(root,&format!("{slug}-order.json"),&manifest),"expected_revision":0}),
        );
        let mut context = json!({"series_slug":slug,"source_language":series["source_language"],"target_language":series["target_language"],"volume":"I","chapter":"Opening","story_timeline_id":order["timeline_id"],"story_scene_key":"baseline","story_viewpoint":"Narrator"});
        let session = tool("hieronymus_session_start", context.clone());
        context["session_id"] = session["session_id"].clone();
        context["series_id"] = id.clone();
        context["position_id"] = order["positions"][0]["id"].clone();
        context["applicability"] = json!({"series_id":id,"timeline_id":order["timeline_id"],"volume_key":"I","chapter_key":"Opening","scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Resolved","knowledge_gates":[{"viewpoint":"All","known_from":null,"known_until":null}]});
        contexts.insert(slug.to_string(), context);
    }
    for doc in fixture["documents"].as_array().unwrap() {
        let path = root.join(format!("{}.txt", doc["doc_id"].as_str().unwrap()));
        std::fs::write(&path, doc["text"].as_str().unwrap()).unwrap();
        let c = &contexts[doc["series_slug"].as_str().unwrap()];
        let reply = tool(
            "hieronymus_rag_import",
            json!({"series_slug":doc["series_slug"],"path":path,"claims":{"0":[{"text":doc["text"],"concept_id":null,"applicability":c["applicability"]}]}}),
        );
        assert_eq!(
            reply["chunk_count"], 1,
            "baseline explicit claim map must cover every chunk: {reply}"
        );
        if require_semantic_queue {
            assert!(reply["semantic_rebuild_job"].is_string(), "{reply}");
        }
    }
    for memory in fixture["memories"].as_array().unwrap() {
        let c = &contexts[memory["series_slug"].as_str().unwrap()];
        tool(
            "hieronymus_short_term_add",
            json!({"session_id":c["session_id"],"kind":memory["kind"],"text":memory["text"],"claims":[{"text":memory["text"],"concept_id":null,"applicability":c["applicability"]}]}),
        );
    }
    for (index, term) in fixture["terms"].as_array().unwrap().iter().enumerate() {
        let c = &contexts[term["series_slug"].as_str().unwrap()];
        let slug = term["series_slug"].as_str().unwrap();
        let concept = tool(
            "hieronymus_concept_create",
            json!({"canonical_name":term["source_text"],"scope_type":"series","scope_key":format!("series:{slug}")}),
        );
        let source = term["source_text"].as_str().unwrap();
        let target = term["canonical_translation"].as_str().unwrap();
        let source_text = format!("{source} is mentioned.\n\n{source} is discussed.");
        let target_text = format!("{target}\n\n{target}");
        let source_snapshot = snapshot(root, &format!("term-{index}-source.txt"), &source_text);
        let target_snapshot = snapshot(root, &format!("term-{index}-target.txt"), &target_text);
        let mut references = vec![];
        for (start, target_start) in [
            (0, 0),
            (
                source_text.rfind(source).unwrap(),
                target_text.rfind(target).unwrap(),
            ),
        ] {
            let binding = json!({"concept_id":concept["id"],"source_language":c["source_language"],"target_language":c["target_language"],"applicability":c["applicability"],"position_id":c["position_id"],"paragraph_start":0,"paragraph_end":0,"identity_anchor":true,"aligned_source_id":null,"rendering":null,"contradicts_rule":null,"conflict_kind":null});
            let evidence = tool(
                "hieronymus_evidence_capture",
                json!({"series_id":c["series_id"],"snapshot":source_snapshot,"kind":"source_passage","selection":{"start":start,"end":start+source.len(),"expected_text":source},"binding":binding}),
            );
            let mut aligned = binding;
            aligned["aligned_source_id"] = evidence["reference"]["id"].clone();
            let alignment = tool(
                "hieronymus_evidence_capture",
                json!({"series_id":c["series_id"],"snapshot":target_snapshot,"kind":"aligned_rendering","selection":{"start":target_start,"end":target_start+target.len(),"expected_text":target},"binding":aligned}),
            );
            references.extend([
                evidence["reference"].clone(),
                alignment["reference"].clone(),
            ]);
        }
        let mut proposal = term.clone();
        proposal["concept_id"] = concept["id"].clone();
        let proposed = tool("hieronymus_termbase_propose", proposal);
        let revision = tool(
            "hieronymus_termbase_contract",
            query_context(c, json!({"raw_text":source})),
        );
        let decision = tool(
            "hieronymus_decide",
            json!({"version":1,"decision_id":format!("77000000-0000-4000-8000-{:012}",index+1),"expected_revision":revision["resulting_revision"],"evidence_refs":references,"series_id":c["series_id"],"concept_id":concept["id"],"source_language":c["source_language"],"target_language":c["target_language"],"applicability":c["applicability"],"operation":{"Activate":{"candidate_id":proposed["id"],"candidate_revision":1}},"session_id":c["session_id"]}),
        );
        assert!(decision.get("Applied").is_some(), "{decision}");
    }
    contexts
}
pub fn query_context(context: &Value, mut args: Value) -> Value {
    for key in [
        "series_slug",
        "source_language",
        "target_language",
        "volume",
        "chapter",
        "story_timeline_id",
        "story_scene_key",
        "story_viewpoint",
    ] {
        args[key] = context[key].clone();
    }
    args
}

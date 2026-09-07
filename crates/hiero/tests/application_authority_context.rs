use hiero::application::Application;
use hieronymus::{
    data_root::HieronymusConfig, story_applicability::Viewpoint, workspace::WorkspaceStore,
};
use serde_json::{Value, json};
fn fixture() -> (tempfile::TempDir, Application, i64) {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book","source_language":"en","target_language":"ru"}),
        "agent",
    )
    .unwrap();
    let session = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug":"book","story_viewpoint":"Narrator"}),
            "agent",
        )
        .unwrap()["session_id"]
        .as_i64()
        .unwrap();
    (root, app, session)
}
#[test]
fn application_preserves_context_and_unknown_annotations() {
    let (_root, app, session) = fixture();
    assert_eq!(
        WorkspaceStore::open(app.config())
            .unwrap()
            .get_session(session)
            .unwrap()
            .context
            .story_viewpoint,
        Viewpoint::Narrator
    );
    app.call("hieronymus_short_term_add",&json!({"session_id":session,"kind":"note","text":"A secret observation without resolved chronology."}),"agent").unwrap();
    let result = app
        .call(
            "hieronymus_recall",
            &json!({"session_id":session,"series_slug":"book","query":"secret"}),
            "agent",
        )
        .unwrap();
    assert!(result["resulting_revision"].is_u64());
    assert_eq!(result["candidate_exhausted"], true);
    assert_eq!(result["results"], json!([]));
    let hidden = &result["non_current"][0];
    assert_eq!(hidden["text"], "");
    assert!(hidden["claim_annotation"].is_object());
}
#[test]
fn every_application_read_honors_required_receipt_including_sessionless() {
    let (_root, app, session) = fixture();
    for (tool, mut args) in [
        (
            "hieronymus_recall",
            json!({"session_id":session,"series_slug":"book","query":"secret"}),
        ),
        (
            "hieronymus_memory_search",
            json!({"series_slug":"book","query":"secret"}),
        ),
        (
            "hieronymus_termbase_contract",
            json!({"series_slug":"book","raw_text":"secret"}),
        ),
        (
            "hieronymus_termbase_validate",
            json!({"series_slug":"book","raw_text":"secret","translated_text":"тайна"}),
        ),
    ] {
        args["required_decision_id"] = json!("missing-receipt");
        let result = app.call(tool, &args, "agent");
        assert!(
            result.is_err(),
            "{tool} ignored required receipt: {result:?}"
        );
    }
}
#[test]
fn ordinary_supplied_claim_fields_are_validated_instead_of_dropped() {
    let (_root, app, session) = fixture();
    let result=app.call("hieronymus_short_term_add",&json!({"session_id":session,"kind":"note","text":"A source assertion.","claims":[{"text":"assertion","concept_id":null,"applicability":{"series_id":999,"timeline_id":null,"volume_key":null,"chapter_key":null,"scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Unspecified","knowledge_gates":[]}}]}),"agent");
    assert!(
        result.is_err(),
        "ordinary claims were silently dropped: {result:?}"
    );
    assert!(
        WorkspaceStore::open(app.config())
            .unwrap()
            .list_short_term_memories(session)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn observed_contract_and_validation_return_revision() {
    let (_root, app, _) = fixture();
    for tool in [
        "hieronymus_termbase_contract",
        "hieronymus_termbase_validate",
    ] {
        let result = app
            .call(
                tool,
                &json!({"series_slug":"book","raw_text":"absent","translated_text":"нет"}),
                "agent",
            )
            .unwrap();
        assert!(result["resulting_revision"].is_u64(), "{result}");
        assert_eq!(result["results"], Value::Array(vec![]));
    }
}
#[test]
fn active_registry_exposes_read_dependencies_and_ordinary_capture() {
    let registry = hiero::daemon::registry::McpRegistry::embedded();
    for name in [
        "hieronymus_recall",
        "hieronymus_memory_search",
        "hieronymus_rag_search",
        "hieronymus_termbase_contract",
        "hieronymus_termbase_validate",
    ] {
        let tool = registry
            .list_tools()
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert!(
            tool.input_schema["properties"]["required_decision_id"].is_object(),
            "{name}"
        );
        assert!(
            tool.input_schema["properties"]["story_viewpoint"].is_object(),
            "{name}"
        );
    }
    let tool = registry
        .list_tools()
        .iter()
        .find(|t| t.name == "hieronymus_short_term_add")
        .unwrap();
    assert!(tool.input_schema["properties"]["claims"].is_object());
}

#[path = "../../hieronymus/tests/support/current_story.rs"]
mod current_story;
#[test]
fn supplied_claims_recall_current_and_future_research_stay_separate_after_restart() {
    let (_root, app, _) = fixture();
    current_story::register(app.config(), "book");
    let session=app.call("hieronymus_session_start",&json!({"series_slug":"book","volume":"I","chapter":"Opening","story_viewpoint":"Narrator"}),"agent").unwrap()["session_id"].as_i64().unwrap();
    let current = current_story::claim(app.config(), "book", "The bridge is visible now.");
    let mut future =
        current_story::claim(app.config(), "book", "The bridge conceals a future secret.");
    let db = hieronymus::db::open_migrated(&app.config().database_path()).unwrap();
    let position: i64 = db
        .query_row(
            "select id from story_positions where chapter_key='Revelation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    future.applicability.knowledge_gates[0].known_from = Some(position);
    for claim in [current, future] {
        app.call(
            "hieronymus_short_term_add",
            &json!({"session_id":session,"kind":"note","text":claim.text,"claims":[claim]}),
            "agent",
        )
        .unwrap();
    }
    let reopened = Application::open(app.config()).unwrap();
    let args = json!({"session_id":session,"series_slug":"book","query":"bridge"});
    let current = reopened.call("hieronymus_recall", &args, "agent").unwrap();
    assert_eq!(current["results"].as_array().unwrap().len(), 1);
    assert_eq!(
        current["results"][0]["claim_annotation"]["disposition"]["status"],
        "current"
    );
    assert_eq!(current["non_current"][0]["text"], "");
    let mut research = args.clone();
    research["story_query_mode"] = json!("OmniscientResearch");
    let research = reopened
        .call("hieronymus_recall", &research, "agent")
        .unwrap();
    assert_eq!(research["results"].as_array().unwrap().len(), 1);
    assert!(
        research["non_current"][0]["text"]
            .as_str()
            .unwrap()
            .contains("future secret")
    );
    let current_again = reopened.call("hieronymus_recall", &args, "agent").unwrap();
    assert_eq!(current_again["non_current"][0]["text"], "");
    WorkspaceStore::open(app.config())
        .unwrap()
        .complete_session(session)
        .unwrap();
    let search=reopened.call("hieronymus_memory_search",&json!({"series_slug":"book","query":"bridge","volume":"I","chapter":"Opening","story_viewpoint":"Narrator"}),"agent").unwrap();
    assert_eq!(search["results"].as_array().unwrap().len(), 1);
    assert_eq!(search["non_current"][0]["text"], "");
}
#[test]
fn rag_and_facet_adapters_preserve_ordinary_claim_identity() {
    let (root, app, _) = fixture();
    current_story::register(app.config(), "book");
    let claim = current_story::claim(app.config(), "book", "Bridge evidence.");
    let concept = app
        .call(
            "hieronymus_concept_create",
            &json!({"canonical_name":"Bridge","scope_type":"series","scope_key":"series:book"}),
            "agent",
        )
        .unwrap();
    let concept_id = concept["id"].as_i64().unwrap();
    let mut facet_claim = claim.clone();
    facet_claim.concept_id = Some(concept_id);
    let facet = app
        .call(
            "hieronymus_concept_facet_add",
            &json!({"concept_id":concept_id,"value":"Bridge evidence.","claims":[facet_claim]}),
            "agent",
        )
        .unwrap();
    let path = root.path().join("source.txt");
    std::fs::write(&path, "Bridge evidence.\n").unwrap();
    let imported = app
        .call(
            "hieronymus_rag_import",
            &json!({"series_slug":"book","path":path,"claims":{"0":[claim]}}),
            "agent",
        )
        .unwrap();
    let db = hieronymus::db::open_migrated(&app.config().database_path()).unwrap();
    let facet_count: i64 = db
        .query_row(
            "select count(*) from claim_bindings where facet_id=?",
            [facet["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(facet_count, 1);
    let chunk: i64 = db
        .query_row(
            "select id from rag_chunks where source_id=?",
            [imported["source_id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from claim_bindings where rag_chunk_id=?",
            [chunk],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let malformed=app.call("hieronymus_rag_import",&json!({"series_slug":"book","path":path,"claim_lineage":{"0":[{"claim_id":999,"concept_id":null,"applicability":current_story::claim(app.config(),"book","x").applicability}]}}),"agent");
    assert!(malformed.is_err());
}
#[test]
fn application_receipt_dependency_checks_status_and_observed_revision() {
    use hieronymus::{authority_models::*, coherent_reads::CoherentReadError};
    let (_root, app, session) = fixture();
    current_story::register(app.config(), "book");
    let db = hieronymus::db::open_migrated(&app.config().database_path()).unwrap();
    // Stored receipt fixture qualifies the application read boundary. Origin
    // minting and authority transaction behavior have separate domain tests.
    let receipt = DecisionReceiptV1 {
        decision_id: "receipt".into(),
        resulting_revision: 3,
        affected_rules: vec![],
        affected_claims: vec![],
        effective_applicability: current_story::claim(app.config(), "book", "x").applicability,
        effective_exclusions: vec![],
        effect: "fixture".into(),
        consolidation_job_id: "receipt".into(),
        origin: OriginReceiptId("origin".into()),
        committed_at: "now".into(),
    };
    let encoded = serde_json::to_string(&DecisionResultV1::Applied { receipt }).unwrap();
    db.execute_batch("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values('origin','agent','fixture','event','fixture','{}','hash','now'); update authority_state set revision=3").unwrap();
    db.execute("insert into decision_records(decision_id,series_id,origin_id,actor_kind,expected_revision,resulting_revision,canonical_request,result_json,status,created_at) values('receipt',1,'origin','agent',2,3,'{}',?,'applied','now')",[encoded]).unwrap();
    for (tool, args) in [
        (
            "hieronymus_recall",
            json!({"series_slug":"book","session_id":session,"query":"absent","required_decision_id":"receipt"}),
        ),
        (
            "hieronymus_memory_search",
            json!({"series_slug":"book","query":"absent","required_decision_id":"receipt"}),
        ),
        (
            "hieronymus_termbase_contract",
            json!({"series_slug":"book","raw_text":"absent","required_decision_id":"receipt"}),
        ),
        (
            "hieronymus_termbase_validate",
            json!({"series_slug":"book","raw_text":"absent","translated_text":"нет","required_decision_id":"receipt"}),
        ),
    ] {
        let applied = app.call(tool, &args, "agent").unwrap();
        assert_eq!(applied["resulting_revision"], 3);
        db.execute("update decision_records set status='tentative'", [])
            .unwrap();
        assert!(matches!(
            app.call(tool, &args, "agent"),
            Err(hiero::application::AppError::Coherent(
                CoherentReadError::DecisionNotApplied
            ))
        ));
        db.execute("update decision_records set status='applied'", [])
            .unwrap();
        db.execute("update authority_state set revision=2", [])
            .unwrap();
        assert!(matches!(
            app.call(tool, &args, "agent"),
            Err(hiero::application::AppError::Coherent(
                CoherentReadError::DecisionNotApplied
            ))
        ));
        db.execute("update authority_state set revision=4", [])
            .unwrap();
        assert_eq!(
            app.call(tool, &args, "agent").unwrap()["resulting_revision"],
            4
        );
        db.execute("update authority_state set revision=3", [])
            .unwrap();
    }
    let registry = hiero::daemon::registry::McpRegistry::embedded();
    let error=registry.call(&app,"hieronymus_termbase_validate",&json!({"series_slug":"book","raw_text":"x","translated_text":"x","required_decision_id":"missing"}),"agent").unwrap();
    assert_eq!(error["isError"], true);
    assert_eq!(error["error_code"], "decision_not_applied");
}

#[test]
fn withheld_application_rows_do_not_leak_prose_through_kind_or_credibility() {
    let (_root, app, session) = fixture();
    app.call("hieronymus_short_term_add",&json!({"session_id":session,"kind":"secret hidden in kind","text":"The secret is unplaced.","source_credibility":"secret hidden in credibility"}),"agent").unwrap();
    let result = app
        .call(
            "hieronymus_recall",
            &json!({"series_slug":"book","session_id":session,"query":"secret"}),
            "agent",
        )
        .unwrap();
    let row = &result["non_current"][0];
    assert_eq!(row["title"], "");
    assert_eq!(row["kind"], "");
    assert_eq!(row["source_credibility"], "");
}

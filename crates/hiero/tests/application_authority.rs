mod common;
use hiero::application::Application;
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn legacy_approve_cannot_infer_human_from_actor_label() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book","source_language":"en","target_language":"ru"}),
        "local-user",
    )
    .unwrap();
    let proposed=app.call("hieronymus_termbase_propose",&json!({"series_slug":"book","category":"name","source_text":"Light","canonical_translation":"Свет","source_language":"en","target_language":"ru"}),"local-user").unwrap();
    let result=app.call("hieronymus_termbase_approve",&json!({"series_slug":"book","term_id":proposed["term_id"],"source_language":"en","target_language":"ru"}),"explicit_user");
    assert!(
        result.is_err(),
        "ordinary application actor string conferred authority: {result:?}"
    );
}
#[test]
fn draft_actor_and_origin_fields_are_not_transport_authority() {
    use hiero::trusted_ingress::DecisionDraftV1;
    let base = json!({"version":1,"decision_id":"00000000-0000-4000-8000-000000000001","expected_revision":0,"evidence_refs":[],"series_id":1,"concept_id":null,"source_language":"en","target_language":"ru","applicability":{"series_id":1,"timeline_id":null,"volume_key":null,"chapter_key":null,"scope_predicates":[],"valid_from":null,"valid_until":null,"metadata_state":"Resolved","knowledge_gates":[]},"operation":{"Archive":{"rule_id":1,"rule_revision":0}}});
    assert!(serde_json::from_value::<DecisionDraftV1>(base.clone()).is_ok());
    // Each forged field must reject independently; use a real valid applicability below.
    for key in ["actor_kind", "source_role", "origin", "event_id"] {
        let mut value = base.clone();
        value[key] = json!("explicit_user");
        assert!(serde_json::from_value::<DecisionDraftV1>(value).is_err());
    }
}

#[test]
fn public_producers_supply_learned_activation_and_immutable_refs() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    let result = app
        .call("hieronymus_decide", &draft, "explicit_user")
        .unwrap();
    assert!(result.get("Applied").is_some(), "{result}");
    let replay = app
        .call("hieronymus_decide", &draft, "explicit_user")
        .unwrap();
    assert!(replay.get("Replayed").is_some(), "{replay}");
}

#[test]
fn legacy_global_protected_projection_cannot_bypass_ingress() {
    use hieronymus::crystals::CrystalStore;
    use hieronymus::terminology::Termbase;
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    std::fs::create_dir_all(config.database_path().parent().unwrap()).unwrap();
    let mut db = rusqlite::Connection::open(config.database_path()).unwrap();
    db.execute_batch(include_str!("../../hieronymus/tests/fixtures/rust-v1.sql"))
        .unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into crystals(id,crystal_type,text,scope_type,scope_key,series_slug,source_language,target_language,strength,confidence,status,created_at,updated_at) values(1,'rule','Alex is А','series','series:book','book','en','ru',1,1,'active','now','now'); insert into term_rules(id,source_language,target_language,source_text,canonical_translation,status,rule_crystal_id,created_at,updated_at) values(1,'en','ru','Alex','А','active',1,'now','now');").unwrap();
    let tx = db.transaction().unwrap();
    hieronymus::schema_upgrade::apply_steps(&tx, 1, 5).unwrap();
    tx.commit().unwrap();
    assert!(
        db.query_row(
            "select legacy_protected=1 from rule_authority where rule_id=1",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    let app = Application::open(&config).unwrap();
    for (tool, args) in [
        (
            "hieronymus_termbase_approve",
            json!({"series_slug":"book","term_id":1}),
        ),
        ("hieronymus_rule_crystal_archive", json!({"crystal_id":1})),
    ] {
        let error = app.call(tool, &args, "explicit_user").unwrap_err();
        assert!(error.to_string().contains("unverified origin"));
    }
    let context =
        hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation");
    assert_eq!(
        Termbase::open(&config, &context)
            .unwrap()
            .get_rule(1)
            .unwrap()
            .status,
        "active"
    );
    assert_eq!(
        CrystalStore::open(&config).unwrap().get(1).unwrap().status,
        "active"
    );
}

#[test]
fn public_schemas_and_nested_drafts_reject_fabricated_fields() {
    let registry = hiero::daemon::McpRegistry::embedded();
    for name in [
        "hieronymus_decide",
        "hieronymus_correct",
        "hieronymus_order_register",
        "hieronymus_evidence_capture",
    ] {
        let tool = registry
            .list_tools()
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(tool.input_schema["additionalProperties"], false);
        assert!(tool.input_schema["properties"].get("actor_kind").is_none());
    }
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    let (draft, _) = common::authority::prepared(&app, root.path());
    for field in ["actor_kind", "source_role", "origin", "event_id"] {
        let mut forged = draft.clone();
        forged[field] = json!("explicit_user");
        assert!(matches!(
            app.call("hieronymus_decide", &forged, "local-user"),
            Err(hiero::application::AppError::Invalid(_))
        ));
    }
    let mut malformed = draft.clone();
    malformed["operation"]["Activate"]["actor_kind"] = json!("explicit_user");
    assert!(matches!(
        app.call("hieronymus_decide", &malformed, "local-user"),
        Err(hiero::application::AppError::Invalid(_))
    ));
    let mut fabricated = draft;
    fabricated["receipt_ref"] = json!("99000000-0000-4000-8000-000000000001");
    assert!(matches!(
        app.call("hieronymus_decide", &fabricated, "local-user"),
        Err(hiero::application::AppError::Authority(
            hieronymus::authority_models::DecisionErrorV1::UnverifiedOrigin
        ))
    ));
}

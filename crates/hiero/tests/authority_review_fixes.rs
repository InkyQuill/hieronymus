mod common;
use hiero::{
    application::Application,
    daemon::correction_worker::{self, CorrectionClock, CorrectionProvider, CorrectionSource},
};
use hieronymus::data_root::HieronymusConfig;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, atomic::AtomicBool};

fn post(
    daemon: &hiero::daemon::Daemon,
    headers: &[(String, String)],
    route: &str,
    value: &Value,
) -> (u16, Value) {
    let response = common::send_request(
        daemon.local_addr().port(),
        "POST",
        route,
        headers,
        &serde_json::to_vec(value).unwrap(),
    );
    (response.status, response.body())
}
fn browser(daemon: &hiero::daemon::Daemon) -> Vec<(String, String)> {
    let (_, cookie) = common::browser_session(daemon);
    vec![
        ("Cookie".into(), format!("hieronymus_session={cookie}")),
        (
            "Origin".into(),
            common::same_origin(daemon.local_addr().port()),
        ),
    ]
}
#[test]
fn unresolved_authentic_signal_is_durable_replayable_and_consumed_by_worker() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book","source_language":"en","target_language":"ru"}),
        "agent",
    )
    .unwrap();
    let headers = browser(&daemon);
    let event = json!({"version":1,"decision_id":"22000000-0000-4000-8000-000000000001","event_id":"ambiguous","expected_revision":0,"series_id":1,"text":format!("that memory is wrong{}"," ".repeat(20_000))});
    let (status, result) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["status"], "tentative");
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let persisted: Option<(String, String)> = db
        .query_row(
            "select canonical_request,result_json from decision_records where decision_id=?",
            [event["decision_id"].as_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    assert!(
        persisted.is_some(),
        "unresolved event must be durable pending work"
    );
    let (request, stored) = persisted.unwrap();
    let request: Value = serde_json::from_str(&request).unwrap();
    assert_eq!(request["kind"], "unresolved_signal");
    assert_eq!(request["reasons"], result["reasons"]);
    assert_eq!(request["detail"], result["detail"]);
    assert_eq!(request["text"], event["text"]);
    assert!(request.get("operation").is_none());
    assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), result);
    assert_eq!(result["resulting_revision"], 1);
    assert!(matches!(app.call("hieronymus_termbase_validate",&json!({"series_slug":"book","raw_text":"x","translated_text":"x","required_decision_id":event["decision_id"]}),"agent"), Err(hiero::application::AppError::Coherent(hieronymus::coherent_reads::CoherentReadError::DecisionNotApplied))));
    assert_eq!(
        post(&daemon, &headers, "/api/authority/correct", &event),
        (200, result.clone())
    );
    let observed = Arc::new(Mutex::new(None));
    let seen = observed.clone();
    let source: CorrectionSource = Arc::new(move || {
        let seen = seen.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "test".into(),
            call: Ok(Box::new(move |context| {
                *seen.lock().unwrap() = Some(context.clone());
                Ok(json!({"decisions":{"version":1,"mutations":[]}}))
            })),
        }
    });
    let clock: CorrectionClock = Arc::new(chrono::Utc::now);
    correction_worker::tick(&config, &AtomicBool::new(false), &clock, &source).unwrap();
    assert_eq!(
        observed.lock().unwrap().as_ref().unwrap()["request"],
        request
    );
    let state: String = db
        .query_row(
            "select state from consolidation_jobs where decision_id=?",
            [event["decision_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "complete");
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select revision from authority_state where series_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn console_current_rules_obey_partial_exclusions_and_viewpoint() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (_draft, mut event) = common::authority::prepared_with(
        |name, mut value| {
            if name == "hieronymus_order_register" {
                let path = root.path().join("two-chapters.json");
                let text=json!({"version":1,"series_id":1,"timeline_name":"story","positions":[{"volume_key":"I","chapter_key":"1","scene_key":"a"},{"volume_key":"I","chapter_key":"2","scene_key":"b"}]}).to_string();
                std::fs::write(&path, &text).unwrap();
                use sha2::{Digest, Sha256};
                value["snapshot"] = json!({"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(text.as_bytes()))});
            }
            if name == "hieronymus_evidence_capture" {
                value["binding"]["applicability"]["chapter_key"] = Value::Null;
                value["binding"]["applicability"]["knowledge_gates"] =
                    json!([{"viewpoint":"Narrator","known_from":null,"known_until":null}]);
            }
            app.call(name, &value, "agent").unwrap()
        },
        root.path(),
    );
    let headers = browser(&daemon);
    event["expected_revision"] = json!(1);
    event["selected_rule"] = Value::Null;
    event["applicability"]["chapter_key"] = Value::Null;
    event["applicability"]["knowledge_gates"] =
        json!([{"viewpoint":"Narrator","known_from":null,"known_until":null}]);
    event["text"] = json!("translate this as А");
    let (status, applied) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_eq!(status, 200, "{applied}");
    assert!(applied.get("Applied").is_some(), "{applied}");
    let old = applied["Applied"]["receipt"]["affected_rules"][0][0].clone();
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let binding: String = db
        .query_row(
            "select binding_json from evidence_records where id=?",
            [event["selected_sources"][0]["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let mut binding: Value = serde_json::from_str(&binding).unwrap();
    binding["applicability"]["chapter_key"] = json!("1");
    let path = root.path().join("narrow.txt");
    std::fs::write(&path, "Alex").unwrap();
    use sha2::{Digest, Sha256};
    let captured=app.call("hieronymus_evidence_capture",&json!({"series_id":1,"snapshot":{"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(b"Alex"))},"kind":"source_passage","selection":{"start":0,"end":4,"expected_text":"Alex"},"binding":binding}),"agent").unwrap();
    event["selected_sources"] = json!([captured["reference"]]);
    event["selected_rule"] =
        json!({"id":old,"revision":applied["Applied"]["receipt"]["affected_rules"][0][1]});
    event["decision_id"] = json!("22000000-0000-4000-8000-000000000003");
    event["event_id"] = json!("narrow-correction");
    event["expected_revision"] = json!(2);
    event["applicability"] = binding["applicability"].clone();
    event["text"] = json!("translate this as Б");
    let (status, result) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_eq!(status, 200, "{result}");
    assert!(result.get("Applied").is_some(), "{result}");
    let (_, options) = post(&daemon, &headers, "/api/authority/options", &json!({}));
    let rules = &options["sources"][0]["rules"];
    assert_eq!(
        rules.as_array().unwrap().len(),
        1,
        "excluded A must not be offered inside B chapter: {rules}"
    );
    assert_eq!(rules[0]["canonical"], "Б");
    let source_id = captured["reference"]["id"].clone();
    let (status, _) = post(
        &daemon,
        &headers,
        "/api/authority/selection",
        &json!({"series_id":1,"source_evidence_id":source_id,"rule_id":old}),
    );
    assert_eq!(
        status, 409,
        "excluded old rule cannot be frozen by direct ID"
    );
    // Same story position, incompatible concrete character viewpoint.
    binding["applicability"]["knowledge_gates"] = json!([{"viewpoint":{"Character":binding["concept_id"]},"known_from":null,"known_until":null}]);
    let path = root.path().join("character.txt");
    std::fs::write(&path, "Alex").unwrap();
    let character=app.call("hieronymus_evidence_capture",&json!({"series_id":1,"snapshot":{"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(b"Alex"))},"kind":"source_passage","selection":{"start":0,"end":4,"expected_text":"Alex"},"binding":binding}),"agent").unwrap();
    let (_, options) = post(&daemon, &headers, "/api/authority/options", &json!({}));
    let selected = options["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == character["reference"]["id"])
        .unwrap();
    assert!(
        selected["rules"].as_array().unwrap().is_empty(),
        "incompatible viewpoint must not offer B: {selected}"
    );
    binding["applicability"]["knowledge_gates"]
        .as_array_mut()
        .unwrap()
        .push(json!({"viewpoint":"Narrator","known_from":null,"known_until":null}));
    let path = root.path().join("ambiguous-viewpoint.txt");
    std::fs::write(&path, "Alex").unwrap();
    let ambiguous=app.call("hieronymus_evidence_capture",&json!({"series_id":1,"snapshot":{"kind":"file","path":path,"expected_hash":format!("{:x}",Sha256::digest(b"Alex"))},"kind":"source_passage","selection":{"start":0,"end":4,"expected_text":"Alex"},"binding":binding}),"agent").unwrap();
    let (_, options) = post(&daemon, &headers, "/api/authority/options", &json!({}));
    let selected = options["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == ambiguous["reference"]["id"])
        .unwrap();
    assert_eq!(selected["context_unresolved"], true);
    assert!(selected["rules"].as_array().unwrap().is_empty());
    // A remains active outside the correction chapter; status alone is insufficient.
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    let old = event["selected_rule"]["id"].as_i64().unwrap();
    assert_eq!(
        db.query_row("select status from term_rules where id=?", [old], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "active"
    );
}

#[test]
fn console_selection_preserves_registered_nondefault_pair() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (mut draft, mut event) = common::authority::prepared_with(
        |name, value| {
            let mut value: Value =
                serde_json::from_str(&value.to_string().replace("\"ru\"", "\"fr\"")).unwrap();
            if name == "hieronymus_termbase_propose" {
                value["source_language"] = json!("en");
                value["target_language"] = json!("fr");
            }
            let result = app.call(name, &value, "agent").unwrap();
            if name == "hieronymus_series_create" {
                let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
                db.execute("insert or ignore into series_language_tags(series_id,language_tag) values(1,'fr')",[]).unwrap();
            }
            result
        },
        root.path(),
    );
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    db.execute(
        "update series set default_target_language='ru' where id=1",
        [],
    )
    .unwrap();
    draft["target_language"] = json!("fr");
    event["target_language"] = json!("fr");
    let applied = app.call("hieronymus_decide", &draft, "agent").unwrap();
    assert!(applied.get("Applied").is_some(), "{applied}");
    let headers = browser(&daemon);
    for rule in [Value::Null, event["selected_rule"]["id"].clone()] {
        let (status, selection) = post(
            &daemon,
            &headers,
            "/api/authority/selection",
            &json!({"series_id":1,"source_evidence_id":event["selected_sources"][0]["id"],"rule_id":rule}),
        );
        assert_eq!(status, 200, "{selection}");
        assert_eq!(selection["target_language"], "fr");
        assert_eq!(selection["source_language"], "en");
    }
    let memory=app.call("hieronymus_short_term_add",&json!({"session_id":event["session_id"],"kind":"note","text":"A claim.","claims":[{"text":"A claim.","concept_id":draft["concept_id"],"applicability":event["applicability"]}]}),"agent").unwrap();
    let (status, claims) = post(
        &daemon,
        &headers,
        "/api/authority/selection",
        &json!({"series_id":1,"target":{"source":"short_term","id":memory["memory_id"]}}),
    );
    assert_eq!(status, 200, "{claims}");
    assert_eq!(
        claims["target_language"], "fr",
        "claim target owns session pair"
    );
    let (status, result) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_eq!(status, 200, "{result}");
    assert!(result.get("Applied").is_some(), "{result}");
}

#[test]
fn unresolved_signal_conflicts_staleness_and_transaction_failure_leave_no_partial_work() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    let (mut draft, mut event) = common::authority::prepared(&app, root.path());
    app.call("hieronymus_decide", &draft, "agent").unwrap();
    event["text"] = json!("unclear correction");
    let headers = browser(&daemon);
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    db.execute_batch("create trigger reject_signal_job before insert on consolidation_jobs begin select raise(abort,'injected job failure'); end;").unwrap();
    let (status, _) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_ne!(status, 200);
    assert_eq!(
        db.query_row(
            "select count(*) from origin_receipts where kind='console_user'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select revision from authority_state where series_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    db.execute_batch("drop trigger reject_signal_job").unwrap();
    let (status, original) = post(&daemon, &headers, "/api/authority/correct", &event);
    assert_eq!(status, 200, "{original}");
    for field in ["text", "event_id", "expected_revision"] {
        let mut changed = event.clone();
        // Same decision can never bind different event or text, even after revision advances.
        changed[field] = if field == "expected_revision" {
            json!(3)
        } else {
            json!("another interpretation")
        };
        let (status, result) = post(&daemon, &headers, "/api/authority/correct", &changed);
        assert_eq!(status, 409);
        assert_eq!(result["error"], "IdempotencyConflict");
    }
    let other = browser(&daemon);
    assert_eq!(
        post(&daemon, &other, "/api/authority/correct", &event).0,
        409
    );
    let mut stale = event.clone();
    stale["decision_id"] = json!("22000000-0000-4000-8000-000000000004");
    stale["event_id"] = json!("new-stale");
    let (status, result) = post(&daemon, &headers, "/api/authority/correct", &stale);
    assert_eq!(status, 409);
    assert_eq!(result["error"]["RevisionConflict"]["current_revision"], 3);
    stale["event_id"] = event["event_id"].clone();
    stale["expected_revision"] = json!(3);
    assert_eq!(
        post(&daemon, &headers, "/api/authority/correct", &stale).0,
        409
    );
    draft["decision_id"] = event["decision_id"].clone();
    draft["expected_revision"] = json!(3);
    let origins: i64 = db
        .query_row("select count(*) from origin_receipts", [], |r| r.get(0))
        .unwrap();
    assert!(matches!(
        app.call("hieronymus_decide", &draft, "agent"),
        Err(hiero::application::AppError::Authority(
            hieronymus::authority_models::DecisionErrorV1::IdempotencyConflict
        ))
    ));
    assert_eq!(
        db.query_row("select count(*) from origin_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        origins
    );
    assert_eq!(
        post(&daemon, &headers, "/api/authority/correct", &event),
        (200, original)
    );
    assert_eq!(
        db.query_row("select count(*) from decision_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

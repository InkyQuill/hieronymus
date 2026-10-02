use hieronymus::{
    claim_reads::ClaimTarget,
    comparison_config::{self, Assignment, ComparisonConfig},
    data_root::HieronymusConfig,
    memory_comparison::{Comparator, Decision, Snapshot},
    provider_config::{ProviderCatalog, ProviderProfile, save_provider_catalog},
    provider_http::{HttpError, HttpResponse, ProviderTransport},
    registry::Registry,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Wire {
    replies: Mutex<VecDeque<Result<Value, HttpError>>>,
    calls: Mutex<Vec<(String, Value)>>,
}
impl ProviderTransport for Wire {
    fn get_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        panic!("unexpected discovery")
    }
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        assert!(
            !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type")),
            "the underlying transport owns content type"
        );
        self.calls.lock().unwrap().push((url.into(), body.clone()));
        if url.ends_with("/api/show") {
            return Ok(HttpResponse{status:200,body:json!({"model_info":{"llama.context_length":8192,"general.architecture":"llama"},"template":"","parameters":"num_ctx 8192"}).to_string()});
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("bounded request count")
            .map(|v| HttpResponse {
                status: 200,
                body: v.to_string(),
            })
    }
}
fn assignment(provider: &str) -> Assignment {
    Assignment {
        provider: provider.into(),
        model: "test-model".into(),
    }
}
fn fixture(
    primary: &str,
    fallback: Option<&str>,
    key: bool,
) -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    Registry::open(&config)
        .unwrap()
        .create_series("book", "Book", "ja", "ru", None)
        .unwrap();
    let mut relevance = hieronymus::relevance_config::load(&config).unwrap();
    if key {
        relevance.key = hieronymus::secret::Secret::new("private-test-key".into());
    }
    hieronymus::relevance_config::save(&config, &relevance).unwrap();
    let catalog = ProviderCatalog::default()
        .with_provider(
            "cloud",
            ProviderProfile::new(
                "Cloud",
                "openai",
                "http://loopback.invalid/v1",
                "private-cloud-key",
                2.0,
            ),
        )
        .with_provider(
            "local",
            ProviderProfile::new("Local", "ollama", "http://loopback.invalid", "", 2.0),
        );
    save_provider_catalog(&config, &catalog).unwrap();
    comparison_config::save(
        &config,
        &ComparisonConfig {
            primary: Some(assignment(primary)),
            fallback: fallback.map(assignment),
            ..Default::default()
        },
    )
    .unwrap();
    (root, config)
}
fn pair() -> (Snapshot, Snapshot) {
    let a = Snapshot {
        target: ClaimTarget::Crystal(1),
        text: "the gate is closed".into(),
        scope: json!({"series":1,"time":1,"viewpoint":"narrator"}),
        version: json!({"revision":1}),
        protected: false,
    };
    let b = Snapshot {
        target: ClaimTarget::Crystal(2),
        text: "the gate remains shut".into(),
        ..a.clone()
    };
    (a, b)
}
fn jev(decision: &str) -> Value {
    let probabilities: serde_json::Map<String, Value> = [
        "equivalent",
        "distinct",
        "contradictory",
        "insufficient_context",
    ]
    .into_iter()
    .map(|label| {
        (
            label.into(),
            json!(if label == decision { 0.99 } else { 0.01 / 3.0 }),
        )
    })
    .collect();
    json!({"answers":{"comparison":{"type":"choice","choice":decision,"confidence":0.99,"probabilities":probabilities}}})
}
fn cloud(decision: &str) -> Value {
    json!({"choices":[{"message":{"content":json!({"decision":decision}).to_string()}}]})
}
fn wire(replies: Vec<Result<Value, HttpError>>) -> Arc<Wire> {
    Arc::new(Wire {
        replies: Mutex::new(replies.into()),
        calls: Mutex::new(vec![]),
    })
}

#[test]
fn jev_success_and_uncertainty_are_terminal_and_durable() {
    for decision in ["equivalent", "insufficient_context"] {
        let (_root, config) = fixture("jev", Some("cloud"), true);
        let transport = wire(vec![Ok(jev(decision))]);
        let (a, b) = pair();
        let result = Comparator::open(&config)
            .unwrap()
            .with_transport(transport.clone())
            .compare(&a, &b)
            .unwrap();
        assert_eq!(
            result.decision,
            if decision == "equivalent" {
                Decision::Equivalent
            } else {
                Decision::InsufficientContext
            }
        );
        let replay = Comparator::open(&config)
            .unwrap()
            .with_transport(transport.clone())
            .compare(&a, &b)
            .unwrap();
        assert_eq!(replay.decision, result.decision);
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
}
#[test]
fn missing_key_timeout_and_invalid_answer_use_only_explicit_fallback() {
    for fault in ["missing", "timeout", "invalid"] {
        let (_root, config) = fixture("jev", Some("cloud"), fault != "missing");
        let mut replies = vec![];
        if fault == "timeout" {
            replies.push(Err(HttpError::Timeout { millis: 10 }));
        } else if fault == "invalid" {
            replies.push(Ok(json!({"wrong":true})));
        }
        replies.push(Ok(cloud("distinct")));
        let transport = wire(replies);
        let (a, b) = pair();
        let result = Comparator::open(&config)
            .unwrap()
            .with_transport(transport.clone())
            .compare(&a, &b)
            .unwrap();
        assert_eq!(result.decision, Decision::Distinct);
        assert_eq!(result.provider, "cloud");
        assert!(result.fallback_reason.is_some());
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
    }
}
#[test]
fn ollama_fallback_fits_complete_request_and_disables_truncation() {
    let (_root, config) = fixture("jev", Some("local"), false);
    let transport = wire(vec![Ok(
        json!({"message":{"content":"{\"decision\":\"equivalent\"}"},"done":true,"done_reason":"stop"}),
    )]);
    let (a, b) = pair();
    let result = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone())
        .compare(&a, &b)
        .unwrap();
    assert_eq!(result.provider, "local");
    assert_eq!(result.decision, Decision::Equivalent);
    assert_eq!(result.reason, "validated pair assessment");
    assert!(result.fallback_reason.is_some());
    let calls = transport.calls.lock().unwrap();
    let body = &calls.last().unwrap().1;
    assert_eq!(body["truncate"], false);
    assert_eq!(body["shift"], false);
    assert!(body["options"]["num_ctx"].as_u64().unwrap() <= 8192);
}
#[test]
fn all_providers_unavailable_are_cached_without_retry_storm() {
    let (_root, config) = fixture("jev", Some("cloud"), true);
    let transport = wire(vec![
        Err(HttpError::Timeout { millis: 10 }),
        Err(HttpError::Network("private-cloud-key".into())),
    ]);
    let (a, b) = pair();
    for _ in 0..3 {
        let r = Comparator::open(&config)
            .unwrap()
            .with_transport(transport.clone())
            .compare(&a, &b)
            .unwrap();
        assert_eq!(r.decision, Decision::InsufficientContext);
        assert!(
            !serde_json::to_string(&r)
                .unwrap()
                .contains("private-cloud-key")
        );
    }
    assert_eq!(transport.calls.lock().unwrap().len(), 2);
}
#[test]
fn adversarial_scope_polarity_number_and_name_changes_cannot_merge() {
    let (_root, config) = fixture("jev", None, true);
    let transport = wire(vec![]);
    let mut service = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone());
    let (mut a, mut b) = pair();
    for (left, right) in [
        (
            "Mira trusts Ren completely",
            "Mira never trusts Ren completely",
        ),
        ("Mira owns 3 horses", "Mira owns 4 horses"),
        ("Mira trusts Ren", "Mira trusts Ben"),
    ] {
        a.text = left.into();
        b.text = right.into();
        assert_ne!(
            service.compare(&a, &b).unwrap().decision,
            Decision::Equivalent
        );
    }
    b.text = a.text.clone();
    for scope in [
        json!({"series":2}),
        json!({"time":2}),
        json!({"viewpoint":"Mira"}),
    ] {
        b.scope = scope;
        assert_ne!(
            service.compare(&a, &b).unwrap().decision,
            Decision::Equivalent
        );
    }
    assert!(transport.calls.lock().unwrap().is_empty());
}
#[test]
fn settings_keep_secrets_out_of_public_readiness_and_reject_implicit_fallback() {
    let (_root, config) = fixture("jev", Some("cloud"), true);
    let settings = comparison_config::load(&config).unwrap();
    let public = comparison_config::public_payload(&config, &settings);
    assert_eq!(public["primary_ready"], true);
    assert_eq!(public["fallback_ready"], true);
    assert!(!public.to_string().contains("private-test-key"));
    assert!(
        ComparisonConfig {
            primary: None,
            fallback: Some(assignment("cloud")),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn pair_budget_invalid_cache_and_outage_cooldown_are_recoverable() {
    let (_root, config) = fixture("jev", None, true);
    let mut settings = comparison_config::load(&config).unwrap();
    settings.max_pairs_per_run = 1;
    comparison_config::save(&config, &settings).unwrap();
    let transport = wire(vec![
        Ok(jev("equivalent")),
        Ok(jev("distinct")),
        Err(HttpError::Timeout { millis: 1 }),
        Ok(jev("equivalent")),
    ]);
    let mut service = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone());
    let (a, mut b) = pair();
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::Equivalent
    );
    b.version = json!({"revision":2});
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::InsufficientContext
    );
    service.begin_run();
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::Distinct
    );
    let db = hieronymus::db::open_migrated(&config.database_path()).unwrap();
    db.execute(
        "update memory_comparison_cache set result_json='broken'",
        [],
    )
    .unwrap();
    service.begin_run();
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::InsufficientContext
    );
    // Immediate replay is local, but a later run can recover without configuration edits.
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::InsufficientContext
    );
    db.execute(
        "update memory_comparison_cache set created_at=datetime('now','-6 minutes')",
        [],
    )
    .unwrap();
    service.begin_run();
    assert_eq!(
        service.compare(&a, &b).unwrap().decision,
        Decision::Equivalent
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 4);
}

#[test]
fn inconsistent_jev_probabilities_never_authorize_equivalence() {
    let (_root, config) = fixture("jev", None, true);
    let mut invalid = jev("equivalent");
    invalid["answers"]["comparison"]["probabilities"]["distinct"] = json!(0.99);
    let transport = wire(vec![Ok(invalid)]);
    let (a, b) = pair();
    assert_eq!(
        Comparator::open(&config)
            .unwrap()
            .with_transport(transport)
            .compare(&a, &b)
            .unwrap()
            .decision,
        Decision::InsufficientContext
    );
}

#[test]
#[ignore = "requires HIERO_TEST_COMPARISON_CREDENTIAL_ROOT; sends synthetic pairs to configured Jev"]
fn real_jev_synthetic_pair_calibration() {
    let credential_root = std::env::var("HIERO_TEST_COMPARISON_CREDENTIAL_ROOT")
        .expect("explicit credential fixture root required");
    let credentials = hieronymus::relevance_config::load(&HieronymusConfig::new(credential_root))
        .expect("private credential fixture");
    assert!(!credentials.key.is_blank(), "Jev key fixture required");
    let (_root, config) = fixture("jev", None, false);
    hieronymus::relevance_config::save(&config, &credentials).unwrap();
    let settings = ComparisonConfig {
        primary: Some(Assignment {
            provider: "jev".into(),
            model: credentials.model,
        }),
        ..Default::default()
    };
    comparison_config::save(&config, &settings).unwrap();
    let mut service = Comparator::open(&config).unwrap();
    let mut accepted_equivalents = 0;
    let cases = [
        ("the gate is closed", "the gate is shut", true),
        ("she purchased a bicycle", "she bought a bicycle", true),
        ("the door is locked", "the door is unlocked", false),
        (
            "the visitor arrived before dawn",
            "the visitor arrived after dawn",
            false,
        ),
        ("the red vial is safe", "the red vial is poisonous", false),
        ("he borrowed the book", "he lent the book", false),
    ];
    let snapshots = cases
        .iter()
        .map(|(left, right, _)| {
            let (mut a, mut b) = pair();
            a.text = (*left).into();
            b.text = (*right).into();
            a.scope["subject_identity"] = json!("same verified subject");
            b.scope = a.scope.clone();
            (a, b)
        })
        .collect::<Vec<_>>();
    let pairs = snapshots.iter().map(|(a, b)| (a, b)).collect::<Vec<_>>();
    let outcomes = service.compare_batch(&pairs).unwrap();
    for ((left, right, equivalent), result) in cases.into_iter().zip(outcomes) {
        eprintln!(
            "synthetic pair: {left:?} / {right:?}: {:?} ({}; {:?})",
            result.decision, result.reason, result.fallback_reason
        );
        if equivalent {
            accepted_equivalents += usize::from(result.decision == Decision::Equivalent);
        } else {
            assert_ne!(result.decision, Decision::Equivalent);
        }
        assert_eq!(
            result.reason, "validated pair assessment",
            "an unavailable provider is not calibration evidence"
        );
    }
    assert!(
        accepted_equivalents > 0,
        "calibration must exercise positive recognition as well as safe abstention"
    );
}

#[test]
fn jev_batches_named_pairs_and_only_falls_back_for_missing_answers() {
    let (_root, config) = fixture("jev", Some("cloud"), true);
    let answer = jev("equivalent")["answers"]["comparison"].clone();
    let transport = wire(vec![
        Ok(json!({"model":"test-model","usage":{},"answers":{"comparison_0":answer}})),
        Ok(cloud("distinct")),
    ]);
    let (a, b) = pair();
    let mut c = a.clone();
    c.target = ClaimTarget::Crystal(3);
    c.text = "the door is closed".into();
    let mut d = b.clone();
    d.target = ClaimTarget::Crystal(4);
    d.text = "the door is shut".into();
    let mut comparator = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone());
    let outcomes = comparator.compare_batch(&[(&a, &b), (&c, &d)]).unwrap();
    assert_eq!(outcomes[0].decision, Decision::Equivalent);
    assert_eq!(outcomes[0].provider, "jev");
    assert_eq!(outcomes[1].decision, Decision::Distinct);
    assert_eq!(outcomes[1].provider, "cloud");
    assert!(outcomes[1].fallback_reason.is_some());
    assert_eq!(transport.calls.lock().unwrap().len(), 2);
    assert_eq!(
        transport.calls.lock().unwrap()[0].1["questions"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    comparator.compare_batch(&[(&a, &b), (&c, &d)]).unwrap();
    assert_eq!(
        transport.calls.lock().unwrap().len(),
        2,
        "each answer cached independently"
    );
}

#[test]
fn batched_uncertainty_is_terminal_and_answer_order_is_irrelevant() {
    let (_root, config) = fixture("jev", Some("cloud"), true);
    let yes = jev("equivalent")["answers"]["comparison"].clone();
    let uncertain = jev("insufficient_context")["answers"]["comparison"].clone();
    let transport = wire(vec![Ok(
        json!({"model":"test-model","usage":{},"answers":{"comparison_1":yes,"comparison_0":uncertain}}),
    )]);
    let (a, b) = pair();
    let mut c = a.clone();
    c.target = ClaimTarget::Crystal(3);
    let result = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone())
        .compare_batch(&[(&a, &b), (&c, &b)])
        .unwrap();
    assert_eq!(result[0].decision, Decision::InsufficientContext);
    assert_eq!(result[1].decision, Decision::Equivalent);
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

#[test]
fn jev_question_batches_respect_shared_pair_budget() {
    let (_root, config) = fixture("jev", None, true);
    let mut settings = comparison_config::load(&config).unwrap();
    settings.max_pairs_per_run = 10;
    comparison_config::save(&config, &settings).unwrap();
    let reply = |count: usize| {
        let answers = (0..count)
            .map(|i| {
                (
                    format!("comparison_{i}"),
                    jev("distinct")["answers"]["comparison"].clone(),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        Ok(json!({"model":"test-model","usage":{},"answers":answers}))
    };
    let transport = wire(vec![reply(8), reply(2)]);
    let snapshots = (0..12)
        .map(|i| {
            let (mut a, b) = pair();
            a.target = ClaimTarget::Crystal(i + 10);
            (a, b)
        })
        .collect::<Vec<_>>();
    let pairs = snapshots.iter().map(|(a, b)| (a, b)).collect::<Vec<_>>();
    let results = Comparator::open(&config)
        .unwrap()
        .with_transport(transport.clone())
        .compare_batch(&pairs)
        .unwrap();
    assert!(
        results[..10]
            .iter()
            .all(|r| r.decision == Decision::Distinct)
    );
    assert!(
        results[10..]
            .iter()
            .all(|r| r.reason == "comparison run budget exhausted")
    );
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1["questions"].as_object().unwrap().len(), 8);
    assert_eq!(calls[1].1["questions"].as_object().unwrap().len(), 2);
}

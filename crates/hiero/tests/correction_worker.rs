//! Actual supervised worker tests: all providers, clocks and databases disposable.
use chrono::{DateTime, Utc};
use hiero::daemon::{
    correction_worker::{self, CorrectionClock, CorrectionProvider, CorrectionSource},
    workers::WorkerGroup,
};
use hieronymus::{
    consolidation::{ConsolidationStore, prepare_correction_draft, select_correction_context},
    data_root::HieronymusConfig,
    db::open_migrated,
    dream_config::default_dream_config,
    dream_locks::dream_cycle_lock,
    dream_output::parse_decisions,
};
use rusqlite::params;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
const JOB: &str = "10000000-0000-4000-8000-000000000001";
fn now() -> DateTime<Utc> {
    "2026-09-07T00:00:00Z".parse().unwrap()
}
fn fixture() -> (tempfile::TempDir, HieronymusConfig) {
    let dir = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(dir.path());
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into authority_state(series_id) values(1); insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values('origin','agent','test','event','signal','{}','hash','now');").unwrap();
    add_job(&config, JOB, 1);
    (dir, config)
}
fn add_job(config: &HieronymusConfig, id: &str, series: i64) {
    let db = open_migrated(&config.database_path()).unwrap();
    let t = now().to_rfc3339();
    db.execute("insert into decision_records(decision_id,series_id,origin_id,actor_kind,expected_revision,resulting_revision,canonical_request,result_json,status,created_at) values(?1,?3,'origin','agent',0,0,'{}','{}','tentative',?2)",params![id,t,series]).unwrap();
    db.execute(
        "insert or ignore into provider_recovery_state values('default','unconfigured',?1,?1)",
        [&t],
    )
    .unwrap();
    db.execute("insert into consolidation_jobs(decision_id,state,provider_slot_id,created_at,updated_at) values(?1,'pending','default',?2,?2)",params![id,t]).unwrap();
}
fn clock() -> (Arc<Mutex<DateTime<Utc>>>, CorrectionClock) {
    let time = Arc::new(Mutex::new(now()));
    let c = time.clone();
    (time, Arc::new(move || *c.lock().unwrap()))
}
fn source(calls: Arc<AtomicUsize>, available: Arc<AtomicBool>) -> CorrectionSource {
    Arc::new(move || {
        let calls = calls.clone();
        let available = available.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "stable".into(),
            call: Ok(Box::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                if available.load(Ordering::SeqCst) {
                    Ok(json!({"decisions":{"version":1,"mutations":[]}}))
                } else {
                    Err("offline".into())
                }
            })),
        }
    })
}
fn state(config: &HieronymusConfig) -> String {
    open_migrated(&config.database_path())
        .unwrap()
        .query_row(
            "select state from consolidation_jobs where decision_id=?",
            [JOB],
            |r| r.get(0),
        )
        .unwrap()
}
fn snapshot(config: &HieronymusConfig) -> String {
    let db = open_migrated(&config.database_path()).unwrap();
    let mut stmt=db.prepare("select json_array(state,attempts,next_attempt_at,lease_until,lease_token,result_generation,last_attempt_at) from consolidation_jobs order by decision_id").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
}
fn tick(config: &HieronymusConfig, clock: &CorrectionClock, source: &CorrectionSource) {
    correction_worker::tick(config, &AtomicBool::new(false), clock, source).unwrap()
}
fn wait(condition: impl Fn() -> bool) {
    let limit = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < limit, "condition did not become true");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn park(
    config: &HieronymusConfig,
    time: &Arc<Mutex<DateTime<Utc>>>,
    clock: &CorrectionClock,
    source: &CorrectionSource,
) {
    for delay in [30, 120, 600, 3600, 21600, 21600] {
        tick(config, clock, source);
        *time.lock().unwrap() += chrono::Duration::seconds(delay);
    }
    assert_eq!(state(config), "degraded");
}
#[test]
fn startup_and_idle_unchanged_configuration_recover_a_single_parked_job() {
    let (_dir, config) = fixture();
    let (time, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let available = Arc::new(AtomicBool::new(false));
    let source = source(calls.clone(), available.clone());
    park(&config, &time, &clock, &source);
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    // Startup performs one due recovery without any unrelated work.
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    correction_worker::start_with_source(config.clone(), &workers, clock.clone(), source.clone())
        .unwrap();
    wait(|| calls.load(Ordering::SeqCst) == 7);
    wait(|| state(&config) == "degraded");
    available.store(true, Ordering::SeqCst);
    *time.lock().unwrap() += chrono::Duration::seconds(21599);
    std::thread::sleep(Duration::from_millis(350));
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    *time.lock().unwrap() += chrono::Duration::seconds(60);
    wait(|| state(&config) == "complete");
    assert_eq!(calls.load(Ordering::SeqCst), 8);
    workers.stop_and_join().unwrap();
    assert_eq!(workers.live_count(), 0);
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn admission_busy_has_no_attempt_or_deadline_effects_and_restart_cannot_reset_cap() {
    let (_dir, config) = fixture();
    let (time, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let source = source(calls.clone(), Arc::new(AtomicBool::new(false)));
    let lock = dream_cycle_lock(&config, "ordinary dream test").unwrap();
    let before = snapshot(&config);
    tick(&config, &clock, &source);
    assert_eq!(snapshot(&config), before);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(lock);
    park(&config, &time, &clock, &source);
    tick(&config, &clock, &source);
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    let before = snapshot(&config);
    let changed: CorrectionSource = Arc::new(|| CorrectionProvider {
        slot: "default".into(),
        fingerprint: "changed model".into(),
        call: Ok(Box::new(|_| panic!("cap bypassed"))),
    });
    tick(&config, &clock, &changed);
    assert_eq!(snapshot(&config), before);
    // New worker and changed selected configuration retain the same persisted slot.
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    correction_worker::start_with_source(config.clone(), &workers, clock, changed).unwrap();
    std::thread::sleep(Duration::from_millis(350));
    workers.stop_and_join().unwrap();
    assert_eq!(snapshot(&config), before);
}
#[test]
fn malformed_provider_output_retries_then_completes_with_fresh_call() {
    let (_dir, config) = fixture();
    let (time, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let source: CorrectionSource = Arc::new(move || {
        let c = c.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "same".into(),
            call: Ok(Box::new(move |_| {
                let n = c.fetch_add(1, Ordering::SeqCst);
                Ok(if n == 0 {
                    json!({"decisions":{"version":1,"mutations":[],"actor_kind":"explicit_user"}})
                } else {
                    json!({"decisions":{"version":1,"mutations":[]}})
                })
            })),
        }
    });
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "retry");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tick(&config, &clock, &source);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    *time.lock().unwrap() += chrono::Duration::seconds(30);
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
fn prepare(config: &HieronymusConfig) {
    let mut db = open_migrated(&config.database_path()).unwrap();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let selected = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    prepare_correction_draft(
        &mut db,
        &lease,
        selected,
        parse_decisions(json!({"decisions":{"version":1,"mutations":[]}})).unwrap(),
        now(),
    )
    .unwrap();
}
#[test]
fn prepared_restart_completes_while_admission_is_busy_without_provider_source() {
    let (_dir, config) = fixture();
    prepare(&config);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let lock = dream_cycle_lock(&config, "busy dream").unwrap();
    let source: CorrectionSource = Arc::new(|| panic!("prepared path constructed provider"));
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
    drop(lock);
}
#[test]
fn corrupt_prepared_protocol_is_terminal_across_ticks_restart_and_config_change() {
    let (_dir, config) = fixture();
    prepare(&config);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let db = open_migrated(&config.database_path()).unwrap();
    // Simulate incompatible persisted protocol without changing frozen evidence.
    db.execute_batch("drop trigger if exists consolidation_result_immutable;")
        .unwrap();
    let triggers: Vec<String> = {
        let mut s=db.prepare("select name from sqlite_master where type='trigger' and tbl_name='consolidation_results'").unwrap();
        s.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for trigger in triggers {
        db.execute_batch(&format!("drop trigger \"{trigger}\";"))
            .unwrap();
    }
    db.execute("update consolidation_results set canonical_output='{}'", [])
        .unwrap();
    drop(db);
    let source: CorrectionSource =
        Arc::new(|| panic!("failed prepared result must not call provider"));
    assert!(correction_worker::tick(&config, &AtomicBool::new(false), &clock, &source).is_err());
    assert_eq!(state(&config), "failed");
    tick(&config, &clock, &source);
    let mut db = open_migrated(&config.database_path()).unwrap();
    ConsolidationStore::new(&mut db)
        .assign_provider("default", "changed", clock())
        .unwrap();
    drop(db);
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    correction_worker::start_with_source(config.clone(), &workers, clock, source).unwrap();
    std::thread::sleep(Duration::from_millis(350));
    workers.stop_and_join().unwrap();
    assert_eq!(state(&config), "failed");
}
#[test]
fn shared_lock_is_held_during_call_and_shutdown_joins_bounded_work() {
    let (_dir, config) = fixture();
    let (_, clock) = clock();
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let e = entered.clone();
    let r = released.clone();
    let cfg = config.clone();
    let source: CorrectionSource = Arc::new(move || {
        let e = e.clone();
        let r = r.clone();
        let cfg = cfg.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "stable".into(),
            call: Ok(Box::new(move |_| {
                assert!(dream_cycle_lock(&cfg, "ordinary dream overlap").is_err());
                e.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !r.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(json!({"decisions":{"version":1,"mutations":[]}}))
            })),
        }
    });
    let workers = Arc::new(WorkerGroup::new(Arc::new(AtomicBool::new(false))));
    correction_worker::start_with_source(config.clone(), &workers, clock, source).unwrap();
    wait(|| entered.load(Ordering::SeqCst));
    let joined = Arc::new(AtomicBool::new(false));
    let j = joined.clone();
    let w = workers.clone();
    let join = std::thread::spawn(move || {
        w.stop_and_join().unwrap();
        j.store(true, Ordering::SeqCst);
    });
    std::thread::sleep(Duration::from_millis(50));
    assert!(!joined.load(Ordering::SeqCst));
    assert!(dream_cycle_lock(&config, "still held").is_err());
    released.store(true, Ordering::SeqCst);
    join.join().unwrap();
    assert!(dream_cycle_lock(&config, "released").is_ok());
    assert_eq!(workers.live_count(), 0);
}
#[test]
fn provider_panic_releases_lock_and_worker_remains_supervised() {
    let (_dir, config) = fixture();
    let (_, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let source: CorrectionSource = Arc::new(move || {
        let c = c.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "stable".into(),
            call: Ok(Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                panic!("injected provider panic")
            })),
        }
    });
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    correction_worker::start_with_source(config.clone(), &workers, clock, source).unwrap();
    wait(|| calls.load(Ordering::SeqCst) == 1);
    wait(|| dream_cycle_lock(&config, "released panic").is_ok());
    workers.stop_and_join().unwrap();
    assert_eq!(state(&config), "leased");
}
#[test]
fn configured_default_map_key_is_stable_and_fingerprint_excludes_credentials() {
    use hieronymus::provider_config::{ProviderCatalog, ProviderDefaults, ProviderProfile};
    let make = |name: &str, key: &str| {
        let mut c = ProviderCatalog::default().with_provider(
            "default",
            ProviderProfile::new(name, "openai", "https://example.invalid/v1", key, 60.),
        );
        c.defaults = ProviderDefaults::new("default", "model");
        c
    };
    let a = CorrectionProvider::from_catalog(make("display1", "key1"));
    let b = CorrectionProvider::from_catalog(make("display2", "key2"));
    assert_eq!(a.slot, "default");
    assert_eq!(a.fingerprint, b.fingerprint);
    let mut no_default = make("display", "key");
    no_default.defaults = ProviderDefaults::new("", "model");
    assert!(CorrectionProvider::from_catalog(no_default).call.is_err());
}
#[test]
fn correction_request_uses_at_most_three_http_attempts_and_bounded_timeout() {
    use hieronymus::{
        dream_providers::LlmDreamProvider,
        provider_config::ProviderProfile,
        provider_http::{HttpError, HttpResponse, ProviderTransport},
    };
    struct Counting(AtomicUsize);
    impl ProviderTransport for Counting {
        fn post_json(
            &self,
            _: &str,
            _: &[(String, String)],
            _: &Value,
            timeout: Duration,
        ) -> Result<HttpResponse, HttpError> {
            assert!(timeout <= Duration::from_secs(30));
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(HttpError::Timeout { millis: 1 })
        }
        fn get_json(
            &self,
            _: &str,
            _: &[(String, String)],
            _: Duration,
        ) -> Result<HttpResponse, HttpError> {
            panic!("not a model probe")
        }
    }
    let transport = Arc::new(Counting(AtomicUsize::new(0)));
    let provider = LlmDreamProvider::new(
        "stable",
        ProviderProfile::new("display", "openai", "http://example.invalid", "fake", 600.),
        "model",
    )
    .unwrap()
    .with_transport(transport.clone())
    .with_retry_backoff(Duration::ZERO);
    assert!(provider.run_correction(&json!({})).is_err());
    assert_eq!(transport.0.load(Ordering::SeqCst), 3);
}

#[test]
fn daemon_start_actually_registers_correction_recovery_before_shutdown() {
    use hiero::daemon::{Daemon, DaemonOptions};
    let (_dir, config) = fixture();
    // A past due job with no configured provider must consume one bounded
    // transient attempt at real Daemon::start, without any HTTP/MCP traffic.
    let daemon = Daemon::start(&DaemonOptions {
        data_root: Some(config.data_root().to_path_buf()),
        port: 0,
        ..Default::default()
    })
    .unwrap();
    wait(|| state(&config) == "retry");
    assert!(config.data_root().join("daemon.json").exists());
    drop(daemon);
    assert!(!config.data_root().join("daemon.json").exists());
    assert!(hieronymus::ownership::RootOwnership::acquire(&config, "test").is_ok());
}

fn capture_two_claims(config: &HieronymusConfig) -> (i64, i64) {
    use hieronymus::{
        claim_capture::{ClaimInput, capture_claim_tx},
        claim_reads::ClaimTarget,
        story_applicability::*,
    };
    let mut db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,scope_key,series_slug,strength,confidence,status,created_at,updated_at) values(1,'fact','input','series','series:book','book',1,1,'active','now','now'),(2,'fact','output','series','series:book','book',1,1,'active','now','now');").unwrap();
    let tx = db.transaction().unwrap();
    let mut ids = vec![];
    for id in [1, 2] {
        ids.push(
            capture_claim_tx(
                &tx,
                ClaimTarget::Crystal(id),
                &ClaimInput {
                    text: format!("claim{id}"),
                    concept_id: None,
                    applicability: ApplicabilityV1 {
                        series_id: 1,
                        timeline_id: None,
                        volume_key: None,
                        chapter_key: None,
                        scope_predicates: vec![],
                        valid_from: None,
                        valid_until: None,
                        metadata_state: MetadataState::Unspecified,
                        knowledge_gates: vec![],
                    },
                },
            )
            .unwrap(),
        );
    }
    tx.commit().unwrap();
    (ids[0], ids[1])
}
#[test]
fn bad_policy_first_good_later_is_a_new_provider_evaluation_then_no_recursive_job() {
    let (_dir, config) = fixture();
    let (input, output) = capture_two_claims(&config);
    let (time, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let source: CorrectionSource = Arc::new(move || {
        let c = c.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "same".into(),
            call: Ok(Box::new(move |context| {
                let selected = context["evidence"].as_array().unwrap();
                assert_eq!(selected.len(), 2);
                let n = c.fetch_add(1, Ordering::SeqCst);
                Ok(
                    json!({"decisions":{"version":1,"mutations":[{"ClaimLineage":{"input_claim_ids":[input],"output_claim_ids":[if n==0{input}else{output}]}}]}}),
                )
            })),
        }
    });
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "retry");
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from origin_receipts where kind='dream'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    *time.lock().unwrap() += chrono::Duration::seconds(30);
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select count(*) from claim_derivations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn two_parked_series_share_recovery_slot_and_alternate_fairly() {
    let (_dir, config) = fixture();
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(2,'other','Other','en','ru','now','now');insert into authority_state(series_id) values(2);").unwrap();
    drop(db);
    let other = "10000000-0000-4000-8000-000000000002";
    add_job(&config, other, 2);
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute("update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?1,last_attempt_at=?1",[now().to_rfc3339()]).unwrap();
    drop(db);
    let (time, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    let source = source(calls.clone(), Arc::new(AtomicBool::new(false)));
    *time.lock().unwrap() += chrono::Duration::seconds(21600);
    tick(&config, &clock, &source);
    tick(&config, &clock, &source);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select attempts from consolidation_jobs where decision_id=?",
            [JOB],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    drop(db);
    *time.lock().unwrap() += chrono::Duration::seconds(21600);
    tick(&config, &clock, &source);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select attempts from consolidation_jobs where decision_id=?",
            [other],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
}
#[test]
fn prepared_completion_never_constructs_provider_even_when_lock_is_available() {
    let (_dir, config) = fixture();
    prepare(&config);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let source: CorrectionSource = Arc::new(|| panic!("prepared path touched provider admission"));
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
}

#[test]
fn policy_change_after_preparation_is_terminal_not_a_provider_free_hot_loop() {
    let (_dir, config) = fixture();
    let (input, output) = capture_two_claims(&config);
    let mut db = open_migrated(&config.database_path()).unwrap();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    prepare_correction_draft(&mut db,&lease,selection,parse_decisions(json!({"decisions":{"version":1,"mutations":[{"ClaimLineage":{"input_claim_ids":[input],"output_claim_ids":[output]}}]}})).unwrap(),now()).unwrap();
    // A graph policy change after valid preparation makes the retained draft
    // form a cycle. Its already prepared bytes must never trigger fresh calls.
    db.execute(
        "insert into claim_derivations values(?1,?2,?3)",
        params![output, input, lease.result_id],
    )
    .unwrap();
    drop(db);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let source: CorrectionSource = Arc::new(|| panic!("terminal prepared policy called provider"));
    assert!(correction_worker::tick(&config, &AtomicBool::new(false), &clock, &source).is_err());
    assert_eq!(state(&config), "failed");
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "failed");
}

#[test]
fn expired_crash_housekeeping_while_busy_does_not_admit_a_new_attempt() {
    let (_dir, config) = fixture();
    let mut db = open_migrated(&config.database_path()).unwrap();
    db.execute("update consolidation_jobs set attempts=5", [])
        .unwrap();
    ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let budget: String = db
        .query_row(
            "select next_recovery_at from provider_recovery_state",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let _lock = dream_cycle_lock(&config, "ordinary dream owns admission").unwrap();
    let source: CorrectionSource = Arc::new(|| panic!("crash recovery admitted provider"));
    tick(&config, &clock, &source);
    let db = open_migrated(&config.database_path()).unwrap();
    let actual: (String, i64, i64, Option<String>) = db
        .query_row(
            "select state,attempts,result_generation,lease_token from consolidation_jobs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(actual, ("degraded".into(), 6, 0, None));
    assert_eq!(
        db.query_row(
            "select next_recovery_at from provider_recovery_state",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        budget
    );
    let due: String = db
        .query_row("select next_attempt_at from consolidation_jobs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        DateTime::parse_from_rfc3339(&due)
            .unwrap()
            .with_timezone(&Utc),
        clock() + chrono::Duration::hours(6)
    );
}

fn rag_claim_fixture(config: &HieronymusConfig) -> std::path::PathBuf {
    use hieronymus::{
        claim_capture::ClaimInput,
        rag::{RagImport, RagStore},
        story_applicability::*,
    };
    let path = config.data_root().join("review-claim.txt");
    std::fs::write(&path, "Original assertion.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(
        0,
        vec![ClaimInput {
            text: "Original assertion.".into(),
            concept_id: None,
            applicability: ApplicabilityV1 {
                series_id: 1,
                timeline_id: None,
                volume_key: None,
                chapter_key: None,
                scope_predicates: vec![],
                valid_from: None,
                valid_until: None,
                metadata_state: MetadataState::Unspecified,
                knowledge_gates: vec![],
            },
        }],
    );
    RagStore::open(config)
        .unwrap()
        .import_file("book", &path, &import)
        .unwrap();
    path
}
fn replace_rag_claim(config: &HieronymusConfig, path: &std::path::Path) {
    std::fs::write(path, "Unmatched replacement.").unwrap();
    hieronymus::rag::RagStore::open(config)
        .unwrap()
        .import_file("book", path, &hieronymus::rag::RagImport::new())
        .unwrap();
}
#[test]
fn review_fix_detached_historical_rag_claim_does_not_poison_unrelated_worker_result() {
    let (_dir, config) = fixture();
    let path = rag_claim_fixture(&config);
    replace_rag_claim(&config, &path);
    let (_, clock) = clock();
    let source: CorrectionSource = Arc::new(|| CorrectionProvider {
        slot: "default".into(),
        fingerprint: "test".into(),
        call: Ok(Box::new(|context| {
            assert!(context["evidence"].as_array().unwrap().is_empty());
            Ok(json!({"decisions":{"version":1,"mutations":[]}}))
        })),
    });
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(db.query_row("select count(*) from evidence_records where json_extract(binding_json,'$.event')='rag_replace_detach'",[],|r|r.get::<_,i64>(0)).unwrap()>0);
}
#[test]
fn review_fix_rag_detachment_during_provider_is_stale_then_reevaluated() {
    let (_dir, config) = fixture();
    let path = rag_claim_fixture(&config);
    let (time, clock) = clock();
    let cfg = config.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let source: CorrectionSource = Arc::new(move || {
        let cfg = cfg.clone();
        let path = path.clone();
        let c = c.clone();
        CorrectionProvider {
            slot: "default".into(),
            fingerprint: "test".into(),
            call: Ok(Box::new(move |context| {
                if c.fetch_add(1, Ordering::SeqCst) == 0 {
                    assert_eq!(context["evidence"].as_array().unwrap().len(), 1);
                    replace_rag_claim(&cfg, &path);
                }
                Ok(json!({"decisions":{"version":1,"mutations":[]}}))
            })),
        }
    });
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "retry");
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select state from consolidation_results", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "stale"
    );
    drop(db);
    *time.lock().unwrap() += chrono::Duration::seconds(30);
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "complete");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
fn crowded_series(config: &HieronymusConfig) -> String {
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute(
        "update consolidation_jobs set state='retry',next_attempt_at=?",
        [(now() + chrono::Duration::hours(6)).to_rfc3339()],
    )
    .unwrap();
    drop(db);
    let mut last = String::new();
    for id in 2..=101 {
        let db = open_migrated(&config.database_path()).unwrap();
        db.execute("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(?1,?2,'Book','en','ru','now','now')",params![id,format!("series{id}")]).unwrap();
        db.execute("insert into authority_state(series_id) values(?)", [id])
            .unwrap();
        drop(db);
        last = format!("10000000-0000-4000-8000-{id:012}");
        add_job(config, &last, id);
        let db = open_migrated(&config.database_path()).unwrap();
        db.execute("update consolidation_jobs set state='retry',next_attempt_at=?2,created_at=?3 where decision_id=?1",params![last,if id<101{(now()+chrono::Duration::hours(6)).to_rfc3339()}else{now().to_rfc3339()},(now()+chrono::Duration::seconds(if id<101{id-200}else{1})).to_rfc3339()]).unwrap();
    }
    last
}
#[test]
fn review_fix_due_series_beyond_one_hundred_future_series_runs_now() {
    let (_dir, config) = fixture();
    let last = crowded_series(&config);
    let (_, clock) = clock();
    let calls = Arc::new(AtomicUsize::new(0));
    tick(
        &config,
        &clock,
        &source(calls.clone(), Arc::new(AtomicBool::new(true))),
    );
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select state from consolidation_jobs where decision_id=?",
            [last],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "complete"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[test]
fn review_fix_prepared_series_beyond_one_hundred_future_series_needs_no_provider() {
    let (_dir, config) = fixture();
    let last = crowded_series(&config);
    let mut db = open_migrated(&config.database_path()).unwrap();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 101)
        .unwrap()
        .unwrap();
    let selected = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    prepare_correction_draft(
        &mut db,
        &lease,
        selected,
        parse_decisions(json!({"decisions":{"version":1,"mutations":[]}})).unwrap(),
        now(),
    )
    .unwrap();
    drop(db);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let source: CorrectionSource = Arc::new(|| panic!("prepared work touched provider"));
    tick(&config, &clock, &source);
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row(
            "select state from consolidation_jobs where decision_id=?",
            [last],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "complete"
    );
}
#[test]
fn review_fix_unconfigured_fingerprint_is_literal() {
    assert_eq!(
        CorrectionProvider::from_catalog(Default::default()).fingerprint,
        "unconfigured"
    );
}

#[test]
fn review_fix_unaudited_binding_loss_remains_local_corruption() {
    for during_call in [false, true] {
        let (_dir, config) = fixture();
        rag_claim_fixture(&config);
        if !during_call {
            open_migrated(&config.database_path())
                .unwrap()
                .execute("delete from claim_bindings", [])
                .unwrap();
        }
        let (_, clock) = clock();
        let cfg = config.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let source: CorrectionSource = Arc::new(move || {
            let cfg = cfg.clone();
            let c = c.clone();
            CorrectionProvider {
                slot: "default".into(),
                fingerprint: "test".into(),
                call: Ok(Box::new(move |_| {
                    c.fetch_add(1, Ordering::SeqCst);
                    open_migrated(&cfg.database_path())
                        .unwrap()
                        .execute("delete from claim_bindings", [])
                        .unwrap();
                    Ok(json!({"decisions":{"version":1,"mutations":[]}}))
                })),
            }
        });
        tick(&config, &clock, &source);
        assert_eq!(state(&config), "failed");
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(during_call));
    }
}
#[test]
fn review_fix_prepared_claim_detachment_commits_stale_without_provider() {
    let (_dir, config) = fixture();
    let path = rag_claim_fixture(&config);
    prepare(&config);
    replace_rag_claim(&config, &path);
    let (time, clock) = clock();
    *time.lock().unwrap() += chrono::Duration::seconds(120);
    let source: CorrectionSource = Arc::new(|| panic!("prepared stale used provider"));
    tick(&config, &clock, &source);
    assert_eq!(state(&config), "retry");
    let db = open_migrated(&config.database_path()).unwrap();
    assert_eq!(
        db.query_row("select state from consolidation_results", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "stale"
    );
}

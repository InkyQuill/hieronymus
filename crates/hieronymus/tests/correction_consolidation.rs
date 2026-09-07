use chrono::{DateTime, Duration, Utc};
use hieronymus::{
    consolidation::{ConsolidationStore, FailureKind},
    db::open_migrated,
};
use rusqlite::{Connection, params};

fn now() -> DateTime<Utc> {
    "2026-09-07T00:00:00Z".parse().unwrap()
}
fn fixture() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = open_migrated(&dir.path().join("test.sqlite")).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into authority_state(series_id) values(1); insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values('origin','agent','test','event','signal','{}','hash','now');").unwrap();
    add_job(&db, "10000000-0000-4000-8000-000000000001");
    (dir, db)
}
fn add_job(db: &Connection, id: &str) {
    let t = now().to_rfc3339();
    db.execute("insert into decision_records(decision_id,series_id,origin_id,actor_kind,expected_revision,resulting_revision,canonical_request,result_json,status,created_at) values(?1,1,'origin','agent',0,0,'{}','{}','tentative',?2)", params![id,t]).unwrap();
    db.execute(
        "insert or ignore into provider_recovery_state values('default','unconfigured',?1,?1)",
        [&t],
    )
    .unwrap();
    db.execute("insert into consolidation_jobs(decision_id,state,provider_slot_id,created_at,updated_at) values(?1,'pending','default',?2,?2)", params![id,t]).unwrap();
}
#[test]
fn lease_expires_at_120_seconds_and_reuses_reserved_identity() {
    let (_dir, mut db) = fixture();
    let first = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    assert_ne!(first.result_id, first.decision_id);
    assert_eq!(first.expires_at, now() + Duration::seconds(120));
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now() + Duration::seconds(119), 1)
            .unwrap()
            .is_none()
    );
    ConsolidationStore::new(&mut db)
        .recovery_tick(now() + Duration::seconds(120))
        .unwrap();
    let second = ConsolidationStore::new(&mut db)
        .lease_next(now() + Duration::seconds(120), 1)
        .unwrap()
        .unwrap();
    assert_eq!(first.result_id, second.result_id);
    assert_ne!(first.token, second.token);
    assert!(
        ConsolidationStore::new(&mut db)
            .fail(
                &first.token,
                FailureKind::Transient,
                "timeout",
                now() + Duration::seconds(120)
            )
            .is_err()
    );
}
#[test]
fn six_failures_then_park_and_recovery_cap_survives_restart_and_config_change() {
    let (dir, mut db) = fixture();
    let mut t = now();
    for delay in [30, 120, 600, 3600, 21600, 21600] {
        let lease = ConsolidationStore::new(&mut db)
            .lease_next(t, 1)
            .unwrap()
            .unwrap();
        ConsolidationStore::new(&mut db)
            .fail(&lease.token, FailureKind::Transient, "provider_output", t)
            .unwrap();
        assert!(
            ConsolidationStore::new(&mut db)
                .lease_next(t + Duration::seconds(delay - 1), 1)
                .unwrap()
                .is_none()
        );
        t += Duration::seconds(delay);
    }
    assert_eq!(
        db.query_row("select state from consolidation_jobs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "degraded"
    );
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(t, 1)
        .unwrap()
        .unwrap();
    ConsolidationStore::new(&mut db)
        .fail(&lease.token, FailureKind::Transient, "timeout", t)
        .unwrap();
    drop(db);
    let mut db = open_migrated(&dir.path().join("test.sqlite")).unwrap();
    ConsolidationStore::new(&mut db)
        .assign_provider("default", "changed-model", t)
        .unwrap();
    ConsolidationStore::new(&mut db).recovery_tick(t).unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(t, 1)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(t + Duration::hours(6), 1)
            .unwrap()
            .is_some()
    );
}
#[test]
fn two_parked_jobs_share_slot_cap_and_alternate() {
    let (_dir, mut db) = fixture();
    add_job(&db, "10000000-0000-4000-8000-000000000002");
    db.execute(
        "update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?",
        [now().to_rfc3339()],
    )
    .unwrap();
    let first = ConsolidationStore::new(&mut db)
        .lease_next(now() + Duration::seconds(1), 1)
        .unwrap()
        .unwrap();
    ConsolidationStore::new(&mut db)
        .fail(
            &first.token,
            FailureKind::Transient,
            "timeout",
            now() + Duration::seconds(1),
        )
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now() + Duration::seconds(1), 1)
            .unwrap()
            .is_none()
    );
    let second = ConsolidationStore::new(&mut db)
        .lease_next(now() + Duration::hours(6) + Duration::seconds(1), 1)
        .unwrap()
        .unwrap();
    assert_ne!(first.decision_id, second.decision_id);
}
#[test]
fn assignment_preserves_parked_job_due_and_existing_named_default_deadline() {
    let (_dir, mut db) = fixture();
    let deadline = now() + Duration::hours(6);
    db.execute(
        "update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?",
        [deadline.to_rfc3339()],
    )
    .unwrap();
    db.execute(
        "update provider_recovery_state set next_recovery_at=?",
        [deadline.to_rfc3339()],
    )
    .unwrap();
    ConsolidationStore::new(&mut db)
        .assign_provider("default", "configured-profile-named-default", now())
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_none()
    );
    ConsolidationStore::new(&mut db)
        .assign_provider("other", "configured", now())
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(deadline, 1)
            .unwrap()
            .is_some()
    );
}
#[test]
fn local_invariant_is_terminal_and_config_change_does_not_requeue() {
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    ConsolidationStore::new(&mut db)
        .fail(
            &lease.token,
            FailureKind::Deterministic,
            "canonical_corrupt",
            now(),
        )
        .unwrap();
    ConsolidationStore::new(&mut db)
        .assign_provider("other", "changed", now() + Duration::days(1))
        .unwrap();
    ConsolidationStore::new(&mut db)
        .recovery_tick(now() + Duration::days(1))
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now() + Duration::days(1), 1)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.query_row("select last_error_code from consolidation_jobs", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "canonical_corrupt"
    );
}

fn prepared_output(
    db: &Connection,
    lease: &hieronymus::consolidation::ConsolidationLease,
) -> String {
    use hieronymus::{authority_models::OriginReceiptId, consolidation::ConsolidationResultV1};
    use sha2::{Digest, Sha256};
    let output = ConsolidationResultV1 {
        version: 1,
        result_id: lease.result_id.clone(),
        job_decision_id: lease.decision_id.clone(),
        generation: lease.generation,
        expected_revision: lease.expected_revision,
        origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
        evidence_refs: vec![],
        mutations: vec![],
    };
    let canonical = serde_json::to_value(&output).unwrap().to_string();
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("correction consolidation\n{canonical}"))
    );
    db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'dream','correction_worker',?2,'correction consolidation',?3,?4,?5)",params![output.origin.0,lease.result_id,canonical,hash,now().to_rfc3339()]).unwrap();
    canonical
}
#[test]
fn preparation_is_immutable_and_restart_reclaims_without_provider_attempt() {
    let (dir, mut db) = fixture();
    let first = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &first);
    ConsolidationStore::new(&mut db)
        .prepare_result(&first.token, &first.result_id, &canonical, now())
        .unwrap();
    ConsolidationStore::new(&mut db)
        .prepare_result(&first.token, &first.result_id, &canonical, now())
        .unwrap();
    let changed = canonical.replace("\"expected_revision\":0", "\"expected_revision\":1");
    assert!(matches!(
        ConsolidationStore::new(&mut db).prepare_result(
            &first.token,
            &first.result_id,
            &changed,
            now()
        ),
        Err(hieronymus::consolidation::ConsolidationError::IdempotencyConflict)
    ));
    drop(db);
    let mut db = open_migrated(&dir.path().join("test.sqlite")).unwrap();
    let t = now() + Duration::seconds(120);
    let recovered = ConsolidationStore::new(&mut db)
        .lease_prepared(t, 1)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.result_id, first.result_id);
    assert_eq!(recovered.attempts, 1);
    assert_eq!(
        recovered.canonical_output.as_deref(),
        Some(canonical.as_str())
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .prepare_result(&first.token, &first.result_id, &canonical, t)
            .is_err()
    );
}
#[test]
fn expired_unprepared_lease_cannot_prepare() {
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    assert!(matches!(
        ConsolidationStore::new(&mut db).prepare_result(
            &lease.token,
            &lease.result_id,
            &canonical,
            now() + Duration::seconds(120)
        ),
        Err(hieronymus::consolidation::ConsolidationError::ExpiredLease)
    ));
}

#[test]
fn clock_jumps_do_not_multiply_recovery_allowance() {
    let (_dir, mut db) = fixture();
    db.execute(
        "update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?",
        [now().to_rfc3339()],
    )
    .unwrap();
    let future = now() + Duration::days(30);
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(future, 1)
        .unwrap()
        .unwrap();
    ConsolidationStore::new(&mut db)
        .fail(&lease.token, FailureKind::Transient, "timeout", future)
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(future, 1)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(future + Duration::hours(6), 1)
            .unwrap()
            .is_some()
    );
}
#[test]
fn recovery_budget_and_result_reservation_roll_back_together() {
    let (_dir, mut db) = fixture();
    db.execute(
        "update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?",
        [now().to_rfc3339()],
    )
    .unwrap();
    db.execute_batch("create trigger fail_budget before update on provider_recovery_state begin select raise(abort,'injected'); end;").unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .is_err()
    );
    assert_eq!(
        db.query_row("select count(*) from consolidation_results", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select attempts from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        6
    );
    db.execute_batch("drop trigger fail_budget").unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_some()
    );
}
#[test]
fn independent_series_can_lease_ordinary_work_concurrently() {
    let (_dir, mut db) = fixture();
    add_job(&db, "10000000-0000-4000-8000-000000000002");
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(2,'other','Other','en','ru','now','now'); insert into authority_state(series_id) values(2); update decision_records set series_id=2 where decision_id='10000000-0000-4000-8000-000000000002';").unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_some()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 2)
            .unwrap()
            .is_some()
    );
}

#[test]
fn oldest_parked_job_is_selected_across_series() {
    let (_dir, mut db) = fixture();
    add_job(&db, "10000000-0000-4000-8000-000000000002");
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(2,'other','Other','en','ru','now','now'); insert into authority_state(series_id) values(2); update decision_records set series_id=2 where decision_id='10000000-0000-4000-8000-000000000002';").unwrap();
    db.execute(
        "update consolidation_jobs set state='degraded',attempts=6,next_attempt_at=?",
        [now().to_rfc3339()],
    )
    .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 2)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(now(), 1)
            .unwrap()
            .is_some()
    );
}
#[test]
fn sixth_attempt_crash_parks_before_recovery() {
    let (_dir, mut db) = fixture();
    db.execute(
        "update consolidation_jobs set state='retry',attempts=5,next_attempt_at=?",
        [now().to_rfc3339()],
    )
    .unwrap();
    ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let expiry = now() + Duration::seconds(120);
    ConsolidationStore::new(&mut db)
        .recovery_tick(expiry)
        .unwrap();
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(expiry, 1)
            .unwrap()
            .is_none()
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .lease_next(expiry + Duration::hours(6), 1)
            .unwrap()
            .is_some()
    );
}

#[test]
fn prepare_rejects_a_different_result_identity_or_unbound_origin() {
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    let changed = canonical.replace(&lease.result_id, "30000000-0000-4000-8000-000000000001");
    assert!(
        ConsolidationStore::new(&mut db)
            .prepare_result(&lease.token, &lease.result_id, &changed, now())
            .is_err()
    );
    let changed = canonical.replace(
        "20000000-0000-4000-8000-000000000001",
        "20000000-0000-4000-8000-000000000002",
    );
    assert!(
        ConsolidationStore::new(&mut db)
            .prepare_result(&lease.token, &lease.result_id, &changed, now())
            .is_err()
    );
    assert_eq!(
        db.query_row("select state from consolidation_results", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "reserved"
    );
}
#[test]
fn prepare_binds_the_reserved_observation_instead_of_silently_rebasing() {
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    db.execute(
        "update authority_state set revision=1 where series_id=1",
        [],
    )
    .unwrap();
    ConsolidationStore::new(&mut db)
        .prepare_result(&lease.token, &lease.result_id, &canonical, now())
        .unwrap();
    let recovered = ConsolidationStore::new(&mut db)
        .lease_prepared(now() + Duration::seconds(120), 1)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.expected_revision, 0);
    assert_eq!(recovered.canonical_output, Some(canonical));
    // Completion must detect the live revision conflict and create a new
    // generation; this core slice deliberately has no completion entry point.
}

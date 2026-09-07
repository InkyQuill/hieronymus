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
    let db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
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
    let mut db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
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
    let output = ConsolidationResultV1 {
        version: 1,
        result_id: lease.result_id.clone(),
        job_decision_id: lease.decision_id.clone(),
        generation: lease.generation,
        expected_revision: lease.expected_revision,
        origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
        evidence_refs: vec![],
        selected_claims: vec![],
        mutations: vec![],
    };
    bind_output(db, &output)
}
fn bind_output(
    db: &Connection,
    output: &hieronymus::consolidation::ConsolidationResultV1,
) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_value(output).unwrap().to_string();
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("correction consolidation\n{canonical}"))
    );
    db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'dream','correction_worker',?2,'correction consolidation',?3,?4,?5)",params![output.origin.0,output.result_id,canonical,hash,now().to_rfc3339()]).unwrap();
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
    let mut db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
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

#[test]
fn empty_completion_replays_exact_receipt_after_restart_and_expiry() {
    use hieronymus::consolidation::{CompletionOutcome, finish_result_tx};
    let (dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    ConsolidationStore::new(&mut db)
        .prepare_result(&lease.token, &lease.result_id, &canonical, now())
        .unwrap();
    let tx = db.transaction().unwrap();
    let first = finish_result_tx(&tx, &lease.result_id, &lease.token, now()).unwrap();
    assert!(matches!(first, CompletionOutcome::Complete { .. }));
    tx.commit().unwrap();
    drop(db);
    let mut db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        finish_result_tx(
            &tx,
            &lease.result_id,
            "old-token",
            now() + Duration::days(1)
        )
        .unwrap(),
        first
    );
    assert_eq!(
        tx.query_row("select revision from authority_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row(
            "select count(*) from consolidation_jobs where state='complete'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row("select count(*) from decision_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn stale_completion_preserves_payload_and_commits_new_generation() {
    use hieronymus::consolidation::{CompletionOutcome, finish_result_tx};
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    ConsolidationStore::new(&mut db)
        .prepare_result(&lease.token, &lease.result_id, &canonical, now())
        .unwrap();
    db.execute("update authority_state set revision=1", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        finish_result_tx(&tx, &lease.result_id, &lease.token, now()).unwrap(),
        CompletionOutcome::Stale { .. }
    ));
    assert_eq!(
        tx.query_row(
            "select canonical_output from consolidation_results",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        canonical
    );
    tx.commit().unwrap();
    let new = ConsolidationStore::new(&mut db)
        .lease_next(now() + Duration::seconds(30), 1)
        .unwrap()
        .unwrap();
    assert_ne!(new.result_id, lease.result_id);
    assert_eq!(new.generation, lease.generation + 1);
}

#[test]
fn unfinished_completion_refuses_expired_lease_without_effects() {
    use hieronymus::consolidation::{ConsolidationError, finish_result_tx};
    let (_dir, mut db) = fixture();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let canonical = prepared_output(&db, &lease);
    ConsolidationStore::new(&mut db)
        .prepare_result(&lease.token, &lease.result_id, &canonical, now())
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        finish_result_tx(
            &tx,
            &lease.result_id,
            &lease.token,
            now() + Duration::seconds(120)
        ),
        Err(ConsolidationError::ExpiredLease)
    ));
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("select state from consolidation_results", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "prepared"
    );
}

use hieronymus::{
    authority::EvidenceBindingV1, authority_models::*, consolidation::*, story_applicability::*,
};
fn hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn app() -> ApplicabilityV1 {
    ApplicabilityV1 {
        series_id: 1,
        timeline_id: Some(1),
        volume_key: Some("I".into()),
        chapter_key: Some("1".into()),
        scope_predicates: vec![],
        valid_from: None,
        valid_until: None,
        metadata_state: MetadataState::Resolved,
        knowledge_gates: vec![KnowledgeGateV1 {
            viewpoint: KnowledgeViewpoint::All,
            known_from: None,
            known_until: None,
        }],
    }
}
fn anchor(db: &Connection, n: i64, rendering: &str) -> Vec<EvidenceRef> {
    anchor_at(db, n, rendering, app(), 1)
}
fn anchor_at(
    db: &Connection,
    n: i64,
    rendering: &str,
    a: ApplicabilityV1,
    pos: i64,
) -> Vec<EvidenceRef> {
    anchor_for(db, n, rendering, a, pos, 1)
}
fn anchor_for(
    db: &Connection,
    n: i64,
    rendering: &str,
    a: ApplicabilityV1,
    pos: i64,
    concept: i64,
) -> Vec<EvidenceRef> {
    let group = (n - 1) / 2;
    let source = format!(
        "Alex walks.\n\nAlex talks.\n\nChapter {pos}, person {concept}, passage group {group}"
    );
    let source = source.as_str();
    let (start, end) = if n % 2 == 1 { (0, 11) } else { (13, 24) };
    let mut binding = EvidenceBindingV1 {
        concept_id: concept,
        source_language: "en".into(),
        target_language: Some("ru".into()),
        applicability: a,
        position_id: pos,
        paragraph_start: start,
        paragraph_end: end,
        identity_anchor: true,
        aligned_source_id: None,
        rendering: None,
        contradicts_rule: None,
        conflict_kind: None,
        contradicts_claim: None,
        claim_effect: None,
    };
    db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,1,'source_passage',?7,?2,?3,?4,?5,?6,'now')",params![n,hash(source),start as i64,end as i64,source,serde_json::to_string(&binding).unwrap(),format!("doc{pos}-{concept}-{group}")]).unwrap();
    binding.aligned_source_id = Some(n);
    binding.rendering = Some(rendering.into());
    db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,1,'aligned_rendering',?2,?3,0,?4,?5,?6,'now')",params![n+10,format!("translation-{n}"),hash(rendering),rendering.len() as i64,rendering,serde_json::to_string(&binding).unwrap()]).unwrap();
    db.execute("insert or ignore into story_positions(id,timeline_id,volume_key,chapter_key,ordinal,evidence_id) values(?2,1,'I',?3,?2,?1)",params![n,pos,pos.to_string()]).unwrap();
    vec![
        EvidenceRef {
            kind: EvidenceKind::SourcePassage,
            id: n,
            content_hash: hash(source),
            span_start: start,
            span_end: end,
        },
        EvidenceRef {
            kind: EvidenceKind::AlignedRendering,
            id: n + 10,
            content_hash: hash(rendering),
            span_start: 0,
            span_end: rendering.len(),
        },
    ]
}

fn derived_fixture() -> (
    tempfile::TempDir,
    Connection,
    ConsolidationLease,
    ConsolidationResultV1,
) {
    let (dir, mut db) = fixture();
    db.execute_batch("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Alex','series','series:book','now','now'); insert into story_timelines(id,series_id,name) values(1,1,'story'); insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(1,1,'en','ru','Alex','A','candidate','now','now'); insert into term_rule_forms(rule_id,form_kind,surface,language) values(1,'source','Alex','en'),(1,'approved','A','ru');").unwrap();
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(now(), 1)
        .unwrap()
        .unwrap();
    let mut evidence = anchor(&db, 1, "A");
    evidence.extend(anchor(&db, 2, "A"));
    let output = ConsolidationResultV1 {
        version: 1,
        result_id: lease.result_id.clone(),
        job_decision_id: lease.decision_id.clone(),
        generation: lease.generation,
        expected_revision: lease.expected_revision,
        origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
        evidence_refs: evidence,
        selected_claims: vec![],
        mutations: vec![DerivedMutationV1::LearnedRule {
            concept_id: 1,
            source_language: "en".into(),
            target_language: "ru".into(),
            applicability: app(),
            operation: Box::new(LearnedRuleOperationV1::Activate {
                candidate_id: 1,
                candidate_revision: 1,
            }),
        }],
    };
    (dir, db, lease, output)
}
fn prepare(db: &mut Connection, lease: &ConsolidationLease, output: &ConsolidationResultV1) {
    let canonical = bind_output(db, output);
    ConsolidationStore::new(db)
        .prepare_result(&lease.token, &lease.result_id, &canonical, now())
        .unwrap();
}
fn complete(
    db: &mut Connection,
    lease: &ConsolidationLease,
) -> Result<CompletionOutcome, ConsolidationError> {
    let tx = db.transaction().unwrap();
    let outcome = finish_result_tx(&tx, &lease.result_id, &lease.token, now());
    tx.commit().unwrap(); // deliberately commit even errors to verify the savepoint
    outcome
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn learned_completion_is_atomic_result_owned_and_exactly_once() {
    let (_dir, mut db, lease, output) = derived_fixture();
    prepare(&mut db, &lease, &output);
    let first = complete(&mut db, &lease).unwrap();
    assert_eq!(complete(&mut db, &lease).unwrap(), first);
    assert_eq!(
        db.query_row("select status from term_rules where id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "active"
    );
    assert_eq!(
        db.query_row(
            "select consolidation_result_id from rule_authority",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        lease.result_id
    );
    assert_eq!(count(&db, "term_rule_actions"), 1);
    assert_eq!(count(&db, "consolidation_jobs"), 1);
    assert_eq!(count(&db, "decision_records"), 1);
}
#[test]
fn learned_completion_rolls_back_effect_audit_and_completion_failures() {
    for trigger in [
        "create trigger failure after update of status on term_rules begin select raise(abort,'effect'); end",
        "create trigger failure after insert on term_rule_actions begin select raise(abort,'audit'); end",
        "create trigger failure before update of completion_receipt on consolidation_results begin select raise(abort,'receipt'); end",
        "create trigger failure after update of state on consolidation_jobs when new.state='complete' begin select raise(abort,'job'); end",
    ] {
        let (_dir, mut db, lease, output) = derived_fixture();
        prepare(&mut db, &lease, &output);
        db.execute_batch(trigger).unwrap();
        assert!(complete(&mut db, &lease).is_err());
        assert_eq!(
            db.query_row("select status from term_rules where id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "candidate"
        );
        assert_eq!(count(&db, "rule_authority"), 0);
        assert_eq!(count(&db, "term_rule_actions"), 0);
        assert_eq!(
            db.query_row("select state from consolidation_results", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "prepared"
        );
        db.execute_batch("drop trigger failure").unwrap();
        assert!(complete(&mut db, &lease).is_ok());
    }
}
#[test]
fn tentative_and_protected_authority_are_refused() {
    for protected in [false, true] {
        let (_dir, mut db, lease, mut output) = derived_fixture();
        if protected {
            db.execute_batch("update term_rules set status='active'; insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(2,1,'en','ru','Alex','A','candidate','now','now'); insert into term_rule_forms(rule_id,form_kind,surface,language) values(2,'source','Alex','en'),(2,'approved','A','ru');").unwrap();
            if let DerivedMutationV1::LearnedRule { operation, .. } = &mut output.mutations[0] {
                **operation = LearnedRuleOperationV1::Activate {
                    candidate_id: 2,
                    candidate_revision: 1,
                };
            }
        } else {
            output.evidence_refs.truncate(2);
        }
        prepare(&mut db, &lease, &output);
        assert!(matches!(
            complete(&mut db, &lease),
            Err(ConsolidationError::Policy(_))
        ));
        assert_eq!(count(&db, "term_rule_actions"), 0);
    }
}
#[test]
fn stale_object_revision_is_committable_without_effects() {
    let (_dir, mut db, lease, output) = derived_fixture();
    prepare(&mut db, &lease, &output);
    db.execute("update term_rules set revision=2", []).unwrap();
    assert!(matches!(
        complete(&mut db, &lease).unwrap(),
        CompletionOutcome::Stale { .. }
    ));
    assert_eq!(count(&db, "term_rule_actions"), 0);
}
#[test]
fn selected_contradiction_is_not_filtered_out_of_learned_policy() {
    let (_dir, mut db, lease, mut output) = derived_fixture();
    output.evidence_refs.extend(anchor(&db, 3, "B"));
    prepare(&mut db, &lease, &output);
    assert!(complete(&mut db, &lease).is_err());
    assert_eq!(count(&db, "term_rule_actions"), 0);
}

fn selected_claim(
    db: &mut Connection,
    output: &mut ConsolidationResultV1,
    target: i64,
    a: ApplicabilityV1,
) -> i64 {
    db.execute("insert or ignore into crystals(id,crystal_type,text,scope_type,scope_key,series_slug,strength,confidence,status,created_at,updated_at) values(?1,'fact','assertion','series','series:book','book',1,1,'active','now','now')",[target]).unwrap();
    let tx = db.transaction().unwrap();
    let claim = hieronymus::claim_capture::capture_claim_tx(
        &tx,
        hieronymus::claim_reads::ClaimTarget::Crystal(target),
        &hieronymus::claim_capture::ClaimInput {
            text: format!("assertion {target}"),
            concept_id: Some(1),
            applicability: a,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let (id, hash, end): (i64, String, i64) = db
        .query_row(
            "select id,source_hash,span_end from evidence_records where source_identity=?",
            [format!("claim:{claim}")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    output.evidence_refs.push(EvidenceRef {
        kind: EvidenceKind::Observation,
        id,
        content_hash: hash,
        span_start: 0,
        span_end: end as usize,
    });
    output.selected_claims.push(SelectedClaimV1 {
        claim_id: claim,
        revision: 1,
        evidence_id: id,
    });
    claim
}
fn add_lineage(output: &mut ConsolidationResultV1, input: i64, out: i64) {
    output.mutations.push(DerivedMutationV1::ClaimLineage {
        input_claim_ids: vec![input],
        output_claim_ids: vec![out],
    });
}
fn query(position: i64) -> StoryQueryV1 {
    StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(position),
        viewpoint: Viewpoint::Narrator,
        scope_predicates: vec!["volume:I".into(), format!("chapter:{position}")],
        mode: QueryMode::Current,
    }
}
#[test]
fn mixed_learned_lineage_preserves_selected_identity_without_unselected_siblings() {
    let (_dir, mut db, lease, mut output) = derived_fixture();
    let input = selected_claim(&mut db, &mut output, 1, app());
    let out = selected_claim(&mut db, &mut output, 2, app());
    let mut unselected = output.clone();
    let sibling = selected_claim(&mut db, &mut unselected, 1, app());
    db.execute(
        "update memory_claims set status='invalid' where id=?",
        [sibling],
    )
    .unwrap();
    add_lineage(&mut output, input, out);
    prepare(&mut db, &lease, &output);
    let first = complete(&mut db, &lease).unwrap();
    assert_eq!(complete(&mut db, &lease).unwrap(), first);
    assert_eq!(count(&db, "claim_derivations"), 1);
    let bound: Vec<i64> = db
        .prepare("select claim_id from claim_bindings where crystal_id=2 order by claim_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(bound, vec![input, out]);
    use hieronymus::claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims};
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::Crystal(2), &query(1)).unwrap(),
        ClaimDisposition::Current
    );
    // A later correction applies through the retained identity, with no rerun.
    db.execute(
        "update memory_claims set status='invalid',revision=revision+1 where id=?",
        [input],
    )
    .unwrap();
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::Crystal(2), &query(1)).unwrap(),
        ClaimDisposition::Invalid
    );
    assert_eq!(count(&db, "consolidation_jobs"), 1);
}
#[test]
fn lineage_rejects_unselected_foreign_capture_and_batch_cycles() {
    for defect in ["unselected", "capture", "foreign", "cycle", "self"] {
        let (_dir, mut db, lease, mut output) = derived_fixture();
        output.mutations.clear();
        let input = selected_claim(&mut db, &mut output, 1, app());
        let out = selected_claim(&mut db, &mut output, 2, app());
        add_lineage(&mut output, input, out);
        match defect {
            "unselected" => {
                output.selected_claims.pop();
            }
            "capture" => {
                output.selected_claims[0].evidence_id = output.selected_claims[1].evidence_id;
            }
            "foreign" => {
                db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(2,'other','Other','en','ru','now','now')").unwrap();
                db.execute("update memory_claims set series_id=2 where id=?", [input])
                    .unwrap();
            }
            "cycle" => add_lineage(&mut output, out, input),
            "self" => add_lineage(&mut output, input, input),
            _ => unreachable!(),
        }
        prepare(&mut db, &lease, &output);
        assert!(complete(&mut db, &lease).is_err(), "{defect}");
        assert_eq!(count(&db, "claim_derivations"), 0);
        assert_eq!(count(&db, "claim_bindings"), 2);
    }
}
#[test]
fn stale_claim_revision_and_lineage_audit_failure_leave_no_bindings() {
    for stale in [true, false] {
        let (_dir, mut db, lease, mut output) = derived_fixture();
        output.mutations.clear();
        let input = selected_claim(&mut db, &mut output, 1, app());
        let out = selected_claim(&mut db, &mut output, 2, app());
        add_lineage(&mut output, input, out);
        prepare(&mut db, &lease, &output);
        if stale {
            db.execute("update memory_claims set revision=2 where id=?", [input])
                .unwrap();
            assert!(matches!(
                complete(&mut db, &lease).unwrap(),
                CompletionOutcome::Stale { .. }
            ));
        } else {
            db.execute_batch("create trigger failure after insert on claim_derivations begin select raise(abort,'lineage audit'); end").unwrap();
            assert!(complete(&mut db, &lease).is_err());
        }
        assert_eq!(count(&db, "claim_derivations"), 0);
        assert_eq!(count(&db, "claim_bindings"), 2);
    }
}
#[test]
fn multiple_concepts_project_selected_evidence_independently() {
    let (_dir, mut db, lease, mut output) = derived_fixture();
    db.execute_batch("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(2,'Other Alex','series','series:book','now','now'); insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(2,2,'en','ru','Alex','B','candidate','now','now'); insert into term_rule_forms(rule_id,form_kind,surface,language) values(2,'source','Alex','en'),(2,'approved','B','ru');").unwrap();
    output
        .evidence_refs
        .extend(anchor_for(&db, 3, "B", app(), 1, 2));
    output
        .evidence_refs
        .extend(anchor_for(&db, 4, "B", app(), 1, 2));
    output.mutations.push(DerivedMutationV1::LearnedRule {
        concept_id: 2,
        source_language: "en".into(),
        target_language: "ru".into(),
        applicability: app(),
        operation: Box::new(LearnedRuleOperationV1::Activate {
            candidate_id: 2,
            candidate_revision: 1,
        }),
    });
    prepare(&mut db, &lease, &output);
    complete(&mut db, &lease).unwrap();
    assert_eq!(count(&db, "rule_authority"), 2);
}
#[test]
fn inherited_scoped_masks_and_knowledge_gates_remain_on_new_output() {
    use hieronymus::claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims};
    for gated in [false, true] {
        let (_dir, mut db, lease, mut output) = derived_fixture();
        output.mutations.clear();
        db.execute_batch("insert into story_positions(id,timeline_id,volume_key,chapter_key,ordinal,evidence_id) values(2,1,'I','2',2,1)").unwrap();
        let mut broad = app();
        broad.chapter_key = None;
        let mut input_app = broad.clone();
        if gated {
            input_app.knowledge_gates[0].known_from = Some(2);
        }
        let input = selected_claim(&mut db, &mut output, 1, input_app.clone());
        let out = selected_claim(&mut db, &mut output, 2, broad.clone());
        let receipt = DecisionReceiptV1 {
            decision_id: lease.decision_id.clone(),
            resulting_revision: 0,
            affected_rules: vec![],
            affected_claims: vec![(input, 1)],
            effective_applicability: input_app,
            effective_exclusions: if gated { vec![] } else { vec![app()] },
            effect: "invalid".into(),
            consolidation_job_id: lease.decision_id.clone(),
            origin: OriginReceiptId("origin".into()),
            committed_at: "now".into(),
        };
        db.execute(
            "update decision_records set result_json=?",
            [serde_json::to_string(&DecisionResultV1::Applied { receipt }).unwrap()],
        )
        .unwrap();
        db.execute("insert into claim_effects(claim_id,decision_id,applicability_id,effect) select id,?1,applicability_id,'invalid' from memory_claims where id=?2",params![lease.decision_id,input]).unwrap();
        add_lineage(&mut output, input, out);
        prepare(&mut db, &lease, &output);
        complete(&mut db, &lease).unwrap();
        assert_eq!(
            rehydrate_claims(&db, ClaimTarget::Crystal(2), &query(1)).unwrap(),
            if gated {
                ClaimDisposition::OutsideContext
            } else {
                ClaimDisposition::Current
            }
        );
        assert_eq!(
            rehydrate_claims(&db, ClaimTarget::Crystal(2), &query(2)).unwrap(),
            ClaimDisposition::Invalid
        );
    }
}

#[test]
fn lineage_reaches_existing_facet_bindings_and_transitive_batch_ancestors() {
    use hieronymus::claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims};
    let (_dir, mut db, lease, mut output) = derived_fixture();
    output.mutations.clear();
    let input = selected_claim(&mut db, &mut output, 1, app());
    let middle = selected_claim(&mut db, &mut output, 2, app());
    let out = selected_claim(&mut db, &mut output, 3, app());
    db.execute_batch("insert into concept_facets(id,concept_id,language,facet_type,value,created_at,updated_at) values(1,1,'en','fact','derived assertion','now','now')").unwrap();
    db.execute(
        "insert into claim_bindings(claim_id,facet_id) values(?1,1)",
        [out],
    )
    .unwrap();
    db.execute(
        "update memory_claims set status='invalid' where id=?",
        [input],
    )
    .unwrap();
    add_lineage(&mut output, middle, out);
    add_lineage(&mut output, input, middle);
    prepare(&mut db, &lease, &output);
    complete(&mut db, &lease).unwrap();
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::Crystal(3), &query(1)).unwrap(),
        ClaimDisposition::Invalid
    );
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::Facet(1), &query(1)).unwrap(),
        ClaimDisposition::Invalid
    );
}

#[test]
fn future_explicit_rag_binding_of_derived_claim_retains_ancestors() {
    use hieronymus::{
        claim_capture::ExistingClaimInput,
        claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims},
        data_root::HieronymusConfig,
        rag::{RagImport, RagStore},
    };
    let (dir, mut db, lease, mut output) = derived_fixture();
    output.mutations.clear();
    let input = selected_claim(&mut db, &mut output, 1, app());
    let out = selected_claim(&mut db, &mut output, 2, app());
    add_lineage(&mut output, input, out);
    prepare(&mut db, &lease, &output);
    complete(&mut db, &lease).unwrap();
    db.execute(
        "update memory_claims set status='invalid' where id=?",
        [input],
    )
    .unwrap();
    let store = RagStore::open(&HieronymusConfig::new(dir.path().to_path_buf())).unwrap();
    let path = dir.path().join("derived.txt");
    std::fs::write(&path, "Derived assertion.").unwrap();
    let mut import = RagImport::new();
    import.claim_lineage.insert(
        0,
        vec![ExistingClaimInput {
            claim_id: out,
            concept_id: Some(1),
            applicability: app(),
        }],
    );
    store.import_file("book", &path, &import).unwrap();
    let chunk: i64 = db
        .query_row("select id from rag_chunks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::RagChunk(chunk), &query(1)).unwrap(),
        ClaimDisposition::Invalid
    );
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where rag_chunk_id=?",
            [chunk],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn audited_rag_reimport_retains_selected_claim_completion_eligibility() {
    use hieronymus::{
        claim_capture::ClaimInput,
        data_root::HieronymusConfig,
        rag::{RagImport, RagStore},
    };
    let (dir, mut db, lease, mut output) = derived_fixture();
    output.mutations.clear();
    let store = RagStore::open(&HieronymusConfig::new(dir.path().to_path_buf())).unwrap();
    let path = dir.path().join("original.txt");
    std::fs::write(&path, "Original assertion.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(
        0,
        vec![ClaimInput {
            text: "Original assertion.".into(),
            concept_id: Some(1),
            applicability: app(),
        }],
    );
    store.import_file("book", &path, &import).unwrap();
    let (claim, original): (i64, i64) = db
        .query_row(
            "select claim_id,rag_chunk_id from claim_bindings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (evidence, hash, end): (i64, String, i64) = db
        .query_row(
            "select id,source_hash,span_end from evidence_records where source_identity=?",
            [format!("claim:{claim}")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    output.evidence_refs.push(EvidenceRef {
        kind: EvidenceKind::Observation,
        id: evidence,
        content_hash: hash,
        span_start: 0,
        span_end: end as usize,
    });
    output.selected_claims.push(SelectedClaimV1 {
        claim_id: claim,
        revision: 1,
        evidence_id: evidence,
    });
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "Unrelated source.").unwrap();
    store
        .import_file("book", &other, &RagImport::new())
        .unwrap();
    std::fs::write(&path, "Original assertion.\n\nAdditional paragraph.").unwrap();
    import.claims.clear();
    store.import_file("book", &path, &import).unwrap();
    let current: i64 = db
        .query_row(
            "select rag_chunk_id from claim_bindings where claim_id=?",
            [claim],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(original, current);
    let out = selected_claim(&mut db, &mut output, 1, app());
    add_lineage(&mut output, claim, out);
    prepare(&mut db, &lease, &output);
    complete(&mut db, &lease).unwrap();
    assert_eq!(count(&db, "claim_derivations"), 1);
}

#[test]
fn trusted_draft_selection_prepares_learned_policy_and_rejects_expansion() {
    use hieronymus::{dream_config::default_dream_config, dream_output::DecisionsDraftV1};
    let (_dir, mut db, lease, output) = derived_fixture();
    let mut expanded = output.mutations.clone();
    if let DerivedMutationV1::LearnedRule { operation, .. } = &mut expanded[0] {
        **operation = LearnedRuleOperationV1::Activate {
            candidate_id: 999,
            candidate_revision: 1,
        };
    }
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    assert!(matches!(
        prepare_correction_draft(
            &mut db,
            &lease,
            selection,
            DecisionsDraftV1 {
                version: 1,
                mutations: expanded
            },
            now()
        ),
        Err(DraftPreparationError::Provider(_))
    ));
    assert_eq!(count(&db, "origin_receipts"), 1); // only original job on a fresh database
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    prepare_correction_draft(
        &mut db,
        &lease,
        selection,
        DecisionsDraftV1 {
            version: 1,
            mutations: output.mutations,
        },
        now(),
    )
    .unwrap();
    assert!(matches!(
        complete(&mut db, &lease).unwrap(),
        CompletionOutcome::Complete { .. }
    ));
    assert_eq!(count(&db, "consolidation_jobs"), 1);
}
#[test]
fn preprepare_policy_refusal_rolls_back_origin_and_allows_budgeted_reevaluation() {
    use hieronymus::{dream_config::default_dream_config, dream_output::DecisionsDraftV1};
    let (_dir, mut db, lease, mut output) = derived_fixture();
    // Candidate replacement is schema valid and selected, but policy forbids it.
    if let DerivedMutationV1::LearnedRule { operation, .. } = &mut output.mutations[0] {
        **operation = LearnedRuleOperationV1::Archive {
            rule_id: 1,
            rule_revision: 1,
        };
    }
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    let before = count(&db, "origin_receipts");
    assert!(matches!(
        prepare_correction_draft(
            &mut db,
            &lease,
            selection,
            DecisionsDraftV1 {
                version: 1,
                mutations: output.mutations
            },
            now()
        ),
        Err(DraftPreparationError::Provider(_))
    ));
    assert_eq!(count(&db, "origin_receipts"), before);
    assert_eq!(
        db.query_row("select state from consolidation_results", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "reserved"
    );
    ConsolidationStore::new(&mut db)
        .fail(
            &lease.token,
            FailureKind::Transient,
            "provider_policy",
            now(),
        )
        .unwrap();
    let t = now() + Duration::seconds(30);
    let lease = ConsolidationStore::new(&mut db)
        .lease_next(t, 1)
        .unwrap()
        .unwrap();
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    prepare_correction_draft(
        &mut db,
        &lease,
        selection,
        DecisionsDraftV1 {
            version: 1,
            mutations: vec![],
        },
        t,
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        finish_result_tx(&tx, &lease.result_id, &lease.token, t).unwrap(),
        CompletionOutcome::Complete { .. }
    ));
    tx.commit().unwrap();
}
#[test]
fn draft_faults_are_typed_and_existing_cycle_is_local_corruption() {
    use hieronymus::{dream_config::default_dream_config, dream_output::DecisionsDraftV1};
    for fault in 0..6 {
        let (_dir, mut db, lease, mut output) = derived_fixture();
        let a = selected_claim(&mut db, &mut output, 20, app());
        let b = selected_claim(&mut db, &mut output, 21, app());
        let mutations = match fault {
            0 => vec![output.mutations[0].clone(), output.mutations[0].clone()],
            1 => vec![DerivedMutationV1::ClaimLineage {
                input_claim_ids: vec![a],
                output_claim_ids: vec![a],
            }],
            2 => vec![
                DerivedMutationV1::ClaimLineage {
                    input_claim_ids: vec![a],
                    output_claim_ids: vec![b],
                },
                DerivedMutationV1::ClaimLineage {
                    input_claim_ids: vec![b],
                    output_claim_ids: vec![a],
                },
            ],
            3 => vec![DerivedMutationV1::ClaimLineage {
                input_claim_ids: vec![],
                output_claim_ids: vec![b],
            }],
            4 => vec![DerivedMutationV1::ClaimLineage {
                input_claim_ids: vec![999],
                output_claim_ids: vec![b],
            }],
            _ => {
                db.execute_batch("insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(2,1,'en','ru','Alex','A','candidate','now','now');insert into term_rule_forms(rule_id,form_kind,surface,language) values(2,'source','Alex','en'),(2,'approved','A','ru');").unwrap();
                let mut second = output.mutations[0].clone();
                if let DerivedMutationV1::LearnedRule { operation, .. } = &mut second {
                    **operation = LearnedRuleOperationV1::Activate {
                        candidate_id: 2,
                        candidate_revision: 1,
                    };
                }
                vec![output.mutations[0].clone(), second]
            }
        };
        let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
        assert!(
            matches!(
                prepare_correction_draft(
                    &mut db,
                    &lease,
                    selection,
                    DecisionsDraftV1 {
                        version: 1,
                        mutations
                    },
                    now()
                ),
                Err(DraftPreparationError::Provider(_))
            ),
            "fault {fault}"
        );
        if fault == 2 {
            db.execute(
                "insert into claim_derivations values(?1,?2,?3),(?2,?1,?3)",
                params![a, b, lease.result_id],
            )
            .unwrap();
            let selection =
                select_correction_context(&db, &lease, &default_dream_config()).unwrap();
            assert!(matches!(
                prepare_correction_draft(
                    &mut db,
                    &lease,
                    selection,
                    DecisionsDraftV1 {
                        version: 1,
                        mutations: vec![]
                    },
                    now()
                ),
                Err(DraftPreparationError::Local(ConsolidationError::Invariant(
                    _
                )))
            ));
        }
    }
}
#[test]
fn trusted_selection_revision_change_persists_then_commits_stale() {
    use hieronymus::{dream_config::default_dream_config, dream_output::DecisionsDraftV1};
    let (_dir, mut db, lease, output) = derived_fixture();
    let selection = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    db.execute("update term_rules set revision=revision+1 where id=1", [])
        .unwrap();
    prepare_correction_draft(
        &mut db,
        &lease,
        selection,
        DecisionsDraftV1 {
            version: 1,
            mutations: output.mutations,
        },
        now(),
    )
    .unwrap();
    assert!(matches!(
        complete(&mut db, &lease).unwrap(),
        CompletionOutcome::Stale {
            next_generation: 1,
            ..
        }
    ));
}

#[test]
fn large_document_projects_only_selected_bytes_with_original_hash_and_offsets() {
    use hieronymus::dream_config::default_dream_config;
    let (_dir, db, lease, _output) = derived_fixture();
    // Immutable evidence is normally written once; create a new large-document
    // observation with the original byte span at the start, never rewrite it.
    let document = format!("Alex walks.\n\n{}", "UNSELECTED BOOK TEXT ".repeat(100_000));
    let binding: String = db
        .query_row(
            "select binding_json from evidence_records where id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(1,'source_passage','large-document',?1,0,11,?2,?3,'now')",params![hash(&document),document,binding]).unwrap();
    let id = db.last_insert_rowid();
    let selected = select_correction_context(&db, &lease, &default_dream_config()).unwrap();
    let projection = selected.projection();
    let row = projection["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["reference"]["id"] == id)
        .unwrap();
    assert_eq!(row["selected_excerpt"], "Alex walks.");
    assert_eq!(row["reference"]["content_hash"], hash(&document));
    assert_eq!(row["reference"]["span_end"], 11);
    assert!(!projection.to_string().contains("UNSELECTED BOOK TEXT"));
    assert!(projection.to_string().len() < 16 * 1024);
}

use hieronymus::{authority::*, authority_models::*, db::open_migrated, story_applicability::*};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
fn hash(s: &str) -> String {
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
fn fixture() -> (tempfile::TempDir, Connection, DecisionRequestV1) {
    let dir = tempfile::tempdir().unwrap();
    let db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Alex-person-1','series','series:book','now','now'); insert into story_timelines(id,series_id,name) values(1,1,'story'); insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(1,1,'en','ru','Alex','A','candidate','now','now'); insert into term_rule_forms(rule_id,form_kind,surface,language) values(1,'source','Alex','en'),(1,'approved','A','ru');").unwrap();
    let req = DecisionRequestV1 {
        version: 1,
        decision_id: "10000000-0000-4000-8000-000000000001".into(),
        expected_revision: 0,
        actor_kind: ActorKind::Agent,
        origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
        evidence_refs: vec![],
        series_id: 1,
        concept_id: Some(1),
        source_language: "en".into(),
        target_language: Some("ru".into()),
        applicability: app(),
        operation: OperationV1::Activate {
            candidate_id: 1,
            candidate_revision: 1,
        },
    };
    (dir, db, req)
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
        target_language: "ru".into(),
        applicability: a,
        position_id: pos,
        paragraph_start: start,
        paragraph_end: end,
        identity_anchor: true,
        aligned_source_id: None,
        rendering: None,
        contradicts_rule: None,
        conflict_kind: None,
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
fn origin(db: &Connection, r: &DecisionRequestV1) {
    let context = serde_json::json!({"decision_id":r.decision_id,"expected_revision":r.expected_revision,"selected_source":if r.actor_kind==ActorKind::ExplicitUser{r.evidence_refs.iter().find(|e|e.id==99)}else{None},"series_id":r.series_id,"concept_id":r.concept_id,"source_language":r.source_language,"target_language":r.target_language,"applicability":r.applicability,"evidence_ids":r.evidence_refs.iter().map(|e|e.id).collect::<Vec<_>>(),"operation":r.operation});
    let context = context.to_string();
    let kind = match r.actor_kind {
        ActorKind::Agent => "agent",
        ActorKind::Dream => "dream",
        ActorKind::ExplicitUser => "console_user",
    };
    db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,?2,'test-ingress',?1,'structured correction',?3,?4,'now')",params![r.origin.0,kind,context,hash(&format!("structured correction\n{context}"))]).unwrap();
}
#[test]
fn one_or_duplicated_anchor_is_tentative() {
    for duplicate in [false, true] {
        let (_d, mut db, mut r) = fixture();
        r.evidence_refs = anchor(&db, 1, "A");
        if duplicate {
            r.evidence_refs.extend(r.evidence_refs.clone())
        }
        origin(&db, &r);
        assert!(matches!(
            DecisionStore::new(&mut db).apply(&r).unwrap(),
            DecisionResultV1::Tentative { .. }
        ));
    }
}
#[test]
fn two_disjoint_aligned_anchors_activate() {
    let (_d, mut db, mut r) = fixture();
    r.evidence_refs = anchor(&db, 1, "A");
    r.evidence_refs.extend(anchor(&db, 2, "A"));
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
}
#[test]
fn contradiction_prevents_activation() {
    let (_d, mut db, mut r) = fixture();
    r.evidence_refs = anchor(&db, 1, "A");
    r.evidence_refs.extend(anchor(&db, 2, "B"));
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r).unwrap(),
        DecisionResultV1::Tentative { .. }
    ));
}
fn ready() -> (tempfile::TempDir, Connection, DecisionRequestV1) {
    let (d, db, mut r) = fixture();
    r.evidence_refs = anchor(&db, 1, "A");
    r.evidence_refs.extend(anchor(&db, 2, "A"));
    origin(&db, &r);
    (d, db, r)
}
fn selected(db: &Connection, r: &mut DecisionRequestV1) {
    db.execute("insert or ignore into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) select 99,series_id,kind,source_identity,source_hash,0,4,content,binding_json,created_at from evidence_records where id=1",[]).unwrap();
    let h: String = db
        .query_row(
            "select source_hash from evidence_records where id=99",
            [],
            |r| r.get(0),
        )
        .unwrap();
    r.evidence_refs.push(EvidenceRef {
        kind: EvidenceKind::SourcePassage,
        id: 99,
        content_hash: h,
        span_start: 0,
        span_end: 4,
    });
}
fn next(
    r: &DecisionRequestV1,
    actor: ActorKind,
    operation: OperationV1,
    n: u8,
) -> DecisionRequestV1 {
    let mut r = r.clone();
    r.decision_id = format!("10000000-0000-4000-8000-{n:012}");
    r.origin = OriginReceiptId(format!("20000000-0000-4000-8000-{n:012}"));
    r.expected_revision += 1;
    r.actor_kind = actor;
    r.operation = operation;
    r
}
fn value(s: &str) -> RenderingV1 {
    RenderingV1 {
        source_forms: vec!["Alex".into()],
        canonical: s.into(),
        approved_variants: vec![],
        forbidden_variants: vec![],
        case_sensitive: false,
    }
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn exact_replay_precedes_revision_and_payload_conflicts() {
    let (_d, mut db, r) = ready();
    let first = DecisionStore::new(&mut db).apply(&r).unwrap();
    let replay = DecisionStore::new(&mut db).apply(&r).unwrap();
    assert!(matches!(replay, DecisionResultV1::Replayed { .. }));
    assert_eq!(first.receipt(), replay.receipt());
    assert_eq!(count(&db, "consolidation_jobs"), 1);
    assert_eq!(count(&db, "term_rule_actions"), 1);
    let mut changed = r.clone();
    changed.expected_revision = 1;
    assert_eq!(
        DecisionStore::new(&mut db).apply(&changed),
        Err(DecisionErrorV1::IdempotencyConflict)
    );
}
#[test]
fn tentative_replay_returns_exact_reasons_and_job() {
    let (_d, mut db, mut r) = fixture();
    r.evidence_refs = anchor(&db, 1, "A");
    origin(&db, &r);
    let first = DecisionStore::new(&mut db).apply(&r).unwrap();
    assert_eq!(first, DecisionStore::new(&mut db).apply(&r).unwrap());
    assert_eq!(count(&db, "consolidation_jobs"), 1);
}
#[test]
fn stale_series_and_rule_revisions_reject() {
    for rule_stale in [false, true] {
        let (_d, mut db, mut r) = ready();
        if rule_stale {
            r.operation = OperationV1::Activate {
                candidate_id: 1,
                candidate_revision: 0,
            };
            db.execute_batch("delete from origin_receipts").unwrap_err(); // immutable provenance cannot be rewritten
            r.origin = OriginReceiptId("20000000-0000-4000-8000-000000000099".into());
            origin(&db, &r);
        } else {
            r.expected_revision = 1;
            r.origin = OriginReceiptId("20000000-0000-4000-8000-000000000099".into());
            origin(&db, &r);
        }
        assert_eq!(
            DecisionStore::new(&mut db).apply(&r),
            Err(DecisionErrorV1::RevisionConflict {
                current_revision: if rule_stale { 1 } else { 0 }
            })
        );
        assert_eq!(count(&db, "decision_records"), 0);
    }
}
#[test]
fn failed_audit_rolls_back_rule_projection_and_ingestion() {
    let (_d, mut db, r) = ready();
    db.execute_batch("create trigger fail_audit before insert on term_rule_actions begin select raise(abort,'injected audit failure'); end;").unwrap();
    assert_eq!(
        DecisionStore::new(&mut db).apply(&r),
        Err(DecisionErrorV1::StorageUnavailable)
    );
    assert_eq!(
        db.query_row("select status from term_rules where id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "candidate"
    );
    for table in [
        "decision_records",
        "consolidation_jobs",
        "rule_authority",
        "applicabilities",
        "term_rule_revisions",
    ] {
        assert_eq!(count(&db, table), 0, "{table}");
    }
}
#[test]
fn learned_replacement_without_contradiction_stays_tentative() {
    let (_d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let r = next(
        &r,
        ActorKind::Dream,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("A"),
        },
        2,
    );
    origin(&db, &r);
    assert!(
        matches!(DecisionStore::new(&mut db).apply(&r).unwrap(),DecisionResultV1::Tentative{reasons,..} if reasons.contains(&TentativeReason::InsufficientEvidence))
    );
    assert_eq!(count(&db, "term_rules"), 1);
}
#[test]
fn explicit_override_resists_dream_and_legacy_lifecycle() {
    let (d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Correct {
            intent: CorrectionIntentV1::Rendering {
                replaces: Some((1, 2)),
                value: value("B"),
            },
        },
        2,
    );
    selected(&db, &mut user);
    origin(&db, &user);
    let applied = DecisionStore::new(&mut db).apply(&user).unwrap();
    let (id, revision) = *applied.receipt().affected_rules.last().unwrap();
    let dream = next(
        &user,
        ActorKind::Dream,
        OperationV1::Replace {
            rule_id: id,
            rule_revision: revision,
            rendering: value("A"),
        },
        3,
    );
    origin(&db, &dream);
    assert_eq!(
        DecisionStore::new(&mut db).apply(&dream),
        Err(DecisionErrorV1::AuthorityConflict)
    );
    let tx = db.transaction().unwrap();
    let mut replay = r.clone();
    replay.decision_id = "10000000-0000-4000-8000-000000000004".into();
    assert!(apply_decision_tx(&tx, &replay).is_err());
    drop(tx);
    drop(d);
}
#[test]
fn forged_actor_or_evidence_rejected_without_receipt() {
    for variant in 0..5 {
        let (_d, mut db, mut r) = ready();
        match variant {
            0 => r.actor_kind = ActorKind::ExplicitUser,
            1 => r.evidence_refs[0].content_hash = "bad".into(),
            2 => r.evidence_refs[0].span_end = usize::MAX,
            3 => r.evidence_refs[0].kind = EvidenceKind::Observation,
            _ => r.concept_id = Some(2),
        }
        assert!(DecisionStore::new(&mut db).apply(&r).is_err());
        assert_eq!(count(&db, "decision_records"), 0);
    }
}
fn contract(d: &tempfile::TempDir, chapter: &str) -> Vec<hieronymus::terminology::ContractTerm> {
    let config = hieronymus::data_root::HieronymusConfig::new(d.path());
    let mut context =
        hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation");
    context.volume = "I".into();
    context.chapter = chapter.into();
    context.story_viewpoint = Viewpoint::Narrator;
    hieronymus::terminology::Termbase::open(&config, &context)
        .unwrap()
        .contract("Alex")
        .unwrap()
}
#[test]
fn chapter_override_preserves_learned_rule_elsewhere_and_survives_reopen() {
    let (d, mut db, mut r) = fixture();
    let mut wide = app();
    wide.chapter_key = None;
    r.applicability = wide.clone();
    for n in 1..=4 {
        r.evidence_refs.extend(anchor_at(
            &db,
            n,
            "A",
            wide.clone(),
            if n <= 2 { 1 } else { 2 },
        ));
    }
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
    assert_eq!(contract(&d, "2")[0].canonical_translation, "A");
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Correct {
            intent: CorrectionIntentV1::Rendering {
                replaces: Some((1, 2)),
                value: value("B"),
            },
        },
        2,
    );
    user.applicability = app();
    user.evidence_refs
        .retain(|e| [1, 2, 11, 12].contains(&e.id));
    selected(&db, &mut user);
    origin(&db, &user);
    let applied = DecisionStore::new(&mut db).apply(&user).unwrap();
    assert!(matches!(applied, DecisionResultV1::Applied { .. }));
    assert_eq!(count(&db, "rule_exclusions"), 1);
    drop(db);
    assert_eq!(contract(&d, "1")[0].canonical_translation, "B");
    assert_eq!(contract(&d, "2")[0].canonical_translation, "A");
}
#[test]
fn source_name_collision_does_not_merge_concepts() {
    let (d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    db.execute_batch("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(2,'Alex-person-2','series','series:book','now','now');insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(2,2,'en','ru','Alex','B','candidate','now','now');insert into term_rule_forms(rule_id,form_kind,surface,language) values(2,'source','Alex','en'),(2,'approved','B','ru');").unwrap();
    let mut other = next(
        &r,
        ActorKind::Agent,
        OperationV1::Activate {
            candidate_id: 2,
            candidate_revision: 1,
        },
        2,
    );
    other.concept_id = Some(2);
    other.evidence_refs.clear();
    origin(&db, &other);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&other).unwrap(),
        DecisionResultV1::Tentative { .. }
    ));
    assert_eq!(contract(&d, "1").len(), 1);
    assert_eq!(
        db.query_row("select status from term_rules where id=2", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "candidate"
    );
}
#[test]
fn scope_cannot_move_learned_authority_into_explicit_scope() {
    let (_d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    selected(&db, &mut user);
    origin(&db, &user);
    let applied = DecisionStore::new(&mut db).apply(&user).unwrap();
    let (id, revision) = *applied.receipt().affected_rules.last().unwrap();
    let dream = next(
        &user,
        ActorKind::Dream,
        OperationV1::Scope {
            rule_id: id,
            rule_revision: revision,
            new_applicability: app(),
        },
        3,
    );
    origin(&db, &dream);
    assert_eq!(
        DecisionStore::new(&mut db).apply(&dream),
        Err(DecisionErrorV1::AuthorityConflict)
    );
}
#[test]
fn legacy_wrappers_cannot_archive_decision_owned_rules() {
    let (d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let config = hieronymus::data_root::HieronymusConfig::new(d.path());
    let context =
        hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation");
    let terms = hieronymus::terminology::Termbase::open(&config, &context).unwrap();
    assert!(
        terms
            .apply_action(&hieronymus::terminology::RuleActionRequest {
                rule_id: 1,
                action: hieronymus::terminology::RuleAction::Archive,
                actor: "local".into(),
                reason: "bypass".into(),
                expected_revision: 2,
                idempotency_key: "bypass".into()
            })
            .is_err()
    );
}
fn link_projection(db: &Connection) {
    db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,strength,confidence,status,created_at,updated_at) values(1,'rule','original projection','series',1,1,'active','now','now');insert into crystals_fts(rowid,text) values(1,'original projection');update term_rules set rule_crystal_id=1 where id=1;").unwrap();
}
#[test]
fn structured_variants_and_case_sensitive_forms_round_trip_into_projection_and_validation() {
    let (d, mut db, r) = ready();
    link_projection(&db);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut v = value("B");
    v.source_forms.push("Lex".into());
    v.approved_variants = vec!["Bee".into(), "Bea".into()];
    v.forbidden_variants = vec!["Wrong".into(), "Bad".into()];
    v.case_sensitive = true;
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: v.clone(),
        },
        2,
    );
    selected(&db, &mut user);
    origin(&db, &user);
    let result = DecisionStore::new(&mut db).apply(&user).unwrap();
    let (id, _) = *result.receipt().affected_rules.last().unwrap();
    let text: String = db
        .query_row("select text from crystals where id=1", [], |r| r.get(0))
        .unwrap();
    let projection: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(projection["rule_id"], id);
    assert_eq!(projection["forms"].as_array().unwrap().len(), 7);
    for form in v
        .source_forms
        .iter()
        .chain(&v.approved_variants)
        .chain(&v.forbidden_variants)
    {
        assert!(
            projection["forms"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["surface"] == *form)
        );
    }
    let config = hieronymus::data_root::HieronymusConfig::new(d.path());
    let mut context =
        hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation");
    context.volume = "I".into();
    context.chapter = "1".into();
    context.story_viewpoint = Viewpoint::Narrator;
    let terms = hieronymus::terminology::Termbase::open(&config, &context).unwrap();
    assert_eq!(terms.contract("Lex").unwrap()[0].canonical_translation, "B");
    assert!(terms.contract("lex").unwrap().is_empty());
    assert!(
        terms
            .validate("Bea", hieronymus::terminology::Source::Raw("Alex".into()))
            .unwrap()
            .is_empty()
    );
    let findings = terms
        .validate(
            "Wrong Bad",
            hieronymus::terminology::Source::Raw("Alex".into()),
        )
        .unwrap();
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.kind == "forbidden_variant")
            .count(),
        2
    );
}
#[test]
fn failed_replacement_audit_preserves_exact_projection_and_forms() {
    let (_d, mut db, r) = ready();
    link_projection(&db);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let before: String = db
        .query_row("select text from crystals where id=1", [], |r| r.get(0))
        .unwrap();
    let forms = count(&db, "term_rule_forms");
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    selected(&db, &mut user);
    origin(&db, &user);
    db.execute_batch("create trigger fail_replacement_audit before insert on term_rule_actions begin select raise(abort,'audit failure');end;").unwrap();
    assert_eq!(
        DecisionStore::new(&mut db).apply(&user),
        Err(DecisionErrorV1::StorageUnavailable)
    );
    assert_eq!(count(&db, "term_rule_forms"), forms);
    assert_eq!(count(&db, "term_rules"), 1);
    assert_eq!(count(&db, "consolidation_jobs"), 1);
    assert_eq!(
        db.query_row("select text from crystals where id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        before
    );
}
#[test]
fn caller_transaction_can_rollback_successful_decision() {
    let (_d, mut db, r) = ready();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        apply_decision_tx(&tx, &r).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
    tx.rollback().unwrap();
    assert_eq!(count(&db, "decision_records"), 0);
    assert_eq!(count(&db, "rule_authority"), 0);
}
#[test]
fn broader_requested_correction_receipts_store_exact_effective_intersection() {
    let (_d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    user.applicability.chapter_key = None;
    selected(&db, &mut user);
    origin(&db, &user);
    let applied = DecisionStore::new(&mut db).apply(&user).unwrap();
    assert_eq!(applied.receipt().effective_applicability, app());
    let replay = DecisionStore::new(&mut db).apply(&user).unwrap();
    assert_eq!(applied.receipt(), replay.receipt());
    let stored: String = db
        .query_row(
            "select canonical_request from decision_records where decision_id=?",
            [&user.decision_id],
            |r| r.get(0),
        )
        .unwrap();
    let request: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert!(request["applicability"]["chapter_key"].is_null());
    assert_eq!(count(&db, "consolidation_jobs"), 2);
}
#[test]
fn registered_nondefault_languages_are_valid_but_unnormalized_values_reject() {
    for normalized in [false, true] {
        let (_d, mut db, mut r) = fixture();
        db.execute("insert into series_language_tags(series_id,language_tag,created_at) values(1,'fr','now')",[]).unwrap();
        r.target_language = Some(if normalized { "fr" } else { "FR" }.into());
        db.execute(
            "update term_rules set target_language=?1 where id=1",
            [r.target_language.as_ref().unwrap()],
        )
        .unwrap();
        origin(&db, &r);
        let result = DecisionStore::new(&mut db).apply(&r);
        if normalized {
            assert!(matches!(
                result.unwrap(),
                DecisionResultV1::Tentative { .. }
            ));
        } else {
            assert_eq!(result, Err(DecisionErrorV1::LanguageMismatch));
        }
    }
}
#[test]
fn corpus_work_intent_is_atomic_and_replay_does_not_increment_it() {
    let (_d, mut db, r) = ready();
    assert_eq!(hieronymus::rag::current_corpus_revision(&db).unwrap(), 0);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    assert_eq!(hieronymus::rag::current_corpus_revision(&db).unwrap(), 1);
    assert_eq!(
        hieronymus::rag::pending_semantic_work_intent(&db).unwrap(),
        Some(1)
    );
    DecisionStore::new(&mut db).apply(&r).unwrap();
    assert_eq!(hieronymus::rag::current_corpus_revision(&db).unwrap(), 1);
}
#[test]
fn unselected_contradiction_cannot_be_hidden() {
    let (_d, mut db, mut r) = fixture();
    r.evidence_refs = anchor(&db, 1, "A");
    anchor(&db, 2, "B");
    origin(&db, &r);
    assert!(
        matches!(DecisionStore::new(&mut db).apply(&r).unwrap(),DecisionResultV1::Tentative{reasons,..} if reasons==vec![TentativeReason::ConflictingEvidence])
    );
}
#[test]
fn non_utf8_boundaries_are_evidence_errors() {
    let (_d, mut db, mut r) = fixture();
    anchor(&db, 1, "A");
    let content = "Свет";
    db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) select 50,series_id,kind,'unicode',?1,1,2,?2,binding_json,created_at from evidence_records where id=1",params![hash(content),content]).unwrap();
    r.evidence_refs = vec![EvidenceRef {
        kind: EvidenceKind::SourcePassage,
        id: 50,
        content_hash: hash(content),
        span_start: 1,
        span_end: 2,
    }];
    origin(&db, &r);
    assert_eq!(
        DecisionStore::new(&mut db).apply(&r),
        Err(DecisionErrorV1::EvidenceMismatch)
    );
}
#[test]
fn moving_a_separate_learned_rule_into_user_scope_is_rejected() {
    let (_d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    selected(&db, &mut user);
    origin(&db, &user);
    DecisionStore::new(&mut db).apply(&user).unwrap();
    db.execute_batch("insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(3,1,'en','ru','Alex','A','candidate','now','now');insert into term_rule_forms(rule_id,form_kind,surface,language) values(3,'source','Alex','en'),(3,'approved','A','ru');").unwrap();
    let mut learned = next(
        &user,
        ActorKind::Agent,
        OperationV1::Activate {
            candidate_id: 3,
            candidate_revision: 1,
        },
        3,
    );
    learned.applicability.chapter_key = Some("2".into());
    learned.evidence_refs = anchor_at(&db, 3, "A", learned.applicability.clone(), 2);
    learned
        .evidence_refs
        .extend(anchor_at(&db, 4, "A", learned.applicability.clone(), 2));
    origin(&db, &learned);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&learned).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
    let mut moved = next(
        &learned,
        ActorKind::Dream,
        OperationV1::Scope {
            rule_id: 3,
            rule_revision: 2,
            new_applicability: app(),
        },
        4,
    );
    moved.applicability = app();
    moved.evidence_refs = r.evidence_refs.clone();
    origin(&db, &moved);
    assert_eq!(
        DecisionStore::new(&mut db).apply(&moved),
        Err(DecisionErrorV1::AuthorityConflict)
    );
    assert_eq!(count(&db, "consolidation_jobs"), 3);
}
#[test]
fn finite_recurrence_does_not_activate_unbounded_series_authority() {
    let (_d, mut db, mut r) = fixture();
    r.applicability.volume_key = None;
    r.applicability.chapter_key = None;
    r.evidence_refs = anchor_at(&db, 1, "A", r.applicability.clone(), 1);
    r.evidence_refs
        .extend(anchor_at(&db, 2, "A", r.applicability.clone(), 1));
    origin(&db, &r);
    assert!(
        matches!(DecisionStore::new(&mut db).apply(&r).unwrap(),DecisionResultV1::Tentative{reasons,..} if reasons.contains(&TentativeReason::InsufficientEvidence))
    );
}
#[test]
fn explicit_scoped_correction_masks_trusted_local_unowned_legacy_rule() {
    let (d, mut db, r) = ready();
    db.execute("update term_rules set status='active' where id=1", [])
        .unwrap();
    let mut user = next(
        &r,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 1,
            rendering: value("B"),
        },
        2,
    );
    user.expected_revision = 0;
    selected(&db, &mut user);
    origin(&db, &user);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&user).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
    assert_eq!(contract(&d, "1")[0].canonical_translation, "B");
    assert_eq!(contract(&d, "2")[0].canonical_translation, "A");
}
#[test]
fn source_grounded_explanation_and_two_new_anchors_allow_learned_replacement() {
    let (_d, mut db, r) = ready();
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let mut replacement = next(
        &r,
        ActorKind::Dream,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    replacement.evidence_refs.clear();
    for n in 1..=2 {
        let (content, raw, start, end): (String, String, i64, i64) = db
            .query_row(
                "select content,binding_json,span_start,span_end from evidence_records where id=?",
                [n],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        let content = format!(
            "{content}\n\nSource correction {n}: the mapping A was erroneous; B is established."
        );
        let mut binding: EvidenceBindingV1 = serde_json::from_str(&raw).unwrap();
        if n == 1 {
            binding.contradicts_rule = Some(1);
            binding.conflict_kind = Some("erroneous_mapping".into());
        }
        db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,1,'source_passage',?2,?3,?4,?5,?6,?7,'now')",params![n+20,format!("corrected-{n}"),hash(&content),start,end,content,serde_json::to_string(&binding).unwrap()]).unwrap();
        replacement.evidence_refs.push(EvidenceRef {
            kind: EvidenceKind::SourcePassage,
            id: n + 20,
            content_hash: hash(&content),
            span_start: start as usize,
            span_end: end as usize,
        });
        binding.aligned_source_id = Some(n + 20);
        binding.rendering = Some("B".into());
        db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,1,'aligned_rendering',?2,?3,0,1,'B',?4,'now')",params![n+30,format!("new-alignment-{n}"),hash("B"),serde_json::to_string(&binding).unwrap()]).unwrap();
        replacement.evidence_refs.push(EvidenceRef {
            kind: EvidenceKind::AlignedRendering,
            id: n + 30,
            content_hash: hash("B"),
            span_start: 0,
            span_end: 1,
        });
    }
    origin(&db, &replacement);
    let applied = DecisionStore::new(&mut db).apply(&replacement).unwrap();
    assert!(matches!(applied, DecisionResultV1::Applied { .. }));
    assert_eq!(count(&db, "term_rules"), 2);
    assert_eq!(
        db.query_row("select status from term_rules where id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "superseded"
    );
}
#[test]
fn learned_character_gated_scope_uses_positive_character_eligibility() {
    let (_d, mut db, mut r) = fixture();
    r.applicability.knowledge_gates = vec![KnowledgeGateV1 {
        viewpoint: KnowledgeViewpoint::Character(1),
        known_from: None,
        known_until: None,
    }];
    r.evidence_refs = anchor_at(&db, 1, "A", r.applicability.clone(), 1);
    r.evidence_refs
        .extend(anchor_at(&db, 2, "A", r.applicability.clone(), 1));
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
}
fn wide_then_chapter_user() -> (
    tempfile::TempDir,
    Connection,
    DecisionRequestV1,
    DecisionRequestV1,
    i64,
    u64,
) {
    let (d, mut db, mut a) = fixture();
    a.applicability.chapter_key = None;
    for n in 1..=4 {
        a.evidence_refs.extend(anchor_at(
            &db,
            n,
            "A",
            a.applicability.clone(),
            if n <= 2 { 1 } else { 2 },
        ));
    }
    origin(&db, &a);
    DecisionStore::new(&mut db).apply(&a).unwrap();
    let mut b = next(
        &a,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 2,
            rendering: value("B"),
        },
        2,
    );
    b.applicability = app();
    b.evidence_refs.retain(|e| [1, 2, 11, 12].contains(&e.id));
    selected(&db, &mut b);
    origin(&db, &b);
    let result = DecisionStore::new(&mut db).apply(&b).unwrap();
    let (id, revision) = *result.receipt().affected_rules.last().unwrap();
    (d, db, a, b, id, revision)
}
#[test]
fn repeated_chapter_correction_subtracts_retained_exclusions() {
    let (d, mut db, _a, b, id, revision) = wide_then_chapter_user();
    let c = next(
        &b,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: id,
            rule_revision: revision,
            rendering: value("C"),
        },
        3,
    );
    origin(&db, &c);
    let result = DecisionStore::new(&mut db).apply(&c).unwrap();
    assert!(matches!(result, DecisionResultV1::Applied { .. }));
    assert_eq!(contract(&d, "1")[0].canonical_translation, "C");
    assert_eq!(contract(&d, "2")[0].canonical_translation, "A");
    assert_eq!(
        DecisionStore::new(&mut db).apply(&c).unwrap().receipt(),
        result.receipt()
    );
}
#[test]
fn broader_correction_inherits_prior_exclusions() {
    let (d, mut db, a, b, _id, _revision) = wide_then_chapter_user();
    let mut c = next(
        &b,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 3,
            rendering: value("C"),
        },
        3,
    );
    c.applicability = a.applicability.clone();
    c.evidence_refs = a.evidence_refs.clone();
    selected(&db, &mut c);
    origin(&db, &c);
    let result = DecisionStore::new(&mut db).apply(&c).unwrap();
    assert!(matches!(result, DecisionResultV1::Applied { .. }));
    assert_eq!(contract(&d, "1")[0].canonical_translation, "B");
    assert_eq!(contract(&d, "2")[0].canonical_translation, "C");
    assert_eq!(result.receipt().effective_applicability, a.applicability);
    assert_eq!(result.receipt().effective_exclusions, vec![app()]);
    let affected = result.receipt().affected_rules.last().unwrap().0;
    assert_eq!(
        db.query_row(
            "select count(*) from rule_exclusions where rule_id=?",
            [affected],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let mut d_request = next(
        &c,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: _id,
            rule_revision: _revision,
            rendering: value("D"),
        },
        4,
    );
    d_request.applicability = app();
    origin(&db, &d_request);
    DecisionStore::new(&mut db).apply(&d_request).unwrap();
    assert_eq!(contract(&d, "1")[0].canonical_translation, "D");

    assert_eq!(
        DecisionStore::new(&mut db).apply(&c).unwrap().receipt(),
        result.receipt()
    );
}
fn add_candidate(db: &Connection, id: i64, concept: i64, rendering: &str) {
    db.execute("insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(?1,?2,'en','ru','Alex',?3,'candidate','now','now')",params![id,concept,rendering]).unwrap();
    db.execute("insert into term_rule_forms(rule_id,form_kind,surface,language) values(?1,'source','Alex','en'),(?1,'approved',?2,'ru')",params![id,rendering]).unwrap();
}
#[test]
fn disjoint_positive_viewpoints_do_not_conflict_or_count_as_contradictions() {
    let (_d, mut db, mut a) = fixture();
    a.actor_kind = ActorKind::ExplicitUser;
    a.applicability.knowledge_gates[0].viewpoint = KnowledgeViewpoint::Narrator;
    for n in 1..=2 {
        a.evidence_refs
            .extend(anchor_at(&db, n, "A", a.applicability.clone(), 1));
    }
    selected(&db, &mut a);
    origin(&db, &a);
    DecisionStore::new(&mut db).apply(&a).unwrap();
    add_candidate(&db, 2, 1, "B");
    let mut b = next(
        &a,
        ActorKind::Agent,
        OperationV1::Activate {
            candidate_id: 2,
            candidate_revision: 1,
        },
        2,
    );
    b.applicability.knowledge_gates[0].viewpoint = KnowledgeViewpoint::Character(1);
    b.evidence_refs.clear();
    for n in 3..=4 {
        b.evidence_refs
            .extend(anchor_at(&db, n, "B", b.applicability.clone(), 1));
    }
    origin(&db, &b);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&b).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
}
#[test]
fn disjoint_knowledge_intervals_do_not_conflict_or_count_as_contradictions() {
    let (_d, mut db, mut a) = fixture();
    a.actor_kind = ActorKind::ExplicitUser;
    a.applicability.knowledge_gates[0].known_until = Some(2);
    a.evidence_refs = anchor_at(&db, 1, "A", a.applicability.clone(), 1);
    db.execute("insert into story_positions(id,timeline_id,volume_key,chapter_key,scene_key,ordinal,evidence_id) values(2,1,'I','1','later',2,1)",[]).unwrap();
    a.evidence_refs
        .extend(anchor_at(&db, 2, "A", a.applicability.clone(), 1));
    selected(&db, &mut a);
    origin(&db, &a);
    DecisionStore::new(&mut db).apply(&a).unwrap();
    add_candidate(&db, 2, 1, "B");
    let mut b = next(
        &a,
        ActorKind::Agent,
        OperationV1::Activate {
            candidate_id: 2,
            candidate_revision: 1,
        },
        2,
    );
    b.applicability.knowledge_gates[0].known_until = None;
    b.applicability.knowledge_gates[0].known_from = Some(2);
    b.evidence_refs.clear();
    for n in 3..=4 {
        b.evidence_refs
            .extend(anchor_at(&db, n, "B", b.applicability.clone(), 2));
    }
    origin(&db, &b);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&b).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
}
#[test]
fn mixed_anchor_applicabilities_do_not_combine_support() {
    let (_d, mut db, mut r) = fixture();
    let mut wide = app();
    wide.chapter_key = None;
    r.evidence_refs = anchor_at(&db, 1, "A", wide, 1);
    r.evidence_refs.extend(anchor_at(&db, 2, "A", app(), 1));
    origin(&db, &r);
    assert!(
        matches!(DecisionStore::new(&mut db).apply(&r).unwrap(),DecisionResultV1::Tentative{reasons,..} if reasons.contains(&TentativeReason::InsufficientEvidence))
    );
}
#[test]
fn established_outside_scope_alex_keeps_occurrence_ambiguous() {
    for outside_viewpoint in [false, true] {
        let (d, mut db, a) = ready();
        DecisionStore::new(&mut db).apply(&a).unwrap();
        db.execute("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(2,'Alex-person-2','series','series:book','now','now')",[]).unwrap();
        add_candidate(&db, 2, 2, "B");
        let mut b = next(
            &a,
            ActorKind::Agent,
            OperationV1::Activate {
                candidate_id: 2,
                candidate_revision: 1,
            },
            2,
        );
        b.concept_id = Some(2);
        b.applicability.chapter_key = Some(if outside_viewpoint { "1" } else { "2" }.into());
        if outside_viewpoint {
            b.applicability.knowledge_gates[0].viewpoint = KnowledgeViewpoint::Character(2);
        }
        b.evidence_refs.clear();
        for n in 3..=4 {
            b.evidence_refs.extend(anchor_for(
                &db,
                n,
                "B",
                b.applicability.clone(),
                if outside_viewpoint { 1 } else { 2 },
                2,
            ));
        }
        origin(&db, &b);
        assert!(matches!(
            DecisionStore::new(&mut db).apply(&b).unwrap(),
            DecisionResultV1::Applied { .. }
        ));
        assert!(contract(&d, "1").is_empty());
        let config = hieronymus::data_root::HieronymusConfig::new(d.path());
        let mut context =
            hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation");
        context.volume = "I".into();
        context.chapter = "1".into();
        context.story_viewpoint = Viewpoint::Narrator;
        let terms = hieronymus::terminology::Termbase::open(&config, &context).unwrap();
        assert!(
            terms
                .validate("A", hieronymus::terminology::Source::Raw("Alex".into()))
                .unwrap()
                .iter()
                .any(|finding| finding.kind == "ambiguous_source")
        );
    }
}
#[test]
fn matching_anchor_group_can_activate_despite_other_binding_group() {
    let (_d, mut db, mut r) = fixture();
    let mut wide = app();
    wide.chapter_key = None;
    r.evidence_refs = anchor_at(&db, 1, "A", wide, 1);
    r.evidence_refs.extend(anchor_at(&db, 2, "A", app(), 1));
    r.evidence_refs.extend(anchor_at(&db, 3, "A", app(), 1));
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
}
#[test]
fn correction_cannot_reenter_a_wholly_excluded_old_region() {
    let (_d, mut db, _a, b, _id, _revision) = wide_then_chapter_user();
    let c = next(
        &b,
        ActorKind::ExplicitUser,
        OperationV1::Replace {
            rule_id: 1,
            rule_revision: 3,
            rendering: value("C"),
        },
        3,
    );
    origin(&db, &c);
    assert_eq!(
        DecisionStore::new(&mut db).apply(&c),
        Err(DecisionErrorV1::ApplicabilityConflict)
    );
    assert_eq!(count(&db, "consolidation_jobs"), 2);
}

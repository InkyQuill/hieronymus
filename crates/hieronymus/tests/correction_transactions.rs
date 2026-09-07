use hieronymus::{
    authority::DecisionStore, authority_models::*, db::open_migrated, story_applicability::*,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
fn fixture() -> (tempfile::TempDir, Connection, DecisionRequestV1) {
    let dir = tempfile::tempdir().unwrap();
    let db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Mira','series','series:book','now','now'); insert into story_timelines(id,series_id,name) values(1,1,'story'); insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(1,1,'observation','manifest','hash',0,1,'x','{}','now'); insert into story_positions(id,timeline_id,volume_key,chapter_key,ordinal,evidence_id) values(1,1,'I','early',1,1),(2,1,'I','late',2,1); insert into applicabilities(id,series_id,timeline_id,volume_key,scope_predicates_json,metadata_state) values(1,1,1,'I','[]','resolved'); insert into knowledge_gates(applicability_id,viewpoint_kind) values(1,'all'); insert into memory_claims(id,series_id,concept_id,text,revision,status,qualification,applicability_id,created_at,updated_at) values(1,1,1,'Mira knows the secret',1,'current','',1,'now','now');").unwrap();
    let app = ApplicabilityV1 {
        series_id: 1,
        timeline_id: Some(1),
        volume_key: Some("I".into()),
        chapter_key: Some("late".into()),
        scope_predicates: vec![],
        valid_from: None,
        valid_until: None,
        metadata_state: MetadataState::Resolved,
        knowledge_gates: vec![KnowledgeGateV1 {
            viewpoint: KnowledgeViewpoint::All,
            known_from: None,
            known_until: None,
        }],
    };
    let r = DecisionRequestV1 {
        version: 1,
        decision_id: "10000000-0000-4000-8000-000000000001".into(),
        expected_revision: 0,
        actor_kind: ActorKind::ExplicitUser,
        origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
        evidence_refs: vec![],
        series_id: 1,
        concept_id: Some(1),
        source_language: "en".into(),
        target_language: None,
        applicability: app,
        operation: OperationV1::Correct {
            intent: CorrectionIntentV1::Fact {
                claim_id: 1,
                claim_revision: 1,
                effect: FactEffect::Invalidate,
            },
        },
    };
    (dir, db, r)
}
fn origin(db: &Connection, r: &DecisionRequestV1) {
    let context=serde_json::json!({"decision_id":r.decision_id,"expected_revision":r.expected_revision,"selected_source":null,"series_id":r.series_id,"concept_id":r.concept_id,"source_language":r.source_language,"target_language":r.target_language,"applicability":r.applicability,"evidence_ids":[],"operation":r.operation}).to_string();
    let text = "that memory is wrong";
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{text}\n{context}").as_bytes())
    );
    let kind = if r.actor_kind == ActorKind::ExplicitUser {
        "console_user"
    } else {
        "agent"
    };
    db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,?2,'test',?1,?3,?4,?5,'now')",params![r.origin.0,kind,text,context,hash]).unwrap();
}
#[test]
fn invalidation_commits_without_replacement_and_replays_exactly() {
    let (_dir, mut db, r) = fixture();
    origin(&db, &r);
    let result = DecisionStore::new(&mut db).apply(&r).unwrap();
    assert!(matches!(result, DecisionResultV1::Applied { .. }));
    assert_eq!(result.receipt().affected_claims, vec![(1, 2)]);
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select status from memory_claims where id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "current"
    );
    assert_eq!(
        db.query_row("select effect from claim_effects", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "invalid"
    );
    assert_eq!(
        DecisionStore::new(&mut db).apply(&r).unwrap().receipt(),
        result.receipt()
    );
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from claim_effects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn qualification_is_verbatim() {
    let (_dir, mut db, mut r) = fixture();
    let qualification = "Mira suspects this; it is not confirmed";
    r.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Fact {
            claim_id: 1,
            claim_revision: 1,
            effect: FactEffect::Qualify {
                qualification: qualification.into(),
            },
        },
    };
    origin(&db, &r);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    assert_eq!(
        db.query_row("select qualification from claim_effects", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        qualification
    );
}
#[test]
fn writes_roll_back_at_each_ingestion_boundary() {
    for trigger in [
        "create trigger failure before insert on claim_effects begin select raise(abort,'before effect'); end",
        "create trigger failure after insert on claim_effects begin select raise(abort,'after effect'); end",
        "create trigger failure after insert on consolidation_jobs begin select raise(abort,'after job'); end",
        "create trigger failure before update of result_json on decision_records begin select raise(abort,'before receipt'); end",
    ] {
        let (_dir, mut db, r) = fixture();
        origin(&db, &r);
        db.execute_batch(trigger).unwrap();
        assert_eq!(
            DecisionStore::new(&mut db).apply(&r).unwrap_err(),
            DecisionErrorV1::StorageUnavailable
        );
        for table in ["claim_effects", "decision_records", "consolidation_jobs"] {
            assert_eq!(
                db.query_row(&format!("select count(*) from {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            db.query_row("select revision from memory_claims where id=1", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
}

#[test]
fn scoped_invalidation_preserves_earlier_fact_after_reopen() {
    let (dir, mut db, r) = fixture();
    origin(&db, &r);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    drop(db);
    let db = open_migrated(&dir.path().join("hieronymus.sqlite")).unwrap();
    let mut q = StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(1),
        viewpoint: Viewpoint::Narrator,
        scope_predicates: vec!["volume:I".into(), "chapter:early".into()],
        mode: QueryMode::Current,
    };
    assert_eq!(
        hieronymus::claim_reads::claim_disposition(&db, 1, &q).unwrap(),
        hieronymus::claim_reads::ClaimDisposition::Current
    );
    q.position_id = Some(2);
    q.scope_predicates = vec!["volume:I".into(), "chapter:late".into()];
    assert_eq!(
        hieronymus::claim_reads::claim_disposition(&db, 1, &q).unwrap(),
        hieronymus::claim_reads::ClaimDisposition::Invalid
    );
}

fn fact_anchors(db: &Connection, r: &mut DecisionRequestV1, duplicate: bool, contradicted: i64) {
    let text = "Mira never learned the secret.\n\nMira cannot name the secret.";
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    for (id, start, end) in [(2, 0, 30), (3, 32, text.len())] {
        let binding = serde_json::json!({"concept_id":1,"source_language":"en","target_language":null,"applicability":r.applicability,"position_id":2,"paragraph_start":start,"paragraph_end":end,"identity_anchor":true,"aligned_source_id":null,"rendering":null,"contradicts_rule":null,"contradicts_claim":contradicted,"claim_effect":"Invalidate","conflict_kind":"erroneous_mapping"});
        db.execute("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,1,'source_passage','book',?2,?3,?4,?5,?6,'now')",params![id,hash,start as i64,end as i64,text,binding.to_string()]).unwrap();
        r.evidence_refs.push(EvidenceRef {
            id,
            kind: EvidenceKind::SourcePassage,
            content_hash: hash.clone(),
            span_start: start,
            span_end: end,
        });
    }
    if duplicate {
        r.evidence_refs[1] = r.evidence_refs[0].clone();
    }
}
#[test]
fn learned_factual_revision_requires_independent_bound_anchors() {
    for duplicate in [false, true] {
        let (_dir, mut db, mut r) = fixture();
        r.actor_kind = ActorKind::Agent;
        fact_anchors(&db, &mut r, duplicate, 1);
        // Fixture origin binds every selected immutable evidence reference.
        origin_with_evidence(&db, &r);
        let result = DecisionStore::new(&mut db).apply(&r).unwrap();
        assert_eq!(
            matches!(result, DecisionResultV1::Applied { .. }),
            !duplicate
        );
        assert_eq!(
            db.query_row("select count(*) from claim_effects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            if duplicate { 0 } else { 1 }
        );
    }
}
fn origin_with_evidence(db: &Connection, r: &DecisionRequestV1) {
    let mut context = serde_json::to_value(hieronymus::authority::OriginContextV1 {
        decision_id: r.decision_id.clone(),
        expected_revision: r.expected_revision,
        selected_source: None,
        series_id: r.series_id,
        concept_id: r.concept_id,
        source_language: r.source_language.clone(),
        target_language: r.target_language.clone(),
        applicability: r.applicability.clone(),
        evidence_ids: r.evidence_refs.iter().map(|e| e.id).collect(),
        operation: r.operation.clone(),
    })
    .unwrap();
    context["evidence_ids"] =
        serde_json::json!(r.evidence_refs.iter().map(|e| e.id).collect::<Vec<_>>());
    let context = context.to_string();
    let text = "source-grounded correction";
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{text}\n{context}").as_bytes())
    );
    db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'agent','test',?1,?2,?3,?4,'now')",params![r.origin.0,text,context,hash]).unwrap();
}

#[test]
fn real_capture_current_recall_filters_invalid_before_limit() {
    use hieronymus::{
        claim_capture::ClaimInput,
        data_root::HieronymusConfig,
        memory_models::TranslationContext,
        recall::RecallService,
        workspace::{ShortTermMemoryInput, WorkspaceStore},
    };
    let (dir, mut db, mut r) = fixture();
    let config = HieronymusConfig::new(dir.path());
    let context = TranslationContext::new("book", "en", "ru", "translation")
        .volume("I")
        .chapter("late");
    let store = WorkspaceStore::open(&config).unwrap();
    let session = store.start_session(&context).unwrap();
    let mut bad = ShortTermMemoryInput::new("note", "secret secret secret obsolete");
    bad.claims = vec![ClaimInput {
        text: bad.text.clone(),
        concept_id: Some(1),
        applicability: r.applicability.clone(),
    }];
    let bad = store.add_short_term_memory(session.id, &bad).unwrap();
    let good = store
        .add_short_term_memory(
            session.id,
            &ShortTermMemoryInput::new("note", "secret valid retained"),
        )
        .unwrap();
    let claim = db
        .query_row(
            "select claim_id from claim_bindings where short_term_id=?",
            [bad.id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    r.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Fact {
            claim_id: claim,
            claim_revision: 1,
            effect: FactEffect::Invalidate,
        },
    };
    origin(&db, &r);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let response = RecallService::open(&config)
        .unwrap()
        .recall(session.id, &context, "secret", 1)
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].item_id(), good.id);
    assert_eq!(response.resulting_revision, 1);
}

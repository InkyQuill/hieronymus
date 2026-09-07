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
    fact_anchors_custom(db, r, duplicate, contradicted, |_| {});
}
fn fact_anchors_custom(
    db: &Connection,
    r: &mut DecisionRequestV1,
    duplicate: bool,
    contradicted: i64,
    mut alter: impl FnMut(&mut serde_json::Value),
) {
    let effect = match &r.operation {
        OperationV1::Correct {
            intent: CorrectionIntentV1::Fact { effect, .. },
        } => effect.clone(),
        _ => unreachable!(),
    };
    let text = "Mira never learned the secret.\n\nMira cannot name the secret.";
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    for (id, start, end) in [(2, 0, 30), (3, 32, text.len())] {
        let mut binding = serde_json::json!({"concept_id":1,"source_language":"en","target_language":null,"applicability":r.applicability,"position_id":2,"paragraph_start":start,"paragraph_end":end,"identity_anchor":true,"aligned_source_id":null,"rendering":null,"contradicts_rule":null,"contradicts_claim":contradicted,"claim_effect":effect,"conflict_kind":"erroneous_mapping"});
        alter(&mut binding);
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
    let mut good_input = ShortTermMemoryInput::new("note", "secret valid retained");
    good_input.claims = vec![ClaimInput {
        text: good_input.text.clone(),
        concept_id: Some(1),
        applicability: r.applicability.clone(),
    }];
    let good = store
        .add_short_term_memory(session.id, &good_input)
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
    let source = store
        .search_short_term_memories(session.id, "secret", 10)
        .unwrap();
    let bad_source = source.iter().find(|item| item.id == bad.id).unwrap();
    assert!(bad_source.claim_annotation.source_inspection);
    assert!(
        bad_source
            .claim_annotation
            .claims
            .iter()
            .flat_map(|claim| &claim.effects)
            .any(|effect| effect.disposition == hieronymus::claim_reads::ClaimDisposition::Invalid)
    );
}
#[test]
fn learned_qualification_is_verbatim_and_unevidenced_signals_remain_advisory() {
    for evidenced in [true, false] {
        let (_d, mut db, mut r) = fixture();
        r.actor_kind = ActorKind::Agent;
        r.operation = OperationV1::Correct {
            intent: CorrectionIntentV1::Fact {
                claim_id: 1,
                claim_revision: 1,
                effect: FactEffect::Qualify {
                    qualification: "Only a suspicion.".into(),
                },
            },
        };
        if evidenced {
            fact_anchors(&db, &mut r, false, 1);
        }
        origin_with_evidence(&db, &r);
        let result = DecisionStore::new(&mut db).apply(&r).unwrap();
        assert_eq!(
            matches!(result, DecisionResultV1::Applied { .. }),
            evidenced
        );
        assert_eq!(
            db.query_row("select count(*) from claim_effects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            i64::from(evidenced)
        );
        if evidenced {
            assert_eq!(
                db.query_row("select qualification from claim_effects", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "Only a suspicion."
            );
        } else {
            let stored: String = db
                .query_row("select canonical_request from decision_records", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
                serde_json::to_value(&r).unwrap()
            );
        }
    }
}
#[test]
fn factual_evidence_rejects_wrong_identity_scope_and_missing_direct_contradiction() {
    for defect in ["claim", "concept", "series", "scope", "missing"] {
        let (_d, mut db, mut r) = fixture();
        r.actor_kind = ActorKind::Agent;
        fact_anchors_custom(&db, &mut r, false, 1, |b| match defect {
            "claim" => b["contradicts_claim"] = serde_json::json!(2),
            "concept" => b["concept_id"] = serde_json::json!(2),
            "series" => b["applicability"]["series_id"] = serde_json::json!(2),
            "scope" => b["applicability"]["chapter_key"] = serde_json::json!("early"),
            _ => {
                b["contradicts_claim"] = serde_json::Value::Null;
                b["claim_effect"] = serde_json::Value::Null;
                b["conflict_kind"] = serde_json::Value::Null;
            }
        });
        origin_with_evidence(&db, &r);
        let result = DecisionStore::new(&mut db).apply(&r);
        if defect == "missing" {
            assert!(matches!(result, Ok(DecisionResultV1::Tentative { .. })));
        } else {
            assert!(result.is_err(), "{defect}");
        }
        assert_eq!(
            db.query_row("select count(*) from claim_effects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn stale_claim_and_series_revisions_and_explicit_masks_reject_corrections() {
    for defect in ["series", "claim", "protected"] {
        let (_d, mut db, mut r) = fixture();
        origin(&db, &r);
        DecisionStore::new(&mut db).apply(&r).unwrap();
        r.decision_id = "10000000-0000-4000-8000-000000000002".into();
        r.origin = OriginReceiptId("20000000-0000-4000-8000-000000000002".into());
        r.expected_revision = if defect == "series" { 0 } else { 1 };
        r.operation = OperationV1::Correct {
            intent: CorrectionIntentV1::Fact {
                claim_id: 1,
                claim_revision: if defect == "claim" { 1 } else { 2 },
                effect: FactEffect::Qualify {
                    qualification: "Maybe.".into(),
                },
            },
        };
        r.actor_kind = ActorKind::Agent;
        fact_anchors(&db, &mut r, false, 1);
        origin_with_evidence(&db, &r);
        let result = DecisionStore::new(&mut db).apply(&r);
        if defect == "protected" {
            assert!(matches!(result, Err(DecisionErrorV1::AuthorityConflict)));
        } else {
            assert!(matches!(
                result,
                Err(DecisionErrorV1::RevisionConflict { .. })
            ));
        }
        assert_eq!(
            db.query_row("select count(*) from claim_effects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
fn table_bytes(db: &Connection, table: &str) -> Vec<Vec<String>> {
    let mut s = db
        .prepare(&format!("select * from {table} order by rowid"))
        .unwrap();
    let columns = s.column_count();
    s.query_map([], |r| {
        (0..columns)
            .map(|i| Ok(format!("{:?}", r.get_ref(i)?)))
            .collect()
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}
fn relevance_fixture(db: &Connection, r: &mut DecisionRequestV1) {
    db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,series_slug,strength,confidence,status,created_at,updated_at) values(1,'lesson','Mira knows','series','book',0.5,0.5,'active','now','now'); insert into task_sessions(id,series_slug,source_language,target_language,task_type,status,created_at,last_activity_at) values(1,'book','en','ru','translation','active','now','now'); insert into crystal_activations(id,crystal_id,session_id,recall_query,rank,score,recall_id,created_at) values(1,1,1,'Mira',1,1.0,'recall-one','now'); insert into term_rules(concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(1,'en','ru','Mira','Мира','candidate','now','now');").unwrap();
    r.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Relevance {
            recall_id: "recall-one".into(),
            useful: vec![],
            missed: vec![1],
        },
    };
    origin(db, r);
}
#[test]
fn relevance_scores_once_after_dropped_response_and_keeps_facts_rules_byte_equivalent() {
    let (_d, mut db, mut r) = fixture();
    relevance_fixture(&db, &mut r);
    let facts = table_bytes(&db, "memory_claims");
    let rules = table_bytes(&db, "term_rules");
    let receipt = DecisionStore::new(&mut db)
        .apply(&r)
        .unwrap()
        .receipt()
        .clone();
    let replay = DecisionStore::new(&mut db).apply(&r).unwrap();
    assert_eq!(&receipt, replay.receipt());
    let (strength, confidence): (f64, f64) = db
        .query_row(
            "select strength,confidence from crystals where id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((strength - 0.45).abs() < 1e-9);
    assert!((confidence - 0.47).abs() < 1e-9);
    assert_eq!(table_bytes(&db, "memory_claims"), facts);
    assert_eq!(table_bytes(&db, "term_rules"), rules);
    assert_eq!(
        db.query_row(
            "select count(*) from memory_events where event_type='recalled_miss'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from consolidation_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    r.decision_id = "10000000-0000-4000-8000-000000000002".into();
    r.origin = OriginReceiptId("20000000-0000-4000-8000-000000000002".into());
    origin(&db, &r);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&r),
        Err(DecisionErrorV1::RevisionConflict { .. })
    ));
}
#[test]
fn relevance_failures_roll_back_scores_events_activation_and_job() {
    for trigger in [
        "before insert on memory_events",
        "after insert on memory_events",
        "after insert on consolidation_jobs",
        "before update of result_json on decision_records",
    ] {
        let (_d, mut db, mut r) = fixture();
        relevance_fixture(&db, &mut r);
        let crystals = table_bytes(&db, "crystals");
        let activations = table_bytes(&db, "crystal_activations");
        let facts = table_bytes(&db, "memory_claims");
        let rules = table_bytes(&db, "term_rules");
        db.execute_batch(&format!(
            "create trigger fail {trigger} begin select raise(abort,'failure'); end"
        ))
        .unwrap();
        assert!(DecisionStore::new(&mut db).apply(&r).is_err());
        assert_eq!(table_bytes(&db, "crystals"), crystals);
        assert_eq!(table_bytes(&db, "crystal_activations"), activations);
        assert_eq!(table_bytes(&db, "memory_claims"), facts);
        assert_eq!(table_bytes(&db, "term_rules"), rules);
        for table in ["decision_records", "memory_events", "consolidation_jobs"] {
            assert!(table_bytes(&db, table).is_empty());
        }
    }
}
#[test]
fn reimport_and_explicit_relocation_preserve_corrected_claim_masks() {
    use hieronymus::{
        claim_capture::{ClaimInput, ExistingClaimInput},
        claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims},
        data_root::HieronymusConfig,
        rag::{RagImport, RagStore},
    };
    for relocated in [false, true] {
        let (dir, mut db, mut r) = fixture();
        let config = HieronymusConfig::new(dir.path());
        let path = dir.path().join("story.txt");
        let store = RagStore::open(&config).unwrap();
        std::fs::write(&path, "Mira knows the secret.\n\nOld paragraph.").unwrap();
        let mut import = RagImport::new();
        import.claims.insert(
            0,
            vec![ClaimInput {
                text: "Mira knows the secret".into(),
                concept_id: Some(1),
                applicability: r.applicability.clone(),
            }],
        );
        store.import_file("book", &path, &import).unwrap();
        let claim: i64 = db
            .query_row(
                "select claim_id from claim_bindings where rag_chunk_id is not null",
                [],
                |r| r.get(0),
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
        if relocated {
            std::fs::write(
                &path,
                "Inserted.\n\nMira knows the secret.\n\nNew paragraph.",
            )
            .unwrap();
            let inputs = import.claims.remove(&0).unwrap();
            import.claims.insert(1, inputs);
            import.claim_lineage.insert(
                1,
                vec![ExistingClaimInput {
                    claim_id: claim,
                    concept_id: Some(1),
                    applicability: r.applicability.clone(),
                }],
            );
        } else {
            std::fs::write(&path, "Mira knows the secret.\n\nNew paragraph.").unwrap();
        }
        store.import_file("book", &path, &import).unwrap();
        let chunk: i64 = db
            .query_row(
                "select id from rag_chunks where text='Mira knows the secret.'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let q = StoryQueryV1 {
            series_id: 1,
            timeline_id: Some(1),
            position_id: Some(2),
            viewpoint: Viewpoint::Narrator,
            scope_predicates: vec!["volume:I".into(), "chapter:late".into()],
            mode: QueryMode::Current,
        };
        assert_eq!(
            rehydrate_claims(&db, ClaimTarget::RagChunk(chunk), &q).unwrap(),
            ClaimDisposition::Invalid
        );
        assert_eq!(
            db.query_row(
                "select count(*) from claim_effects where claim_id=?",
                [claim],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
#[test]
fn concurrent_corrections_with_one_expected_revision_commit_only_once() {
    let (dir, db, r) = fixture();
    origin(&db, &r);
    let mut second = r.clone();
    second.decision_id = "10000000-0000-4000-8000-000000000002".into();
    second.origin = OriginReceiptId("20000000-0000-4000-8000-000000000002".into());
    origin(&db, &second);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = [r, second]
        .into_iter()
        .map(|request| {
            let path = dir.path().join("hieronymus.sqlite");
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Connection::open(path).unwrap();
                db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                db.execute_batch("pragma foreign_keys=on").unwrap();
                barrier.wait();
                DecisionStore::new(&mut db).apply(&request)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, Ok(DecisionResultV1::Applied { .. })))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, Err(DecisionErrorV1::RevisionConflict { .. })))
            .count(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from claim_effects", [], |r| r
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
fn compound_invalid_member_dominates_unknown_member() {
    use hieronymus::{
        claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims},
        crystals::{CrystalStore, NewCrystal},
        data_root::HieronymusConfig,
    };
    let (dir, db, r) = fixture();
    let config = HieronymusConfig::new(dir.path());
    let crystal = CrystalStore::open(&config)
        .unwrap()
        .add_crystal(
            &hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation"),
            "thought",
            &NewCrystal::new("thought", "secret"),
        )
        .unwrap();
    db.execute_batch("update memory_claims set status='invalid' where id=1; insert into memory_claims(series_id,concept_id,text,revision,status,qualification,applicability_id,created_at,updated_at) values(1,1,'unknown member',1,'tentative','',1,'now','now');").unwrap();
    let unknown = db.last_insert_rowid();
    // Unknown sorts first, the exact early-return defect.
    db.execute("update memory_claims set status='tentative' where id=1", [])
        .unwrap();
    db.execute(
        "update memory_claims set status='invalid' where id=?",
        [unknown],
    )
    .unwrap();
    db.execute(
        "insert into claim_bindings(claim_id,crystal_id) values(1,?1),(?2,?1)",
        params![crystal, unknown],
    )
    .unwrap();
    let query = StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(2),
        viewpoint: Viewpoint::Unspecified,
        scope_predicates: vec!["volume:I".into(), "chapter:late".into()],
        mode: QueryMode::Current,
    };
    assert_eq!(
        rehydrate_claims(&db, ClaimTarget::Crystal(crystal), &query).unwrap(),
        ClaimDisposition::Invalid
    );
    let _ = r;
}

#[test]
fn unresolved_recall_returns_typed_metadata_without_assertion_fields() {
    use hieronymus::{
        data_root::HieronymusConfig,
        memory_models::TranslationContext,
        recall::{RecallHit, RecallService},
        workspace::{ShortTermMemoryInput, WorkspaceStore},
    };
    let (dir, db, _) = fixture();
    db.execute(
        "insert into authority_state(series_id,revision) values(1,0)",
        [],
    )
    .unwrap();
    let config = HieronymusConfig::new(dir.path());
    let context = TranslationContext::new("book", "en", "ru", "translation")
        .volume("I")
        .chapter("late");
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session = workspace.start_session(&context).unwrap();
    let memory = workspace
        .add_short_term_memory(
            session.id,
            &ShortTermMemoryInput::new("note", "secret unknown assertion"),
        )
        .unwrap();
    let response = RecallService::open(&config)
        .unwrap()
        .recall(session.id, &context, "secret", 1)
        .unwrap();
    assert!(response.hits.is_empty());
    assert_eq!(response.non_current.len(), 1);
    match &response.non_current[0] {
        RecallHit::ShortTerm { memory: item, .. } => {
            assert_eq!(item.id, memory.id);
            assert!(item.text.is_empty());
            assert!(!item.claim_annotation.claims.is_empty());
            assert_eq!(
                item.claim_annotation.disposition,
                hieronymus::claim_reads::ClaimDisposition::OutsideContext
            );
        }
        _ => panic!("short-term metadata expected"),
    }
}

#[test]
fn required_receipt_rejects_missing_tentative_foreign_and_future_and_respects_later_effect() {
    use hieronymus::{
        claim_reads::{ClaimDisposition, claim_disposition},
        coherent_reads::{CoherentReadError, stable_read},
        data_root::HieronymusConfig,
    };
    let (dir, mut db, mut r) = fixture();
    let config = HieronymusConfig::new(dir.path());
    origin(&db, &r);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let read = |id: &str| -> Result<_, CoherentReadError> {
        stable_read(&config, "book", Some(id), |_| Ok(()))
    };
    assert!(matches!(
        read("missing"),
        Err(CoherentReadError::DecisionNotApplied)
    ));
    db.execute("update decision_records set status='tentative'", [])
        .unwrap();
    assert!(matches!(
        read(&r.decision_id),
        Err(CoherentReadError::DecisionNotApplied)
    ));
    db.execute(
        "update decision_records set status='applied',result_json='{}'",
        [],
    )
    .unwrap();
    assert!(matches!(
        read(&r.decision_id),
        Err(CoherentReadError::DecisionNotApplied)
    ));
    // Restore the original stored receipt via an independent fresh fixture's canonical result.
    let (_other, mut other_db, original) = fixture();
    origin(&other_db, &original);
    let result = DecisionStore::new(&mut other_db).apply(&original).unwrap();
    db.execute(
        "update decision_records set result_json=?",
        [serde_json::to_string(&result).unwrap()],
    )
    .unwrap();
    db.execute("update authority_state set revision=0", [])
        .unwrap();
    assert!(matches!(
        read(&r.decision_id),
        Err(CoherentReadError::DecisionNotApplied)
    ));
    db.execute("update authority_state set revision=1", [])
        .unwrap();
    assert_eq!(read(&r.decision_id).unwrap().resulting_revision, 1);
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(2,'foreign','Foreign','en','ru','now','now'); insert into authority_state(series_id,revision) values(2,1)").unwrap();
    let foreign: Result<_, CoherentReadError> =
        stable_read(&config, "foreign", Some(&r.decision_id), |_| Ok(()));
    assert!(matches!(
        foreign,
        Err(CoherentReadError::DecisionNotApplied)
    ));
    let first = r.decision_id.clone();
    r.decision_id = "10000000-0000-4000-8000-000000000099".into();
    r.origin = OriginReceiptId("20000000-0000-4000-8000-000000000099".into());
    r.expected_revision = 1;
    r.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Fact {
            claim_id: 1,
            claim_revision: 2,
            effect: FactEffect::Qualify {
                qualification: "Mira only suspects this".into(),
            },
        },
    };
    origin(&db, &r);
    DecisionStore::new(&mut db).apply(&r).unwrap();
    let query = StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(2),
        viewpoint: Viewpoint::Unspecified,
        scope_predicates: vec!["volume:I".into(), "chapter:late".into()],
        mode: QueryMode::Current,
    };
    let observed: Result<_, hieronymus::recall::RecallError> =
        stable_read(&config, "book", Some(&first), |snapshot| {
            Ok(claim_disposition(snapshot, 1, &query)?)
        });
    let observed = observed.unwrap();
    assert_eq!(observed.resulting_revision, 2);
    assert_eq!(
        observed.value,
        ClaimDisposition::Qualified(vec!["Mira only suspects this".into()])
    );
}

#[test]
fn scoped_qualification_history_only_discloses_the_effect_current_for_this_query() {
    use hieronymus::claim_reads::{ClaimTarget, read_annotation};
    let (_dir, mut db, mut request) = fixture();
    db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,series_slug,strength,confidence,status,created_at,updated_at) values(1,'lesson','Mira knows the secret','series','book',0.5,0.5,'active','now','now'); insert into claim_bindings(claim_id,crystal_id) values(1,1); insert into knowledge_gates(applicability_id,viewpoint_kind,viewpoint_concept_id) values(1,'character',1)").unwrap();
    let hidden = "HIDDEN_CHAPTER_REVELATION";
    request.applicability.knowledge_gates[0].viewpoint = KnowledgeViewpoint::Narrator;
    request.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Fact {
            claim_id: 1,
            claim_revision: 1,
            effect: FactEffect::Qualify {
                qualification: hidden.into(),
            },
        },
    };
    origin(&db, &request);
    assert!(matches!(
        DecisionStore::new(&mut db).apply(&request).unwrap(),
        DecisionResultV1::Applied { .. }
    ));
    let query = |chapter: &str, viewpoint| StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(if chapter == "early" { 1 } else { 2 }),
        viewpoint,
        scope_predicates: vec!["volume:I".into(), format!("chapter:{chapter}")],
        mode: QueryMode::Current,
    };
    for q in [
        query("early", Viewpoint::Narrator),
        query("late", Viewpoint::Character(1)),
    ] {
        let annotation = read_annotation(&db, ClaimTarget::Crystal(1), &q).unwrap();
        assert_eq!(
            annotation.disposition,
            hieronymus::claim_reads::ClaimDisposition::Current
        );
        assert!(!serde_json::to_string(&annotation).unwrap().contains(hidden));
        let mut research = q;
        research.mode = QueryMode::OmniscientResearch;
        assert!(
            serde_json::to_string(
                &read_annotation(&db, ClaimTarget::Crystal(1), &research).unwrap()
            )
            .unwrap()
            .contains(hidden)
        );
    }
    let late = query("late", Viewpoint::Narrator);
    assert!(
        serde_json::to_string(&read_annotation(&db, ClaimTarget::Crystal(1), &late).unwrap())
            .unwrap()
            .contains(hidden)
    );
    request.decision_id = "10000000-0000-4000-8000-000000000002".into();
    request.origin = OriginReceiptId("20000000-0000-4000-8000-000000000002".into());
    request.expected_revision = 1;
    request.operation = OperationV1::Correct {
        intent: CorrectionIntentV1::Fact {
            claim_id: 1,
            claim_revision: 2,
            effect: FactEffect::Qualify {
                qualification: "CURRENT_QUALIFICATION".into(),
            },
        },
    };
    origin(&db, &request);
    DecisionStore::new(&mut db).apply(&request).unwrap();
    let serialized =
        serde_json::to_string(&read_annotation(&db, ClaimTarget::Crystal(1), &late).unwrap())
            .unwrap();
    assert!(!serialized.contains(hidden));
    assert!(serialized.contains("CURRENT_QUALIFICATION"));
    // Stored receipt-mask fixture: factual ingress currently produces no
    // exclusions, but the shared read DTO must honor valid excluded history.
    let mut receipt: serde_json::Value = serde_json::from_str(
        &db.query_row(
            "select result_json from decision_records where decision_id=?",
            [&request.decision_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    receipt["Applied"]["receipt"]["effective_exclusions"] =
        serde_json::json!([request.applicability]);
    db.execute(
        "update decision_records set result_json=?1 where decision_id=?2",
        params![receipt.to_string(), request.decision_id],
    )
    .unwrap();
    let masked =
        serde_json::to_string(&read_annotation(&db, ClaimTarget::Crystal(1), &late).unwrap())
            .unwrap();
    assert!(!masked.contains("CURRENT_QUALIFICATION"));
    assert!(
        masked.contains(hidden),
        "the older unmasked qualification now controls"
    );
}

#[test]
fn facets_inherit_real_source_corrections_before_and_after_derivation() {
    use hieronymus::{
        claim_reads::{ClaimDisposition, ClaimTarget, rehydrate_claims},
        concepts::{ConceptStore, FacetFields, FacetPatch},
        data_root::HieronymusConfig,
    };
    for correct_first in [true, false] {
        let (dir, mut db, request) = fixture();
        let config = HieronymusConfig::new(dir.path());
        hieronymus::registry::Registry::open(&config)
            .unwrap()
            .create_series("book", "Book", "en", "ru", None)
            .unwrap();
        db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,series_slug,strength,confidence,status,created_at,updated_at) values(1,'lesson','Mira knows the secret','series','book',0.5,0.5,'active','now','now'); insert into claim_bindings(claim_id,crystal_id) values(1,1)").unwrap();
        origin(&db, &request);
        if correct_first {
            DecisionStore::new(&mut db).apply(&request).unwrap();
        }
        let supplemental = hieronymus::claim_capture::ClaimInput {
            text: "supplemental".into(),
            concept_id: Some(1),
            applicability: request.applicability.clone(),
        };
        let store = ConceptStore::open(&config).unwrap();
        let context =
            hieronymus::memory_models::TranslationContext::new("book", "en", "ru", "translation")
                .volume("I")
                .chapter("late");
        let mut target =
            hieronymus::crystals::NewCrystal::new("lesson", "secret alias independent memory");
        target.claims = vec![supplemental.clone()];
        let target_id = hieronymus::crystals::CrystalStore::open(&config)
            .unwrap()
            .add_crystal(&context, "lesson", &target)
            .unwrap();
        store.link_crystal(target_id, 1, "evidence", 0.8).unwrap();
        let recall = hieronymus::recall::RecallService::open(&config).unwrap();
        let score = |response: hieronymus::recall::RecallResponse| {
            response
                .hits
                .iter()
                .find(|h| h.item_id() == target_id)
                .unwrap()
                .score()
        };
        let baseline = score(recall.recall_context(&context, "secret alias", 5).unwrap());
        let facet = store
            .add_facet_with_claims(
                1,
                "secret alias",
                &FacetFields {
                    source_crystal_id: Some(1),
                    ..Default::default()
                },
                0.8,
                false,
                std::slice::from_ref(&supplemental),
            )
            .unwrap();
        if !correct_first {
            DecisionStore::new(&mut db).apply(&request).unwrap();
        }
        let query = StoryQueryV1 {
            series_id: 1,
            timeline_id: Some(1),
            position_id: Some(2),
            viewpoint: Viewpoint::Narrator,
            scope_predicates: vec!["volume:I".into(), "chapter:late".into()],
            mode: QueryMode::Current,
        };
        assert_eq!(
            rehydrate_claims(&db, ClaimTarget::Facet(facet.id), &query).unwrap(),
            ClaimDisposition::Invalid
        );
        assert_eq!(
            score(recall.recall_context(&context, "secret alias", 5).unwrap()),
            baseline,
            "invalid derived facet must not boost a valid linked crystal"
        );
        let changed = store
            .update_facet_with_claims(
                facet.id,
                &FacetPatch {
                    value: Some(Some("new secret alias".into())),
                    ..Default::default()
                },
                &[supplemental],
            )
            .unwrap();
        assert_eq!(
            rehydrate_claims(&db, ClaimTarget::Facet(changed.id), &query).unwrap(),
            ClaimDisposition::Invalid
        );
        assert!(db.query_row("select exists(select 1 from evidence_records where json_extract(binding_json,'$.event')='copy' and json_extract(binding_json,'$.to.id')=?1)",[facet.id],|r|r.get::<_,bool>(0)).unwrap());
    }
}

use hieronymus::{
    authority::EvidenceBindingV1, authority_models::*, authority_producers::*, db::open_migrated,
    story_applicability::*,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
fn selection(start: usize, end: usize, text: &str) -> DocumentSelection {
    DocumentSelection {
        start,
        end,
        expected_text: text.get(start..end).unwrap_or("").into(),
    }
}
fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn fixture() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = open_migrated(&dir.path().join("db.sqlite")).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'),(2,'other','Other','en','ru','now','now'); insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Alex','series','series:book','now','now'),(2,'Other','series','series:other','now','now');").unwrap();
    (dir, db)
}
fn manifest(dir: &std::path::Path, scenes: &[&str]) -> SnapshotInput {
    let text=serde_json::json!({"version":1,"series_id":1,"timeline_name":"story","positions":scenes.iter().map(|s|serde_json::json!({"volume_key":"I","chapter_key":"1","scene_key":s})).collect::<Vec<_>>()}).to_string();
    file(dir, "order.json", &text)
}
fn file(dir: &std::path::Path, name: &str, text: &str) -> SnapshotInput {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    SnapshotInput::File {
        path,
        expected_hash: hash(text),
    }
}
fn binding(timeline: i64, position: i64) -> EvidenceBindingV1 {
    EvidenceBindingV1 {
        concept_id: 1,
        source_language: "en".into(),
        target_language: Some("ru".into()),
        applicability: ApplicabilityV1 {
            series_id: 1,
            timeline_id: Some(timeline),
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
        },
        position_id: position,
        paragraph_start: 0,
        paragraph_end: 0,
        identity_anchor: true,
        aligned_source_id: None,
        rendering: None,
        contradicts_rule: None,
        contradicts_claim: None,
        claim_effect: None,
        conflict_kind: None,
    }
}
#[test]
fn manifest_order_is_exact_stable_and_atomic() {
    let (d, mut db) = fixture();
    let first = manifest(d.path(), &["ten", "two"]);
    let result = EvidenceProducer::new(&mut db)
        .register_manifest(1, &first, None, 0)
        .unwrap();
    assert_eq!(result.revision, 1);
    assert_eq!(result.positions[0].position.scene_key, "ten");
    let reordered = manifest(d.path(), &["two", "ten"]);
    let result2 = EvidenceProducer::new(&mut db)
        .register_manifest(1, &reordered, Some(result.timeline_id), 1)
        .unwrap();
    assert_eq!(result2.positions[1].id, result.positions[0].id);
    let count: i64 = db
        .query_row("select count(*) from evidence_records", [], |r| r.get(0))
        .unwrap();
    for (input, expected) in [
        (manifest(d.path(), &["two", "two"]), 2),
        (manifest(d.path(), &["two"]), 2),
    ] {
        assert!(
            EvidenceProducer::new(&mut db)
                .register_manifest(1, &input, Some(result.timeline_id), expected)
                .is_err()
        );
    }
    assert_eq!(
        db.query_row("select count(*) from evidence_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        count
    );
    let valid = manifest(d.path(), &["two", "ten"]);
    assert!(
        EvidenceProducer::new(&mut db)
            .register_manifest(1, &valid, Some(result.timeline_id), 0)
            .is_err()
    );
    assert!(
        EvidenceProducer::new(&mut db)
            .register_manifest(2, &valid, Some(result.timeline_id), 2)
            .is_err()
    );
    assert!(
        EvidenceProducer::new(&mut db)
            .register_manifest(1, &first, None, 2)
            .is_err()
    ); // file hash changed
    assert_eq!(
        db.query_row("select revision from story_timelines", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn full_utf8_crlf_snapshot_replay_conflict_and_retention() {
    let (d, mut db) = fixture();
    let order = EvidenceProducer::new(&mut db)
        .register_manifest(1, &manifest(d.path(), &["a"]), None, 0)
        .unwrap();
    let b = binding(order.timeline_id, order.positions[0].id);
    let text = "Préface.\r\n\r\nAlex walks.\r\n\r\nFin.";
    let start = text.find("Alex").unwrap();
    let input = file(d.path(), "source.txt", text);
    let capture = EvidenceProducer::new(&mut db)
        .capture(
            1,
            &input,
            EvidenceKind::SourcePassage,
            &selection(start, start + 4, text),
            &b,
        )
        .unwrap();
    let chunk_relative = DocumentSelection {
        start: 0,
        end: 4,
        expected_text: "Alex".into(),
    };
    assert!(
        EvidenceProducer::new(&mut db)
            .capture(1, &input, EvidenceKind::SourcePassage, &chunk_relative, &b)
            .is_err()
    );
    assert_eq!(capture.reference.content_hash, hash(text));
    assert_eq!(capture.selected_text, "Alex");
    assert_eq!(capture.paragraph_start, start);
    assert_eq!(capture.paragraph_end, start + 11);
    assert_eq!(
        db.query_row(
            "select content from evidence_records where id=?",
            [capture.reference.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        text
    );
    assert_eq!(
        EvidenceProducer::new(&mut db)
            .capture(
                1,
                &input,
                EvidenceKind::SourcePassage,
                &selection(start, start + 4, text),
                &b
            )
            .unwrap()
            .reference,
        capture.reference
    );
    let mut conflict = b.clone();
    conflict.identity_anchor = false;
    assert!(matches!(
        EvidenceProducer::new(&mut db).capture(
            1,
            &input,
            EvidenceKind::SourcePassage,
            &selection(start, start + 4, text),
            &conflict
        ),
        Err(ProducerError::BindingConflict)
    ));
    assert!(
        EvidenceProducer::new(&mut db)
            .capture(
                1,
                &input,
                EvidenceKind::SourcePassage,
                &selection(3, 5, text),
                &b
            )
            .is_err()
    );
    let changed = file(d.path(), "source.txt", "Alex changed.");
    assert!(
        EvidenceProducer::new(&mut db)
            .capture(
                1,
                &input,
                EvidenceKind::SourcePassage,
                &selection(start, start + 4, text),
                &b
            )
            .is_err()
    );
    let newer = EvidenceProducer::new(&mut db)
        .capture(
            1,
            &changed,
            EvidenceKind::SourcePassage,
            &selection(0, 4, "Alex changed."),
            &b,
        )
        .unwrap();
    assert_ne!(newer.reference.id, capture.reference.id);
    let retained = SnapshotInput::Retained {
        evidence_id: capture.reference.id,
        expected_hash: hash(text),
    };
    assert_eq!(
        EvidenceProducer::new(&mut db)
            .capture(
                1,
                &retained,
                EvidenceKind::SourcePassage,
                &selection(start, start + 4, text),
                &b
            )
            .unwrap()
            .reference,
        capture.reference
    );
}
#[test]
fn alignment_uses_absolute_target_span_and_source_paragraph() {
    let (d, mut db) = fixture();
    let order = EvidenceProducer::new(&mut db)
        .register_manifest(1, &manifest(d.path(), &["a", "b"]), None, 0)
        .unwrap();
    let mut b = binding(order.timeline_id, order.positions[0].id);
    let source = "Intro.\n\nAlex walks.";
    let input = file(d.path(), "s.txt", source);
    let s = EvidenceProducer::new(&mut db)
        .capture(
            1,
            &input,
            EvidenceKind::SourcePassage,
            &selection(8, 12, source),
            &b,
        )
        .unwrap();
    b.aligned_source_id = Some(s.reference.id);
    let target = "Начало.\r\n\r\nАлекс идёт.";
    let offset = target.find("Алекс").unwrap();
    let t = file(d.path(), "t.txt", target);
    let a = EvidenceProducer::new(&mut db)
        .capture(
            1,
            &t,
            EvidenceKind::AlignedRendering,
            &selection(offset, offset + 10, target),
            &b,
        )
        .unwrap();
    assert_eq!(a.selected_text, "Алекс");
    assert_eq!(a.paragraph_start, 8);
    assert_eq!(a.paragraph_end, source.len());
    let count = db
        .query_row("select count(*) from evidence_records", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    for mutation in 0..6 {
        let mut wrong = b.clone();
        match mutation {
            0 => wrong.concept_id = 2,
            1 => wrong.source_language = "xx".into(),
            2 => wrong.position_id = order.positions[1].id,
            3 => wrong.applicability.series_id = 2,
            4 => wrong.source_language = "ru".into(),
            _ => wrong.target_language = Some("en".into()),
        };
        let invalid_target = file(d.path(), &format!("invalid-{mutation}.txt"), target);
        assert!(
            EvidenceProducer::new(&mut db)
                .capture(
                    1,
                    &invalid_target,
                    EvidenceKind::AlignedRendering,
                    &selection(offset, offset + 10, target),
                    &wrong
                )
                .is_err()
        );
    }
    assert_eq!(
        db.query_row("select count(*) from evidence_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        count
    );
    assert!(
        EvidenceProducer::new(&mut db)
            .capture(
                2,
                &t,
                EvidenceKind::AlignedRendering,
                &selection(offset, offset + 10, target),
                &b
            )
            .is_err()
    );
}

#[test]
fn producer_refs_drive_learned_policy_without_staged_evidence() {
    use hieronymus::authority::{DecisionStore, OriginContextV1};
    for scenario in ["eligible", "one", "overlap", "contrary"] {
        let (d, mut db) = fixture();
        let order = EvidenceProducer::new(&mut db)
            .register_manifest(1, &manifest(d.path(), &["a"]), None, 0)
            .unwrap();
        let b = binding(order.timeline_id, order.positions[0].id);
        let source = "Alex walks.\r\n\r\nAlex talks.";
        let target = if scenario == "contrary" {
            "А\r\n\r\nБ"
        } else {
            "А\r\n\r\nА"
        };
        let source_input = file(d.path(), "source.txt", source);
        let target_input = file(d.path(), "target.txt", target);
        let mut refs = vec![];
        for (start, target_start) in [(0, 0), (15, 6)] {
            if scenario == "one" && start > 0 {
                continue;
            }
            let start = if scenario == "overlap" { 0 } else { start };
            let target_start = if scenario == "overlap" {
                0
            } else {
                target_start
            };
            let source_ref = EvidenceProducer::new(&mut db)
                .capture(
                    1,
                    &source_input,
                    EvidenceKind::SourcePassage,
                    &selection(start, start + 4, source),
                    &b,
                )
                .unwrap();
            let mut aligned = b.clone();
            aligned.aligned_source_id = Some(source_ref.reference.id);
            let target_ref = EvidenceProducer::new(&mut db)
                .capture(
                    1,
                    &target_input,
                    EvidenceKind::AlignedRendering,
                    &selection(target_start, target_start + 2, target),
                    &aligned,
                )
                .unwrap();
            refs.extend([source_ref.reference, target_ref.reference]);
        }
        db.execute_batch("insert into term_rules(id,concept_id,source_language,target_language,source_text,canonical_translation,status,created_at,updated_at) values(1,1,'en','ru','Alex','А','candidate','now','now'); insert into term_rule_forms(rule_id,form_kind,surface,language) values(1,'source','Alex','en'),(1,'approved','А','ru');").unwrap();
        let request = DecisionRequestV1 {
            version: 1,
            decision_id: "10000000-0000-4000-8000-000000000001".into(),
            expected_revision: 1,
            actor_kind: ActorKind::Agent,
            origin: OriginReceiptId("20000000-0000-4000-8000-000000000001".into()),
            evidence_refs: refs,
            series_id: 1,
            concept_id: Some(1),
            source_language: "en".into(),
            target_language: Some("ru".into()),
            applicability: b.applicability.clone(),
            operation: OperationV1::Activate {
                candidate_id: 1,
                candidate_revision: 1,
            },
        };
        // Private synthetic ingress only: all timeline/order/evidence IDs above are real producer returns.
        let context = serde_json::to_string(&OriginContextV1 {
            decision_id: request.decision_id.clone(),
            expected_revision: request.expected_revision,
            selected_source: None,
            series_id: 1,
            concept_id: Some(1),
            source_language: request.source_language.clone(),
            target_language: request.target_language.clone(),
            applicability: request.applicability.clone(),
            evidence_ids: request.evidence_refs.iter().map(|e| e.id).collect(),
            operation: request.operation.clone(),
        })
        .unwrap();
        db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'agent','private-test',?1,'capture decision',?2,?3,'now')",rusqlite::params![request.origin.0,context,hash(&format!("capture decision\n{context}"))]).unwrap();
        let result = DecisionStore::new(&mut db).apply(&request).unwrap();
        assert_eq!(
            matches!(result, DecisionResultV1::Applied { .. }),
            scenario == "eligible",
            "{scenario}: {result:?}"
        );
        if scenario != "eligible" {
            assert!(matches!(result, DecisionResultV1::Tentative { .. }));
        }
    }
}

#[test]
fn failed_fresh_manifest_leaves_no_timeline_or_evidence() {
    let (d, mut db) = fixture();
    let input = manifest(d.path(), &["a", "a"]);
    assert!(
        EvidenceProducer::new(&mut db)
            .register_manifest(1, &input, None, 0)
            .is_err()
    );
    for table in [
        "story_timelines",
        "story_positions",
        "evidence_records",
        "authority_state",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let input = file(
        d.path(),
        "bad.json",
        r#"{"version":1,"series_id":1,"timeline_name":"story","positions":[{"volume_key":"I","chapter_key":"1","scene_key":"a","ordinal":99}]}"#,
    );
    assert!(
        EvidenceProducer::new(&mut db)
            .register_manifest(1, &input, None, 0)
            .is_err()
    );
}

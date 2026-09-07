use hieronymus::{db::open_migrated, memory_models::TranslationContext, story_applicability::*};
use rusqlite::Connection;
fn fixture() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = open_migrated(&dir.path().join("test.sqlite")).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into story_timelines(id,series_id,name) values(1,1,'reading'); insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(1,1,'source_passage','manifest','hash',0,1,'manifest','{}','now');").unwrap();
    db.execute("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(7,'Mira','series','series:book','now','now')",[]).unwrap();
    for i in 1..=15 {
        db.execute("insert into story_positions(id,timeline_id,volume_key,chapter_key,ordinal,evidence_id) values(?1,1,'Book I',?2,?1,1)", rusqlite::params![i, format!("p{i}")]).unwrap();
    }
    (dir, db)
}
fn app() -> ApplicabilityV1 {
    ApplicabilityV1 {
        series_id: 1,
        timeline_id: Some(1),
        volume_key: None,
        chapter_key: None,
        scope_predicates: vec![],
        valid_from: None,
        valid_until: None,
        metadata_state: MetadataState::Resolved,
        knowledge_gates: vec![],
    }
}
fn query(p: i64, v: Viewpoint) -> StoryQueryV1 {
    StoryQueryV1 {
        series_id: 1,
        timeline_id: Some(1),
        position_id: Some(p),
        viewpoint: v,
        scope_predicates: vec![],
        mode: QueryMode::Current,
    }
}
#[test]
fn table_world_and_knowledge_are_separate() {
    let (_dir, db) = fixture();
    for (from, until, gate, q, expected) in [
        (
            None,
            None,
            KnowledgeViewpoint::Character(7),
            query(3, Viewpoint::Character(7)),
            Eligibility::FutureOrOutsideViewpoint,
        ),
        (
            None,
            None,
            KnowledgeViewpoint::Narrator,
            query(3, Viewpoint::Character(7)),
            Eligibility::FutureOrOutsideViewpoint,
        ),
        (
            None,
            None,
            KnowledgeViewpoint::Character(7),
            query(15, Viewpoint::Character(7)),
            Eligibility::Current,
        ),
        (
            Some(2),
            Some(8),
            KnowledgeViewpoint::All,
            query(8, Viewpoint::Narrator),
            Eligibility::Excluded,
        ),
    ] {
        let mut a = app();
        a.valid_from = from;
        a.valid_until = until;
        a.knowledge_gates.push(KnowledgeGateV1 {
            viewpoint: gate,
            known_from: Some(if q.position_id == Some(15) { 15 } else { 12 }),
            known_until: None,
        });
        assert_eq!(StoryApplicability::evaluate(&db, &a, &q).unwrap(), expected);
    }
}
#[test]
fn missing_context_and_narrator_never_infer_knowledge() {
    let (_dir, db) = fixture();
    assert_eq!(
        StoryApplicability::evaluate(&db, &app(), &query(15, Viewpoint::Narrator)).unwrap(),
        Eligibility::FutureOrOutsideViewpoint
    );
    let context =
        TranslationContext::new("book", "en", "ru", "translate").chapter("Appendix: Letters");
    assert_eq!(
        StoryApplicability::resolve_context(&db, &context)
            .unwrap()
            .position_id,
        None
    );
    assert_eq!(
        StoryApplicability::compare(&db, Some(15), None).unwrap(),
        StoryOrdering::Unknown
    );
    db.execute("insert into story_positions(timeline_id,volume_key,chapter_key,ordinal,evidence_id) values(1,'Book I','III',16,1),(1,'Book II','III',17,1)",[]).unwrap();
    assert!(matches!(
        StoryApplicability::resolve_context(&db, &context.chapter("III")),
        Err(ApplicabilityError::AmbiguousPosition)
    ));
}
#[test]
fn manifest_order_stable_ids_revisions_evidence_and_rollback() {
    let (_dir, mut db) = fixture();
    db.execute("delete from story_positions", []).unwrap();
    let mut m = OrderManifest {
        series_id: 1,
        timeline_id: 1,
        positions: [
            ("Book I", "Prologue"),
            ("Book I", "The Orchard"),
            ("Book I", "Interlude α"),
            ("Book II", "III"),
        ]
        .into_iter()
        .map(|(v, c)| ManifestPosition {
            volume_key: v.into(),
            chapter_key: c.into(),
            scene_key: String::new(),
        })
        .collect(),
    };
    {
        let tx = db.transaction().unwrap();
        assert_eq!(
            StoryApplicability::register_order_tx(&tx, &m, 1, 0).unwrap(),
            1
        );
        tx.commit().unwrap();
    }
    let resolve = |db: &Connection, v: &str, ch: &str| {
        StoryApplicability::resolve_context(
            db,
            &TranslationContext::new("book", "en", "ru", "translate")
                .volume(v)
                .chapter(ch),
        )
        .unwrap()
        .position_id
    };
    let first = resolve(&db, "Book I", "Prologue");
    let last = resolve(&db, "Book II", "III");
    assert_eq!(
        StoryApplicability::compare(&db, first, last).unwrap(),
        StoryOrdering::Before
    );
    assert_eq!(
        StoryApplicability::compare(&db, last, resolve(&db, "Book II", "Appendix: Letters"))
            .unwrap(),
        StoryOrdering::Unknown
    );
    m.positions.reverse();
    {
        let tx = db.transaction().unwrap();
        assert!(matches!(
            StoryApplicability::register_order_tx(&tx, &m, 999, 1),
            Err(ApplicabilityError::InvalidEvidence)
        ));
        assert!(matches!(
            StoryApplicability::register_order_tx(&tx, &m, 1, 0),
            Err(ApplicabilityError::RevisionConflict)
        ));
        assert_eq!(
            StoryApplicability::register_order_tx(&tx, &m, 1, 1).unwrap(),
            2
        );
        assert_eq!(resolve(&tx, "Book I", "Prologue"), first);
        assert_eq!(
            StoryApplicability::compare(&tx, first, last).unwrap(),
            StoryOrdering::After
        );
        tx.rollback().unwrap();
    }
    assert_eq!(
        StoryApplicability::compare(&db, first, last).unwrap(),
        StoryOrdering::Before
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
    m.positions.pop();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        StoryApplicability::register_order_tx(&tx, &m, 1, 1),
        Err(ApplicabilityError::OmittedPosition)
    ));
}
#[test]
fn scopes_before_gates_research_and_relationship_evolution() {
    let (_dir, db) = fixture();
    let mut old = app();
    old.valid_until = Some(8);
    old.knowledge_gates = vec![KnowledgeGateV1 {
        viewpoint: KnowledgeViewpoint::All,
        known_from: None,
        known_until: None,
    }];
    let mut new = old.clone();
    new.valid_from = Some(8);
    new.valid_until = None;
    for (p, old_expected, new_expected) in [
        (
            3,
            Eligibility::Current,
            Eligibility::FutureOrOutsideViewpoint,
        ),
        (8, Eligibility::Excluded, Eligibility::Current),
    ] {
        let q = query(p, Viewpoint::Unspecified);
        assert_eq!(
            StoryApplicability::evaluate(&db, &old, &q).unwrap(),
            old_expected
        );
        assert_eq!(
            StoryApplicability::evaluate(&db, &new, &q).unwrap(),
            new_expected
        );
    }
    let mut q = query(3, Viewpoint::Narrator);
    q.mode = QueryMode::OmniscientResearch;
    assert_eq!(
        StoryApplicability::evaluate(&db, &new, &q).unwrap(),
        Eligibility::FutureOrOutsideViewpoint
    );
    new.scope_predicates = vec!["arc:Orchard".into()];
    new.valid_from = Some(15);
    new.valid_until = Some(2);
    assert_eq!(
        StoryApplicability::evaluate(&db, &new, &q).unwrap(),
        Eligibility::Excluded
    );
    q.scope_predicates = vec!["arc:Orchard".into()];
    assert!(matches!(
        StoryApplicability::evaluate(&db, &new, &q),
        Err(ApplicabilityError::InvalidInterval)
    ));
}
#[test]
fn flashback_uses_reading_position_and_unplaced_scenes_are_unknown() {
    let (_dir, db) = fixture();
    let mut a = app();
    a.knowledge_gates = vec![KnowledgeGateV1 {
        viewpoint: KnowledgeViewpoint::Narrator,
        known_from: Some(12),
        known_until: None,
    }];
    a.valid_from = Some(2); // Event happened earlier, but flashback reveals it at p12.
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &query(3, Viewpoint::Narrator)).unwrap(),
        Eligibility::FutureOrOutsideViewpoint
    );
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &query(12, Viewpoint::Narrator)).unwrap(),
        Eligibility::Current
    );
    a.metadata_state = MetadataState::Unspecified;
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &query(15, Viewpoint::Narrator)).unwrap(),
        Eligibility::Unknown
    );
    db.execute("insert into story_positions(timeline_id,volume_key,chapter_key,scene_key,ordinal,evidence_id) values(1,'Book I','Scenes','one',16,1),(1,'Book I','Scenes','two',17,1)",[]).unwrap();
    let mut c = TranslationContext::new("book", "en", "ru", "translate")
        .volume("Book I")
        .chapter("Scenes");
    assert_eq!(
        StoryApplicability::resolve_context(&db, &c)
            .unwrap()
            .position_id,
        None
    );
    c.story_scene_key = Some("two".into());
    assert!(
        StoryApplicability::resolve_context(&db, &c)
            .unwrap()
            .position_id
            .is_some()
    );
}
#[test]
fn wrong_series_timeline_and_failed_reorder_do_not_mutate() {
    let (_dir, mut db) = fixture();
    let mut q = query(3, Viewpoint::Character(7));
    q.series_id = 2;
    assert!(matches!(
        StoryApplicability::evaluate(&db, &app(), &q),
        Err(ApplicabilityError::WrongSeries)
    ));
    q.series_id = 1;
    q.timeline_id = Some(99);
    assert!(matches!(
        StoryApplicability::evaluate(&db, &app(), &q),
        Err(ApplicabilityError::WrongTimeline)
    ));
    db.execute("insert into applicabilities(series_id,timeline_id,scope_predicates_json,valid_from,valid_until,metadata_state) values(1,1,'[]',2,8,'resolved')",[]).unwrap();
    let m = OrderManifest {
        series_id: 1,
        timeline_id: 1,
        positions: (1..=15)
            .rev()
            .map(|i| ManifestPosition {
                volume_key: "Book I".into(),
                chapter_key: format!("p{i}"),
                scene_key: String::new(),
            })
            .collect(),
    };
    {
        let tx = db.transaction().unwrap();
        assert!(matches!(
            StoryApplicability::register_order_tx(&tx, &m, 1, 0),
            Err(ApplicabilityError::InvalidInterval)
        ));
        tx.commit().unwrap();
    }
    assert_eq!(
        StoryApplicability::compare(&db, Some(2), Some(8)).unwrap(),
        StoryOrdering::Before
    );
    assert_eq!(
        db.query_row("select revision from story_timelines where id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn exact_identity_scope_and_unknown_endpoints() {
    let (_dir, db) = fixture();
    let mut a = app();
    a.knowledge_gates = vec![KnowledgeGateV1 {
        viewpoint: KnowledgeViewpoint::All,
        known_from: None,
        known_until: None,
    }];
    a.chapter_key = Some("p3".into());
    let mut q = query(3, Viewpoint::Unspecified);
    q.scope_predicates = vec!["chapter:p3".into()];
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &q).unwrap(),
        Eligibility::Current
    );
    q.position_id = Some(4);
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &q).unwrap(),
        Eligibility::Excluded
    );
    q.position_id = Some(3);
    q.scope_predicates = vec!["chapter:P3".into()];
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &q).unwrap(),
        Eligibility::Excluded
    );
    a.chapter_key = None;
    a.valid_from = Some(999);
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &q).unwrap(),
        Eligibility::Unknown
    );
    a.valid_from = None;
    q.position_id = None;
    assert_eq!(
        StoryApplicability::evaluate(&db, &a, &q).unwrap(),
        Eligibility::Unknown
    );
}

use hieronymus::{
    claim_capture::{ClaimInput, capture_claim_tx},
    claim_reads::ClaimTarget,
    data_root::HieronymusConfig,
    db::open_migrated,
    story_applicability::*,
};
use rusqlite::Connection;
fn fixture() -> (tempfile::TempDir, HieronymusConfig, Connection) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    hieronymus::registry::Registry::open(&config)
        .unwrap()
        .create_series("book", "Book", "en", "ru", None)
        .unwrap();
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into series values(2,'other','Other','en','ru','now','now'); insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Mira','series','series:book','now','now'),(2,'Other','series','series:other','now','now'); insert into crystals(id,crystal_type,text,scope_type,series_slug,strength,confidence,status,created_at,updated_at) values(1,'lesson','Mira knows','series','book',0.5,0.5,'active','now','now');").unwrap();
    (root, config, db)
}
fn claim() -> ClaimInput {
    ClaimInput {
        text: "Mira knows".into(),
        concept_id: Some(1),
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
    }
}
#[test]
fn capture_rejects_cross_series_target_and_concept_without_partial_writes() {
    for wrong_target in [true, false] {
        let (_root, _config, mut db) = fixture();
        let tx = db.transaction().unwrap();
        let mut input = claim();
        if wrong_target {
            input.applicability.series_id = 2;
            input.concept_id = Some(2);
        } else {
            input.concept_id = Some(2);
        }
        assert!(capture_claim_tx(&tx, ClaimTarget::Crystal(1), &input).is_err());
        tx.commit().unwrap();
        assert_eq!(
            db.query_row("select count(*) from memory_claims", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn failed_capture_audit_rolls_back_even_if_caller_commits() {
    let (_root, _config, mut db) = fixture();
    db.execute_batch("create trigger fail_audit before insert on evidence_records begin select raise(abort,'failure'); end").unwrap();
    let tx = db.transaction().unwrap();
    assert!(capture_claim_tx(&tx, ClaimTarget::Crystal(1), &claim()).is_err());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn crystal_explicit_claims_merge_source_lineage() {
    use hieronymus::{
        crystals::{CrystalStore, NewCrystal},
        memory_models::TranslationContext,
        workspace::{ShortTermMemoryInput, WorkspaceStore},
    };
    let (_root, config, db) = fixture();
    let context = TranslationContext::new("book", "en", "ru", "translation");
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session = workspace.start_session(&context).unwrap();
    let mut input = ShortTermMemoryInput::new("note", "Mira knows the secret.");
    input.claims = vec![claim()];
    let memory = workspace.add_short_term_memory(session.id, &input).unwrap();
    let original: i64 = db
        .query_row(
            "select claim_id from claim_bindings where short_term_id=?",
            [memory.id],
            |r| r.get(0),
        )
        .unwrap();
    let mut crystal = NewCrystal::new("lesson", "Mira knows the secret.");
    crystal.claims = vec![claim()];
    crystal.source_memory_ids = vec![memory.id];
    let id = CrystalStore::open(&config)
        .unwrap()
        .add_crystal(&context, "lesson", &crystal)
        .unwrap();
    assert!(
        db.query_row(
            "select exists(select 1 from claim_bindings where crystal_id=?1 and claim_id=?2)",
            rusqlite::params![id, original],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where crystal_id=?",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn rag_replacements_preserve_lineage_and_accept_supplemental_claims_with_audit() {
    use hieronymus::rag::{RagImport, RagStore};
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    std::fs::write(&path, "Mira knows.\n\nOld paragraph.").unwrap();
    let store = RagStore::open(&config).unwrap();
    let mut import = RagImport::new();
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    let old: i64 = db
        .query_row(
            "select claim_id from claim_bindings where rag_chunk_id is not null",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Same source is still a typed capture operation, not a discarded field.
    store.import_file("book", &path, &import).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where rag_chunk_id is not null",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    std::fs::write(&path, "Mira knows.\n\nNew paragraph.").unwrap();
    import.claims.insert(1, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where rag_chunk_id is not null",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    assert!(
        db.query_row(
            "select exists(select 1 from claim_bindings where claim_id=?)",
            [old],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    let audits:i64=db.query_row("select count(*) from evidence_records where json_extract(binding_json,'$.event')='rag_rebind'",[],|r|r.get(0)).unwrap();
    assert_eq!(audits, 2);
}
#[test]
fn rag_rejects_out_of_range_typed_claims_without_replacing_source() {
    use hieronymus::rag::{RagImport, RagStore};
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Old paragraph.").unwrap();
    store.import_file("book", &path, &RagImport::new()).unwrap();
    std::fs::write(&path, "New paragraph.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(5, vec![claim()]);
    assert!(store.import_file("book", &path, &import).is_err());
    assert_eq!(
        db.query_row("select text from rag_chunks", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "Old paragraph."
    );
}
#[test]
fn rag_ambiguous_anchor_is_rejected_without_guessing() {
    use hieronymus::rag::{RagError, RagImport, RagStore};
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Same.\n\nSame.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    db.execute(
        "update rag_chunks set location=(select location from rag_chunks order by id limit 1)",
        [],
    )
    .unwrap();
    let before: i64 = db
        .query_row("select count(*) from evidence_records", [], |r| r.get(0))
        .unwrap();
    std::fs::write(&path, "Same.\n\nChanged.").unwrap();
    assert!(matches!(
        store.import_file("book", &path, &import),
        Err(RagError::AmbiguousClaimAnchor { .. })
    ));
    assert_eq!(
        db.query_row(
            "select count(*) from rag_chunks where text='Same.'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("select count(*) from evidence_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
}
#[test]
fn rag_rebind_failure_preserves_prior_search_binding_and_source_state() {
    use hieronymus::rag::{RagImport, RagStore};
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Mira knows.\n\nOld paragraph.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    let before: (String, i64) = db
        .query_row(
            "select checksum,(select count(*) from evidence_records) from rag_sources",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute_batch("create trigger fail_rebind before insert on evidence_records when json_extract(new.binding_json,'$.event')='rag_rebind' begin select raise(abort,'failure'); end").unwrap();
    std::fs::write(&path, "Mira knows.\n\nChanged paragraph.").unwrap();
    assert!(store.import_file("book", &path, &RagImport::new()).is_err());
    let after: (String, i64) = db
        .query_row(
            "select checksum,(select count(*) from evidence_records) from rag_sources",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        store.search("book", "Old", 5, &[], &[], &[]).unwrap().len(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where rag_chunk_id is not null",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn facet_typed_capture_update_and_merge_preserve_immutable_history() {
    use hieronymus::concepts::{ConceptStore, FacetFields, FacetPatch, NewConcept};
    let (_root, config, db) = fixture();
    let store = ConceptStore::open(&config).unwrap();
    let f = store
        .add_facet_with_claims(
            1,
            "Mira knows",
            &FacetFields::default(),
            0.5,
            false,
            &[claim()],
        )
        .unwrap();
    let old: i64 = db
        .query_row(
            "select claim_id from claim_bindings where facet_id=?",
            [f.id],
            |r| r.get(0),
        )
        .unwrap();
    store
        .update_facet(
            f.id,
            &FacetPatch {
                value: Some(Some("Mira suspects".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where facet_id=?",
            [f.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select text from memory_claims where id=?", [old], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "Mira knows"
    );
    store
        .update_facet_with_claims(f.id, &FacetPatch::default(), &[claim()])
        .unwrap();
    let target = store
        .create_concept(
            "Alias",
            &NewConcept {
                scope_type: "series".into(),
                scope_key: "series:book".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let other = store
        .add_facet(
            target.id,
            "Mira suspects",
            &FacetFields::default(),
            0.5,
            false,
        )
        .unwrap();
    store.merge_concepts(1, target.id, "same identity").unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where facet_id=?",
            [other.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
#[test]
fn relocated_same_identity_requires_explicit_lineage_but_distinct_identity_does_not() {
    use hieronymus::rag::{RagImport, RagStore};
    for distinct in [false, true] {
        let (root, config, db) = fixture();
        let path = root.path().join("source.txt");
        let store = RagStore::open(&config).unwrap();
        std::fs::write(&path, "Mira knows.").unwrap();
        let mut import = RagImport::new();
        import.claims.insert(0, vec![claim()]);
        store.import_file("book", &path, &import).unwrap();
        std::fs::write(&path, "Inserted.\n\nMira knows.").unwrap();
        import.claims.clear();
        let mut input = claim();
        if distinct {
            db.execute("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(3,'Other Mira','series','series:book','now','now')",[]).unwrap();
            input.concept_id = Some(3);
        }
        import.claims.insert(1, vec![input]);
        assert_eq!(store.import_file("book", &path, &import).is_ok(), distinct);
    }
}
#[test]
fn explicit_relocation_lineage_retains_original_claim_and_rejects_forged_scope() {
    use hieronymus::{
        claim_capture::ExistingClaimInput,
        rag::{RagImport, RagStore},
    };
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Mira knows.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    let old: i64 = db
        .query_row(
            "select claim_id from claim_bindings where rag_chunk_id is not null",
            [],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(&path, "Inserted.\n\nMira knows.").unwrap();
    import.claims.clear();
    import.claims.insert(1, vec![claim()]);
    let mut lineage = ExistingClaimInput {
        claim_id: old,
        concept_id: Some(1),
        applicability: claim().applicability,
    };
    lineage.applicability.series_id = 2;
    import.claim_lineage.insert(1, vec![lineage.clone()]);
    assert!(store.import_file("book", &path, &import).is_err());
    lineage.applicability.series_id = 1;
    import.claim_lineage.insert(1, vec![lineage]);
    store.import_file("book", &path, &import).unwrap();
    import.claim_lineage.clear();
    store.import_file("book", &path, &import).unwrap();
    assert!(db.query_row("select exists(select 1 from claim_bindings b join rag_chunks c on c.id=b.rag_chunk_id where b.claim_id=? and c.text='Mira knows.')",[old],|r|r.get::<_,bool>(0)).unwrap());
}
#[test]
fn legacy_capture_does_not_invent_all_viewpoint_knowledge() {
    use hieronymus::{
        memory_models::TranslationContext,
        workspace::{ShortTermMemoryInput, WorkspaceStore},
    };
    let (_root, config, db) = fixture();
    let store = WorkspaceStore::open(&config).unwrap();
    let session = store
        .start_session(&TranslationContext::new("book", "en", "ru", "translation"))
        .unwrap();
    let memory = store
        .add_short_term_memory(
            session.id,
            &ShortTermMemoryInput::new("note", "A meaningful observation."),
        )
        .unwrap();
    assert_eq!(db.query_row("select count(*) from knowledge_gates g join memory_claims m on m.applicability_id=g.applicability_id join claim_bindings b on b.claim_id=m.id where b.short_term_id=?",[memory.id],|r|r.get::<_,i64>(0)).unwrap(),0);
}
#[test]
fn no_op_facet_scope_update_retains_original_lineage() {
    use hieronymus::concepts::{ConceptStore, FacetFields, FacetPatch};
    let (_root, config, db) = fixture();
    let store = ConceptStore::open(&config).unwrap();
    let facet = store
        .add_facet_with_claims(
            1,
            "Mira knows",
            &FacetFields::default(),
            0.5,
            false,
            &[claim()],
        )
        .unwrap();
    store
        .update_facet(
            facet.id,
            &FacetPatch {
                story_scopes: Some(Some(vec![])),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where facet_id=?",
            [facet.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn invalid_applicability_and_foreign_facet_identity_are_rejected_atomically() {
    use hieronymus::concepts::{ConceptStore, FacetFields};
    let (_root, config, mut db) = fixture();
    let facet = ConceptStore::open(&config)
        .unwrap()
        .add_facet(1, "Mira", &FacetFields::default(), 0.5, false)
        .unwrap();
    for variant in 0..3 {
        let mut input = claim();
        if variant == 0 {
            input.concept_id = None;
        } else if variant == 1 {
            input.applicability.valid_from = Some(999);
        } else {
            input.applicability.knowledge_gates = vec![KnowledgeGateV1 {
                viewpoint: KnowledgeViewpoint::Character(2),
                known_from: None,
                known_until: None,
            }];
        }
        let tx = db.transaction().unwrap();
        assert!(capture_claim_tx(&tx, ClaimTarget::Facet(facet.id), &input).is_err());
        tx.commit().unwrap();
    }
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn rebind_audit_retains_old_source_anchor_and_is_immutable() {
    use hieronymus::rag::{RagImport, RagStore};
    let (root, config, db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Mira knows.\n\nOld.").unwrap();
    let mut import = RagImport::new().source_ref("chapter-source");
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    let checksum: String = db
        .query_row("select checksum from rag_sources", [], |r| r.get(0))
        .unwrap();
    std::fs::write(&path, "Mira knows.\n\nNew.").unwrap();
    store.import_file("book", &path, &import).unwrap();
    let (id,old_source,old_hash):(i64,String,String)=db.query_row("select e.id,json_extract(d.binding_json,'$.source_snapshot.source_ref'),json_extract(d.binding_json,'$.source_snapshot.checksum') from evidence_records e join evidence_records d on d.id=json_extract(e.binding_json,'$.prior_detach_evidence_id') where json_extract(e.binding_json,'$.event')='rag_rebind'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(old_source, "chapter-source");
    assert_eq!(old_hash, checksum);
    assert!(
        db.execute("delete from evidence_records where id=?", [id])
            .is_err()
    );
    assert!(
        db.execute(
            "update evidence_records set binding_json='{}' where id=?",
            [id]
        )
        .is_err()
    );
}
#[test]
fn previously_detached_same_anchor_cannot_be_recreated_as_fresh_claim() {
    use hieronymus::rag::{RagImport, RagStore};
    let (root, config, _db) = fixture();
    let path = root.path().join("source.txt");
    let store = RagStore::open(&config).unwrap();
    std::fs::write(&path, "Mira knows.").unwrap();
    let mut import = RagImport::new();
    import.claims.insert(0, vec![claim()]);
    store.import_file("book", &path, &import).unwrap();
    std::fs::write(&path, "Different.").unwrap();
    store.import_file("book", &path, &RagImport::new()).unwrap();
    std::fs::write(&path, "Mira knows.").unwrap();
    assert!(store.import_file("book", &path, &import).is_err());
}
#[test]
fn crystal_rejects_unbound_foreign_series_source_memory() {
    use hieronymus::{
        crystals::{CrystalStore, NewCrystal},
        memory_models::TranslationContext,
        workspace::{ShortTermMemoryInput, WorkspaceStore},
    };
    let (_root, config, db) = fixture();
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session = workspace
        .start_session(&TranslationContext::new("other", "en", "ru", "translation"))
        .unwrap();
    let memory = workspace
        .add_short_term_memory(
            session.id,
            &ShortTermMemoryInput::new("note", "An unrelated observation."),
        )
        .unwrap();
    db.execute(
        "delete from claim_bindings where short_term_id=?",
        [memory.id],
    )
    .unwrap();
    let mut crystal = NewCrystal::new("lesson", "Unrelated source.");
    crystal.source_memory_ids = vec![memory.id];
    assert!(
        CrystalStore::open(&config)
            .unwrap()
            .add_crystal(
                &TranslationContext::new("book", "en", "ru", "translation"),
                "lesson",
                &crystal
            )
            .is_err()
    );
}
#[test]
fn facet_merge_rejects_foreign_ownership_and_rolls_back_caller_transaction() {
    use hieronymus::concepts::{ConceptStore, FacetFields};
    let (_root, config, mut db) = fixture();
    let store = ConceptStore::open(&config).unwrap();
    let facet = store
        .add_facet_with_claims(
            1,
            "Mira knows",
            &FacetFields::default(),
            0.5,
            false,
            &[claim()],
        )
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(
        ConceptStore::merge_concepts_in_transaction(&tx, 1, 2, "same name is not same story")
            .is_err()
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "select concept_id from concept_facets where id=?",
            [facet.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select status from concepts where id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "candidate"
    );
}
#[test]
fn facet_metadata_change_cannot_clear_same_content_claim_lineage() {
    use hieronymus::concepts::{ConceptStore, FacetFields, FacetPatch};
    let (_root, config, db) = fixture();
    let store = ConceptStore::open(&config).unwrap();
    let facet = store
        .add_facet_with_claims(
            1,
            "Mira knows",
            &FacetFields::default(),
            0.5,
            false,
            &[claim()],
        )
        .unwrap();
    store
        .update_facet(
            facet.id,
            &FacetPatch {
                story_scopes: Some(Some(vec!["chapter:later".into()])),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from claim_bindings where facet_id=?",
            [facet.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn facet_source_switch_copies_all_masks_and_rejects_foreign_unbound_sources_atomically() {
    use hieronymus::concepts::{ConceptStore, FacetFields, FacetPatch};
    let (_root, config, mut db) = fixture();
    db.execute_batch("insert into crystals(id,crystal_type,text,scope_type,series_slug,strength,confidence,status,created_at,updated_at) values(2,'lesson','second','series','book',0.5,0.5,'active','now','now'),(3,'lesson','foreign unbound','series','other',0.5,0.5,'active','now','now')").unwrap();
    let tx = db.transaction().unwrap();
    let original = capture_claim_tx(&tx, ClaimTarget::Crystal(1), &claim()).unwrap();
    let second = capture_claim_tx(&tx, ClaimTarget::Crystal(2), &claim()).unwrap();
    tx.commit().unwrap();
    let store = ConceptStore::open(&config).unwrap();
    let facet = store
        .add_facet_with_claims(
            1,
            "derived",
            &FacetFields {
                source_crystal_id: Some(1),
                ..Default::default()
            },
            0.8,
            false,
            &[claim()],
        )
        .unwrap();
    store
        .update_facet_with_claims(
            facet.id,
            &FacetPatch {
                source_crystal_id: Some(Some(2)),
                value: Some(Some("changed derived".into())),
                ..Default::default()
            },
            &[claim()],
        )
        .unwrap();
    let ids: Vec<i64> = db
        .prepare("select claim_id from claim_bindings where facet_id=?")
        .unwrap()
        .query_map([facet.id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(ids.contains(&original) && ids.contains(&second));
    assert_eq!(
        ids.len(),
        4,
        "originals and both supplemental observations survive"
    );
    let before = db
        .query_row("select count(*) from memory_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert!(
        store
            .add_facet_with_claims(
                1,
                "foreign",
                &FacetFields {
                    source_crystal_id: Some(3),
                    ..Default::default()
                },
                0.8,
                false,
                &[claim()]
            )
            .is_err()
    );
    assert!(
        store
            .update_facet_with_claims(
                facet.id,
                &FacetPatch {
                    source_crystal_id: Some(Some(3)),
                    ..Default::default()
                },
                &[claim()]
            )
            .is_err()
    );
    assert_eq!(
        db.query_row("select count(*) from memory_claims", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        db.query_row(
            "select source_crystal_id from concept_facets where id=?",
            [facet.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

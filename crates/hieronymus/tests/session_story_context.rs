use hieronymus::{
    data_root::HieronymusConfig,
    db::open_migrated,
    memory_models::TranslationContext,
    registry::Registry,
    story_applicability::{QueryMode, Viewpoint},
    workspace::WorkspaceStore,
};
fn fixture() -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("data"));
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series("book", "Book", "en", "ru", None)
        .unwrap();
    registry
        .create_series("other", "Other", "en", "ru", None)
        .unwrap();
    open_migrated(&config.database_path()).unwrap().execute_batch("insert into story_timelines(id,series_id,name) values(1,1,'main'),(2,2,'other'); insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Mira','series','series:book','now','now'),(2,'Other','series','series:other','now','now');").unwrap();
    (root, config)
}
fn context() -> TranslationContext {
    let mut c = TranslationContext::new("book", "en", "ru", "translation")
        .volume("one")
        .chapter("two")
        .with_metadata(
            None,
            Some(vec!["chapter:two".into(), "volume:one".into()]),
            None,
        );
    c.story_timeline_id = Some(1);
    c.story_scene_key = Some("unplaced scene".into());
    c.story_viewpoint = Viewpoint::Character(1);
    c
}
#[test]
fn supplied_story_context_survives_restart_and_default_lookup_isolated() {
    let (_root, config) = fixture();
    let store = WorkspaceStore::open(&config).unwrap();
    let c = context();
    let session = store.start_session(&c).unwrap();
    drop(store);
    let reopened = WorkspaceStore::open(&config).unwrap();
    assert_eq!(reopened.get_session(session.id).unwrap().context, c);
    assert_eq!(
        reopened.active_default_session(&c).unwrap().unwrap().id,
        session.id
    );
    let mut other = c.clone();
    other.story_viewpoint = Viewpoint::Narrator;
    assert!(reopened.active_default_session(&other).unwrap().is_none());
    let other_session = reopened.start_session(&other).unwrap();
    assert_ne!(session.id, other_session.id);
    assert_eq!(
        reopened.active_default_session(&c).unwrap().unwrap().id,
        session.id
    );
}
#[test]
fn session_rejects_foreign_context_and_sticky_research() {
    let (_root, config) = fixture();
    let store = WorkspaceStore::open(&config).unwrap();
    for case in 0..3 {
        let mut c = context();
        match case {
            0 => c.story_timeline_id = Some(2),
            1 => c.story_viewpoint = Viewpoint::Character(2),
            _ => c.story_query_mode = QueryMode::OmniscientResearch,
        };
        assert!(store.start_session(&c).is_err(), "accepted case {case}");
    }
}

#[test]
fn reloaded_corrupt_or_foreign_story_context_is_not_silently_defaulted() {
    let (_root, config) = fixture();
    let ws = WorkspaceStore::open(&config).unwrap();
    let id = ws.start_session(&context()).unwrap().id;
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute(
        "update task_sessions set story_viewpoint_json='\"Corrupt\"' where id=?",
        [id],
    )
    .unwrap();
    assert!(ws.get_session(id).is_err());
    db.execute("update task_sessions set story_viewpoint_json='\"Narrator\"',story_timeline_id=2 where id=?",[id]).unwrap();
    assert!(ws.get_session(id).is_err());
}

#[test]
fn narrated_capture_uses_persisted_real_position_and_remains_current_after_restart() {
    use hieronymus::{recall::RecallService, workspace::ShortTermMemoryInput};
    let (_root, config) = fixture();
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into evidence_records(id,series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(1,1,'observation','manifest','hash',0,1,'x','{}','now'); insert into story_positions(timeline_id,volume_key,chapter_key,scene_key,ordinal,evidence_id) values(1,'one','two','unplaced scene',0,1)").unwrap();
    let ws = WorkspaceStore::open(&config).unwrap();
    let mut c = context();
    c.story_viewpoint = Viewpoint::Narrator;
    let id = ws.start_session(&c).unwrap().id;
    drop(ws);
    let ws = WorkspaceStore::open(&config).unwrap();
    ws.add_short_term_memory(
        id,
        &ShortTermMemoryInput::new("note", "Mira discovers the hidden bridge."),
    )
    .unwrap();
    let c = ws.get_session(id).unwrap().context;
    let response = RecallService::open(&config)
        .unwrap()
        .recall(id, &c, "hidden bridge", 3)
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert!(response.non_current.is_empty());
}

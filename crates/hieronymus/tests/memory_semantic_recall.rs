#[path = "support/current_story.rs"]
mod current_story;
use hieronymus::{
    crystals::{CrystalStore, NewCrystal},
    data_root::HieronymusConfig,
    db::open_migrated,
    memory_models::TranslationContext,
    recall::{RecallHit, RecallService},
    registry::Registry,
    semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider},
    semantic_error::SemanticError,
    semantic_jobs::{AuthoritativeChunk, ChunkTokenizer},
    semantic_recall::SemanticLane,
    workspace::{ShortTermMemoryInput, WorkspaceStore},
};

/// Controlled bilingual embedding seam verifies candidate plumbing; it is not
/// presented as a native-model quality benchmark.
struct Multilingual(FakeEmbeddingProvider);
impl EmbeddingProvider for Multilingual {
    fn identity(&self) -> &EmbeddingIdentity {
        self.0.identity()
    }
    fn embed_document(&mut self, t: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.0.embed_document(t)
    }
    fn embed_query(&mut self, t: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.0.embed_query(t)
    }
    fn embed_document_text(&mut self, text: &str, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        Ok(meaning(text))
    }
    fn embed_query_text(&mut self, text: &str, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        Ok(meaning(text))
    }
}
fn meaning(text: &str) -> Vec<f32> {
    if ["tea", "чай", "お茶"].iter().any(|s| text.contains(s)) {
        vec![1., 0., 0., 0.]
    } else {
        vec![0., 1., 0., 0.]
    }
}
struct Tokens;
impl ChunkTokenizer for Tokens {
    fn tokenize(&mut self, _: &AuthoritativeChunk) -> Result<Vec<u32>, SemanticError> {
        Ok(vec![1])
    }
}
fn service(config: &HieronymusConfig) -> RecallService {
    RecallService::open(config)
        .unwrap()
        .with_semantic_lane(SemanticLane::new(
            Box::new(Multilingual(FakeEmbeddingProvider::new(4))),
            Box::new(Tokens),
        ))
}
fn setup() -> (tempfile::TempDir, HieronymusConfig, TranslationContext) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    for slug in ["book", "other"] {
        Registry::open(&config)
            .unwrap()
            .create_series(slug, slug, "ja", "ru", None)
            .unwrap();
        current_story::register_public(&config, slug, "I", "Opening");
    }
    let mut context = current_story::context("book", "ja", "ru", "translate");
    context.story_viewpoint = hieronymus::story_applicability::Viewpoint::Narrator;
    (root, config, context)
}
#[test]
fn russian_and_japanese_queries_find_english_memory_across_sessions() {
    let (_root, config, context) = setup();
    let ws = WorkspaceStore::open(&config).unwrap();
    let first = ws.start_session(&context).unwrap();
    let crystal = CrystalStore::open(&config)
        .unwrap()
        .add_crystal(
            &context,
            "lesson",
            &claimed(&config, "book", "Mira drinks green tea at dawn."),
        )
        .unwrap();
    let text = "The tea kettle belongs to Ren.";
    let mut input = ShortTermMemoryInput::new("note", text);
    input.claims = vec![current_story::claim(&config, "book", text)];
    let memory = ws.add_short_term_memory(first.id, &input).unwrap();
    ws.complete_session(first.id).unwrap();
    let second = ws.start_session(&context).unwrap();
    let recall = service(&config);
    for query in ["кто пьёт чай утром", "朝のお茶"] {
        let result = recall.recall(second.id, &context, query, 10).unwrap();
        assert!(
            result
                .hits
                .iter()
                .any(|h| matches!(h,RecallHit::LongTerm{crystal:c,..} if c.id==crystal))
        );
        assert!(
            result
                .hits
                .iter()
                .any(|h| matches!(h,RecallHit::ShortTerm{memory:m,..} if m.id==memory.id))
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.kind.starts_with("memory_semantic")),
            "{:?}",
            result.warnings
        );
    }
}
#[test]
fn stale_deleted_archived_and_foreign_memory_never_survive_hydration() {
    let (_root, config, context) = setup();
    let store = CrystalStore::open(&config).unwrap();
    let id = store
        .add_crystal(
            &context,
            "lesson",
            &claimed(&config, "book", "Mira drinks tea."),
        )
        .unwrap();
    let mut foreign = context.clone();
    foreign.series_slug = "other".into();
    let foreign_id = store
        .add_crystal(
            &foreign,
            "lesson",
            &claimed(&config, "other", "Mira drinks tea."),
        )
        .unwrap();
    let recall = service(&config);
    let first = recall.recall_context(&context, "чай", 10).unwrap();
    assert!(
        first
            .hits
            .iter()
            .any(|h| matches!(h,RecallHit::LongTerm{crystal,..} if crystal.id==id))
    );
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute("update crystals set status='superseded' where id=?", [id])
        .unwrap();
    let result = recall.recall_context(&context, "чай", 10).unwrap();
    assert!(!result.hits.iter().any(
        |h| matches!(h,RecallHit::LongTerm{crystal,..} if [id,foreign_id].contains(&crystal.id))
    ));
    assert_eq!(
        db.query_row(
            "select count(*) from memory_vectors where kind='crystal' and id=?",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn bounded_indexing_resumes_after_restart_and_updates_changed_text() {
    let (_root, config, context) = setup();
    let store = CrystalStore::open(&config).unwrap();
    let mut last = 0;
    for n in 0..40 {
        last = store
            .add_crystal(
                &context,
                "lesson",
                &NewCrystal::new("lesson", format!("tea observation {n}")),
            )
            .unwrap();
    }
    let first = service(&config)
        .recall_context(&context, "чай", 10)
        .unwrap();
    assert!(
        first
            .warnings
            .iter()
            .any(|w| w.kind == "memory_semantic_pending")
    );
    let second = service(&config)
        .recall_context(&context, "お茶", 10)
        .unwrap();
    assert!(
        !second
            .warnings
            .iter()
            .any(|w| w.kind == "memory_semantic_pending")
    );
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute(
        "update crystals set text='New corrected prose' where id=?",
        [last],
    )
    .unwrap();
    service(&config)
        .recall_context(&context, "new prose", 10)
        .unwrap();
    assert_eq!(
        db.query_row(
            "select source_text from memory_vectors where kind='crystal' and id=?",
            [last],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "New corrected prose"
    );
}

fn claimed(config: &HieronymusConfig, slug: &str, text: &str) -> NewCrystal {
    let mut v = NewCrystal::new("lesson", text);
    v.claims = vec![current_story::claim(config, slug, text)];
    v
}

#[test]
fn malformed_derived_vectors_rebuild_without_source_changes() {
    let (_root, config, context) = setup();
    let id = CrystalStore::open(&config)
        .unwrap()
        .add_crystal(
            &context,
            "lesson",
            &claimed(&config, "book", "tea ceremony at dawn"),
        )
        .unwrap();
    let recall = service(&config);
    recall.recall_context(&context, "чай", 10).unwrap();
    let db = open_migrated(&config.database_path()).unwrap();
    for bad in ["broken", "[0,0,0,0]", "[1,0]", "[1,\"bad\",0,0]"] {
        db.execute(
            "update memory_vectors set vector_json=? where kind='crystal' and id=?",
            rusqlite::params![bad, id],
        )
        .unwrap();
        let result = recall.recall_context(&context, "чай", 10).unwrap();
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.kind.starts_with("memory_semantic")),
            "{:?}",
            result.warnings
        );
        assert!(
            result
                .hits
                .iter()
                .any(|h| matches!(h,RecallHit::LongTerm{crystal,..} if crystal.id==id))
        );
    }
}

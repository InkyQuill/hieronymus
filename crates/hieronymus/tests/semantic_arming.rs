//! Semantic lane arming (Task 9 review ruling): a real, non-test path from
//! model identity to an armed `RecallService` — explicit acquisition only,
//! ONNX provider construction, `with_semantic_lane`, and FTS-only degraded
//! mode whenever the model is absent or its identity no longer matches the
//! active generation. The end-to-end armed runs here use the deterministic
//! fake provider; the real ONNX path stays behind the env-gated live test.

#[path = "support/current_story.rs"]
mod current_story;

use std::path::{Path, PathBuf};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::recall::{RecallHit, WARNING_SEMANTIC_UNAVAILABLE};
use hieronymus::registry::Registry;
use hieronymus::semantic_arming::{
    LaneState, arm_recall_service, arm_with_provider, semantic_status,
};
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider,
    OnnxEmbeddingProvider,
};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_model::{MODEL_BYTES, ModelStatus};
use hieronymus::semantic_recall::{BYTE_FOLD_TOKENIZER_ID, SemanticLane};
use hieronymus::semantic_store::{SemanticChunk, SemanticSample, SemanticStore};
use hieronymus::semantic_tokenizer::ModelTokenizer;
use hieronymus::workspace::WorkspaceStore;

// ---------------------------------------------------------------------------
// Fixtures (mirrors semantic_recall_port)
// ---------------------------------------------------------------------------

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    session_id: i64,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();
    registry
        .create_series("demo", "demo", "ja", "en", None)
        .unwrap();
    current_story::register_public(&config, "demo", "I", "Opening");
    let workspace = WorkspaceStore::open(&config).unwrap();
    let context = context();
    let session = workspace.start_session(&context).unwrap();
    Fixture {
        root,
        config,
        session_id: session.id,
    }
}

fn context() -> TranslationContext {
    current_story::context("demo", "ja", "en", "translation")
}

fn import_text(fixture: &Fixture, name: &str, content: &str) {
    let path = fixture.root.path().join(name);
    std::fs::write(&path, content).unwrap();
    let mut import = RagImport::new();
    import.claims = std::collections::BTreeMap::from([(
        0,
        vec![current_story::claim(&fixture.config, "demo", content)],
    )]);
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file("demo", &path, &import)
        .unwrap();
}

/// Builds and activates a whole-corpus generation over the fixture's chunks
/// with the given provider identity (the exact tokenizer the armed lane uses
/// for queries, so document and query vectors share one mapping).
fn activate_generation_with(fixture: &Fixture, provider: &mut dyn EmbeddingProvider) {
    let tokenizer = model_tokenizer();
    let store = SemanticStore::open(&fixture.config).unwrap();
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    loop {
        let pending = store.pending_chunk_ids("gen-a", 8).unwrap();
        if pending.is_empty() {
            break;
        }
        let chunks: Vec<SemanticChunk> = pending
            .iter()
            .map(|chunk_id| {
                let (series_slug, text) = store
                    .chunk_row(*chunk_id)
                    .unwrap()
                    .expect("chunk exists in the authoritative store");
                SemanticChunk {
                    chunk_id: *chunk_id,
                    series_slug,
                    token_ids: tokenizer.encode(&text).unwrap(),
                }
            })
            .collect();
        store.write_batch("gen-a", provider, &chunks).unwrap();
    }
    store
        .activate_generation(
            "gen-a",
            provider,
            &SemanticSample {
                series_slug: "demo".to_string(),
                token_ids: tokenizer.encode("probe").unwrap(),
            },
        )
        .unwrap();
}

fn model_tokenizer() -> ModelTokenizer {
    ModelTokenizer::from_bytes(include_bytes!("fixtures/minilm-tokenizer.json")).unwrap()
}

fn activate_generation(fixture: &Fixture) {
    activate_generation_with(
        fixture,
        &mut FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS),
    );
}

/// Seeds a model file with the pinned size (sparse write: no data, only the
/// expected length) so `model_status` reports `Available`.
fn seed_model_file(fixture: &Fixture) -> PathBuf {
    let store = SemanticStore::open(&fixture.config).unwrap();
    let path = store.model_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MODEL_BYTES).unwrap();
    path
}

// ---------------------------------------------------------------------------
// Arming
// ---------------------------------------------------------------------------

/// Arming itself never fails on a missing model — that is the point of the
/// disarmed lane state. What the disarmed service must NOT do (task C5,
/// review finding A5) is answer as if every required lane had run: recall
/// still serves its FTS hits, and it reports the missing semantic half.
#[test]
fn absent_model_disarms_to_a_reported_fts_only_lane_without_failing() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");

    let armed = arm_recall_service(&fixture.config, Path::new("/nonexistent/libonnxruntime.so"))
        .expect("arming never fails on a missing model");
    match armed.lane {
        LaneState::Disarmed { reason } => {
            assert!(reason.contains("acquired"), "{reason}");
        }
        LaneState::Armed => panic!("a missing model must not arm the lane"),
    }

    // The disarmed service still recalls the FTS hits — and says outright
    // that required semantics did not run.
    let response = armed
        .service
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE),
        "a disarmed lane must be reported, not silent: {:?}",
        response.warnings
    );
    assert!(
        response
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. }))
    );
}

#[test]
fn invalid_model_file_disarms_with_the_verdict() {
    let fixture = fixture();
    let path = seed_model_file(&fixture);
    // Truncate: a present but wrongly sized file is Invalid, not Missing.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MODEL_BYTES - 1).unwrap();

    let armed =
        arm_recall_service(&fixture.config, Path::new("/nonexistent/libonnxruntime.so")).unwrap();
    match armed.lane {
        LaneState::Disarmed { reason } => {
            assert!(reason.contains("pre-check"), "{reason}");
        }
        LaneState::Armed => panic!("an invalid model must not arm the lane"),
    }
}

#[test]
fn armed_lane_runs_end_to_end_with_a_matching_provider() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);

    let armed = arm_with_provider(
        &fixture.config,
        Box::new(FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)),
        Box::new(model_tokenizer()),
    )
    .expect("arming with a matching provider");
    assert!(matches!(armed.lane, LaneState::Armed), "{:?}", armed.lane);

    let response = armed
        .service
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    assert!(response.warnings.is_empty(), "{:?}", response.warnings);
    assert!(
        response
            .hits
            .iter()
            .any(|hit| matches!(hit, RecallHit::Rag { .. })),
        "the fused lane must answer the query"
    );
}

#[test]
fn model_identity_mismatch_degrades_the_armed_lane() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);

    let armed = arm_with_provider(
        &fixture.config,
        Box::new(FakeEmbeddingProvider::with_model(
            EMBEDDING_DIMENSIONS,
            "other-model",
        )),
        Box::new(model_tokenizer()),
    )
    .unwrap();
    assert!(
        matches!(armed.lane, LaneState::Armed),
        "arming attaches the lane; the identity check happens per recall"
    );
    let response = armed
        .service
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE),
        "{:?}",
        response.warnings
    );
}

#[test]
fn a_persisted_byte_fold_generation_cannot_serve_new_queries() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    // A generation persisted under the retired byte-fold tokenization: the
    // identity is identical to the pinned one except the tokenizer id.
    activate_generation_with(&fixture, &mut StaticTokenizerProvider::byte_fold());

    // The current lane — same model, pinned WordPiece tokenizer — differs in
    // exactly one identity field. It can never query the old generation:
    // embeddings changed with the tokenizer, so the lane degrades until a
    // rebuild re-embeds everything under the new identity (the old
    // generation is never relabeled).
    let armed = arm_with_provider(
        &fixture.config,
        Box::new(FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)),
        Box::new(model_tokenizer()),
    )
    .unwrap();
    let response = armed
        .service
        .recall(fixture.session_id, &context(), "Cooking Talent", 10)
        .unwrap();
    let unavailable = response
        .warnings
        .iter()
        .find(|warning| warning.kind == WARNING_SEMANTIC_UNAVAILABLE)
        .expect("tokenizer swap must degrade the lane");
    assert!(
        unavailable.reason.contains("identity"),
        "{}",
        unavailable.reason
    );
}

/// A fake provider that reports the pinned model identity except for the
/// tokenizer: the minimal stand-in for "same model, new tokenization".
struct StaticTokenizerProvider {
    inner: FakeEmbeddingProvider,
    identity: EmbeddingIdentity,
}

impl StaticTokenizerProvider {
    /// The pinned model identity under the given tokenizer id; the embeddings
    /// themselves come from the deterministic fake provider, so no runtime or
    /// model file is needed.
    fn with_tokenizer(tokenizer: &str) -> Self {
        let inner = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
        let pinned = OnnxEmbeddingProvider::static_identity();
        let identity = EmbeddingIdentity::new(
            pinned.provider(),
            pinned.model(),
            pinned.revision(),
            pinned.dimensions(),
            pinned.normalization(),
            tokenizer,
            pinned.max_input_tokens(),
            pinned.max_batch_inputs(),
        )
        .expect("identity is valid");
        Self { inner, identity }
    }

    fn byte_fold() -> Self {
        Self::with_tokenizer(BYTE_FOLD_TOKENIZER_ID)
    }
}

impl EmbeddingProvider for StaticTokenizerProvider {
    fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.inner.embed_document(token_ids)
    }

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.inner.embed_query(token_ids)
    }
}

// ---------------------------------------------------------------------------
// Status (report-only; doctor and the CLI consume it)
// ---------------------------------------------------------------------------

#[test]
fn semantic_status_reports_missing_model_and_no_generation() {
    let fixture = fixture();
    let status = semantic_status(&fixture.config).unwrap();
    assert_eq!(status.model_status, ModelStatus::Missing);
    assert!(status.active_generation.is_none());
    assert!(status.generation_intact);
    assert_eq!(
        status.tokenizer,
        hieronymus::semantic_tokenizer::MINILM_TOKENIZER_ID
    );
}

#[test]
fn semantic_status_reports_an_active_generation_and_its_integrity() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);
    seed_model_file(&fixture);

    let status = semantic_status(&fixture.config).unwrap();
    assert_eq!(status.model_status, ModelStatus::Available);
    let manifest = status.active_generation.expect("active generation");
    assert_eq!(manifest.generation_id, "gen-a");
    assert!(status.generation_intact);
}

#[test]
fn semantic_status_reports_a_lost_index_as_rebuild_required() {
    let fixture = fixture();
    import_text(&fixture, "a.txt", "Cooking Talent appears here.");
    activate_generation(&fixture);
    let status = semantic_status(&fixture.config).unwrap();
    assert!(status.generation_intact);

    // Losing the LanceDB directory is detectable without any download.
    let index_root = SemanticStore::open(&fixture.config).unwrap().index_root();
    std::fs::remove_dir_all(index_root).unwrap();
    let status = semantic_status(&fixture.config).unwrap();
    assert!(!status.generation_intact);
}

// ---------------------------------------------------------------------------
// Identity covers the tokenizer (Task 9 review follow-up)
// ---------------------------------------------------------------------------

#[test]
fn identity_equality_covers_the_tokenizer() {
    let a = EmbeddingIdentity::new("fake", "m", "r", 4, "l2", "tok-a", 8, 2).unwrap();
    let same = EmbeddingIdentity::new("fake", "m", "r", 4, "l2", "tok-a", 8, 2).unwrap();
    let other_tokenizer = EmbeddingIdentity::new("fake", "m", "r", 4, "l2", "tok-b", 8, 2).unwrap();
    assert_eq!(a, same);
    assert_ne!(a, other_tokenizer);
    assert_eq!(a.tokenizer(), "tok-a");
    assert!(
        EmbeddingIdentity::new("fake", "m", "r", 4, "l2", "", 8, 2).is_err(),
        "the tokenizer must be named"
    );
}

#[test]
fn generation_manifest_persists_the_tokenizer() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    store
        .begin_generation("gen-tok", &StaticTokenizerProvider::byte_fold().identity)
        .unwrap();
    let manifest = store.generation_manifest("gen-tok").unwrap().unwrap();
    assert_eq!(manifest.identity.tokenizer(), BYTE_FOLD_TOKENIZER_ID);

    // The pinned WordPiece lane can never query this generation.
    let lane = SemanticLane::new(
        Box::new(FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)),
        Box::new(model_tokenizer()),
    );
    let run = lane.run(&fixture.config, &context(), "probe", 5);
    assert!(run.degraded);
}

#[test]
fn pinned_identity_carries_the_wordpiece_tokenizer() {
    let identity = OnnxEmbeddingProvider::static_identity();
    assert_eq!(
        identity.tokenizer(),
        hieronymus::semantic_tokenizer::MINILM_TOKENIZER_ID
    );
    // And it is a different identity from every retired byte-fold generation.
    assert_ne!(identity.tokenizer(), BYTE_FOLD_TOKENIZER_ID);
}

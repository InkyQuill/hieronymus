//! Semantic RAG data plane: embedding identity, model acquisition over a
//! loopback transport seam, the LanceDB vector store with series pre-filter
//! before ANN ranking, the generation lifecycle, and FTS-only degraded mode.
//! Tests never egress and never download models: the fake embedding provider
//! and the loopback HTTP server stand in for the real artifacts.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::registry::Registry;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider,
    OnnxEmbeddingProvider,
};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_index::{IndexRow, RowFingerprint, VectorIndex};
use hieronymus::semantic_model::{
    MODEL_BYTES, MODEL_NAME, MODEL_REVISION, MODEL_SHA256, ModelStatus, ModelTransport,
};
use hieronymus::semantic_store::{SemanticChunk, SemanticSample, SemanticStore};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    #[allow(dead_code)] // holds the temp directory alive for the config paths
    root: tempfile::TempDir,
    config: HieronymusConfig,
    series_slug: String,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path().join("hieronymus"));
    let registry = Registry::open(&config).unwrap();
    let series = registry
        .create_series("only-sense-online", "Only Sense Online", "ja", "ru", None)
        .unwrap();
    Fixture {
        root,
        config,
        series_slug: series.slug,
    }
}

fn write_source(fixture: &Fixture, name: &str, content: &str) -> PathBuf {
    let path = fixture.root.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Import one source with one chunk per paragraph and return the store.
fn import_source(fixture: &Fixture, name: &str, content: &str) {
    let path = write_source(fixture, name, content);
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();
}

/// Token stream standing in for a tokenizer: deterministic per chunk text.
fn tokens_for(text: &str) -> Vec<u32> {
    text.bytes()
        .enumerate()
        .map(|(index, byte)| ((u32::from(byte) * 31 + index as u32) % 30_000) + 1)
        .collect()
}

/// Writes every pending chunk of the generation using deterministic token
/// streams, one bounded batch at a time.
fn drain_generation(
    store: &SemanticStore,
    generation: &str,
    provider: &mut dyn EmbeddingProvider,
    batch_size: usize,
) -> usize {
    let mut written = 0;
    loop {
        let pending = store.pending_chunk_ids(generation, batch_size).unwrap();
        if pending.is_empty() {
            return written;
        }
        let chunks: Vec<SemanticChunk> = pending
            .iter()
            .map(|chunk_id| SemanticChunk {
                chunk_id: *chunk_id,
                series_slug: series_of_chunk(store, *chunk_id),
                token_ids: tokens_for(&format!("chunk-{chunk_id}")),
            })
            .collect();
        written += store.write_batch(generation, provider, &chunks).unwrap();
        if pending.len() < batch_size {
            return written;
        }
    }
}

fn series_of_chunk(store: &SemanticStore, chunk_id: i64) -> String {
    store
        .chunk_series_slug(chunk_id)
        .unwrap()
        .expect("chunk exists in the authoritative store")
}

// ---------------------------------------------------------------------------
// Loopback HTTP server (binary responses, mirrors the Task 4 seam)
// ---------------------------------------------------------------------------

struct LoopbackFile {
    url: String,
    requests: Arc<Mutex<usize>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackFile {
    /// Serves `body` with `Content-Length: advertised`, streaming at most
    /// `sent` bytes before closing the connection (a truncated download).
    fn start(body: &'static [u8], advertised: usize, sent: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let accept_thread = std::thread::spawn(move || {
            loop {
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                // Non-blocking accept so the stop flag is checked while no
                // request arrives; the flag makes teardown deterministic.
                let Ok((socket, _peer)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let mut stream = socket;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer);
                *thread_requests.lock().unwrap() += 1;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body[..sent.min(body.len())]);
                let _ = stream.flush();
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/model.onnx"),
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().unwrap()
    }
}

impl Drop for LoopbackFile {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().unwrap();
        }
    }
}

/// A 64-byte payload with a known sha256, small enough for fast loopback runs.
const PAYLOAD: &[u8; 64] = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PAYLOAD_SHA256: &str = "a8ae6e6ee929abea3afcfc5258c8ccd6f85273e0d4626d26c7279f3250f77c8e";

fn expected_payload_checksum() -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(PAYLOAD);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Embedding identity and providers
// ---------------------------------------------------------------------------

#[test]
fn fake_provider_is_deterministic_l2_normalized_and_width_stable() {
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let identity = provider.identity().clone();
    assert_eq!(identity.provider(), "fake");
    assert_eq!(identity.dimensions(), EMBEDDING_DIMENSIONS);
    assert_eq!(identity.normalization(), "l2");

    let first = provider.embed_document(&[10, 20, 30]).unwrap();
    let second = provider.embed_document(&[10, 20, 30]).unwrap();
    assert_eq!(first.len(), EMBEDDING_DIMENSIONS);
    assert_eq!(first, second);
    let other = provider.embed_document(&[10, 20, 31]).unwrap();
    assert_ne!(first, other);
    let norm: f32 = first.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-5,
        "embeddings must be L2 normalized, got {norm}"
    );
    // Query embeddings share the document identity and the mapping.
    let query = provider.embed_query(&[10, 20, 30]).unwrap();
    assert_eq!(query, first);
}

#[test]
fn provider_identities_reject_empty_and_oversized_token_inputs() {
    let mut fake = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    assert!(matches!(
        fake.embed_document(&[]),
        Err(SemanticError::InvalidEmbedding(_))
    ));
    let oversized = vec![1_u32; fake.identity().max_input_tokens() + 1];
    assert!(matches!(
        fake.embed_document(&oversized),
        Err(SemanticError::InvalidEmbedding(_))
    ));

    let onnx_identity = OnnxEmbeddingProvider::static_identity();
    assert!(matches!(
        onnx_identity.check_tokens(0),
        Err(SemanticError::InvalidEmbedding(_))
    ));
    assert!(
        onnx_identity
            .check_tokens(onnx_identity.max_input_tokens())
            .is_ok()
    );
}

#[test]
fn onnx_identity_reports_the_qualified_model() {
    let identity = OnnxEmbeddingProvider::static_identity();
    assert_eq!(identity.provider(), "onnx");
    assert_eq!(identity.model(), MODEL_NAME);
    assert_eq!(identity.revision(), MODEL_REVISION);
    assert_eq!(identity.dimensions(), EMBEDDING_DIMENSIONS);
    assert_eq!(identity.normalization(), "l2");
    assert_eq!(identity.max_input_tokens(), 512);
    assert_eq!(identity.max_batch_inputs(), 32);
    // The well-known model constants come verbatim from the qualification.
    assert_eq!(MODEL_SHA256.len(), 64);
    assert_eq!(MODEL_BYTES, 470_301_610);
}

#[test]
fn onnx_provider_load_rejects_missing_runtime_and_checksum_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("libonnxruntime.so");
    let model = root.path().join("model.onnx");
    std::fs::write(&model, b"not really the model").unwrap();

    let missing_runtime = OnnxEmbeddingProvider::load(&runtime, &model);
    let error = missing_runtime.expect_err("missing runtime is rejected");
    assert!(
        error.to_string().contains("runtime"),
        "unexpected error: {error}"
    );

    std::fs::write(&runtime, b"pretend runtime").unwrap();
    let bad_checksum = OnnxEmbeddingProvider::load(&runtime, &model);
    let error =
        bad_checksum.expect_err("checksum mismatch must be rejected before anything executes");
    assert!(
        error.to_string().contains("checksum"),
        "unexpected error: {error}"
    );
    assert!(matches!(error, SemanticError::ChecksumMismatch { .. }));
}

#[test]
fn identities_with_different_models_or_dimensions_are_not_equal() {
    let a =
        EmbeddingIdentity::new("fake", "model-a", "rev-1", 4, "l2", "byte-fold-v1", 8, 2).unwrap();
    let b =
        EmbeddingIdentity::new("fake", "model-b", "rev-1", 4, "l2", "byte-fold-v1", 8, 2).unwrap();
    let c =
        EmbeddingIdentity::new("fake", "model-a", "rev-1", 8, "l2", "byte-fold-v1", 8, 2).unwrap();
    assert_eq!(a, a.clone());
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert!(EmbeddingIdentity::new("fake", "", "rev", 4, "l2", "byte-fold-v1", 8, 2).is_err());
    assert!(EmbeddingIdentity::new("fake", "m", "rev", 0, "l2", "byte-fold-v1", 8, 2).is_err());
    assert!(EmbeddingIdentity::new("fake", "m", "rev", 4, "", "byte-fold-v1", 8, 2).is_err());
}

// ---------------------------------------------------------------------------
// Model acquisition (loopback seam, no egress)
// ---------------------------------------------------------------------------

#[test]
fn acquire_model_verifies_and_promotes_atomically() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    assert_eq!(store.model_status(), ModelStatus::Missing);

    let server = LoopbackFile::start(PAYLOAD, PAYLOAD.len(), PAYLOAD.len());
    let transport = hieronymus::semantic_model::HttpModelTransport::new(Duration::from_secs(10));
    let checksum = expected_payload_checksum();
    assert_eq!(checksum, PAYLOAD_SHA256); // the loopback payload digest is precomputed
    let acquired = store
        .acquire_model_verifying(&transport, &server.url, &checksum, PAYLOAD.len() as u64)
        .unwrap();
    assert_eq!(acquired.bytes, PAYLOAD.len() as u64);
    assert_eq!(acquired.checksum, checksum);
    // `model_status` is the verdict over the pinned model; a custom verified
    // artifact was promoted, so the promoted file must hold exactly the
    // downloaded bytes.
    let model_path = store.model_path();
    assert_eq!(std::fs::read(&model_path).unwrap(), PAYLOAD.to_vec());
    // No temporary leftovers survive the promotion.
    let model_dir = model_path.parent().unwrap().to_path_buf();
    let leftovers: Vec<String> = std::fs::read_dir(&model_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["model.onnx".to_string()]);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn acquire_model_rejects_checksum_mismatch_without_promotion() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    let server = LoopbackFile::start(PAYLOAD, PAYLOAD.len(), PAYLOAD.len());
    let transport = hieronymus::semantic_model::HttpModelTransport::new(Duration::from_secs(10));

    let wrong = store
        .acquire_model_verifying(
            &transport,
            &server.url,
            "0000000000000000000000000000000000000000000000000000000000000000",
            PAYLOAD.len() as u64,
        )
        .expect_err("wrong checksum must be rejected");
    assert!(
        matches!(wrong, SemanticError::ChecksumMismatch { .. }),
        "unexpected error: {wrong}"
    );
    assert_eq!(store.model_status(), ModelStatus::Missing);
    assert!(!store.model_path().exists());
}

#[test]
fn acquire_model_rejects_truncated_download_without_promotion() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    // Advertise the full payload but close the connection after 8 bytes.
    let server = LoopbackFile::start(PAYLOAD, PAYLOAD.len(), 8);
    let transport = hieronymus::semantic_model::HttpModelTransport::new(Duration::from_secs(10));

    let error = store
        .acquire_model_verifying(
            &transport,
            &server.url,
            &expected_payload_checksum(),
            PAYLOAD.len() as u64,
        )
        .expect_err("truncated download must be rejected");
    assert!(
        error.to_string().contains("truncated"),
        "unexpected error: {error}"
    );
    assert_eq!(store.model_status(), ModelStatus::Missing);
    assert!(!store.model_path().exists());
}

#[test]
fn failed_promotion_cleans_up_and_keeps_previous_model_absent() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    // A directory squatting on the final path makes the atomic rename fail.
    std::fs::create_dir_all(store.model_path()).unwrap();
    let server = LoopbackFile::start(PAYLOAD, PAYLOAD.len(), PAYLOAD.len());
    let transport = hieronymus::semantic_model::HttpModelTransport::new(Duration::from_secs(10));

    let error = store
        .acquire_model_verifying(
            &transport,
            &server.url,
            &expected_payload_checksum(),
            PAYLOAD.len() as u64,
        )
        .expect_err("promotion failure must propagate");
    assert!(
        error.to_string().contains("promote"),
        "unexpected error: {error}"
    );
    // The temp file is gone; only the squatting directory remains.
    let model_path = store.model_path();
    let model_dir = model_path.parent().unwrap();
    let leftovers: Vec<String> = std::fs::read_dir(model_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["model.onnx".to_string()]);
    assert!(store.model_path().is_dir());
}

#[test]
fn download_transport_fails_closed_on_non_http_urls() {
    let fixture = fixture();
    let _store = SemanticStore::open(&fixture.config).unwrap();
    let transport = hieronymus::semantic_model::HttpModelTransport::new(Duration::from_secs(10));
    // https is supported since the TLS slice (see tests/tls_transport.rs);
    // genuinely unsupported schemes still fail closed without dialing.
    for url in [
        "ftp://example.invalid/model.onnx",
        "gopher://example.invalid/model.onnx",
        "example.invalid/model.onnx",
    ] {
        let error = transport
            .download_to(url, &fixture.root.path().join("out.bin"), 1024)
            .expect_err("unsupported schemes must fail closed");
        assert!(
            matches!(error, SemanticError::UnsupportedUrl(_)),
            "{url}: {error}"
        );
    }
    assert!(!fixture.root.path().join("out.bin").exists());
}

// ---------------------------------------------------------------------------
// Vector store: series pre-filter before ANN
// ---------------------------------------------------------------------------

/// One adversarial population mirroring the qualified harness criterion:
/// 100 ineligible decoys strictly closer to the needle than the 10 eligible
/// rows, plus 150 distant filler rows so the 8-bit PQ codebook can train.
const NEEDLE_SERIES: &str = "eligible";
const DECOY_SERIES: &str = "ineligible";

fn rotated_vector(angle: f32, fan: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[0] = angle.cos();
    vector[1 + (fan % (EMBEDDING_DIMENSIONS - 1))] = angle.sin();
    vector
}

fn needle() -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[0] = 1.0;
    vector
}

fn adversarial_rows() -> Vec<IndexRow> {
    let identity = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)
        .identity()
        .clone();
    let mut rows = Vec::new();
    for id in 11..=110_i64 {
        let decoy_index = (id - 11) as f32;
        rows.push(IndexRow {
            chunk_id: id,
            series_slug: DECOY_SERIES.to_string(),
            checksum: format!("decoy-{id:03}"),
            generation_id: "generation-a".to_string(),
            model: identity.model().to_string(),
            model_revision: identity.revision().to_string(),
            vector: rotated_vector(0.055 + 0.005 * decoy_index, (id - 11) as usize),
        });
    }
    for id in 1..=10_i64 {
        rows.push(IndexRow {
            chunk_id: id,
            series_slug: NEEDLE_SERIES.to_string(),
            checksum: format!("eligible-{id:02}"),
            generation_id: "generation-a".to_string(),
            model: identity.model().to_string(),
            model_revision: identity.revision().to_string(),
            vector: rotated_vector(0.6 + 0.05 * id as f32, 0),
        });
    }
    for offset in 0..150_i64 {
        rows.push(IndexRow {
            chunk_id: 1000 + offset,
            series_slug: "filler".to_string(),
            checksum: format!("filler-{offset:03}"),
            generation_id: "generation-a".to_string(),
            model: identity.model().to_string(),
            model_revision: identity.revision().to_string(),
            vector: rotated_vector(std::f32::consts::FRAC_PI_2, offset as usize),
        });
    }
    rows
}

#[test]
fn series_prefilter_beats_closer_decoys_and_returns_zero_cross_series_hits() {
    let root = tempfile::tempdir().unwrap();
    let identity = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)
        .identity()
        .clone();
    let mut index = VectorIndex::open(root.path(), identity, "generation-a").unwrap();
    index.append(adversarial_rows()).unwrap();
    index.create_ann_index().unwrap();

    let hits = index.search(NEEDLE_SERIES, &needle(), 10).unwrap();
    assert_eq!(hits.len(), 10, "pre-filter must recover the eligible rows");
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>()
    );
    assert!(
        hits.iter()
            .all(|hit| hit.series_slug == NEEDLE_SERIES && hit.generation_id == "generation-a"),
        "cross-series leak: {hits:?}"
    );

    // The adversarial control: the same top-k with the predicate applied after
    // ANN cannot recover the eligible rows — decoys crowd them out.
    let postfiltered = index
        .search_postfiltered(NEEDLE_SERIES, &needle(), 10)
        .unwrap();
    assert!(
        postfiltered
            .iter()
            .filter(|hit| hit.series_slug == NEEDLE_SERIES)
            .count()
            < 10,
        "post-filtered search must starve the eligible series: {postfiltered:?}"
    );
}

#[test]
fn vector_store_rejects_mismatched_rows_and_query_widths() {
    let root = tempfile::tempdir().unwrap();
    let identity = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)
        .identity()
        .clone();
    let mut index = VectorIndex::open(root.path(), identity, "generation-a").unwrap();
    let model = index.identity().model().to_string();
    let revision = index.identity().revision().to_string();
    let base = move |chunk_id: i64, vector: Vec<f32>| IndexRow {
        chunk_id,
        series_slug: NEEDLE_SERIES.to_string(),
        checksum: "checksum".to_string(),
        generation_id: "generation-a".to_string(),
        model: model.clone(),
        model_revision: revision.clone(),
        vector,
    };

    assert!(index.append(vec![base(1, vec![0.5; 3])]).is_err());
    let mut infinite = base(2, vec![f32::NAN; EMBEDDING_DIMENSIONS]);
    assert!(index.append(vec![infinite.clone()]).is_err());
    infinite.vector = vec![0.1; EMBEDDING_DIMENSIONS];
    index.append(vec![infinite]).unwrap();

    assert!(
        index.search(NEEDLE_SERIES, &[0.5; 7], 5).is_err(),
        "query width must match the identity width"
    );
    // Snapshot fingerprints are vector-free and complete.
    let fingerprints = index.snapshot_rows(100).unwrap();
    assert_eq!(fingerprints.len(), 1);
    assert_eq!(
        fingerprints[0],
        RowFingerprint {
            chunk_id: 2,
            series_slug: NEEDLE_SERIES.to_string(),
            checksum: "checksum".to_string(),
            generation_id: "generation-a".to_string(),
        }
    );
    assert_eq!(
        index.stored_vector_width().unwrap(),
        Some(EMBEDDING_DIMENSIONS)
    );
}

// ---------------------------------------------------------------------------
// Generation lifecycle
// ---------------------------------------------------------------------------

#[test]
fn build_activate_rebuild_stays_isolated_until_activation() {
    let fixture = fixture();
    import_source(
        &fixture,
        "a.txt",
        "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.",
    );
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);

    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    assert_eq!(drain_generation(&store, "gen-a", &mut provider, 2), 3);
    store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .unwrap();
    let active = store.active_generation().unwrap().expect("gen-a active");
    assert_eq!(active.generation_id, "gen-a");
    assert_eq!(active.status, "active");
    assert!(active.active);
    assert!(store.active_generation_intact().unwrap());

    // Rebuild writes an isolated generation while gen-a keeps serving.
    store
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    let partial = store.pending_chunk_ids("gen-b", 1).unwrap();
    let chunks: Vec<SemanticChunk> = partial
        .iter()
        .map(|chunk_id| SemanticChunk {
            chunk_id: *chunk_id,
            series_slug: series_of_chunk(&store, *chunk_id),
            token_ids: tokens_for(&format!("chunk-{chunk_id}")),
        })
        .collect();
    store.write_batch("gen-b", &mut provider, &chunks).unwrap();
    // During the build, gen-a still serves a pre-filtered sample query.
    {
        let index_root = store.index_root();
        let index = VectorIndex::open(&index_root, provider.identity().clone(), "gen-a").unwrap();
        let hits = index
            .search(
                &fixture.series_slug,
                &provider.embed_query(&tokens_for("chunk-1")).unwrap(),
                3,
            )
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0].chunk_id, 1,
            "the identical probe vector ranks first"
        );
        assert!(
            hits.iter().all(|hit| hit.generation_id == "gen-a"),
            "cross-generation leak: {hits:?}"
        );
    }
    // An incomplete generation cannot activate; gen-a stays active.
    let rejection = store.activate_generation(
        "gen-b",
        &mut provider,
        &SemanticSample {
            text: "probe".into(),
            series_slug: fixture.series_slug.clone(),
            token_ids: tokens_for("chunk-1"),
        },
    );
    assert!(matches!(rejection, Err(SemanticError::ValidationFailed(_))));
    assert_eq!(
        store.active_generation().unwrap().unwrap().generation_id,
        "gen-a"
    );

    drain_generation(&store, "gen-b", &mut provider, 2);
    store
        .activate_generation(
            "gen-b",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .unwrap();
    assert_eq!(
        store.active_generation().unwrap().unwrap().generation_id,
        "gen-b"
    );
    let superseded = store.generation_manifest("gen-a").unwrap().unwrap();
    assert_eq!(superseded.status, "superseded");
    assert!(!superseded.active);
}

#[test]
fn activation_rejects_count_mismatch_and_second_activation() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "Alpha paragraph.\n\nBeta paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);

    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    // Write only one of two chunks.
    let pending = store.pending_chunk_ids("gen-a", 1).unwrap();
    let chunks: Vec<SemanticChunk> = pending
        .iter()
        .map(|chunk_id| SemanticChunk {
            chunk_id: *chunk_id,
            series_slug: series_of_chunk(&store, *chunk_id),
            token_ids: tokens_for(&format!("chunk-{chunk_id}")),
        })
        .collect();
    store.write_batch("gen-a", &mut provider, &chunks).unwrap();
    let error = store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .expect_err("incomplete generation must not activate");
    assert!(
        error.to_string().contains("count"),
        "unexpected error: {error}"
    );
    assert!(store.active_generation().unwrap().is_none());

    // Finish the build and activate; a second activation is rejected.
    drain_generation(&store, "gen-a", &mut provider, 2);
    store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .unwrap();
    let again = store.activate_generation(
        "gen-a",
        &mut provider,
        &SemanticSample {
            text: "probe".into(),
            series_slug: fixture.series_slug.clone(),
            token_ids: tokens_for("chunk-1"),
        },
    );
    assert!(matches!(again, Err(SemanticError::InvalidState(_))));
}

#[test]
fn activation_rejects_checksum_mismatch_after_reimport() {
    let fixture = fixture();
    let path = write_source(&fixture, "story.txt", "Original paragraph one.");
    let rag = RagStore::open(&fixture.config).unwrap();
    rag.import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();
    import_source(&fixture, "other.txt", "Unrelated paragraph stays.");

    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-a", &mut provider, 2);

    // Re-import with changed content: the old chunk rows are replaced, so the
    // generation holds stale checksums against current authoritative rows.
    std::fs::write(&path, "Rewritten paragraph one.").unwrap();
    rag.import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();

    let error = store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .expect_err("stale checksums must reject activation");
    assert!(
        error.to_string().contains("checksum"),
        "unexpected error: {error}"
    );
    assert!(store.active_generation().unwrap().is_none());
}

#[test]
fn activation_rejects_dimension_and_identity_mismatches() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "Solo paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();

    // Build a 4-dimensional generation, then tamper the manifest dimensions so
    // activation runs against a 384-wide expectation.
    let mut narrow = FakeEmbeddingProvider::new(4);
    store.begin_generation("gen-a", narrow.identity()).unwrap();
    drain_generation(&store, "gen-a", &mut narrow, 2);

    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update semantic_generations set dimensions = 385 where generation_id = 'gen-a'",
            [],
        )
        .unwrap();
    drop(connection);

    let error = store
        .activate_generation(
            "gen-a",
            &mut narrow,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .expect_err("dimension mismatch must reject activation");
    assert!(matches!(error, SemanticError::IdentityMismatch { .. }));

    // A provider with a different identity never activates the generation.
    let mut other = FakeEmbeddingProvider::with_model(EMBEDDING_DIMENSIONS, "other-model");
    store.begin_generation("gen-b", other.identity()).unwrap();
    drain_generation(&store, "gen-b", &mut other, 2);
    let mut default = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let error = store
        .activate_generation(
            "gen-b",
            &mut default,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .expect_err("identity mismatch must reject activation");
    assert!(matches!(error, SemanticError::IdentityMismatch { .. }));
    assert!(store.active_generation().unwrap().is_none());
}

#[test]
fn write_batch_rejects_unknown_missing_or_foreign_chunks() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "First paragraph.\n\nSecond paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();

    let unknown = SemanticChunk {
        chunk_id: 999_999,
        series_slug: fixture.series_slug.clone(),
        token_ids: vec![1, 2, 3],
    };
    let error = store
        .write_batch("gen-a", &mut provider, &[unknown])
        .expect_err("unknown chunk ids must be rejected");
    assert!(matches!(error, SemanticError::NotFound(_)));

    let pending = store.pending_chunk_ids("gen-a", 1).unwrap();
    let foreign = SemanticChunk {
        chunk_id: pending[0],
        series_slug: "other-series".to_string(),
        token_ids: vec![1, 2, 3],
    };
    let error = store
        .write_batch("gen-a", &mut provider, &[foreign])
        .expect_err("series mismatch must be rejected");
    assert!(matches!(error, SemanticError::ValidationFailed(_)));

    // Re-writing an already-cursor-covered chunk is rejected.
    let chunks: Vec<SemanticChunk> = pending
        .iter()
        .map(|chunk_id| SemanticChunk {
            chunk_id: *chunk_id,
            series_slug: series_of_chunk(&store, *chunk_id),
            token_ids: tokens_for(&format!("chunk-{chunk_id}")),
        })
        .collect();
    store.write_batch("gen-a", &mut provider, &chunks).unwrap();
    let error = store
        .write_batch("gen-a", &mut provider, &chunks)
        .expect_err("double writes behind the cursor must be rejected");
    assert!(matches!(error, SemanticError::ValidationFailed(_)));
}

#[test]
fn write_batch_enforces_the_provider_batch_limit() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "First paragraph.\n\nSecond paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();

    let over_limit = provider.identity().max_batch_inputs() + 1;
    let chunks: Vec<SemanticChunk> = (0..over_limit)
        .map(|offset| SemanticChunk {
            chunk_id: offset as i64 + 1,
            series_slug: fixture.series_slug.clone(),
            token_ids: tokens_for(&format!("chunk-{}", offset + 1)),
        })
        .collect();
    let error = store
        .write_batch("gen-a", &mut provider, &chunks)
        .expect_err("batches beyond the provider limit must be rejected");
    assert!(
        error.to_string().contains("provider limit"),
        "unexpected error: {error}"
    );
    // Nothing was written and the cursor did not move.
    assert_eq!(
        store
            .generation_manifest("gen-a")
            .unwrap()
            .unwrap()
            .written_count,
        0
    );
}

#[test]
fn garbage_collection_drops_only_terminal_generations() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "Solo paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let sample = SemanticSample {
        text: "probe".into(),
        series_slug: fixture.series_slug.clone(),
        token_ids: tokens_for("chunk-1"),
    };

    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-a", &mut provider, 2);
    store
        .activate_generation("gen-a", &mut provider, &sample)
        .unwrap();

    store
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-b", &mut provider, 2);
    store
        .activate_generation("gen-b", &mut provider, &sample)
        .unwrap();

    store
        .begin_generation("gen-c", provider.identity())
        .unwrap();
    store.cancel_generation("gen-c").unwrap();

    store
        .begin_generation("gen-d", provider.identity())
        .unwrap(); // still building

    let dropped = store.collect_garbage().unwrap();
    let dropped: HashSet<String> = dropped.into_iter().collect();
    assert_eq!(
        dropped,
        HashSet::from(["gen-a".to_string(), "gen-c".to_string()])
    );

    assert!(store.generation_manifest("gen-a").unwrap().is_none());
    assert!(store.generation_manifest("gen-c").unwrap().is_none());
    assert!(
        store.generation_manifest("gen-b").unwrap().is_some(),
        "the active generation is never collected"
    );
    assert!(
        store.generation_manifest("gen-d").unwrap().is_some(),
        "building generations have no terminal state yet and are never collected"
    );
    assert_eq!(
        store.active_generation().unwrap().unwrap().generation_id,
        "gen-b"
    );
    assert!(store.active_generation_intact().unwrap());
}

#[test]
fn losing_the_lance_directory_is_a_rebuild_never_data_loss() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "First paragraph.\n\nSecond paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let sample = SemanticSample {
        text: "probe".into(),
        series_slug: fixture.series_slug.clone(),
        token_ids: tokens_for("chunk-1"),
    };
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-a", &mut provider, 2);
    store
        .activate_generation("gen-a", &mut provider, &sample)
        .unwrap();

    let index_root = store.index_root();
    let index = VectorIndex::open(&index_root, provider.identity().clone(), "gen-a").unwrap();
    let before: HashSet<i64> = index
        .search(
            &fixture.series_slug,
            &provider.embed_query(&sample.token_ids).unwrap(),
            10,
        )
        .unwrap()
        .into_iter()
        .map(|hit| hit.chunk_id)
        .collect();
    drop(index);

    // The complete LanceDB directory disappears (disk loss, accidental rm).
    std::fs::remove_dir_all(store.index_root()).unwrap();
    assert!(!store.active_generation_intact().unwrap());

    // Authoritative rows are untouched: FTS search still works.
    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "paragraph", 10, &[], &[], &[])
        .unwrap();
    assert_eq!(hits.len(), 2);

    // A rebuild produces the same eligible chunk ids and reactivates cleanly.
    store
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-b", &mut provider, 2);
    store
        .activate_generation("gen-b", &mut provider, &sample)
        .unwrap();
    let index_root = store.index_root();
    let index = VectorIndex::open(&index_root, provider.identity().clone(), "gen-b").unwrap();
    let after: HashSet<i64> = index
        .search(
            &fixture.series_slug,
            &provider.embed_query(&sample.token_ids).unwrap(),
            10,
        )
        .unwrap()
        .into_iter()
        .map(|hit| hit.chunk_id)
        .collect();
    assert_eq!(before, after);
    assert!(store.active_generation_intact().unwrap());
}

// ---------------------------------------------------------------------------
// FTS-only degraded mode
// ---------------------------------------------------------------------------

#[test]
fn model_absence_degrades_to_fts_only_without_any_download() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    assert_eq!(store.model_status(), ModelStatus::Missing);

    // Opening the semantic store downloads nothing and creates no model file.
    assert!(!store.model_path().exists());

    // The real ONNX path fails closed with a typed unavailable error.
    let error = store
        .load_embedding_provider(&fixture.root.path().join("libonnxruntime.so"))
        .expect_err("a missing model must fail closed");
    assert!(matches!(error, SemanticError::ModelUnavailable(_)));

    // Import and FTS search keep working throughout.
    import_source(&fixture, "a.txt", "Sense menu note.");
    let hits = RagStore::open(&fixture.config)
        .unwrap()
        .search(&fixture.series_slug, "Sense", 5, &[], &[], &[])
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(store.model_status(), ModelStatus::Missing);
    assert!(!store.model_path().exists());
}

#[test]
fn corrupt_model_file_is_reported_invalid() {
    let fixture = fixture();
    let store = SemanticStore::open(&fixture.config).unwrap();
    std::fs::create_dir_all(store.model_path().parent().unwrap()).unwrap();
    std::fs::write(store.model_path(), b"truncated").unwrap();
    assert!(matches!(store.model_status(), ModelStatus::Invalid(_)));
    let error = store
        .load_embedding_provider(&fixture.root.path().join("libonnxruntime.so"))
        .expect_err("an invalid model must fail closed");
    assert!(matches!(error, SemanticError::ModelUnavailable(_)));
}

// ---------------------------------------------------------------------------
// Optional live test (never part of the default gate)
// ---------------------------------------------------------------------------

/// Runs the real MiniLM model when `HIERONYMUS_SEMANTIC_LIVE=1` and the
/// qualification artifacts are present locally. Ignored by default: the gate
/// suite never downloads anything and never egresses.
#[test]
#[ignore = "requires HIERONYMUS_SEMANTIC_LIVE=1 plus locally present qualification artifacts"]
fn live_onnx_provider_embeds_normalized_384_vectors() {
    if std::env::var("HIERONYMUS_SEMANTIC_LIVE").ok().as_deref() != Some("1") {
        println!("HIERONYMUS_SEMANTIC_LIVE=1 not set; skipping live model run");
        return;
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifacts = manifest
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the repository root")
        .join("qualification/.artifacts/models");
    let runtime = artifacts.join("onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so");
    let model = artifacts.join("paraphrase-multilingual-MiniLM-L12-v2/model.onnx");
    if !runtime.is_file() || !model.is_file() {
        println!("qualification artifacts not present; skipping live model run");
        return;
    }
    let mut provider = OnnxEmbeddingProvider::load(&runtime, &model).unwrap();
    let identity = provider.identity();
    assert_eq!(identity.model(), MODEL_NAME);
    assert_eq!(identity.dimensions(), EMBEDDING_DIMENSIONS);
    let tokens: Vec<u32> = (1010..1042).collect();
    let first = provider.embed_document(&tokens).unwrap();
    let second = provider.embed_document(&tokens).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), EMBEDDING_DIMENSIONS);
    let norm: f32 = first.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "embedding norm {norm}");
    let query = provider.embed_query(&tokens).unwrap();
    assert_eq!(query, first);
}

#[test]
fn active_probe_rejects_empty_and_corrupt_lance_directories() {
    let fixture = fixture();
    import_source(&fixture, "a.txt", "First paragraph.");
    let store = SemanticStore::open(&fixture.config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    drain_generation(&store, "gen-a", &mut provider, 2);
    store
        .activate_generation(
            "gen-a",
            &mut provider,
            &SemanticSample {
                text: "probe".into(),
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for("chunk-1"),
            },
        )
        .unwrap();
    assert!(
        SemanticStore::probe_active_generation(&fixture.config)
            .unwrap()
            .1
    );
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute(
            "update semantic_generations set expected_count = expected_count + 1 where active = 1",
            [],
        )
        .unwrap();
    assert!(
        !SemanticStore::probe_active_generation(&fixture.config)
            .unwrap()
            .1
    );
    connection.execute("update semantic_generations set expected_count = expected_count - 1, model = 'wrong-model' where active = 1", []).unwrap();
    assert!(
        !SemanticStore::probe_active_generation(&fixture.config)
            .unwrap()
            .1
    );
    connection
        .execute(
            "update semantic_generations set model = ?1 where active = 1",
            [provider.identity().model()],
        )
        .unwrap();
    assert!(
        SemanticStore::probe_active_generation(&fixture.config)
            .unwrap()
            .1
    );
    let table = std::fs::read_dir(store.index_root())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "lance"))
        .unwrap();
    std::fs::remove_dir_all(&table).unwrap();
    std::fs::create_dir(&table).unwrap();
    for corrupt in [false, true] {
        if corrupt {
            std::fs::write(table.join("garbage"), b"not a Lance dataset").unwrap();
        }
        assert!(
            !SemanticStore::probe_active_generation(&fixture.config)
                .unwrap()
                .1
        );
        assert!(!store.active_generation_intact().unwrap());
        assert_eq!(
            std::fs::read_dir(&table).unwrap().count(),
            usize::from(corrupt),
            "probing must not create an index"
        );
    }
}

#[test]
fn active_index_probe_rejects_damaged_ann_files() {
    use hieronymus::semantic_index::generation_table_intact;
    let root = tempfile::tempdir().unwrap();
    let identity = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS)
        .identity()
        .clone();
    let rows = adversarial_rows();
    let count = rows.len() as u64;
    let mut index = VectorIndex::open(root.path(), identity.clone(), "generation-a").unwrap();
    index.append(rows).unwrap();
    index.create_ann_index().unwrap();
    drop(index);
    assert!(generation_table_intact(
        root.path(),
        "generation-a",
        &identity,
        count
    ));
    let table = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lance")
        })
        .unwrap();
    let indices = table.join("_indices");
    assert!(
        indices.is_dir(),
        "fixture must contain a physical ANN index"
    );
    std::fs::remove_dir_all(&indices).unwrap();
    // Table schema/count/identity remain readable, but actual ANN search fails.
    let index = VectorIndex::open(root.path(), identity.clone(), "generation-a").unwrap();
    assert_eq!(index.count_rows().unwrap() as u64, count);
    assert!(index.search(NEEDLE_SERIES, &needle(), 1).is_err());
    assert!(
        !generation_table_intact(root.path(), "generation-a", &identity, count),
        "ready requires a usable vector query, not only readable table metadata"
    );
}

//! Durable semantic rebuild jobs: transactional leases, bounded batch claims,
//! cancellation, and crash recovery over real SQLite and real LanceDB. Ported
//! from the qualified harness's `durable-sqlite-job-state`, `crash-recovery`,
//! and `cancel-recovery` criteria onto the library's store patterns.
//!
//! Tests never egress: the fake embedding provider and a deterministic fake
//! tokenizer stand in for the model and the recall-lane tokenizer. A "crash"
//! is simulated at the durable-state level — a worker that panics mid-batch
//! leaves its lease, manifest, and job rows exactly as an aborted process
//! would, because SQLite is the only recovery state.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::registry::Registry;
use hieronymus::semantic_embeddings::{
    EMBEDDING_DIMENSIONS, EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_index::{IndexRow, VectorIndex};
use hieronymus::semantic_jobs::{
    AuthoritativeChunk, ChunkTokenizer, JobOutcome, LeaseClaim, RebuildConfig, RebuildInputs,
    SemanticJobStore, rebuild_job_id,
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

fn paragraph_text(index: i64) -> String {
    format!("Paragraph number {index} for the semantic corpus.")
}

/// Imports `paragraphs` paragraphs (one authoritative chunk each).
fn import_paragraphs(fixture: &Fixture, paragraphs: usize) {
    let source = (1..=paragraphs as i64)
        .map(paragraph_text)
        .collect::<Vec<_>>()
        .join("\n\n");
    let path = fixture.root.path().join("source.txt");
    std::fs::write(&path, source).unwrap();
    RagStore::open(&fixture.config)
        .unwrap()
        .import_file(&fixture.series_slug, &path, &RagImport::new())
        .unwrap();
}

/// Token stream standing in for a tokenizer: deterministic per chunk text,
/// exactly like the Task 7 suite's fixture.
fn tokens_for(text: &str) -> Vec<u32> {
    text.bytes()
        .enumerate()
        .map(|(index, byte)| ((u32::from(byte) * 31 + index as u32) % 30_000) + 1)
        .collect()
}

fn sample_for(fixture: &Fixture) -> SemanticSample {
    SemanticSample {
        series_slug: fixture.series_slug.clone(),
        token_ids: tokens_for(&paragraph_text(1)),
    }
}

fn generation_status(fixture: &Fixture, generation: &str) -> String {
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .query_row(
            "select status from semantic_generations where generation_id = ?1",
            [generation],
            |row| row.get(0),
        )
        .unwrap()
}

fn active_generation(database_path: &std::path::Path) -> Option<String> {
    let connection = rusqlite::Connection::open(database_path).unwrap();
    connection
        .query_row(
            "select generation_id from semantic_generations where active = 1",
            [],
            |row| row.get(0),
        )
        .ok()
}

/// Simulates crash residue or out-of-band state by editing the durable rows
/// directly (SQLite is the only recovery state, so this is exactly the state a
/// dead process leaves behind).
fn sql_update(fixture: &Fixture, statement: &str) {
    let connection = rusqlite::Connection::open(fixture.config.database_path()).unwrap();
    connection
        .execute_batch(&format!("pragma busy_timeout = 5000; {statement}"))
        .unwrap();
}

fn fts_hits(config: &HieronymusConfig, series_slug: &str, query: &str) -> usize {
    RagStore::open(config)
        .unwrap()
        .search(series_slug, query, 10, &[], &[], &[])
        .unwrap()
        .len()
}

/// Vector-level search against a specific generation's table (the data-plane
/// search is the serving path; recall integration is a later slice).
fn semantic_hits(
    config: &HieronymusConfig,
    series_slug: &str,
    identity: &EmbeddingIdentity,
    generation: &str,
) -> usize {
    let store = SemanticStore::open(config).unwrap();
    let index = VectorIndex::open(&store.index_root(), identity.clone(), generation).unwrap();
    let mut provider = FakeEmbeddingProvider::new(identity.dimensions());
    let query = provider
        .embed_query(&tokens_for(&paragraph_text(1)))
        .unwrap();
    index.search(series_slug, &query, 10).unwrap().len()
}

fn index_row_count(
    config: &HieronymusConfig,
    identity: &EmbeddingIdentity,
    generation: &str,
) -> usize {
    let store = SemanticStore::open(config).unwrap();
    let index = VectorIndex::open(&store.index_root(), identity.clone(), generation).unwrap();
    index.count_rows().unwrap()
}

/// Builds and activates `generation` through the Task 7 APIs directly.
fn activate_baseline(fixture: &Fixture, generation: &str, provider: &mut FakeEmbeddingProvider) {
    let store = SemanticStore::open(&fixture.config).unwrap();
    store
        .begin_generation(generation, provider.identity())
        .unwrap();
    loop {
        let pending = store.pending_chunk_ids(generation, 2).unwrap();
        if pending.is_empty() {
            break;
        }
        let chunks: Vec<SemanticChunk> = pending
            .iter()
            .map(|chunk_id| SemanticChunk {
                chunk_id: *chunk_id,
                series_slug: fixture.series_slug.clone(),
                token_ids: tokens_for(&paragraph_text(*chunk_id)),
            })
            .collect();
        store.write_batch(generation, provider, &chunks).unwrap();
        if pending.len() < 2 {
            break;
        }
    }
    store
        .activate_generation(generation, provider, &sample_for(fixture))
        .unwrap();
}

// ---------------------------------------------------------------------------
// Fake tokenizer with crash / failure / observation hooks
// ---------------------------------------------------------------------------

type Hook = Arc<dyn Fn(usize) + Send + Sync>;

#[derive(Default)]
struct FakeTokenizer {
    calls: usize,
    hook: Option<Hook>,
    /// Simulated crash: panics on the nth tokenize call, exactly like an
    /// aborted process — no cleanup code runs afterwards.
    panic_on_call: Option<usize>,
    /// Transient failures on the given call numbers (the bounded-retry path).
    fail_calls: Vec<usize>,
}

impl FakeTokenizer {
    fn new() -> Self {
        Self::default()
    }

    fn panicking_on(call: usize) -> Self {
        Self {
            panic_on_call: Some(call),
            ..Self::default()
        }
    }
}

impl ChunkTokenizer for FakeTokenizer {
    fn tokenize(&mut self, chunk: &AuthoritativeChunk) -> Result<Vec<u32>, SemanticError> {
        self.calls += 1;
        let call = self.calls;
        if let Some(hook) = &self.hook {
            hook(call);
        }
        if self.panic_on_call == Some(call) {
            panic!("simulated worker crash during embedding");
        }
        if self.fail_calls.contains(&call) {
            return Err(SemanticError::InvalidEmbedding(format!(
                "transient tokenizer failure on call {call}"
            )));
        }
        Ok(tokens_for(&chunk.text))
    }
}

/// Runs one rebuild job with a fresh (identity-equal) provider.
fn run_job(
    store: &SemanticJobStore,
    job_id: &str,
    sample: &SemanticSample,
    tokenizer: &mut FakeTokenizer,
    config: &RebuildConfig,
) -> Result<JobOutcome, SemanticError> {
    let mut provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    store.run_rebuild(
        job_id,
        RebuildInputs {
            provider: &mut provider,
            tokenizer,
            sample: sample.clone(),
        },
        config,
    )
}

/// Repeats a run until the dead worker's lease expires (the harness's
/// lease-poll loop), returning the first non-Busy outcome.
fn run_until_claimable(
    store: &SemanticJobStore,
    job_id: &str,
    sample: &SemanticSample,
    mut tokenizer: FakeTokenizer,
    config: &RebuildConfig,
) -> JobOutcome {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match run_job(store, job_id, sample, &mut tokenizer, config) {
            Ok(JobOutcome::Busy { .. }) => {
                assert!(Instant::now() < deadline, "the lease never expired");
                std::thread::sleep(Duration::from_millis(25));
            }
            other => return other.unwrap(),
        }
    }
}

fn batch_config(ttl: Duration) -> RebuildConfig {
    RebuildConfig {
        batch_size: 2,
        lease_ttl: ttl,
        max_batch_attempts: 3,
    }
}

// ---------------------------------------------------------------------------
// Job records
// ---------------------------------------------------------------------------

#[test]
fn job_record_captures_identity_counts_and_queue_state() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-a", provider.identity())
        .unwrap();

    let job = store.enqueue_rebuild("gen-a", provider.identity()).unwrap();
    assert_eq!(job.job_id, rebuild_job_id("gen-a"));
    assert_eq!(job.generation_id, "gen-a");
    assert_eq!(job.status, "queued");
    assert_eq!(job.identity.provider, "fake");
    assert_eq!(job.identity.model, "fake-model");
    assert_eq!(job.identity.dimensions, EMBEDDING_DIMENSIONS);
    assert_eq!(job.total_chunks, 5);
    assert_eq!(job.cursor_chunk_id, 0);
    assert_eq!(job.completed_chunks, 0);
    assert_eq!(job.completed_batches, 0);
    assert_eq!(job.attempts, 0);
    assert!(!job.cancel_requested);
    assert_eq!(job.last_error, None);
    assert_eq!(job.lease_owner, None);

    let stored = store.job(&rebuild_job_id("gen-a")).unwrap().unwrap();
    assert_eq!(stored, job);
    assert!(store.job("rebuild:missing").unwrap().is_none());

    // One job per generation: a duplicate enqueue is rejected.
    let duplicate = store
        .enqueue_rebuild("gen-a", provider.identity())
        .unwrap_err();
    assert!(
        matches!(duplicate, SemanticError::InvalidState(_)),
        "{duplicate}"
    );
    // Unknown generation: rejected before any row is written.
    let unknown = store
        .enqueue_rebuild("gen-z", provider.identity())
        .unwrap_err();
    assert!(matches!(unknown, SemanticError::NotFound(_)), "{unknown}");
    // A different model identity never drives this generation's rebuild.
    let foreign = FakeEmbeddingProvider::with_model(EMBEDDING_DIMENSIONS, "other-model");
    let mismatched = store
        .enqueue_rebuild("gen-a", foreign.identity())
        .unwrap_err();
    assert!(
        matches!(mismatched, SemanticError::IdentityMismatch { .. }),
        "identity mismatch must be rejected, got {mismatched}"
    );
}

#[test]
fn enqueue_snapshots_manifest_progress_independent_of_the_cursor() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    // A partially built generation whose written count and cursor disagree
    // (authoritative rows vanished between receipts). The enqueue snapshots
    // the manifest verbatim; judging that state belongs to the runner's
    // takeover verification.
    sql_update(
        &fixture,
        "update semantic_generations set written_count = 2, last_chunk_id = 4
         where generation_id = 'gen-a';",
    );
    let job = store.enqueue_rebuild("gen-a", provider.identity()).unwrap();
    assert_eq!(job.cursor_chunk_id, 4);
    assert_eq!(job.completed_chunks, 2);
    assert_eq!(job.total_chunks, 5);
}

// ---------------------------------------------------------------------------
// Transactional lease
// ---------------------------------------------------------------------------

#[test]
fn lease_claim_is_exclusive_until_expiry() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-a", provider.identity()).unwrap();
    let job_id = rebuild_job_id("gen-a");

    // First claimant wins.
    let claimed = store
        .claim_lease(&job_id, "worker-a", Duration::from_millis(150))
        .unwrap();
    let LeaseClaim::Claimed(job) = claimed else {
        panic!("the first claimant must win the lease");
    };
    assert_eq!(job.generation_id, "gen-a");
    assert!(!job.cancel_requested);

    // A second, distinct claimant is held off while the lease is live.
    let held = store
        .claim_lease(&job_id, "worker-b", Duration::from_millis(150))
        .unwrap();
    assert!(
        matches!(held, LeaseClaim::Held),
        "a live foreign lease must be held"
    );
    let record = store.job(&job_id).unwrap().unwrap();
    assert_eq!(record.status, "running");
    assert_eq!(record.lease_owner.as_deref(), Some("worker-a"));
    assert!(record.lease_expires_unix_ms.is_some());

    // The same owner may renew its own lease even while it is live.
    let renewed = store
        .claim_lease(&job_id, "worker-a", Duration::from_millis(150))
        .unwrap();
    assert!(matches!(renewed, LeaseClaim::Claimed(_)));

    // Once the foreign lease expires, the waiting claimant takes over.
    std::thread::sleep(Duration::from_millis(200));
    let takeover = store
        .claim_lease(&job_id, "worker-b", Duration::from_millis(150))
        .unwrap();
    let LeaseClaim::Claimed(job) = takeover else {
        panic!("an expired lease must be reclaimable");
    };
    assert_eq!(job.generation_id, "gen-a");
    let record = store.job(&job_id).unwrap().unwrap();
    assert_eq!(record.lease_owner.as_deref(), Some("worker-b"));
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

#[test]
fn rebuild_completes_activates_once_and_supersedes_the_prior_generation() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let mut baseline = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    activate_baseline(&fixture, "gen-a", &mut baseline);
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
    assert_eq!(
        fts_hits(&fixture.config, &fixture.series_slug, "Paragraph"),
        5
    );

    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();

    let config = batch_config(Duration::from_secs(60));
    let outcome = run_job(
        &store,
        &rebuild_job_id("gen-b"),
        &sample_for(&fixture),
        &mut FakeTokenizer::new(),
        &config,
    )
    .unwrap();
    match outcome {
        JobOutcome::Completed { generation_id } => assert_eq!(generation_id, "gen-b"),
        other => panic!("expected completion, got {other:?}"),
    }

    // Activation happened exactly once, through Task 7's checks.
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-b")
    );
    assert_eq!(generation_status(&fixture, "gen-a"), "superseded");
    assert_eq!(
        index_row_count(&fixture.config, provider.identity(), "gen-b"),
        5,
        "exactly one vector per authoritative chunk"
    );

    let job = store.job(&rebuild_job_id("gen-b")).unwrap().unwrap();
    assert_eq!(job.status, "completed");
    assert_eq!(job.completed_chunks, 5);
    assert_eq!(job.total_chunks, 5);
    assert_eq!(job.completed_batches, 3, "5 chunks in bounded batches of 2");
    assert_eq!(job.attempts, 0);
    assert_eq!(job.lease_owner, None);
    assert_eq!(job.cursor_chunk_id, 5);

    // A completed job can neither be claimed nor rerun.
    let claim = store.claim_lease(&rebuild_job_id("gen-b"), "worker-c", Duration::from_secs(1));
    assert!(
        matches!(claim, Err(SemanticError::InvalidState(_))),
        "{claim:?}"
    );
    let rerun = run_job(
        &store,
        &rebuild_job_id("gen-b"),
        &sample_for(&fixture),
        &mut FakeTokenizer::new(),
        &config,
    );
    assert!(
        matches!(rerun, Err(SemanticError::InvalidState(_))),
        "{rerun:?}"
    );

    // Task 7's GC rules are unchanged: the superseded generation is collectable.
    let collected = SemanticStore::open(&fixture.config)
        .unwrap()
        .collect_garbage()
        .unwrap();
    assert_eq!(collected, vec!["gen-a".to_string()]);
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-b")
    );
}

// ---------------------------------------------------------------------------
// Crash recovery
// ---------------------------------------------------------------------------

#[test]
fn crash_mid_batch_recovers_and_completes_with_the_prior_generation_serving() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let mut baseline = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    activate_baseline(&fixture, "gen-a", &mut baseline);

    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();
    let job_id = rebuild_job_id("gen-b");
    let sample = sample_for(&fixture);
    let database_path = fixture.config.database_path();

    // Worker A crashes mid-batch 2 (after batch 1's receipt is durable).
    let crashing = batch_config(Duration::from_millis(200));
    let crash = catch_unwind(AssertUnwindSafe(|| {
        run_job(
            &store,
            &job_id,
            &sample,
            &mut FakeTokenizer::panicking_on(3),
            &crashing,
        )
    }));
    assert!(crash.is_err(), "the simulated crash must panic the worker");

    // The crash left durable state exactly as an aborted process would: the
    // lease is still held, the manifest counted only batch 1, and the job
    // cursor mirrors it.
    assert_eq!(generation_status(&fixture, "gen-b"), "building");
    let after_crash = store.job(&job_id).unwrap().unwrap();
    assert_eq!(after_crash.status, "running");
    assert_eq!(after_crash.completed_chunks, 2);
    assert!(after_crash.lease_owner.is_some());
    assert!(after_crash.lease_expires_unix_ms.is_some());
    // The prior active generation stayed intact and kept serving throughout.
    assert_eq!(active_generation(&database_path).as_deref(), Some("gen-a"));
    assert_eq!(
        fts_hits(&fixture.config, &fixture.series_slug, "Paragraph"),
        5
    );
    assert_eq!(
        semantic_hits(
            &fixture.config,
            &fixture.series_slug,
            provider.identity(),
            "gen-a"
        ),
        5
    );

    // The dead worker's lease is reclaimable after expiry; the resumed worker
    // continues from the manifest cursor without duplicating rows. Mid-resume,
    // the prior generation is still the active one and keeps serving.
    let probe_config = fixture.config.clone();
    let probe_series = fixture.series_slug.clone();
    let probe_identity = provider.identity().clone();
    let observed_active: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let observed_writer = Arc::clone(&observed_active);
    let mut observing_tokenizer = FakeTokenizer::new();
    observing_tokenizer.hook = Some(Arc::new(move |_call| {
        let mut slot = observed_writer.lock().unwrap();
        if slot.is_none() {
            *slot = active_generation(&probe_config.database_path());
            assert_eq!(fts_hits(&probe_config, &probe_series, "Paragraph"), 5);
            assert_eq!(
                semantic_hits(&probe_config, &probe_series, &probe_identity, "gen-a"),
                5
            );
        }
    }));
    let resume = run_until_claimable(&store, &job_id, &sample, observing_tokenizer, &crashing);
    match resume {
        JobOutcome::Completed { generation_id } => assert_eq!(generation_id, "gen-b"),
        other => panic!("expected the resumed job to complete, got {other:?}"),
    }
    let active_during_resume = observed_active.lock().unwrap().clone().unwrap();
    assert_eq!(
        active_during_resume, "gen-a",
        "gen-a served during the entire resume"
    );

    // Exactly-once activation, exactly-once rows.
    assert_eq!(active_generation(&database_path).as_deref(), Some("gen-b"));
    assert_eq!(generation_status(&fixture, "gen-a"), "superseded");
    assert_eq!(
        index_row_count(&fixture.config, provider.identity(), "gen-b"),
        5
    );
    let job = store.job(&job_id).unwrap().unwrap();
    assert_eq!(job.status, "completed");
    assert_eq!(job.completed_chunks, 5);
    assert_eq!(
        job.completed_batches, 3,
        "batch 1 from the dead worker plus two from the resumed worker"
    );
}

#[test]
fn orphaned_candidate_rows_fail_the_job_without_duplicating_state() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let mut baseline = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    activate_baseline(&fixture, "gen-a", &mut baseline);

    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let semantic = SemanticStore::open(&fixture.config).unwrap();
    semantic
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();

    // Simulate a crash between the LanceDB append and its receipt commit: the
    // row is physically present but the manifest counted none of it.
    let mut index =
        VectorIndex::open(&semantic.index_root(), provider.identity().clone(), "gen-b").unwrap();
    let checksum = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(paragraph_text(1).as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    };
    let orphan = IndexRow {
        chunk_id: 1,
        series_slug: fixture.series_slug.clone(),
        checksum,
        generation_id: "gen-b".to_string(),
        model: provider.identity().model().to_string(),
        model_revision: provider.identity().revision().to_string(),
        vector: vec![0.5; EMBEDDING_DIMENSIONS],
    };
    index.append(vec![orphan]).unwrap();
    drop(index);

    let config = batch_config(Duration::from_secs(60));
    let outcome = run_job(
        &store,
        &rebuild_job_id("gen-b"),
        &sample_for(&fixture),
        &mut FakeTokenizer::new(),
        &config,
    )
    .unwrap();
    match outcome {
        JobOutcome::Failed {
            generation_id,
            error,
        } => {
            assert_eq!(generation_id, "gen-b");
            assert!(error.contains("rows"), "unexpected error: {error}");
        }
        other => panic!("orphaned rows must fail the job, got {other:?}"),
    }

    // The job and the candidate generation are failed-and-GC-able; the prior
    // active generation was never touched.
    let job = store.job(&rebuild_job_id("gen-b")).unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.last_error.is_some());
    assert_eq!(generation_status(&fixture, "gen-b"), "failed");
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
    let collected = semantic.collect_garbage().unwrap();
    assert_eq!(collected, vec!["gen-b".to_string()]);
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

#[test]
fn cancel_mid_batch_stops_at_the_boundary_and_never_activates() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let mut baseline = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    activate_baseline(&fixture, "gen-a", &mut baseline);

    let store = Arc::new(SemanticJobStore::open(&fixture.config).unwrap());
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();
    let job_id = rebuild_job_id("gen-b");
    let sample = sample_for(&fixture);

    // The cancellation flag is requested from inside batch 2 (mid-batch). The
    // runner finishes that bounded batch, then stops at the next boundary.
    let cancel_store = Arc::clone(&store);
    let cancel_job = job_id.clone();
    let requested = Arc::new(AtomicBool::new(false));
    let requested_writer = Arc::clone(&requested);
    let hook: Hook = Arc::new(move |call| {
        // Call 3 is the first chunk of batch 2 (batch 1 covered calls 1-2).
        if call >= 3 && !requested_writer.swap(true, Ordering::SeqCst) {
            cancel_store.request_cancel(&cancel_job).unwrap();
        }
    });
    let mut tokenizer = FakeTokenizer::new();
    tokenizer.hook = Some(hook);

    // FTS keeps serving while the cancellation lands.
    assert_eq!(
        fts_hits(&fixture.config, &fixture.series_slug, "Paragraph"),
        5
    );

    let config = batch_config(Duration::from_secs(60));
    let outcome = run_job(&store, &job_id, &sample, &mut tokenizer, &config).unwrap();
    match outcome {
        JobOutcome::Cancelled { generation_id } => assert_eq!(generation_id, "gen-b"),
        other => panic!("expected cancellation, got {other:?}"),
    }

    // The in-flight batch finished; the third never started.
    let job = store.job(&job_id).unwrap().unwrap();
    assert_eq!(job.status, "cancelled");
    assert!(job.cancel_requested);
    assert_eq!(
        job.completed_chunks, 4,
        "batches 1 and 2 committed, batch 3 never ran"
    );
    assert_eq!(job.lease_owner, None);

    // The candidate generation was never activated and Task 7's GC collects it.
    assert_eq!(generation_status(&fixture, "gen-b"), "cancelled");
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
    assert_eq!(
        semantic_hits(
            &fixture.config,
            &fixture.series_slug,
            provider.identity(),
            "gen-a"
        ),
        5
    );
    assert_eq!(
        fts_hits(&fixture.config, &fixture.series_slug, "Paragraph"),
        5
    );
    let collected = SemanticStore::open(&fixture.config)
        .unwrap()
        .collect_garbage()
        .unwrap();
    assert_eq!(collected, vec!["gen-b".to_string()]);
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
}

// ---------------------------------------------------------------------------
// Bounded retry
// ---------------------------------------------------------------------------

#[test]
fn batch_failures_retry_within_bounds_and_record_the_last_error() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let semantic = SemanticStore::open(&fixture.config).unwrap();
    let sample = sample_for(&fixture);
    let config = batch_config(Duration::from_secs(60));

    // One transient failure retries and still completes.
    semantic
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-a", provider.identity()).unwrap();
    let mut flaky = FakeTokenizer::new();
    flaky.fail_calls = vec![1];
    let outcome = run_job(
        &store,
        &rebuild_job_id("gen-a"),
        &sample,
        &mut flaky,
        &config,
    )
    .unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let job = store.job(&rebuild_job_id("gen-a")).unwrap().unwrap();
    assert_eq!(job.status, "completed");
    assert_eq!(job.failed_batches, 1);
    assert_eq!(
        job.attempts, 0,
        "a successful receipt resets the retry counter"
    );
    assert!(
        job.last_error
            .as_deref()
            .unwrap_or("")
            .contains("transient")
    );
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );

    // Failures beyond the bound fail the job and the candidate generation.
    semantic
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();
    let mut broken = FakeTokenizer::new();
    broken.fail_calls = (1..=100).collect();
    let mut bounded = config.clone();
    bounded.max_batch_attempts = 2;
    let outcome = run_job(
        &store,
        &rebuild_job_id("gen-b"),
        &sample,
        &mut broken,
        &bounded,
    )
    .unwrap();
    match outcome {
        JobOutcome::Failed {
            generation_id,
            error,
        } => {
            assert_eq!(generation_id, "gen-b");
            assert!(error.contains("transient"), "unexpected error: {error}");
        }
        other => panic!("expected failure after the attempt bound, got {other:?}"),
    }
    let job = store.job(&rebuild_job_id("gen-b")).unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert_eq!(job.attempts, 2, "attempts stop at the configured bound");
    assert_eq!(job.failed_batches, 2);
    assert_eq!(generation_status(&fixture, "gen-b"), "failed");
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-a")
    );
    let collected = semantic.collect_garbage().unwrap();
    assert_eq!(collected, vec!["gen-b".to_string()]);
}

// ---------------------------------------------------------------------------
// Reconciliation
// ---------------------------------------------------------------------------

#[test]
fn reconcile_recovers_stale_job_and_generation_state() {
    let fixture = fixture();
    import_paragraphs(&fixture, 5);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let semantic = SemanticStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    let sample = sample_for(&fixture);
    let config = batch_config(Duration::from_secs(60));

    // 1. A dead worker's expired lease is reclaimable and its stale job cursor
    //    resyncs from the manifest.
    semantic
        .begin_generation("gen-b", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-b", provider.identity()).unwrap();
    // Batch 1 commits, then the worker crashes before the next renewal.
    let crashing = batch_config(Duration::from_millis(50));
    let _ = catch_unwind(AssertUnwindSafe(|| {
        run_job(
            &store,
            &rebuild_job_id("gen-b"),
            &sample,
            &mut FakeTokenizer::panicking_on(3),
            &crashing,
        )
    }));
    // Simulate the pre-receipt crash window: the job cursor lags the manifest.
    sql_update(
        &fixture,
        "update semantic_jobs set cursor_chunk_id = 0, completed_chunks = 0
         where job_id = 'rebuild:gen-b';",
    );
    std::thread::sleep(Duration::from_millis(120));
    let report = store.reconcile().unwrap();
    assert!(
        report.reclaimable_jobs.contains(&rebuild_job_id("gen-b")),
        "unexpected report: {report:?}"
    );
    let job = store.job(&rebuild_job_id("gen-b")).unwrap().unwrap();
    assert_eq!(
        job.status, "running",
        "resumable jobs are left for the next claim"
    );
    assert_eq!(job.cursor_chunk_id, 2, "cursor resynced from the manifest");
    assert_eq!(job.completed_chunks, 2);
    assert_eq!(generation_status(&fixture, "gen-b"), "building");

    // The resumed job completes cleanly afterwards.
    let outcome = run_until_claimable(
        &store,
        &rebuild_job_id("gen-b"),
        &sample,
        FakeTokenizer::new(),
        &config,
    );
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        active_generation(&fixture.config.database_path()).as_deref(),
        Some("gen-b")
    );

    // 2. A running job whose generation already activated (crash between the
    //    activation and job-completion transactions) completes idempotently.
    sql_update(
        &fixture,
        "update semantic_jobs set status = 'running', lease_owner = 'ghost',
                lease_expires_unix_ms = 1
         where job_id = 'rebuild:gen-b';",
    );
    let report = store.reconcile().unwrap();
    assert!(
        report.completed_jobs.contains(&rebuild_job_id("gen-b")),
        "{report:?}"
    );
    assert_eq!(
        store.job(&rebuild_job_id("gen-b")).unwrap().unwrap().status,
        "completed"
    );

    // 3. A building generation whose job is already terminal is marked
    //    failed-and-GC-able (Task 7's declared-but-never-set failed status).
    semantic
        .begin_generation("gen-c", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-c", provider.identity()).unwrap();
    sql_update(
        &fixture,
        "update semantic_jobs set status = 'failed' where job_id = 'rebuild:gen-c';",
    );
    let report = store.reconcile().unwrap();
    assert!(
        report.failed_generations.contains(&"gen-c".to_string()),
        "{report:?}"
    );
    assert_eq!(generation_status(&fixture, "gen-c"), "failed");
    let collected = semantic.collect_garbage().unwrap();
    assert!(collected.contains(&"gen-c".to_string()), "{collected:?}");

    // 4. An in-process Task 7 build without any job row is never touched.
    semantic
        .begin_generation("gen-d", provider.identity())
        .unwrap();
    store.reconcile().unwrap();
    assert_eq!(generation_status(&fixture, "gen-d"), "building");
}

#[test]
fn run_rebuild_rejects_missing_and_foreign_identity_jobs() {
    let fixture = fixture();
    import_paragraphs(&fixture, 2);
    let store = SemanticJobStore::open(&fixture.config).unwrap();
    let provider = FakeEmbeddingProvider::new(EMBEDDING_DIMENSIONS);
    SemanticStore::open(&fixture.config)
        .unwrap()
        .begin_generation("gen-a", provider.identity())
        .unwrap();
    store.enqueue_rebuild("gen-a", provider.identity()).unwrap();
    let sample = sample_for(&fixture);
    let config = batch_config(Duration::from_secs(60));

    let missing = run_job(
        &store,
        "rebuild:missing",
        &sample,
        &mut FakeTokenizer::new(),
        &config,
    );
    assert!(
        matches!(missing, Err(SemanticError::NotFound(_))),
        "{missing:?}"
    );

    let mut foreign = FakeEmbeddingProvider::with_model(EMBEDDING_DIMENSIONS, "other-model");
    let mismatch = store.run_rebuild(
        &rebuild_job_id("gen-a"),
        RebuildInputs {
            provider: &mut foreign,
            tokenizer: &mut FakeTokenizer::new(),
            sample,
        },
        &config,
    );
    assert!(
        matches!(mismatch, Err(SemanticError::IdentityMismatch { .. })),
        "{mismatch:?}"
    );
    // The rejected run consumed nothing: the job is still queued.
    assert_eq!(
        store.job(&rebuild_job_id("gen-a")).unwrap().unwrap().status,
        "queued"
    );
}

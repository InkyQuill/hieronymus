//! Task C4 (review finding A4): the semantic index cannot silently fall
//! behind the authoritative corpus.
//!
//! Three defects, one root cause. The import committed its authoritative
//! transaction and only THEN queued the semantic rebuild out of band, and the
//! only question anyone asked about coverage was "does the chunk count match?":
//!
//! 1. **the missed commit/enqueue window.** A crash — or a plain enqueue
//!    failure — between the commit and the queueing left the new chunks
//!    unindexed while an older generation kept the active slot, so the
//!    controller reported `Ready` over text no query could reach, and only
//!    another import would ever fix it.
//! 2. **equal-count dedup.** A queued rebuild whose frozen `expected_count`
//!    equalled the current count was treated as already covering the corpus.
//!    Replacing a document with one of the same length changes the text of
//!    every chunk it owns and leaves the count alone, so the stale candidate
//!    was allowed to activate.
//! 3. **stale startup.** Reconciliation queued missing work only when NO
//!    active generation existed, and it reconciled job records rather than
//!    comparing the corpus against what the active generation covers.
//!
//! The fix is a monotonic corpus revision written inside the authoritative
//! transaction, a durable `semantic_work_intent` written beside it, and
//! generations that record the revision they cover. Everything here runs
//! offline against the fake embedding provider and real SQLite.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hiero::application::Application;
use hiero::daemon::semantic_worker::{
    ArmedPair, EMPTY_CORPUS_JOB_ID, RequiredSemanticState, SemanticArm, SemanticController,
};
use hiero::daemon::workers::WorkerGroup;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::semantic_embeddings::{
    EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_recall::{QueueOutcome, SemanticLane, queue_semantic_rebuild};
use hieronymus::semantic_store::{SemanticSample, SemanticStore, UNKNOWN_CORPUS_REVISION};
use hieronymus::semantic_tokenizer::ModelTokenizer;
use serde_json::{Value, json};

use common::wait_until;

const DIMENSIONS: usize = 384;

// ---------------------------------------------------------------------------
// The predicate itself
// ---------------------------------------------------------------------------

/// The headline regression: an equal chunk count is not coverage. Two corpus
/// revisions differ, therefore the queued build is not the requested build,
/// no matter how many chunks each side happens to hold.
#[test]
fn equal_chunk_count_does_not_equate_different_corpus_revisions() {
    let identity = FakeEmbeddingProvider::new(DIMENSIONS).identity().clone();
    assert!(!hieronymus::semantic_recall::same_build_request(
        9, 8, &identity, &identity
    ));
    assert!(hieronymus::semantic_recall::same_build_request(
        9, 9, &identity, &identity
    ));
}

/// The full truth table: revision match/mismatch crossed with identity
/// match/mismatch. Only the both-match corner is a dedup hit.
#[test]
fn same_build_request_is_revision_and_identity() {
    use hieronymus::semantic_recall::same_build_request;
    let identity = FakeEmbeddingProvider::new(DIMENSIONS).identity().clone();
    let other = FakeEmbeddingProvider::new(256).identity().clone();
    assert_ne!(identity, other);

    for (current, queued, left, right, expected) in [
        (4, 4, &identity, &identity, true),
        (4, 3, &identity, &identity, false),
        (4, 4, &identity, &other, false),
        (4, 3, &identity, &other, false),
        // The pre-C4 sentinel is behind every real revision, 0 included.
        (0, UNKNOWN_CORPUS_REVISION, &identity, &identity, false),
    ] {
        assert_eq!(
            same_build_request(current, queued, left, right),
            expected,
            "same_build_request({current}, {queued}, ..)"
        );
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A fake provider that, on every document embedding, PROVES no SQLite write
/// transaction is open anywhere in this process: it opens its own connection
/// with a zero busy timeout and takes `BEGIN IMMEDIATE`. A write transaction
/// held across inference — the thing the store's discipline forbids — would
/// make that fail instantly with `SQLITE_BUSY`.
struct LockProbingProvider {
    inner: FakeEmbeddingProvider,
    database: std::path::PathBuf,
    per_embed_delay: Duration,
    lock_contended: Arc<AtomicBool>,
    embeds: Arc<AtomicUsize>,
}

impl LockProbingProvider {
    fn probe(&self) {
        let Ok(mut connection) = rusqlite::Connection::open(&self.database) else {
            self.lock_contended.store(true, Ordering::SeqCst);
            return;
        };
        // Zero timeout: no waiting, no masking. Either the write lock is free
        // right now or it is not.
        let _ = connection.pragma_update(None, "busy_timeout", 0);
        match connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
            Ok(transaction) => {
                let _ = transaction.rollback();
            }
            Err(_) => self.lock_contended.store(true, Ordering::SeqCst),
        }
    }
}

impl EmbeddingProvider for LockProbingProvider {
    fn identity(&self) -> &EmbeddingIdentity {
        self.inner.identity()
    }

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embeds.fetch_add(1, Ordering::SeqCst);
        self.probe();
        if !self.per_embed_delay.is_zero() {
            std::thread::sleep(self.per_embed_delay);
        }
        self.inner.embed_document(token_ids)
    }

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.inner.embed_query(token_ids)
    }
}

/// The test arm: the S1-pinned tokenizer fixture plus a lock-probing fake
/// provider, registered for exactly this test's data root.
struct TestArm {
    database: std::path::PathBuf,
    per_embed_delay: Duration,
    lock_contended: Arc<AtomicBool>,
    embeds: Arc<AtomicUsize>,
}

impl TestArm {
    fn new(config: &HieronymusConfig) -> Self {
        Self {
            database: config.database_path(),
            per_embed_delay: Duration::ZERO,
            lock_contended: Arc::new(AtomicBool::new(false)),
            embeds: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn identity_of() -> EmbeddingIdentity {
        FakeEmbeddingProvider::new(DIMENSIONS).identity().clone()
    }
}

impl SemanticArm for TestArm {
    fn identity(&self) -> EmbeddingIdentity {
        Self::identity_of()
    }

    fn precheck(&self, _config: &HieronymusConfig) -> Result<(), String> {
        Ok(())
    }

    fn arm(&self, _config: &HieronymusConfig) -> Result<ArmedPair, String> {
        let tokenizer = model_tokenizer();
        Ok(ArmedPair {
            provider: Box::new(LockProbingProvider {
                inner: FakeEmbeddingProvider::new(DIMENSIONS),
                database: self.database.clone(),
                per_embed_delay: self.per_embed_delay,
                lock_contended: Arc::clone(&self.lock_contended),
                embeds: Arc::clone(&self.embeds),
            }),
            tokenizer: Box::new(tokenizer),
        })
    }
}

fn model_tokenizer() -> ModelTokenizer {
    ModelTokenizer::from_bytes(include_bytes!(
        "../../hieronymus/tests/fixtures/minilm-tokenizer.json"
    ))
    .unwrap()
}

fn start_controller(
    config: &HieronymusConfig,
    arm: Arc<dyn SemanticArm>,
) -> (WorkerGroup, SemanticController) {
    let stop = Arc::new(AtomicBool::new(false));
    let mut group = WorkerGroup::new(Arc::clone(&stop));
    let controller =
        SemanticController::start_with(config.clone(), &mut group, arm, Box::new(|_| Ok(())))
            .unwrap();
    (group, controller)
}

fn wait_ready(controller: &SemanticController) -> bool {
    wait_until(
        || controller.state() == RequiredSemanticState::Ready,
        Duration::from_secs(30),
    )
}

/// A data root with one series and no imports yet.
fn seeded_root() -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    hieronymus::registry::Registry::open(&config)
        .unwrap()
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    (root, config)
}

/// Import `text` under `name`, exactly as a tool call would — and NOTHING
/// else. No controller notification: this is the commit side of the window a
/// crash used to open.
fn import(config: &HieronymusConfig, root: &Path, name: &str, text: &str) -> usize {
    let source = root.join(name);
    std::fs::write(&source, text).unwrap();
    RagStore::open(config)
        .unwrap()
        .import_file("demo", &source, &RagImport::new())
        .unwrap()
        .chunk_count
}

fn revision(config: &HieronymusConfig) -> i64 {
    RagStore::open(config).unwrap().corpus_revision().unwrap()
}

fn work_intent(config: &HieronymusConfig) -> Option<i64> {
    RagStore::open(config)
        .unwrap()
        .semantic_work_intent()
        .unwrap()
}

fn chunk_count(config: &HieronymusConfig) -> i64 {
    rusqlite::Connection::open(config.database_path())
        .unwrap()
        .query_row("select count(*) from rag_chunks", [], |row| row.get(0))
        .unwrap()
}

fn active_generation(config: &HieronymusConfig) -> hieronymus::semantic_store::GenerationManifest {
    SemanticStore::open(config)
        .unwrap()
        .active_generation()
        .unwrap()
        .expect("a generation is active")
}

/// The chunk ids the ARMED SEMANTIC LANE can actually reach for `query` —
/// i.e. what a `hieronymus_recall` call would get back from the vector index,
/// not what the manifest claims.
fn semantically_reachable(config: &HieronymusConfig, query: &str) -> Vec<i64> {
    let lane = SemanticLane::new(
        Box::new(FakeEmbeddingProvider::new(DIMENSIONS)),
        Box::new(model_tokenizer()),
    );
    let context = TranslationContext::new("demo", "ja", "en", "translation");
    let run = lane.run(config, &context, query, 25);
    assert!(
        !run.degraded,
        "the lane degraded instead of searching: {:?}",
        run.warnings
            .iter()
            .map(|warning| warning.reason.clone())
            .collect::<Vec<_>>()
    );
    run.records.iter().map(|record| record.id).collect()
}

/// Embed and write every authoritative chunk into `generation_id`, leaving it
/// `building` with nothing pending — the exact state a durable job reaches
/// just before it calls `activate_generation`.
fn drain_candidate(
    config: &HieronymusConfig,
    store: &SemanticStore,
    generation_id: &str,
    provider: &mut FakeEmbeddingProvider,
    tokenizer: &mut ModelTokenizer,
) {
    loop {
        let pending = store.pending_chunk_ids(generation_id, 16).unwrap();
        if pending.is_empty() {
            break;
        }
        let chunks: Vec<hieronymus::semantic_store::SemanticChunk> = pending
            .iter()
            .map(|chunk_id| {
                let (series_slug, text) = store.chunk_row(*chunk_id).unwrap().unwrap();
                hieronymus::semantic_store::SemanticChunk {
                    chunk_id: *chunk_id,
                    series_slug,
                    token_ids: tokenizer.encode(&text).unwrap(),
                }
            })
            .collect();
        store.write_batch(generation_id, provider, &chunks).unwrap();
    }
    let _ = config;
}

/// The activation sample the worker would build: the first authoritative
/// chunk's own series and text, so the sample query is guaranteed rows.
fn sample(config: &HieronymusConfig, tokenizer: &mut ModelTokenizer) -> SemanticSample {
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    let (series_slug, text): (String, String) = connection
        .query_row(
            "select series_slug, text from rag_chunks order by id limit 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    SemanticSample {
        series_slug,
        token_ids: tokenizer.encode(&text).unwrap(),
    }
}

/// Import through the `hieronymus_rag_import` TOOL, which is where the payload
/// contract lives. Writing the file first and reusing one `source_ref` keeps
/// the checksum-skip and replacement paths addressable.
fn tool_import(application: &Application, root: &Path, name: &str, text: &str) -> Value {
    let source = root.join(name);
    std::fs::write(&source, text).unwrap();
    application
        .call(
            "hieronymus_rag_import",
            &json!({
                "series_slug": "demo",
                "path": source.to_str().unwrap(),
                "source_ref": name,
            }),
            "local-user",
        )
        .unwrap()
}

/// The invariants every `hieronymus_rag_import` payload must satisfy, whatever
/// happened to the indexing. Asserted on every case so no branch can drop the
/// key or smuggle a non-id into the job field.
fn assert_indexing_contract(payload: &Value) {
    let indexing = payload["semantic_indexing"]
        .as_str()
        .unwrap_or_else(|| panic!("semantic_indexing must always be present: {payload}"));
    assert!(
        matches!(indexing, "queued" | "owed" | "not-required"),
        "semantic_indexing must be one of the three named outcomes: {payload}"
    );
    let job = &payload["semantic_rebuild_job"];
    assert!(
        job.is_null() || job.is_string(),
        "semantic_rebuild_job is a job id or null, never a structured error: {payload}"
    );
    assert_eq!(
        job.is_string(),
        indexing == "queued",
        "a job id is present exactly when indexing was queued: {payload}"
    );
    if let Some(error) = payload.get("semantic_indexing_error")
        && !error.is_null()
    {
        assert_eq!(
            indexing, "owed",
            "only an owed outcome may carry a cause: {payload}"
        );
    }
}

fn chunk_ids_containing(config: &HieronymusConfig, needle: &str) -> Vec<i64> {
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    let mut statement = connection
        .prepare("select id from rag_chunks where text like ?1 order by id")
        .unwrap();
    let ids: Vec<i64> = statement
        .query_map([format!("%{needle}%")], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(!ids.is_empty(), "no authoritative chunk contains {needle}");
    ids
}

// ---------------------------------------------------------------------------
// The missed commit/enqueue window
// ---------------------------------------------------------------------------

/// The A4 headline. An import commits, the in-memory worker notification is
/// never delivered (a crash, a dead daemon, a failed enqueue — the fault is
/// injected by simply not calling the controller), and the daemon restarts
/// with an OLDER generation still sitting in the active slot.
///
/// Before C4 that root reported `Ready` forever and the new text was
/// unreachable until somebody imported again. Now the durable corpus revision
/// makes the active generation read as behind, the durable work intent makes
/// the rebuild owed, and reconciliation queues and runs it — with no second
/// import and no operator action.
#[test]
fn a_commit_whose_enqueue_was_lost_is_repaired_at_startup() {
    let (root, config) = seeded_root();

    // A first, fully indexed corpus.
    import(
        &config,
        root.path(),
        "one.txt",
        "The first scroll speaks of tides.",
    );
    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(wait_ready(&controller), "{:?}", controller.state());
    let first = active_generation(&config);
    assert_eq!(first.corpus_revision, revision(&config));
    assert!(work_intent(&config).is_none(), "the intent was retired");
    group.stop_and_join().unwrap();

    // The window: the second import commits and NOTHING tells the worker.
    let covered_revision = revision(&config);
    import(
        &config,
        root.path(),
        "two.txt",
        "The second scroll names the leviathan of the deep.",
    );
    assert!(
        revision(&config) > covered_revision,
        "an import that changes text must bump the corpus revision"
    );
    assert_eq!(
        work_intent(&config),
        Some(revision(&config)),
        "the owed indexing is durable, not an in-memory hope"
    );
    // The stale generation is still active and still intact on disk: nothing
    // about it looks broken, which is exactly why the count-only check was
    // fooled.
    assert_eq!(
        active_generation(&config).generation_id,
        first.generation_id
    );
    assert!(
        SemanticStore::open(&config)
            .unwrap()
            .active_generation_intact()
            .unwrap()
    );

    let leviathan = chunk_ids_containing(&config, "leviathan");
    assert!(
        !semantically_reachable(&config, "leviathan of the deep")
            .iter()
            .any(|id| leviathan.contains(id)),
        "the new text cannot be in an index that was built before it existed"
    );

    // Restart. No import, no notification, no operator.
    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(
        wait_ready(&controller),
        "reconciliation must repair the missed indexing: {:?}",
        controller.state()
    );

    let repaired = active_generation(&config);
    assert_ne!(
        repaired.generation_id, first.generation_id,
        "a fresh generation must cover the new corpus"
    );
    assert_eq!(
        repaired.corpus_revision,
        revision(&config),
        "the repaired generation records the revision it covers"
    );
    assert!(
        work_intent(&config).is_none(),
        "the durable intent is retired once the work is queued"
    );
    let reachable = semantically_reachable(&config, "leviathan of the deep");
    assert!(
        leviathan.iter().any(|id| reachable.contains(id)),
        "the new text must be semantically retrievable without another import: \
         {reachable:?} vs {leviathan:?}"
    );

    assert!(
        !arm.lock_contended.load(Ordering::SeqCst),
        "a SQLite write transaction was held across inference"
    );
    assert!(
        arm.embeds.load(Ordering::SeqCst) > 0,
        "nothing was embedded"
    );
    group.stop_and_join().unwrap();
}

/// The same window from the queue's side: with no active generation at all and
/// the one-shot startup recovery already spent, a later import's durable
/// intent still gets the work queued. Without the intent this state is
/// indistinguishable from "the operator cancelled the rebuild on purpose",
/// which reconciliation must not override.
#[test]
fn a_durable_work_intent_survives_a_spent_startup_recovery() {
    let (root, config) = seeded_root();
    let identity = TestArm::identity_of();

    // A first import whose queued rebuild is then abandoned: the candidate is
    // cancelled and no generation is ever activated.
    import(
        &config,
        root.path(),
        "one.txt",
        "Abandoned first indexing run.",
    );
    let QueueOutcome::Enqueued(job_id) = queue_semantic_rebuild(&config, &identity).unwrap() else {
        panic!("the first import must queue a rebuild");
    };
    let jobs = hieronymus::semantic_jobs::SemanticJobStore::open(&config).unwrap();
    jobs.request_cancel(&job_id).unwrap();
    assert!(
        work_intent(&config).is_none(),
        "queueing retired the intent"
    );

    // The second import commits and its notification is lost.
    import(
        &config,
        root.path(),
        "two.txt",
        "The second import is owed indexing.",
    );
    assert_eq!(work_intent(&config), Some(revision(&config)));

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, arm as Arc<dyn SemanticArm>);
    assert!(
        wait_ready(&controller),
        "the durable intent must reach the queue: {:?}",
        controller.state()
    );
    assert_eq!(
        active_generation(&config).corpus_revision,
        revision(&config)
    );
    assert!(work_intent(&config).is_none());
    group.stop_and_join().unwrap();
}

// ---------------------------------------------------------------------------
// Equal-count dedup
// ---------------------------------------------------------------------------

/// Replacement at an equal chunk count. The source is re-imported with
/// DIFFERENT text of the same shape, so `expected_count == chunk_count` still
/// holds for the queued candidate — the pre-C4 dedup hit. The revision moved,
/// so the queued candidate is superseded instead.
#[test]
fn an_equal_count_replacement_supersedes_the_queued_rebuild() {
    let (root, config) = seeded_root();
    let identity = TestArm::identity_of();

    import(&config, root.path(), "scroll.txt", "alpha beta gamma delta");
    let before_count = chunk_count(&config);
    let QueueOutcome::Enqueued(first_job) = queue_semantic_rebuild(&config, &identity).unwrap()
    else {
        panic!("the first import must queue a rebuild");
    };

    // Requeueing with nothing changed IS a dedup hit: same revision, same
    // identity. This is the control that proves the test below is about the
    // revision and not about the predicate rejecting everything.
    assert_eq!(
        queue_semantic_rebuild(&config, &identity).unwrap(),
        QueueOutcome::AlreadyQueued(first_job.clone())
    );

    // Same source_ref, same chunk count, entirely different text.
    import(
        &config,
        root.path(),
        "scroll.txt",
        "omega sigma kappa theta",
    );
    assert_eq!(
        chunk_count(&config),
        before_count,
        "the fixture must keep the chunk count equal, or it tests nothing"
    );

    let outcome = queue_semantic_rebuild(&config, &identity).unwrap();
    let QueueOutcome::Enqueued(second_job) = &outcome else {
        panic!("an equal-count replacement must queue fresh work, got {outcome:?}");
    };
    assert_ne!(second_job, &first_job);

    let jobs = hieronymus::semantic_jobs::SemanticJobStore::open(&config).unwrap();
    assert!(
        jobs.job(&first_job).unwrap().unwrap().cancel_requested,
        "the superseded candidate must be durably cancelled"
    );
}

/// A removal that leaves the count alone is the same class of change: the
/// replacement text parses to fewer chunks, so a count comparison would see
/// movement — but the point is that the revision moves for removals too, and
/// the queued candidate is superseded rather than allowed to activate over
/// chunks that no longer exist.
#[test]
fn removing_content_supersedes_the_queued_rebuild() {
    let (root, config) = seeded_root();
    let identity = TestArm::identity_of();

    import(
        &config,
        root.path(),
        "scroll.txt",
        "# One\n\nfirst section\n\n# Two\n\nsecond section\n",
    );
    let QueueOutcome::Enqueued(first_job) = queue_semantic_rebuild(&config, &identity).unwrap()
    else {
        panic!("the first import must queue a rebuild");
    };
    let before = revision(&config);

    import(
        &config,
        root.path(),
        "scroll.txt",
        "# One\n\nfirst section\n",
    );
    assert!(revision(&config) > before, "a removal is a text change");

    let outcome = queue_semantic_rebuild(&config, &identity).unwrap();
    assert!(
        matches!(&outcome, QueueOutcome::Enqueued(job) if job != &first_job),
        "a removal must queue fresh work, got {outcome:?}"
    );
}

/// A changed embedding identity at an UNCHANGED revision is also not a dedup
/// hit: the queued vectors would be built under a model this daemon cannot
/// query with, however completely they cover the corpus.
#[test]
fn a_changed_identity_at_the_same_revision_is_not_a_dedup_hit() {
    let (root, config) = seeded_root();
    let identity = TestArm::identity_of();
    let swapped = FakeEmbeddingProvider::new(256).identity().clone();
    assert_ne!(identity, swapped);

    import(
        &config,
        root.path(),
        "scroll.txt",
        "The model was swapped underneath us.",
    );
    let QueueOutcome::Enqueued(first_job) = queue_semantic_rebuild(&config, &identity).unwrap()
    else {
        panic!("the first import must queue a rebuild");
    };
    let unchanged = revision(&config);

    let outcome = queue_semantic_rebuild(&config, &swapped).unwrap();
    assert_eq!(
        revision(&config),
        unchanged,
        "no text changed; only the identity did"
    );
    assert!(
        matches!(&outcome, QueueOutcome::Enqueued(job) if job != &first_job),
        "a swapped identity must queue fresh work, got {outcome:?}"
    );
}

/// An identical-checksum re-import is the opposite case: it refreshes metadata
/// tags and changes no indexable text, so the revision must NOT move and the
/// active generation must stay ready. A bump here would order a pointless
/// rebuild on every re-import.
#[test]
fn an_identical_reimport_does_not_move_the_corpus_revision() {
    let (root, config) = seeded_root();
    let source = root.path().join("scroll.txt");
    std::fs::write(&source, "Nothing about this text has changed.").unwrap();
    let store = RagStore::open(&config).unwrap();
    store
        .import_file("demo", &source, &RagImport::new())
        .unwrap();

    let after_first = revision(&config);
    let repeat = store
        .import_file("demo", &source, &RagImport::new())
        .unwrap();
    assert!(
        repeat.skipped,
        "an identical checksum is a metadata refresh"
    );
    assert_eq!(
        revision(&config),
        after_first,
        "a metadata refresh must not invalidate the active generation"
    );
}

// ---------------------------------------------------------------------------
// Stale startup, layered over the C3 checks
// ---------------------------------------------------------------------------

/// The C3 path still works with the revision check layered on: a generation
/// whose index directory was deleted is *unusable*, so it is invalidated
/// (driven terminal, out of the active slot) and rebuilt — not merely reported
/// as stale.
#[test]
fn a_vanished_index_is_still_invalidated_and_rebuilt() {
    let (root, config) = seeded_root();
    import(
        &config,
        root.path(),
        "scroll.txt",
        "The index directory vanished; the authoritative chunks never did.",
    );

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(wait_ready(&controller), "{:?}", controller.state());
    let built = active_generation(&config);
    // The generation is current in every C4 sense: same revision, same
    // identity. Only the index on disk is about to go.
    assert_eq!(built.corpus_revision, revision(&config));
    group.stop_and_join().unwrap();

    std::fs::remove_dir_all(SemanticStore::open(&config).unwrap().index_root()).unwrap();
    assert!(
        !SemanticStore::open(&config)
            .unwrap()
            .active_generation_intact()
            .unwrap()
    );

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, arm as Arc<dyn SemanticArm>);
    assert!(
        wait_ready(&controller),
        "the lost index must be rebuilt: {:?}",
        controller.state()
    );
    let rebuilt = active_generation(&config);
    assert_ne!(rebuilt.generation_id, built.generation_id);
    assert_eq!(
        SemanticStore::open(&config)
            .unwrap()
            .generation_manifest(&built.generation_id)
            .unwrap()
            .unwrap()
            .status,
        "failed",
        "an unusable generation is invalidated, never relabelled"
    );
    group.stop_and_join().unwrap();
}

/// A generation carrying the pre-C4 sentinel (`corpus_revision = -1`) cannot
/// be shown to cover anything, so it must be REBUILT — and never relabelled
/// with a revision somebody guessed. This is the state a v2 database that had
/// armed semantics arrives in.
#[test]
fn a_generation_predating_revisions_is_rebuilt_never_relabelled() {
    let (root, config) = seeded_root();
    import(
        &config,
        root.path(),
        "scroll.txt",
        "Vectors from before revisions existed.",
    );

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(wait_ready(&controller), "{:?}", controller.state());
    let legacy = active_generation(&config);
    group.stop_and_join().unwrap();

    // Rewind the recorded coverage to the sentinel, exactly as the 2 -> 3
    // migration leaves a generation begun under the old code. Everything else
    // — identity, index, status — stays perfectly healthy.
    rusqlite::Connection::open(config.database_path())
        .unwrap()
        .execute(
            "update semantic_generations set corpus_revision = ?2 where generation_id = ?1",
            rusqlite::params![legacy.generation_id, UNKNOWN_CORPUS_REVISION],
        )
        .unwrap();

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, arm as Arc<dyn SemanticArm>);
    assert!(
        wait_ready(&controller),
        "a sentinel generation must be rebuilt: {:?}",
        controller.state()
    );
    let rebuilt = active_generation(&config);
    assert_ne!(
        rebuilt.generation_id, legacy.generation_id,
        "the sentinel generation must be replaced, not relabelled"
    );
    assert_eq!(rebuilt.corpus_revision, revision(&config));
    assert_eq!(
        SemanticStore::open(&config)
            .unwrap()
            .generation_manifest(&legacy.generation_id)
            .unwrap()
            .unwrap()
            .corpus_revision,
        UNKNOWN_CORPUS_REVISION,
        "the old row keeps its sentinel: nothing may claim coverage it cannot prove"
    );
    group.stop_and_join().unwrap();
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// Concurrent imports: the revision is strictly monotonic (one increment per
/// text-changing commit, no lost update), no intent is lost, and the state the
/// worker then settles on covers the LATEST revision — not whichever import
/// happened to finish first.
#[test]
fn concurrent_imports_keep_the_revision_monotonic_and_lose_no_intent() {
    let (root, config) = seeded_root();
    const WRITERS: i64 = 6;

    let start = Arc::new(std::sync::Barrier::new(WRITERS as usize));
    let handles: Vec<_> = (0..WRITERS)
        .map(|index| {
            let config = config.clone();
            let directory = root.path().to_path_buf();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                let source = directory.join(format!("racer-{index}.txt"));
                std::fs::write(&source, format!("Racing scroll number {index} arrives.")).unwrap();
                start.wait();
                RagStore::open(&config)
                    .unwrap()
                    .import_file("demo", &source, &RagImport::new())
                    .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        revision(&config),
        WRITERS,
        "each committed text change must bump the revision exactly once"
    );
    assert_eq!(
        work_intent(&config),
        Some(WRITERS),
        "the coalesced intent must name the latest revision"
    );

    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(wait_ready(&controller), "{:?}", controller.state());
    assert_eq!(
        active_generation(&config).corpus_revision,
        WRITERS,
        "the final state must cover the latest revision"
    );
    for index in 0..WRITERS {
        let wanted = chunk_ids_containing(&config, &format!("number {index} arrives"));
        let reachable = semantically_reachable(&config, &format!("Racing scroll number {index}"));
        assert!(
            wanted.iter().any(|id| reachable.contains(id)),
            "racer {index} is not semantically reachable: {reachable:?} vs {wanted:?}"
        );
    }
    assert!(
        !arm.lock_contended.load(Ordering::SeqCst),
        "a SQLite write transaction was held across inference"
    );
    group.stop_and_join().unwrap();
}

/// Inference must never run under the SQLite write lock, so an import stays
/// responsive while a rebuild is grinding. The provider probes the lock on
/// every document embedding (see [`LockProbingProvider`]); this test adds the
/// caller's side of the same claim, with a real import racing a deliberately
/// slow rebuild and a bounded deadline.
#[test]
fn an_import_is_not_blocked_by_a_running_rebuild() {
    let (root, config) = seeded_root();
    for index in 0..12 {
        import(
            &config,
            root.path(),
            &format!("bulk-{index}.txt"),
            &format!("Bulk scroll {index} adds another slow embedding to the queue."),
        );
    }

    let lock_contended = Arc::new(AtomicBool::new(false));
    let arm = Arc::new(TestArm {
        database: config.database_path(),
        per_embed_delay: Duration::from_millis(40),
        lock_contended: Arc::clone(&lock_contended),
        embeds: Arc::new(AtomicUsize::new(0)),
    });
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(
        wait_until(
            || controller.state() == RequiredSemanticState::Rebuilding,
            Duration::from_secs(20)
        ),
        "the slow rebuild must be observably in flight: {:?}",
        controller.state()
    );

    // A write while the rebuild is mid-flight. The store's busy timeout is
    // seconds long, so a transaction held across inference would show up as a
    // multi-second stall (or an outright failure), not a passing assertion.
    let began = Instant::now();
    import(
        &config,
        root.path(),
        "interleaved.txt",
        "An import that must not wait for the rebuild.",
    );
    let elapsed = began.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "the import waited {elapsed:?} on a rebuild's lock"
    );
    assert!(
        !lock_contended.load(Ordering::SeqCst),
        "a SQLite write transaction was held across inference"
    );

    assert!(
        wait_ready(&controller),
        "the rebuild must still settle over the interleaved import: {:?}",
        controller.state()
    );
    assert_eq!(
        active_generation(&config).corpus_revision,
        revision(&config)
    );
    group.stop_and_join().unwrap();
}

// ---------------------------------------------------------------------------
// The v2 -> v3 schema step
// ---------------------------------------------------------------------------

/// The v2 -> v3 upgrade over a database that HAD armed semantics: the
/// `semantic_generations` table exists, holds an active row, and is not part of
/// any schema baseline. The step must add `corpus_revision` to it (via the
/// typed converter, because the same step has to succeed on a v2 root where
/// the table does not exist at all), leave the existing row on the `-1`
/// sentinel, and publish both version markers.
#[test]
fn the_v2_upgrade_marks_an_existing_generation_as_never_covered() {
    const RUST_V1_SQL: &str = include_str!("../../hieronymus/tests/fixtures/rust-v1.sql");
    /// The `semantic_generations` shape as of schema version 2: no
    /// `corpus_revision` column. Written out here rather than derived from the
    /// current code, because the point is to test the OLD shape.
    const V2_SEMANTIC_GENERATIONS_SQL: &str = "
        create table semantic_generations (
            generation_id text primary key,
            status text not null,
            provider text not null,
            model text not null,
            model_revision text not null,
            dimensions integer not null,
            normalization text not null,
            tokenizer text not null default 'byte-fold-v1',
            max_input_tokens integer not null,
            max_batch_inputs integer not null,
            expected_count integer not null,
            written_count integer not null default 0,
            last_chunk_id integer not null default 0,
            active integer not null default 0,
            created_at text not null,
            updated_at text not null
        );";

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection.execute_batch(RUST_V1_SQL).unwrap();
    {
        let transaction = connection.transaction().unwrap();
        hieronymus::schema_upgrade::apply_steps(&transaction, 1, 2).unwrap();
        transaction.commit().unwrap();
    }
    connection
        .execute_batch(V2_SEMANTIC_GENERATIONS_SQL)
        .unwrap();
    let identity = TestArm::identity_of();
    connection
        .execute(
            "insert into semantic_generations(
               generation_id, status, provider, model, model_revision, dimensions,
               normalization, tokenizer, max_input_tokens, max_batch_inputs,
               expected_count, written_count, last_chunk_id, active, created_at, updated_at
             )
             values ('legacy-active', 'active', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 3, 3, 9, 1,
                     '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            rusqlite::params![
                identity.provider(),
                identity.model(),
                identity.revision(),
                identity.dimensions() as i64,
                identity.normalization(),
                identity.tokenizer(),
                identity.max_input_tokens() as i64,
                identity.max_batch_inputs() as i64,
            ],
        )
        .unwrap();

    // The upgrade.
    {
        let transaction = connection.transaction().unwrap();
        hieronymus::schema_upgrade::apply_steps(
            &transaction,
            2,
            hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION,
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    // Both markers reached 3 (`apply_steps` verifies them, but say so here so
    // a future step cannot quietly change what this test covers).
    let marked: i64 = connection
        .query_row("select schema_version from hieronymus_meta", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(marked, 3);
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 3);

    // The pre-existing generation is on the sentinel, and it survived intact:
    // an upgrade converts schema, it does not touch coverage claims.
    let (recorded, status, active): (i64, String, i64) = connection
        .query_row(
            "select corpus_revision, status, active from semantic_generations
             where generation_id = 'legacy-active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        recorded, UNKNOWN_CORPUS_REVISION,
        "a generation from before revisions must read as never-covered"
    );
    assert_eq!(status, "active");
    assert_eq!(active, 1);

    // The new tables landed EMPTY: the Rust->Rust upgrade verification
    // requires it, and the readers coalesce the missing singleton to its
    // identity value.
    for table in [
        "corpus_revision",
        "semantic_work_intent",
        "dream_retry_state",
    ] {
        let rows: i64 = connection
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "{table} must be created empty");
    }
    assert_eq!(
        hieronymus::rag::current_corpus_revision(&connection).unwrap(),
        0,
        "an absent singleton reads as revision 0"
    );
    assert!(
        hieronymus::rag::pending_semantic_work_intent(&connection)
            .unwrap()
            .is_none()
    );
    // And -1 is behind 0, so the legacy generation is stale from the very
    // first tick rather than accidentally current.
    assert!(recorded < hieronymus::rag::current_corpus_revision(&connection).unwrap());
    drop(connection);

    assert_eq!(
        hieronymus::db::classify_database(&path),
        hieronymus::db::DatabaseState::RustSchema {
            version: hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION
        }
    );
}

/// The other half of the same step: a v2 root that never armed semantics has
/// NO `semantic_generations` table, and the upgrade must still succeed. This
/// is why the column is added by a typed converter instead of an `alter table`
/// in the step SQL.
#[test]
fn the_v2_upgrade_succeeds_without_a_semantic_generations_table() {
    const RUST_V1_SQL: &str = include_str!("../../hieronymus/tests/fixtures/rust-v1.sql");

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("hieronymus.sqlite");
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("pragma foreign_keys = on;")
        .unwrap();
    connection.execute_batch(RUST_V1_SQL).unwrap();
    {
        let transaction = connection.transaction().unwrap();
        hieronymus::schema_upgrade::apply_steps(
            &transaction,
            1,
            hieronymus::db::SUPPORTED_RUST_SCHEMA_VERSION,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    let present: i64 = connection
        .query_row(
            "select count(*) from sqlite_master where type = 'table'
             and name = 'semantic_generations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(present, 0, "the converter must not create the table");
    drop(connection);
    assert_eq!(hieronymus::db::verify_current_rust_schema(&path), Ok(()));

    // And the table created lazily afterwards carries the column, so both
    // shapes converge.
    let config = HieronymusConfig::new(root.path());
    SemanticStore::open(&config).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let mut statement = connection
        .prepare("select name from pragma_table_info('semantic_generations')")
        .unwrap();
    let columns: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        columns.iter().any(|name| name == "corpus_revision"),
        "a lazily created table must carry the column: {columns:?}"
    );
}

// ---------------------------------------------------------------------------
// Activation re-checks the corpus revision (I1)
// ---------------------------------------------------------------------------

/// Activation validates the candidate against CURRENT authoritative state, and
/// the corpus revision is half of that state (the embedding identity is the
/// other half). A rebuild that began at revision N must refuse to activate
/// once an import has moved the corpus to N+1: its vectors describe text the
/// store no longer holds, and publishing them would be the A4 defect arriving
/// by a different road — an "up to date" active generation over stale text.
///
/// Driven at store level so the refusal itself is observable, with no worker
/// in the way.
#[test]
fn activation_refuses_a_candidate_whose_corpus_revision_moved() {
    let (root, config) = seeded_root();
    import(
        &config,
        root.path(),
        "one.txt",
        "The candidate was built from this text.",
    );

    let store = SemanticStore::open(&config).unwrap();
    let mut provider = FakeEmbeddingProvider::new(DIMENSIONS);
    let mut tokenizer = model_tokenizer();
    let identity = provider.identity().clone();
    let begun = store.begin_generation("candidate", &identity).unwrap();
    assert_eq!(begun.corpus_revision, revision(&config));
    drain_candidate(&config, &store, "candidate", &mut provider, &mut tokenizer);

    // A second, entirely separate source lands before the candidate activates.
    let stale_revision = revision(&config);
    import(
        &config,
        root.path(),
        "two.txt",
        "A later import the candidate never saw.",
    );
    assert!(revision(&config) > stale_revision);

    let error = store
        .activate_generation("candidate", &mut provider, &sample(&config, &mut tokenizer))
        .expect_err("a revision-stale candidate must not activate");
    assert!(
        matches!(error, SemanticError::ValidationFailed(_)),
        "the refusal must be a validation failure so the job is marked terminal: {error:?}"
    );
    assert!(
        error.to_string().contains("revision"),
        "the refusal must name the corpus revision: {error}"
    );
    assert!(
        store.active_generation().unwrap().is_none(),
        "nothing was published"
    );
}

/// The same race through the real controller, end to end: the worker is
/// driving a deliberately slow rebuild when an import lands. The in-flight
/// candidate is refused at activation, and reconciliation queues and completes
/// a fresh generation that covers the NEW revision — with the late import's
/// text semantically reachable and without a second import.
#[test]
fn a_rebuild_overtaken_by_an_import_is_replaced_not_published() {
    let (root, config) = seeded_root();
    for index in 0..10 {
        import(
            &config,
            root.path(),
            &format!("bulk-{index}.txt"),
            &format!("Bulk scroll {index} keeps the slow rebuild busy for a while."),
        );
    }

    let arm = Arc::new(TestArm {
        database: config.database_path(),
        per_embed_delay: Duration::from_millis(60),
        lock_contended: Arc::new(AtomicBool::new(false)),
        embeds: Arc::new(AtomicUsize::new(0)),
    });
    let (group, controller) = start_controller(&config, Arc::clone(&arm) as Arc<dyn SemanticArm>);
    assert!(
        wait_until(
            || controller.state() == RequiredSemanticState::Rebuilding,
            Duration::from_secs(20)
        ),
        "the slow rebuild must be observably in flight: {:?}",
        controller.state()
    );

    // Overtake it. No notification: the durable revision and intent are the
    // only things that can save this.
    import(
        &config,
        root.path(),
        "late.txt",
        "The late scroll speaks of the drowned cathedral.",
    );
    let latest = revision(&config);

    assert!(
        wait_ready(&controller),
        "reconciliation must converge on a generation covering the late import: {:?}",
        controller.state()
    );
    let active = active_generation(&config);
    assert_eq!(
        active.corpus_revision, latest,
        "the published generation must cover the latest revision, never the overtaken one"
    );
    assert!(work_intent(&config).is_none());

    let cathedral = chunk_ids_containing(&config, "drowned cathedral");
    let reachable = semantically_reachable(&config, "the drowned cathedral");
    assert!(
        cathedral.iter().any(|id| reachable.contains(id)),
        "the late import must be semantically reachable: {reachable:?} vs {cathedral:?}"
    );
    assert!(
        !arm.lock_contended.load(Ordering::SeqCst),
        "a SQLite write transaction was held across inference"
    );
    group.stop_and_join().unwrap();
}

// ---------------------------------------------------------------------------
// The hieronymus_rag_import payload contract (I2, I3)
// ---------------------------------------------------------------------------

/// A normal successful enqueue: `semantic_indexing` is `queued` and
/// `semantic_rebuild_job` carries the id of a job that really is in the durable
/// queue (the hook is the real controller's, so the id is looked up, not
/// assumed).
#[test]
fn rag_import_reports_queued_with_a_real_durable_job() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();
    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, arm as Arc<dyn SemanticArm>);
    let hook_controller = controller.clone();
    application.set_rebuild_hook(Arc::new(move |series| {
        hook_controller.request_rebuild(series)
    }));

    let payload = tool_import(
        &application,
        root.path(),
        "scroll.txt",
        "A freshly queued scroll.",
    );
    assert_indexing_contract(&payload);
    assert_eq!(payload["semantic_indexing"], json!("queued"), "{payload}");
    assert!(payload["semantic_indexing_error"].is_null(), "{payload}");
    let job_id = payload["semantic_rebuild_job"]
        .as_str()
        .expect("a queued import carries a job id");
    assert!(
        hieronymus::semantic_jobs::SemanticJobStore::open(&config)
            .unwrap()
            .job(job_id)
            .unwrap()
            .is_some(),
        "the reported job id must name a real durable job: {job_id}"
    );
    group.stop_and_join().unwrap();
}

/// The A4 payload defect itself. The enqueue fails; the import is committed
/// and its intent durable, so this is NOT a tool error — but it must never
/// read as queued. `semantic_indexing` says `owed`, the cause is on its own
/// key, and `semantic_rebuild_job` is `null` rather than the pre-C4
/// `{"error": ...}` object that looked like success to anything that did not
/// inspect the field's type.
#[test]
fn rag_import_reports_owed_with_its_cause_when_the_enqueue_fails() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();
    application.set_rebuild_hook(Arc::new(|_series| {
        Err("scripted enqueue failure".to_string())
    }));

    let payload = tool_import(
        &application,
        root.path(),
        "scroll.txt",
        "An unqueued scroll.",
    );
    assert_indexing_contract(&payload);
    assert_eq!(payload["semantic_indexing"], json!("owed"), "{payload}");
    assert_eq!(
        payload["semantic_rebuild_job"],
        Value::Null,
        "a failed enqueue must never be dressed up as a job: {payload}"
    );
    assert!(
        payload["semantic_indexing_error"]
            .as_str()
            .unwrap_or_default()
            .contains("scripted enqueue failure"),
        "the cause must be surfaced: {payload}"
    );
    // And the claim `owed` makes is true: the work is durably recorded.
    assert_eq!(work_intent(&config), Some(revision(&config)));

    // Which reconciliation then honours, with no further import.
    let arm = Arc::new(TestArm::new(&config));
    let (group, controller) = start_controller(&config, arm as Arc<dyn SemanticArm>);
    assert!(wait_ready(&controller), "{:?}", controller.state());
    assert_eq!(
        active_generation(&config).corpus_revision,
        revision(&config)
    );
    group.stop_and_join().unwrap();
}

/// No daemon owns this application, so there is no hook to queue through. That
/// is also `owed` — the intent is durable and reconciliation covers it — but
/// with no error, because nothing failed.
#[test]
fn rag_import_reports_owed_without_an_error_when_no_daemon_owns_it() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();

    let payload = tool_import(
        &application,
        root.path(),
        "scroll.txt",
        "A daemonless scroll.",
    );
    assert_indexing_contract(&payload);
    assert_eq!(payload["semantic_indexing"], json!("owed"), "{payload}");
    assert_eq!(payload["semantic_rebuild_job"], Value::Null, "{payload}");
    assert!(
        payload["semantic_indexing_error"].is_null(),
        "nothing failed, so no cause may be invented: {payload}"
    );
    assert_eq!(work_intent(&config), Some(revision(&config)));
}

/// The identical-checksum refresh: no indexable text changed, so there is
/// nothing owed and nothing queued. The revision must not move either — a bump
/// here would order a pointless rebuild on every re-import.
#[test]
fn rag_import_reports_not_required_for_an_identical_reimport() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();
    application.set_rebuild_hook(Arc::new(|_series| Ok("job-1".to_string())));

    let first = tool_import(&application, root.path(), "scroll.txt", "Unchanged text.");
    assert_eq!(first["semantic_indexing"], json!("queued"), "{first}");
    let after_first = revision(&config);

    let again = tool_import(&application, root.path(), "scroll.txt", "Unchanged text.");
    assert_eq!(again["skipped"], json!(true), "{again}");
    assert_indexing_contract(&again);
    assert_eq!(
        again["semantic_indexing"],
        json!("not-required"),
        "a metadata refresh owes no indexing: {again}"
    );
    assert_eq!(again["semantic_rebuild_job"], Value::Null, "{again}");
    assert_eq!(revision(&config), after_first);
}

/// The empty-corpus no-op marker must never reach a caller as a job id (I3).
///
/// `SemanticController::request_rebuild` answers `rebuild:empty-corpus` when
/// there is nothing to index. That is the worker's internal signal, not a
/// durable job — `SemanticJobStore::job` will never find it — so the payload
/// reports `not-required` with a `null` job instead of advertising a job
/// nobody can look up.
///
/// Driven through the hook because the marker is unreachable via a real import
/// (see the sibling test below): the mapping is the belt-and-braces half of
/// the "a job id or null, never anything else" contract, and it is asserted
/// here against exactly the value the real controller produces.
#[test]
fn rag_import_never_reports_the_empty_corpus_marker_as_a_job() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();
    application.set_rebuild_hook(Arc::new(|_series| Ok(EMPTY_CORPUS_JOB_ID.to_string())));

    let payload = tool_import(
        &application,
        root.path(),
        "scroll.txt",
        "Some real text here.",
    );
    assert_indexing_contract(&payload);
    assert_eq!(
        payload["semantic_indexing"],
        json!("not-required"),
        "nothing to index owes no indexing: {payload}"
    );
    assert_eq!(
        payload["semantic_rebuild_job"],
        Value::Null,
        "the empty-corpus marker must never reach a caller as a job id: {payload}"
    );
    assert_ne!(
        payload["semantic_rebuild_job"],
        json!(EMPTY_CORPUS_JOB_ID),
        "specifically not the sentinel: {payload}"
    );
    assert!(payload["semantic_indexing_error"].is_null(), "{payload}");
    // The marker names nothing durable, which is the whole reason it is not
    // reported as a job.
    assert!(
        hieronymus::semantic_jobs::SemanticJobStore::open(&config)
            .unwrap()
            .job(EMPTY_CORPUS_JOB_ID)
            .unwrap()
            .is_none()
    );
}

/// Why the branch above is defensive rather than a scenario: an import can
/// never empty the corpus. A source that parses to zero chunks is refused as
/// an invalid source BEFORE the import transaction is opened, so a committed
/// non-skipped import always added at least one chunk — and the corpus, the
/// revision, and the active generation are all left exactly as they were.
#[test]
fn an_import_that_would_empty_the_corpus_is_refused_before_committing() {
    let (root, config) = seeded_root();
    let application = Application::open(&config).unwrap();
    application.set_rebuild_hook(Arc::new(|_series| Ok("job-1".to_string())));
    tool_import(
        &application,
        root.path(),
        "scroll.txt",
        "Some real text here.",
    );
    let chunks_before = chunk_count(&config);
    let revision_before = revision(&config);
    assert!(chunks_before > 0);

    // The same source_ref, replaced by content that parses to nothing.
    let blank = root.path().join("scroll.txt");
    std::fs::write(&blank, "   \n\n   \n").unwrap();
    let error = application
        .call(
            "hieronymus_rag_import",
            &json!({
                "series_slug": "demo",
                "path": blank.to_str().unwrap(),
                "source_ref": "scroll.txt",
            }),
            "local-user",
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("no chunks"),
        "a chunkless source is a clean tool error: {error}"
    );

    assert_eq!(
        chunk_count(&config),
        chunks_before,
        "the refused import must not have deleted the existing chunks"
    );
    assert_eq!(
        revision(&config),
        revision_before,
        "and it must not have moved the corpus revision"
    );
}

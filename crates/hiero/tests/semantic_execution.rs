//! Task S2: the daemon actually executes durable semantic rebuild jobs and
//! arms its recall lane. Covers the required-readiness regression, the typed
//! `/status` deserializer (an FTS-only lane is not healthy for the strict
//! gate), and the full daemon flows with a test-injected embedding provider:
//! import → queue → active generation → recall, cancel/retry, worker failure
//! and bounded retry, concurrent imports, stale-lease takeover, restart
//! reconciliation, and graceful SIGTERM-style join.
//!
//! The fake provider keeps everything offline; the real-model version of the
//! same flow is S3/F2 evidence, not this file.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hiero::daemon::semantic_worker::{
    ArmedPair, EMPTY_CORPUS_JOB_ID, RequiredSemanticState, SemanticArm, SemanticController,
    install_test_arm, require_semantic_ready,
};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_jobs::SemanticJobStore;
use hieronymus::semantic_store::SemanticStore;
use hieronymus::semantic_tokenizer::ModelTokenizer;
use serde::Deserialize;
use serde_json::{Value, json};

use common::{mcp_headers, send_request, start_daemon, wait_until};

// --------------------------------------------------------------- the regression

#[test]
fn required_semantics_cannot_be_reported_ready_when_disarmed() {
    assert!(
        require_semantic_ready(&RequiredSemanticState::Failed("runtime missing".into())).is_err()
    );
    assert!(require_semantic_ready(&RequiredSemanticState::Rebuilding).is_err());
    assert!(require_semantic_ready(&RequiredSemanticState::Ready).is_ok());
}

// ------------------------------------------------------------ injection harness

/// A deterministic test provider over the fake embedding math: optional
/// per-embed delay (so a rebuild is observably in flight) and scripted
/// failures (transient retry or permanent exhaustion).
struct ScriptedProvider {
    inner: FakeEmbeddingProvider,
    failures_remaining: AtomicUsize,
    permanent_failure: bool,
    per_embed_delay: Duration,
}

impl ScriptedProvider {
    fn new(failures_remaining: usize, per_embed_delay: Duration) -> Self {
        Self {
            inner: FakeEmbeddingProvider::new(384),
            failures_remaining: AtomicUsize::new(failures_remaining),
            permanent_failure: false,
            per_embed_delay,
        }
    }

    fn permanently_failing() -> Self {
        Self {
            inner: FakeEmbeddingProvider::new(384),
            failures_remaining: AtomicUsize::new(0),
            permanent_failure: true,
            per_embed_delay: Duration::ZERO,
        }
    }

    fn maybe_fail(&self) -> Result<(), SemanticError> {
        if self.permanent_failure {
            return Err(SemanticError::Store(
                "scripted permanent inference failure".to_string(),
            ));
        }
        loop {
            let remaining = self.failures_remaining.load(Ordering::SeqCst);
            if remaining == 0 {
                return Ok(());
            }
            if self
                .failures_remaining
                .compare_exchange(remaining, remaining - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Err(SemanticError::Store(
                    "scripted transient inference failure".to_string(),
                ));
            }
        }
    }
}

impl EmbeddingProvider for ScriptedProvider {
    fn identity(&self) -> &hieronymus::semantic_embeddings::EmbeddingIdentity {
        self.inner.identity()
    }

    fn embed_document(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.maybe_fail()?;
        if !self.per_embed_delay.is_zero() {
            std::thread::sleep(self.per_embed_delay);
        }
        self.inner.embed_document(token_ids)
    }

    fn embed_query(&mut self, token_ids: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.maybe_fail()?;
        self.inner.embed_query(token_ids)
    }
}

/// The test arm: the S1-pinned tokenizer fixture plus scripted fake
/// providers, registered for exactly this test's data root.
struct TestArm {
    failures_remaining: usize,
    per_embed_delay: Duration,
    permanent_failure: bool,
}

impl TestArm {
    fn fast() -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            failures_remaining: 0,
            per_embed_delay: Duration::ZERO,
            permanent_failure: false,
        })
    }

    fn delayed(per_embed_delay: Duration) -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            failures_remaining: 0,
            per_embed_delay,
            permanent_failure: false,
        })
    }

    fn flaky(failures_remaining: usize) -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            failures_remaining,
            per_embed_delay: Duration::ZERO,
            permanent_failure: false,
        })
    }

    fn failing() -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            failures_remaining: 0,
            per_embed_delay: Duration::ZERO,
            permanent_failure: true,
        })
    }
}

impl SemanticArm for TestArm {
    fn identity(&self) -> hieronymus::semantic_embeddings::EmbeddingIdentity {
        FakeEmbeddingProvider::new(384).identity().clone()
    }

    fn precheck(&self, _config: &HieronymusConfig) -> Result<(), String> {
        Ok(())
    }

    fn arm(&self, _config: &HieronymusConfig) -> Result<ArmedPair, String> {
        let provider = if self.permanent_failure {
            ScriptedProvider::permanently_failing()
        } else {
            ScriptedProvider::new(self.failures_remaining, self.per_embed_delay)
        };
        let tokenizer = ModelTokenizer::from_bytes(include_bytes!(
            "../../hieronymus/tests/fixtures/minilm-tokenizer.json"
        ))
        .map_err(|error| error.to_string())?;
        Ok(ArmedPair {
            provider: Box::new(provider),
            tokenizer: Box::new(tokenizer),
        })
    }
}

fn start_semantic_daemon(root: &Path, arm: Arc<dyn SemanticArm>) -> hiero::daemon::Daemon {
    install_test_arm(root, arm);
    start_daemon(root)
}

// -------------------------------------------------------- HTTP helpers

fn tools_call(id: i64, name: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {
                    "name": "semantic-execution",
                    "version": "1.0.0"
                },
                "io.modelcontextprotocol/protocolVersion": common::PROTOCOL_REVISION
            },
            "arguments": arguments,
            "name": name
        }
    })
}

fn call_tool(daemon: &hiero::daemon::Daemon, id: i64, name: &str, arguments: Value) -> Value {
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(id, name, arguments)).unwrap(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    let body = response.body();
    // Tool errors fail the test loudly.
    assert!(
        body["result"].get("isError").and_then(Value::as_bool) != Some(true),
        "tool {name} failed: {body}"
    );
    serde_json::from_str::<Value>(body["result"]["content"][0]["text"].as_str().unwrap())
        .expect("tool payload is JSON")
}

fn status_body(daemon: &hiero::daemon::Daemon) -> Value {
    let response = send_request(
        daemon.local_addr().port(),
        "GET",
        "/status",
        &[(
            "Authorization".to_string(),
            format!("Bearer {}", daemon.bearer().expose_secret()),
        )],
        b"",
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    response.body()
}

/// The typed projection of `/status`'s semantic surface: deserializing (not
/// string-matching) proves the readiness gate consumes a real DTO shape.
#[derive(Debug, Deserialize)]
struct StatusDto {
    semantic: SemanticStateDto,
}

#[derive(Debug, Deserialize)]
struct SemanticStateDto {
    state: String,
    detail: Option<String>,
}

impl SemanticStateDto {
    fn required(&self) -> RequiredSemanticState {
        match self.state.as_str() {
            "acquiring" => RequiredSemanticState::Acquiring,
            "rebuilding" => RequiredSemanticState::Rebuilding,
            "ready" => RequiredSemanticState::Ready,
            other => RequiredSemanticState::Failed(
                self.detail.clone().unwrap_or_else(|| other.to_string()),
            ),
        }
    }
}

fn semantic_state(daemon: &hiero::daemon::Daemon) -> SemanticStateDto {
    let dto: StatusDto = serde_json::from_value(status_body(daemon)).unwrap();
    dto.semantic
}

fn wait_for_state(daemon: &hiero::daemon::Daemon, wanted: &str) -> SemanticStateDto {
    assert!(
        wait_until(
            || semantic_state(daemon).state == wanted,
            Duration::from_secs(15)
        ),
        "semantic state never reached {wanted:?}: last seen {:?}",
        semantic_state(daemon)
    );
    semantic_state(daemon)
}

fn seed_series_and_session(daemon: &hiero::daemon::Daemon) -> i64 {
    call_tool(
        daemon,
        1,
        "hieronymus_series_create",
        json!({"slug": "demo", "title": "Demo", "source_language": "ja", "target_language": "en"}),
    );
    let session = call_tool(
        daemon,
        2,
        "hieronymus_session_start",
        json!({"series_slug": "demo"}),
    );
    session["session_id"].as_i64().unwrap()
}

fn write_source(root: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = root.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn import(daemon: &hiero::daemon::Daemon, root: &Path, name: &str, text: &str) -> Value {
    let path = write_source(root, name, text);
    call_tool(
        daemon,
        3,
        "hieronymus_rag_import",
        json!({"series_slug": "demo", "path": path.to_str().unwrap()}),
    )
}

/// Recall with a query the FTS lane cannot match: any returned rag row must
/// have come from the armed semantic lane.
fn semantic_recall(daemon: &hiero::daemon::Daemon, session_id: i64, query: &str) -> Vec<Value> {
    let payload = call_tool(
        daemon,
        4,
        "hieronymus_recall",
        json!({
            "session_id": session_id,
            "series_slug": "demo",
            "query": query,
            "limit": 5,
        }),
    );
    payload["results"].as_array().unwrap().clone()
}

// ------------------------------------------------------------------ the tests

/// A daemon with no semantic configuration (fresh root, no persisted runtime)
/// reports an actionable Failed — an FTS-only lane is not healthy for the
/// required semantic readiness gate, proven through the typed DTO.
#[test]
fn an_fts_only_lane_is_not_ready_for_the_required_semantic_gate() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();

    let state = wait_for_state(&daemon, "failed");
    assert!(
        state
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("hiero semantic enable --runtime"),
        "actionable setup failure expected, got {state:?}"
    );

    // The typed deserializer feeds the gate; FTS-only must not pass.
    let dto: StatusDto = serde_json::from_value(status_body(&daemon)).unwrap();
    let error = require_semantic_ready(&dto.semantic.required()).unwrap_err();
    assert!(error.contains("semantic retrieval unavailable"), "{error}");

    daemon.shutdown().unwrap();
    drop(root);
}

/// The core flow: RAG import queues a durable rebuild after its authoritative
/// commit, the supervised worker executes it to an active generation, and the
/// daemon's recall serves a semantic-lane hit the FTS lane cannot match.
#[test]
fn import_queues_rebuild_activates_generation_and_serves_semantic_recall() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_semantic_daemon(root.path(), TestArm::fast());
    let session_id = seed_series_and_session(&daemon);

    let empty = wait_for_state(&daemon, "ready");
    assert_eq!(
        empty.state, "ready",
        "an empty corpus with a loaded model/runtime is ready-for-ingest, never an error"
    );

    let imported = import(
        &daemon,
        root.path(),
        "chapter-1.txt",
        "The archivist catalogued every rumour of the drowned city before the tide returned.",
    );
    let job_id = imported["semantic_rebuild_job"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        job_id.starts_with("rebuild:rebuild-"),
        "durable job id expected, got {imported}"
    );

    let ready = wait_for_state(&daemon, "ready");
    assert_eq!(ready.state, "ready");

    // The job is terminal and its generation is active.
    let config = HieronymusConfig::new(root.path());
    let jobs = SemanticJobStore::open(&config).unwrap();
    let record = jobs.job(&job_id).unwrap().expect("job row");
    assert_eq!(record.status, "completed");
    let store = SemanticStore::open(&config).unwrap();
    let active = store
        .active_generation()
        .unwrap()
        .expect("active generation");
    assert_eq!(active.identity, TestArm::fast().identity());
    assert_eq!(active.written_count, active.expected_count);

    // Typed DTO: ready passes the strict gate.
    let dto: StatusDto = serde_json::from_value(status_body(&daemon)).unwrap();
    require_semantic_ready(&dto.semantic.required()).unwrap();

    // A query with no lexical overlap still returns the chunk through the
    // armed semantic lane.
    let results = semantic_recall(&daemon, session_id, "zzqxj nonlexical probe");
    let semantic_hits: Vec<&Value> = results
        .iter()
        .filter(|row| row["rank_reason"].as_str().unwrap_or_default() == "rag semantic match")
        .collect();
    assert!(
        semantic_hits.iter().any(|row| row["text"]
            .as_str()
            .unwrap_or_default()
            .contains("drowned city")),
        "semantic lane must surface the chunk: {results:?}"
    );

    daemon.shutdown().unwrap();
}

/// Graceful shutdown (the same drain-and-join path SIGTERM takes) and a
/// restart: the active generation keeps serving recall, ready with no new
/// work.
#[test]
fn restart_keeps_imported_documents_retrievable_and_ready() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_semantic_daemon(root.path(), TestArm::fast());
    let session_id = seed_series_and_session(&daemon);
    import(
        &daemon,
        root.path(),
        "chapter-1.txt",
        "The archivist catalogued every rumour of the drowned city before the tide returned.",
    );
    wait_for_state(&daemon, "ready");

    // Graceful stop: admission stops, workers join, ownership releases. A
    // second daemon can only start if the join actually completed.
    daemon.shutdown().unwrap();

    let restarted = start_semantic_daemon(root.path(), TestArm::fast());
    let session = call_tool(
        &restarted,
        5,
        "hieronymus_session_start",
        json!({"series_slug": "demo"}),
    );
    let new_session = session["session_id"].as_i64().unwrap();
    // Restart reconciliation settles straight to ready: the active generation
    // survives, no rebuild is re-queued.
    let state = wait_for_state(&restarted, "ready");
    assert_eq!(state.state, "ready");
    let results = semantic_recall(&restarted, new_session, "zzqxj nonlexical probe");
    assert!(
        results.iter().any(|row| row["text"]
            .as_str()
            .unwrap_or_default()
            .contains("drowned city")),
        "imported documents stay retrievable after restart: {results:?}"
    );
    let _ = session_id;
    restarted.shutdown().unwrap();
}

/// A queued job left behind by a previous daemon (missed wakeup / crash) is
/// recovered by startup reconciliation and driven to activation.
#[test]
fn startup_reconciliation_recovers_a_queued_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());

    // Authoritative chunks plus a durable queued job, all without a daemon
    // (the "wakeup was missed" scenario: the durable rows are the only
    // state).
    let registry = hieronymus::registry::Registry::open(&config).unwrap();
    registry
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    let source = root.path().join("offline.txt");
    std::fs::write(
        &source,
        "Offline material queued before the daemon ever started.",
    )
    .unwrap();
    hieronymus::rag::RagStore::open(&config)
        .unwrap()
        .import_file("demo", &source, &hieronymus::rag::RagImport::new())
        .unwrap();
    let store = SemanticStore::open(&config).unwrap();
    let identity = TestArm::fast().identity();
    store
        .begin_generation("rebuild-pending", &identity)
        .unwrap();
    let job = SemanticJobStore::open(&config)
        .unwrap()
        .enqueue_rebuild("rebuild-pending", &identity)
        .unwrap();
    assert_eq!(job.status, "queued");

    let daemon = start_semantic_daemon(root.path(), TestArm::fast());
    wait_for_state(&daemon, "ready");
    let jobs = SemanticJobStore::open(&config).unwrap();
    assert_eq!(
        jobs.job(&job.job_id).unwrap().unwrap().status,
        "completed",
        "startup reconciliation must drive the queued job to completion"
    );
    daemon.shutdown().unwrap();
}

/// Cancelling a running rebuild honors the durable flag at a batch boundary,
/// never activates the candidate, and a later import retries with a fresh
/// generation that does activate.
#[test]
fn cancel_stops_the_rebuild_and_a_retry_rebuilds() {
    let root = tempfile::tempdir().unwrap();
    // Slow embedding keeps the rebuild observably in flight so the cancel
    // lands at a batch boundary instead of racing completion.
    let daemon = start_semantic_daemon(root.path(), TestArm::delayed(Duration::from_millis(50)));
    seed_series_and_session(&daemon);
    let body = "Cancel me. ".repeat(64);
    let imported = import(&daemon, root.path(), "big.txt", &body);
    let job_id = imported["semantic_rebuild_job"]
        .as_str()
        .unwrap()
        .to_string();

    let config = HieronymusConfig::new(root.path());
    let jobs = SemanticJobStore::open(&config).unwrap();
    jobs.request_cancel(&job_id).unwrap();

    assert!(wait_until(
        || jobs
            .job(&job_id)
            .unwrap()
            .map(|job| job.status == "cancelled")
            .unwrap_or(false),
        Duration::from_secs(20)
    ));
    let store = SemanticStore::open(&config).unwrap();
    assert!(
        store.active_generation().unwrap().is_none(),
        "a cancelled candidate must never activate"
    );
    wait_for_state(&daemon, "ready");

    // Retry: new authoritative rows queue a fresh rebuild that completes.
    import(
        &daemon,
        root.path(),
        "retry.txt",
        "The retry chapter mentions the drowned city again after cancellation.",
    );
    wait_for_state(&daemon, "ready");
    let active = store.active_generation().unwrap().expect("retry activates");
    assert!(active.written_count > 0);
    daemon.shutdown().unwrap();
}

/// Scripted failures: bounded retry heals transient failures, and a
/// permanently failing worker fails the job with an actionable state instead
/// of pretending success.
#[test]
fn worker_failures_retry_then_fail_honestly() {
    let root = tempfile::tempdir().unwrap();
    // Two consecutive batch failures, then recovery: the attempt bound (3)
    // must not be reached and the job completes.
    let daemon = start_semantic_daemon(root.path(), TestArm::flaky(2));
    seed_series_and_session(&daemon);
    let imported = import(
        &daemon,
        root.path(),
        "flaky.txt",
        "Transient inference failures must not exhaust the attempt bound.",
    );
    let job_id = imported["semantic_rebuild_job"]
        .as_str()
        .unwrap()
        .to_string();
    wait_for_state(&daemon, "ready");
    let jobs = SemanticJobStore::open(&HieronymusConfig::new(root.path())).unwrap();
    let record = jobs.job(&job_id).unwrap().unwrap();
    assert_eq!(record.status, "completed");
    assert!(
        record.failed_batches >= 1,
        "the scripted failures must have been recorded"
    );
    daemon.shutdown().unwrap();

    // A permanently failing provider exhausts the bound: job failed,
    // candidate failed and GC-able, /status carries the actionable error.
    let second_root = tempfile::tempdir().unwrap();
    let failing = start_semantic_daemon(second_root.path(), TestArm::failing());
    seed_series_and_session(&failing);
    let imported = import(
        &failing,
        second_root.path(),
        "doomed.txt",
        "Permanent inference failure must fail the job honestly.",
    );
    let job_id = imported["semantic_rebuild_job"]
        .as_str()
        .unwrap()
        .to_string();
    let state = wait_for_state(&failing, "failed");
    assert!(
        state
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("scripted permanent inference failure"),
        "actionable failure detail expected: {state:?}"
    );
    let config = HieronymusConfig::new(second_root.path());
    let jobs = SemanticJobStore::open(&config).unwrap();
    let record = jobs.job(&job_id).unwrap().unwrap();
    assert_eq!(record.status, "failed");
    let dto: StatusDto = serde_json::from_value(status_body(&failing)).unwrap();
    assert!(require_semantic_ready(&dto.semantic.required()).is_err());
    failing.shutdown().unwrap();
}

/// Two imports back to back: the first rebuild's frozen expectation goes
/// stale, is durably cancelled, and one fresh whole-corpus rebuild covers
/// both sources.
#[test]
fn concurrent_imports_converge_on_one_current_generation() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_semantic_daemon(root.path(), TestArm::delayed(Duration::from_millis(20)));
    seed_series_and_session(&daemon);
    let first = import(
        &daemon,
        root.path(),
        "a.txt",
        "The first source describes the harbour lights failing one by one.",
    );
    let second = import(
        &daemon,
        root.path(),
        "b.txt",
        "The second source describes the lighthouse keeper's ledger.",
    );
    let first_job = first["semantic_rebuild_job"].as_str().unwrap().to_string();
    let second_job = second["semantic_rebuild_job"].as_str().unwrap().to_string();
    assert_ne!(first_job, second_job);

    wait_for_state(&daemon, "ready");
    let config = HieronymusConfig::new(root.path());
    let jobs = SemanticJobStore::open(&config).unwrap();
    assert_eq!(
        jobs.job(&second_job).unwrap().unwrap().status,
        "completed",
        "the current rebuild must complete"
    );
    let store = SemanticStore::open(&config).unwrap();
    let active = store
        .active_generation()
        .unwrap()
        .expect("one active generation");
    let expected = active.expected_count;
    let counted: i64 = {
        let connection = rusqlite::Connection::open(config.database_path()).unwrap();
        connection
            .query_row("select count(*) from rag_chunks", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(
        expected, counted as u64,
        "the active generation covers the whole corpus"
    );
    daemon.shutdown().unwrap();
}

/// Controller-level (no HTTP): a foreign, unexpired lease is respected; after
/// expiry the supervised worker takes the lease over and completes the job.
/// Also exercises the worker-group join contract directly.
#[test]
fn a_stale_foreign_lease_is_taken_over_after_expiry() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());

    let registry = hieronymus::registry::Registry::open(&config).unwrap();
    registry
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    let source = root.path().join("lease.txt");
    std::fs::write(&source, "Lease contention text for the takeover probe.").unwrap();
    hieronymus::rag::RagStore::open(&config)
        .unwrap()
        .import_file("demo", &source, &hieronymus::rag::RagImport::new())
        .unwrap();
    let identity = TestArm::fast().identity();
    let store = SemanticStore::open(&config).unwrap();
    store.begin_generation("rebuild-lease", &identity).unwrap();
    let jobs = SemanticJobStore::open(&config).unwrap();
    let job = jobs.enqueue_rebuild("rebuild-lease", &identity).unwrap();

    // A foreign worker holds a short lease and makes no progress.
    jobs.claim_lease(&job.job_id, "foreign-worker:1", Duration::from_secs(1))
        .unwrap();

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut group = hiero::daemon::workers::WorkerGroup::new(Arc::clone(&stop));
    let controller = SemanticController::start_with(
        config.clone(),
        &mut group,
        TestArm::fast(),
        Box::new(|_| {}),
    )
    .unwrap();

    // The takeover completes once the foreign lease lapses.
    assert!(
        wait_until(
            || jobs
                .job(&job.job_id)
                .unwrap()
                .map(|job| job.status == "completed")
                .unwrap_or(false),
            Duration::from_secs(20)
        ),
        "the expired foreign lease must be reclaimed: {:?}",
        jobs.job(&job.job_id).unwrap()
    );
    assert!(
        wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(5)
        ),
        "the controller settles ready after the takeover: {:?}",
        controller.state()
    );
    assert!(store.active_generation().unwrap().is_some());

    // The supervised worker joins cleanly on the shared stop edge.
    group.stop_and_join().unwrap();
}

/// The empty-corpus job marker: requesting a rebuild with no documents is
/// ready-for-ingest, not a queued failure.
#[test]
fn an_empty_corpus_rebuild_request_is_a_no_op_marker() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut group = hiero::daemon::workers::WorkerGroup::new(Arc::clone(&stop));
    let controller = SemanticController::start_with(
        config.clone(),
        &mut group,
        TestArm::fast(),
        Box::new(|_| {}),
    )
    .unwrap();
    assert!(
        wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(10)
        ),
        "an armed controller over an empty corpus is ready: {:?}",
        controller.state()
    );
    let job_id = controller.request_rebuild("any-series").unwrap();
    assert_eq!(job_id, EMPTY_CORPUS_JOB_ID);
    group.stop_and_join().unwrap();
}

/// The queue seam is atomic: concurrent queue_semantic_rebuild calls over the
/// same corpus observe one another — exactly one fresh generation is queued,
/// every other caller dedups onto it. Two fresh generations from racing
/// callers would leave an orphaned building candidate behind.
#[test]
fn concurrent_queue_calls_dedup_onto_one_generation() {
    use hieronymus::semantic_recall::{QueueOutcome, queue_semantic_rebuild};

    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let registry = hieronymus::registry::Registry::open(&config).unwrap();
    registry
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    let source = root.path().join("race.txt");
    std::fs::write(
        &source,
        "Racing importers must never queue two generations.",
    )
    .unwrap();
    hieronymus::rag::RagStore::open(&config)
        .unwrap()
        .import_file("demo", &source, &hieronymus::rag::RagImport::new())
        .unwrap();

    let identity = TestArm::fast().identity();
    let mut callers = Vec::new();
    for _ in 0..8 {
        let config = config.clone();
        let identity = identity.clone();
        callers.push(std::thread::spawn(move || {
            queue_semantic_rebuild(&config, &identity).unwrap()
        }));
    }
    let outcomes: Vec<QueueOutcome> = callers
        .into_iter()
        .map(|caller| caller.join().unwrap())
        .collect();
    let enqueued: Vec<&String> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            QueueOutcome::Enqueued(job_id) => Some(job_id),
            _ => None,
        })
        .collect();
    let deduped: Vec<&String> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            QueueOutcome::AlreadyQueued(job_id) => Some(job_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        enqueued.len(),
        1,
        "exactly one caller may queue a fresh generation: {outcomes:?}"
    );
    assert_eq!(deduped.len(), 7, "every other caller dedups: {outcomes:?}");
    assert_eq!(enqueued[0], deduped[0]);

    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    let building: i64 = connection
        .query_row(
            "select count(*) from semantic_generations where status = 'building'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(building, 1, "no orphaned building candidate may survive");
}

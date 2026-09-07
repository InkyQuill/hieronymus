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
    ArmedPair, EMPTY_CORPUS_JOB_ID, ReadinessEvidence, RequiredSemanticState, SemanticArm,
    SemanticController, install_test_arm, readiness_from_evidence, require_semantic_ready,
};
use hiero::daemon::workers::WorkerGroup;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider};
use hieronymus::semantic_error::SemanticError;
use hieronymus::semantic_jobs::SemanticJobStore;
use hieronymus::semantic_recall::SemanticLane;
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

/// Task C3's regression (review finding A3): readiness is a claim that a
/// query can be answered, so a controller with no installed query lane can
/// never report `Ready` — not even over an empty corpus, where every other
/// signal says "nothing to do".
#[test]
fn ready_requires_an_installed_query_lane() {
    let evidence = ReadinessEvidence {
        query_installed: false,
        corpus_empty: true,
        generation_valid: false,
        rebuild_pending: false,
    };
    assert_ne!(
        readiness_from_evidence(&evidence),
        RequiredSemanticState::Ready
    );
}

/// The whole truth table of the pure decision core: every combination of the
/// four facts, so no branch can drift into an optimistic default.
#[test]
fn readiness_from_evidence_covers_every_combination() {
    for corpus_empty in [false, true] {
        for generation_valid in [false, true] {
            for rebuild_pending in [false, true] {
                let disarmed = ReadinessEvidence {
                    query_installed: false,
                    corpus_empty,
                    generation_valid,
                    rebuild_pending,
                };
                // No lane installed dominates everything else.
                assert!(
                    matches!(
                        readiness_from_evidence(&disarmed),
                        RequiredSemanticState::Failed(_)
                    ),
                    "{disarmed:?} must not pass"
                );

                let armed = ReadinessEvidence {
                    query_installed: true,
                    ..disarmed
                };
                let expected = if rebuild_pending {
                    // A queued or in-flight rebuild is reported as such even
                    // when an older generation still serves: strict callers
                    // must wait for the current target.
                    RequiredSemanticState::Rebuilding
                } else if corpus_empty || generation_valid {
                    RequiredSemanticState::Ready
                } else {
                    RequiredSemanticState::Failed("no valid current semantic generation".into())
                };
                assert_eq!(readiness_from_evidence(&armed), expected, "{armed:?}");
            }
        }
    }
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
    /// How many `arm()` calls succeed before every later one fails. `None`
    /// means all of them do. The worker arms twice per attempt — once for the
    /// indexing pair, once for the query lane — so `Some(1)` is exactly the
    /// C3 regression: the indexing lane loads, the query lane does not.
    successful_arms: Option<usize>,
    arms_seen: AtomicUsize,
}

impl TestArm {
    fn new() -> Self {
        Self {
            failures_remaining: 0,
            per_embed_delay: Duration::ZERO,
            permanent_failure: false,
            successful_arms: None,
            arms_seen: AtomicUsize::new(0),
        }
    }

    fn fast() -> Arc<dyn SemanticArm> {
        Arc::new(Self::new())
    }

    fn delayed(per_embed_delay: Duration) -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            per_embed_delay,
            ..Self::new()
        })
    }

    fn flaky(failures_remaining: usize) -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            failures_remaining,
            ..Self::new()
        })
    }

    fn failing() -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            permanent_failure: true,
            ..Self::new()
        })
    }

    /// Arms exactly `successful_arms` times, then refuses. Used to fail the
    /// query lane while the indexing lane succeeds.
    fn arming_only(successful_arms: usize) -> Arc<dyn SemanticArm> {
        Arc::new(Self {
            successful_arms: Some(successful_arms),
            ..Self::new()
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
        let seen = self.arms_seen.fetch_add(1, Ordering::SeqCst);
        if let Some(budget) = self.successful_arms
            && seen >= budget
        {
            return Err("scripted arm failure".to_string());
        }
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

// ------------------------------------------------- controller-level harness

/// A supervised controller with no HTTP surface, so a test can own the
/// query-lane install callback (the C3 readiness fact) directly.
fn start_controller(
    config: &HieronymusConfig,
    arm: Arc<dyn SemanticArm>,
    install: Box<dyn Fn(SemanticLane) -> Result<(), String> + Send>,
) -> (WorkerGroup, SemanticController) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut group = WorkerGroup::new(Arc::clone(&stop));
    let controller =
        SemanticController::start_with(config.clone(), &mut group, arm, install).unwrap();
    (group, controller)
}

/// An install callback that counts its calls and answers `Ok` for the first
/// `succeed_for` of them, then `Err`. `succeed_for = usize::MAX` never fails.
fn counting_install(
    calls: Arc<AtomicUsize>,
    succeed_for: usize,
) -> Box<dyn Fn(SemanticLane) -> Result<(), String> + Send> {
    Box::new(move |_lane| {
        let seen = calls.fetch_add(1, Ordering::SeqCst);
        if seen < succeed_for {
            Ok(())
        } else {
            Err("scripted install failure".to_string())
        }
    })
}

fn wait_for_controller(controller: &SemanticController, wanted: &RequiredSemanticState) -> bool {
    wait_until(|| &controller.state() == wanted, Duration::from_secs(20))
}

/// Seed an authoritative corpus (series plus one imported file) with no
/// daemon: the controller-level tests need real `rag_chunks` rows.
fn seed_offline_corpus(config: &HieronymusConfig, root: &Path, text: &str) {
    hieronymus::registry::Registry::open(config)
        .unwrap()
        .create_series("demo", "Demo", "ja", "en", None)
        .unwrap();
    let source = root.join("offline.txt");
    std::fs::write(&source, text).unwrap();
    hieronymus::rag::RagStore::open(config)
        .unwrap()
        .import_file("demo", &source, &hieronymus::rag::RagImport::new())
        .unwrap();
}

/// Hand-write an ACTIVE generation manifest row under `identity` with no
/// vector index behind it. This is exactly the shape a byte-fold-era
/// database, a model swap, or a wiped index directory leaves behind: a row
/// that exists and answers `active_generation()` while every query against it
/// is empty or wrong.
fn forge_active_generation(
    config: &HieronymusConfig,
    generation_id: &str,
    identity: &hieronymus::semantic_embeddings::EmbeddingIdentity,
) {
    SemanticStore::open(config)
        .unwrap()
        .begin_generation(generation_id, identity)
        .unwrap();
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute(
            "update semantic_generations set status = 'active', active = 1
             where generation_id = ?1",
            [generation_id],
        )
        .unwrap();
}

/// The identity of a byte-fold-era generation: every field of the running
/// identity except the tokenizer, which the `semantic_generations` migration
/// back-fills with `byte-fold-v1`. Different embeddings, same manifest shape.
fn byte_fold_variant(
    identity: &hieronymus::semantic_embeddings::EmbeddingIdentity,
) -> hieronymus::semantic_embeddings::EmbeddingIdentity {
    hieronymus::semantic_embeddings::EmbeddingIdentity::new(
        identity.provider(),
        identity.model(),
        identity.revision(),
        identity.dimensions(),
        identity.normalization(),
        hieronymus::semantic_embeddings::BYTE_FOLD_TOKENIZER_ID,
        identity.max_input_tokens(),
        identity.max_batch_inputs(),
    )
    .unwrap()
}

fn manifest_status(config: &HieronymusConfig, generation_id: &str) -> (String, bool) {
    let manifest = SemanticStore::open(config)
        .unwrap()
        .generation_manifest(generation_id)
        .unwrap()
        .expect("the manifest row survives invalidation");
    (manifest.status, manifest.active)
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
    // C3: a cancelled FIRST rebuild leaves nothing serving — no older
    // generation, a non-empty corpus — so the honest verdict is `failed`.
    // The pre-C3 loop reported `ready` here from the cancellation alone.
    let cancelled = wait_for_state(&daemon, "failed");
    assert!(
        cancelled
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("no valid current semantic generation"),
        "the cancellation must be reported as an uncovered corpus: {cancelled:?}"
    );
    assert!(require_semantic_ready(&cancelled.required()).is_err());

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

/// Two imports back to back: the second import moves the corpus revision past
/// what the first rebuild's candidate covers, so that candidate is durably
/// cancelled and one fresh whole-corpus rebuild covers both sources. (Task C4
/// replaced the frozen `expected_count` comparison this used to turn on; an
/// equal count is not coverage.)
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

    let (group, controller) = start_controller(&config, TestArm::fast(), Box::new(|_| Ok(())));

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
    let (group, controller) = start_controller(&config, TestArm::fast(), Box::new(|_| Ok(())));
    assert!(
        wait_for_controller(&controller, &RequiredSemanticState::Ready),
        "an armed controller over an empty corpus is ready: {:?}",
        controller.state()
    );
    let job_id = controller.request_rebuild("any-series").unwrap();
    assert_eq!(job_id, EMPTY_CORPUS_JOB_ID);
    group.stop_and_join().unwrap();
}

// ------------------------------------------- Task C3: readiness is evidence

/// The A3 regression at controller level: the indexing lane arms, the QUERY
/// lane does not. The pre-C3 worker dropped that second `Err` and went on to
/// advertise `Ready` with zero query-lane installations.
#[test]
fn a_query_lane_that_cannot_arm_is_never_ready() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let installs = Arc::new(AtomicUsize::new(0));
    // One successful arm: the indexing pair. The query pair is refused.
    let (group, controller) = start_controller(
        &config,
        TestArm::arming_only(1),
        counting_install(Arc::clone(&installs), usize::MAX),
    );

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(10)
        ),
        "an uninstalled query lane must fail: {:?}",
        controller.state()
    );
    assert_eq!(
        installs.load(Ordering::SeqCst),
        0,
        "no lane may reach the application when the query pair never armed"
    );
    // The corpus is empty, i.e. every other signal says ready-for-ingest.
    assert!(
        !wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(2)
        ),
        "readiness must never be reported without an installed lane: {:?}",
        controller.state()
    );
    let RequiredSemanticState::Failed(detail) = controller.state() else {
        panic!("failed state expected: {:?}", controller.state());
    };
    assert!(detail.contains("scripted arm failure"), "{detail}");

    group.stop_and_join().unwrap();
}

/// The application refusing the lane is the same readiness fact as a lane
/// that could not be armed: the controller reports its cause, not `Ready`.
#[test]
fn an_install_failure_is_never_ready() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let installs = Arc::new(AtomicUsize::new(0));
    let (group, controller) = start_controller(
        &config,
        TestArm::fast(),
        counting_install(Arc::clone(&installs), 0),
    );

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(10)
        ),
        "a refused lane must fail: {:?}",
        controller.state()
    );
    assert!(
        installs.load(Ordering::SeqCst) >= 1,
        "the install was tried"
    );
    let RequiredSemanticState::Failed(detail) = controller.state() else {
        panic!("failed state expected: {:?}", controller.state());
    };
    assert!(detail.contains("scripted install failure"), "{detail}");

    group.stop_and_join().unwrap();
}

/// A completed rebuild whose query-lane replacement fails is NOT ready: the
/// generation activated, but queries would still run on the superseded lane.
/// The pre-C3 code dropped this error and published `Ready`.
#[test]
fn a_failed_query_lane_replacement_after_a_rebuild_is_never_ready() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_offline_corpus(
        &config,
        root.path(),
        "The replacement lane must be installed before this generation counts as ready.",
    );
    let installs = Arc::new(AtomicUsize::new(0));
    // The first install (initial arming) succeeds; the post-activation
    // replacement does not.
    let (group, controller) = start_controller(
        &config,
        TestArm::fast(),
        counting_install(Arc::clone(&installs), 1),
    );

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(20)
        ),
        "a lost query lane must fail: {:?}",
        controller.state()
    );
    // The rebuild really did finish: this is not a rebuild failure being
    // mistaken for a lane failure.
    let store = SemanticStore::open(&config).unwrap();
    assert!(
        store.active_generation().unwrap().is_some(),
        "the rebuild activated a generation"
    );
    let RequiredSemanticState::Failed(detail) = controller.state() else {
        panic!("failed state expected: {:?}", controller.state());
    };
    assert!(detail.contains("scripted install failure"), "{detail}");

    group.stop_and_join().unwrap();
}

/// A manifest row is not evidence. A generation whose vector index did not
/// survive on disk is invalidated (it loses the active slot, it is not
/// relabelled) and rebuilt from the authoritative rows — which never left
/// SQLite, so this is recovery, not data loss.
#[test]
fn a_generation_whose_index_vanished_is_invalidated_and_rebuilt() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_offline_corpus(
        &config,
        root.path(),
        "The index directory vanished, but the authoritative chunks never did.",
    );
    forge_active_generation(&config, "stale-gen", &TestArm::fast().identity());
    assert!(
        !SemanticStore::open(&config)
            .unwrap()
            .active_generation_intact()
            .unwrap(),
        "the forged generation has no index behind it"
    );

    let (group, controller) = start_controller(&config, TestArm::fast(), Box::new(|_| Ok(())));

    assert!(
        wait_for_controller(&controller, &RequiredSemanticState::Ready),
        "the recovery rebuild settles ready: {:?}",
        controller.state()
    );
    let active = SemanticStore::open(&config)
        .unwrap()
        .active_generation()
        .unwrap()
        .expect("a rebuilt generation is active");
    assert_ne!(
        active.generation_id, "stale-gen",
        "the stale generation must be replaced, never relabelled"
    );
    assert_eq!(
        manifest_status(&config, "stale-gen"),
        ("failed".to_string(), false),
        "the invalidated generation is terminal and out of the active slot"
    );

    group.stop_and_join().unwrap();
}

/// The same invalidation with nothing able to rebuild: the verdict stays
/// `Failed` with the rebuild's own cause. Proves the stale generation is
/// never counted as serving while it waits.
#[test]
fn a_stale_generation_with_no_usable_rebuild_reports_failed() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_offline_corpus(
        &config,
        root.path(),
        "Nothing can rebuild this corpus, so nothing may report it ready.",
    );
    forge_active_generation(&config, "stale-gen", &TestArm::fast().identity());

    let (group, controller) = start_controller(&config, TestArm::failing(), Box::new(|_| Ok(())));

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(20)
        ),
        "an uncovered corpus must fail: {:?}",
        controller.state()
    );
    assert!(
        SemanticStore::open(&config)
            .unwrap()
            .active_generation()
            .unwrap()
            .is_none(),
        "the unusable generation left the active slot"
    );
    assert!(
        !wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(2)
        ),
        "a stale generation may never be reported ready: {:?}",
        controller.state()
    );

    group.stop_and_join().unwrap();
}

/// A byte-fold-era manifest (the `tokenizer` column back-filled with
/// `byte-fold-v1`) is a DIFFERENT embedding identity: its vectors cannot
/// answer this daemon's queries. Startup must not accept it as ready just
/// because the row exists.
#[test]
fn a_byte_fold_manifest_is_never_ready_at_startup() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed_offline_corpus(
        &config,
        root.path(),
        "Byte-fold vectors cannot answer model-tokenizer queries.",
    );
    let byte_fold = byte_fold_variant(&TestArm::fast().identity());
    forge_active_generation(&config, "byte-fold-gen", &byte_fold);
    assert_ne!(byte_fold, TestArm::fast().identity());

    let (group, controller) = start_controller(&config, TestArm::failing(), Box::new(|_| Ok(())));

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(20)
        ),
        "an identity-mismatched generation must fail: {:?}",
        controller.state()
    );
    assert_eq!(
        manifest_status(&config, "byte-fold-gen"),
        ("failed".to_string(), false),
        "the mismatched generation is invalidated, not relabelled"
    );
    assert!(
        !wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(2)
        ),
        "a foreign-identity generation may never be reported ready: {:?}",
        controller.state()
    );

    group.stop_and_join().unwrap();
}

/// An unreadable authoritative database is not an empty corpus and not a
/// quiet queue: the controller surfaces the store error instead of settling
/// on the ready-for-ingest branch.
#[test]
fn an_unreadable_store_reports_failed_with_its_cause() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let database = config.database_path();
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    std::fs::write(&database, b"this is emphatically not a SQLite database").unwrap();

    let (group, controller) = start_controller(&config, TestArm::fast(), Box::new(|_| Ok(())));

    assert!(
        wait_until(
            || matches!(controller.state(), RequiredSemanticState::Failed(_)),
            Duration::from_secs(10)
        ),
        "a corrupt store must fail: {:?}",
        controller.state()
    );
    let RequiredSemanticState::Failed(detail) = controller.state() else {
        panic!("failed state expected: {:?}", controller.state());
    };
    assert!(!detail.is_empty(), "the store error must be surfaced");
    assert!(
        !wait_until(
            || controller.state() == RequiredSemanticState::Ready,
            Duration::from_secs(2)
        ),
        "a corrupt store may never be reported ready: {:?}",
        controller.state()
    );

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

// ------------------------------- task C5: the public semantic RAG search

/// The accepted Rust delta for `hieronymus_rag_search` (review finding A5).
/// Loaded, not restated: the fixture is the reviewed record of the envelope
/// and the refusal, and this file proves the runtime matches it.
const RAG_SEARCH_V2: &str = include_str!("../../../compatibility/rust/rag-search-v2.json");

fn rag_search_expectation(state: &str, corpus: &str) -> Value {
    let fixture: Value = serde_json::from_str(RAG_SEARCH_V2).unwrap();
    assert_eq!(fixture["expectation_set"], json!("rag-search-v2"));
    fixture["expectations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["semantic_state"] == json!(state) && entry["corpus"] == json!(corpus))
        .unwrap_or_else(|| panic!("no rag-search-v2 expectation for {state}/{corpus}"))
        .clone()
}

/// A tool call whose failure is the point: returns the tool-error text.
fn call_tool_error(
    daemon: &hiero::daemon::Daemon,
    id: i64,
    name: &str,
    arguments: Value,
) -> String {
    let response = send_request(
        daemon.local_addr().port(),
        "POST",
        "/mcp",
        &mcp_headers(daemon, &[("Mcp-Method", "tools/call"), ("Mcp-Name", name)]),
        &serde_json::to_vec(&tools_call(id, name, arguments)).unwrap(),
    );
    assert_eq!(response.status, 200, "{:?}", response.raw_body);
    let body = response.body();
    assert_eq!(
        body["result"]["isError"],
        json!(true),
        "tool {name} was expected to fail: {body}"
    );
    body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn rag_search(daemon: &hiero::daemon::Daemon, id: i64, series: &str, query: &str) -> Vec<Value> {
    let payload = call_tool(
        daemon,
        id,
        "hieronymus_rag_search",
        json!({"series_slug": series, "query": query, "limit": 5}),
    );
    payload
        .as_array()
        .unwrap_or_else(|| panic!("rag search must answer a bare row array: {payload}"))
        .clone()
}

/// THE step-1 regression (task C5, review finding A5): a bare application
/// with no semantic service at all must refuse strict `hieronymus_rag_search`
/// instead of answering with the lexical FTS lane. Before C5 this call ran
/// `RagStore::search` directly and returned an ordinary successful array, so a
/// cold, unconfigured, or failed semantic runtime was indistinguishable from a
/// complete answer.
#[test]
fn rag_search_rejects_an_unavailable_required_semantic_service() {
    let root = tempfile::tempdir().unwrap();
    let app = hiero::application::Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book"}),
        "test",
    )
    .unwrap();
    let error = app
        .call(
            "hieronymus_rag_search",
            &json!({"series_slug": "book", "query": "physician", "limit": 3}),
            "test",
        )
        .unwrap_err();

    let expectation = rag_search_expectation("absent", "any");
    let needle = expectation["expected"]["error_contains"].as_str().unwrap();
    assert!(
        error.to_string().contains(needle),
        "the refusal must carry {needle:?}: {error}"
    );

    // Argument validation still runs first, so a malformed call keeps its own
    // diagnostic rather than being masked by the semantic gate.
    let invalid = app
        .call(
            "hieronymus_rag_search",
            &json!({"series_slug": "book", "query": "physician", "limit": 0}),
            "test",
        )
        .unwrap_err();
    assert!(
        invalid.to_string().contains("limit must be at least 1"),
        "{invalid}"
    );
}

/// A semantic service that exists but cannot serve is refused with its own
/// actionable reason — including over a series whose text IS indexed, which is
/// exactly the case where a lexical-only answer would look complete while the
/// paraphrase half of the corpus stayed unreachable.
#[test]
fn rag_search_refuses_a_semantic_service_that_is_not_ready() {
    let (root, daemon) = common::start_daemon_on_ephemeral_port();
    seed_series_and_session(&daemon);
    import(
        &daemon,
        root.path(),
        "chapter-1.txt",
        "The ship's physician bandaged the drowned sailor at dawn.",
    );
    let state = wait_for_state(&daemon, "failed");
    assert_eq!(state.state, "failed");

    let error = call_tool_error(
        &daemon,
        10,
        "hieronymus_rag_search",
        json!({"series_slug": "demo", "query": "physician", "limit": 5}),
    );
    let expectation = rag_search_expectation("failed", "indexed");
    let needle = expectation["expected"]["error_contains"].as_str().unwrap();
    assert!(
        error.contains(needle),
        "the refusal must carry {needle:?}: {error}"
    );
    // The reason is the service's own, not a generic placeholder.
    assert!(
        error.contains("hiero semantic enable --runtime"),
        "the refusal must stay actionable: {error}"
    );

    daemon.shutdown().unwrap();
    drop(root);
}

/// The one valid empty answer: everything loaded, nothing imported yet. The
/// controller reports ready-for-ingest, so zero rows is a complete answer, not
/// an error.
#[test]
fn rag_search_over_a_ready_empty_corpus_is_an_empty_success() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_semantic_daemon(root.path(), TestArm::fast());
    seed_series_and_session(&daemon);
    wait_for_state(&daemon, "ready");

    let rows = rag_search(&daemon, 11, "demo", "physician");
    let expectation = rag_search_expectation("ready", "empty");
    assert_eq!(
        expectation["expected"]["results"],
        json!([]),
        "the fixture pins the empty-success case"
    );
    assert!(rows.is_empty(), "{rows:?}");

    daemon.shutdown().unwrap();
}

/// The connected search: a ready service answers with the SAME hybrid
/// retrieval `hieronymus_recall` runs — a query with no lexical overlap still
/// finds the chunk through the semantic lane — with `rag` provenance, the
/// frozen row shape, and no foreign-series leak.
#[test]
fn rag_search_serves_semantic_rows_for_the_queried_series_only() {
    let root = tempfile::tempdir().unwrap();
    let daemon = start_semantic_daemon(root.path(), TestArm::fast());
    seed_series_and_session(&daemon);
    call_tool(
        &daemon,
        12,
        "hieronymus_series_create",
        json!({"slug": "ghost", "title": "Ghost", "source_language": "ja", "target_language": "en"}),
    );
    import(
        &daemon,
        root.path(),
        "chapter-1.txt",
        "The archivist catalogued every rumour of the drowned city before the tide returned.",
    );
    let foreign = write_source(
        root.path(),
        "foreign.txt",
        "A ledger from another series mentions the drowned city too.",
    );
    call_tool(
        &daemon,
        13,
        "hieronymus_rag_import",
        json!({"series_slug": "ghost", "path": foreign.to_str().unwrap()}),
    );
    wait_for_state(&daemon, "ready");

    let rows = rag_search(&daemon, 14, "demo", "zzqxj nonlexical probe");
    assert!(
        !rows.is_empty(),
        "a ready service must contribute real semantic results"
    );
    let expectation = rag_search_expectation("ready", "indexed");
    // The envelope and row shape are unchanged from the frozen Python
    // fixture: the accepted delta is what the rows MEAN, never their keys.
    let expected_keys: Vec<&str> = expectation["expected"]["row_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap())
        .collect();
    for row in &rows {
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut wanted = expected_keys.clone();
        keys.sort_unstable();
        wanted.sort_unstable();
        assert_eq!(keys, wanted, "row shape drifted: {row}");
        assert_eq!(row["source"], json!("rag"));
        assert!(
            row["source_ref"]
                .as_str()
                .unwrap_or_default()
                .contains("chapter-1.txt"),
            "foreign series must never leak into a series-scoped search: {row}"
        );
    }
    // Semantic provenance: the probe shares no term with the chunk, so the
    // only lane that could have surfaced it is the semantic one.
    assert!(
        rows.iter().any(|row| {
            row["rank_reason"].as_str()
                == Some(
                    expectation["expected"]["semantic_rank_reason"]
                        .as_str()
                        .unwrap(),
                )
                && row["text"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("drowned city")
        }),
        "semantic-matched rows must stay distinguishable: {rows:?}"
    );

    daemon.shutdown().unwrap();
}

/// Mixed recall is the other half of the contract: it never hard-fails on
/// missing semantics — memory rows and the deterministic contract still serve
/// — but it says so, every time. Before C5 an unarmed lane was silent.
#[test]
fn mixed_recall_warns_when_required_semantics_did_not_run() {
    let root = tempfile::tempdir().unwrap();
    let app = hiero::application::Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug": "book", "title": "Book"}),
        "test",
    )
    .unwrap();
    let session = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug": "book"}),
            "test",
        )
        .unwrap();
    let session_id = session["session_id"].as_i64().unwrap();
    app.call(
        "hieronymus_short_term_add",
        &json!({
            "session_id": session_id,
            "kind": "note",
            "text": "The physician keeps a ledger of the drowned.",
        }),
        "test",
    )
    .unwrap();

    let payload = app
        .call(
            "hieronymus_recall",
            &json!({
                "session_id": session_id,
                "series_slug": "book",
                "query": "physician ledger",
                "limit": 5,
            }),
            "test",
        )
        .expect("mixed recall keeps serving what it has");

    // The memory lane still answers.
    assert!(
        payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"].as_str().unwrap_or_default().contains("drowned")),
        "memory rows must still be served: {payload}"
    );
    // ...and the response admits the semantic half never ran.
    let warnings = payload["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|warning| warning["kind"].as_str()
            == Some(hieronymus::recall::WARNING_SEMANTIC_UNAVAILABLE)),
        "an absent semantic service must be reported: {payload}"
    );
    assert_eq!(
        warnings.len(),
        1,
        "the condition is reported once, not logged: {payload}"
    );
}

/// The complement: with a ready semantic service the warning must NOT appear,
/// or it would be noise nobody could act on.
#[test]
fn mixed_recall_over_ready_semantics_carries_no_unavailable_warning() {
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

    let payload = call_tool(
        &daemon,
        15,
        "hieronymus_recall",
        json!({
            "session_id": session_id,
            "series_slug": "demo",
            "query": "zzqxj nonlexical probe",
            "limit": 5,
        }),
    );
    assert!(
        payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| warning["kind"].as_str()
                != Some(hieronymus::recall::WARNING_SEMANTIC_UNAVAILABLE)),
        "a ready service must not warn: {payload}"
    );
    assert!(
        payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["rank_reason"].as_str() == Some("rag semantic match")),
        "the ready lane must actually contribute semantic rows: {payload}"
    );

    daemon.shutdown().unwrap();
}

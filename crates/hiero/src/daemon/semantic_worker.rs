//! The semantic controller (Task S2): supervised execution of durable
//! semantic rebuild jobs plus the required-readiness state the daemon's
//! recall lane reports.
//!
//! Ownership follows the R5 worker model: [`SemanticController::start_with`]
//! registers one worker thread on the caller's [`WorkerGroup`]. Everything
//! non-`Send` about arming (the ONNX session) stays inside the structures
//! that own it — the worker keeps its own provider pair, and the query-time
//! lane installed into the application wraps its pair in the lane's existing
//! mutex boundary. No session is ever shared unsynchronized.
//!
//! Job discipline (design §Durable Jobs):
//! - the worker reconciles crash residue (expired leases, terminal
//!   generations) before claiming new work, on startup and on every poll
//!   tick, so a missed in-memory wakeup recovers from SQLite alone;
//! - rebuilds run through [`SemanticJobStore::run_rebuild`] verbatim: bounded
//!   batches, lease/generation checks, no SQLite transaction spanning
//!   inference or LanceDB I/O;
//! - the controller's state machine (`Acquiring`/`Rebuilding`/`Ready`/
//!   `Failed`) is what `/status` serves and what
//!   [`require_semantic_ready`] gates strict callers on. A missing runtime,
//!   model, or tokenizer is an actionable `Failed`, never a silent
//!   "semantic enabled"; an empty corpus with everything loaded is `Ready`
//!   (ready-for-ingest), never an error.
//!
//! Readiness discipline (Task C3, review finding A3): `Ready` is a claim
//! about what a `hieronymus_recall` call would actually get back, so it is
//! decided in exactly ONE place — [`readiness_from_evidence`] over
//! [`ReadinessEvidence`] gathered from real service state on every tick. The
//! worker never publishes `Ready` from whichever branch it happened to reach.
//! Three concrete ways the pre-C3 loop lied, each now closed by an evidence
//! field:
//!
//! - it armed the indexing lane, dropped the `Err` of the *second* (query)
//!   arm, and went on to advertise `Ready` with zero query-lane
//!   installations (`query_installed`);
//! - it treated a cancelled first rebuild as `Ready` without any older
//!   generation to serve from (`generation_valid` / `corpus_empty`);
//! - it accepted a manifest row that merely *existed* — a byte-fold-era
//!   identity, a model swap, an index directory deleted underneath the
//!   daemon — as an active generation (`generation_valid`).
//!
//! Store, queue, sample and lane-install errors are published as `Failed`
//! with their cause rather than swallowed: a degraded service reports its
//! limits truthfully, and both memory and semantic RAG are mandatory, so a
//! false `Ready` here would defeat the R4 update gate that consumes this very
//! state through `/status`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::semantic_arming::{load_runtime_library, save_runtime_library};
use hieronymus::semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider};
use hieronymus::semantic_jobs::{
    AuthoritativeChunk, ChunkTokenizer, JobOutcome, RebuildConfig, RebuildInputs, SemanticJobStore,
};
use hieronymus::semantic_recall::{SemanticLane, queue_semantic_rebuild};
use hieronymus::semantic_store::{SemanticSample, SemanticStore};
use rusqlite::OptionalExtension;

use super::workers::WorkerGroup;

/// The required (non-degraded) semantic readiness states. `Ready` means the
/// lane is armed and a verified generation is active — or the corpus is empty
/// and everything is loaded, i.e. ready-for-ingest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequiredSemanticState {
    Acquiring,
    Rebuilding,
    Ready,
    Failed(String),
}

/// The strict gate: strict RAG callers and release/update health must refuse
/// to count a disarmed lane as ready. An FTS-only lane never passes.
pub fn require_semantic_ready(state: &RequiredSemanticState) -> Result<(), String> {
    match state {
        RequiredSemanticState::Ready => Ok(()),
        RequiredSemanticState::Acquiring => Err("semantic assets are still being acquired".into()),
        RequiredSemanticState::Rebuilding => Err("semantic indexing is still in progress".into()),
        RequiredSemanticState::Failed(reason) => {
            Err(format!("semantic retrieval unavailable: {reason}"))
        }
    }
}

/// The four facts the readiness decision consumes, each one read from real
/// service state rather than inferred from a previous verdict.
///
/// Why this exists as a struct plus a pure function: readiness used to be set
/// from whichever branch of the worker loop ran last, which is how a worker
/// with a failed query-lane installation, a cancelled first rebuild, or a
/// stale manifest row all reached `Ready`. Collecting the facts first and
/// deciding once makes every `Ready` traceable to the evidence that justified
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadinessEvidence {
    /// A query-time [`SemanticLane`] was armed AND accepted by the
    /// application. Without it there is nothing to answer a query with, no
    /// matter how healthy the index on disk is.
    pub query_installed: bool,
    /// The authoritative corpus holds no chunks, so there is nothing to
    /// index: ready-for-ingest. Read from the same `rag_chunks` count
    /// `queue_semantic_rebuild` uses for
    /// [`hieronymus::semantic_recall::QueueOutcome::EmptyCorpus`] — never
    /// from a failed read, which is `Failed`, not "empty".
    pub corpus_empty: bool,
    /// The active generation exists, was built under the embedding identity
    /// this controller runs, and its vector index survived on disk. Manifest
    /// existence alone is explicitly NOT enough.
    pub generation_valid: bool,
    /// A durable rebuild job is queued or in flight for the current target,
    /// so the current corpus is not covered yet.
    pub rebuild_pending: bool,
}

/// The pure readiness decision: the single place `Ready` can be produced.
///
/// Order matters. A missing query lane dominates every other fact — an index
/// nobody can query is not a service. A pending rebuild is reported as such
/// even when an older generation still answers, because strict callers gate
/// on the *current* corpus being covered. Only then is `Ready` allowed, and
/// only for the two states that can honestly serve: an empty corpus
/// (ready-for-ingest) or a verified current generation.
pub fn readiness_from_evidence(evidence: &ReadinessEvidence) -> RequiredSemanticState {
    if !evidence.query_installed {
        return RequiredSemanticState::Failed("query lane is not installed".into());
    }
    if evidence.rebuild_pending {
        return RequiredSemanticState::Rebuilding;
    }
    if evidence.corpus_empty || evidence.generation_valid {
        RequiredSemanticState::Ready
    } else {
        RequiredSemanticState::Failed("no valid current semantic generation".into())
    }
}

/// One armed provider/tokenizer pair. Document (rebuild) and query (lane)
/// paths each own their pair; both resolve to the same identity by
/// construction.
pub struct ArmedPair {
    pub provider: Box<dyn EmbeddingProvider>,
    pub tokenizer: Box<dyn ChunkTokenizer>,
}

/// The arming seam: how the daemon turns a data root into provider/tokenizer
/// pairs. Production uses [`OnnxArm`] over the persisted runtime location;
/// integration tests inject deterministic fakes through
/// [`install_test_arm`].
pub trait SemanticArm: Send + Sync {
    /// The identity generations are queued under (cheap; never loads the
    /// runtime or model).
    fn identity(&self) -> EmbeddingIdentity;
    /// Cheap pre-checks (runtime file, model/tokenizer presence) with an
    /// actionable message. Never loads anything heavy.
    fn precheck(&self, config: &HieronymusConfig) -> Result<(), String>;
    /// Loads a provider/tokenizer pair. Fails closed with an actionable
    /// reason; never panics.
    fn arm(&self, config: &HieronymusConfig) -> Result<ArmedPair, String>;
}

/// Production arm: the pinned ONNX model plus the S1 tokenizer, loaded
/// through the verified store path against an explicitly configured runtime
/// library.
pub struct OnnxArm {
    runtime: PathBuf,
}

impl OnnxArm {
    pub fn new(runtime: PathBuf) -> Self {
        Self { runtime }
    }
}

impl SemanticArm for OnnxArm {
    fn identity(&self) -> EmbeddingIdentity {
        hieronymus::semantic_embeddings::OnnxEmbeddingProvider::static_identity()
    }

    fn precheck(&self, config: &HieronymusConfig) -> Result<(), String> {
        if !self.runtime.is_file() {
            return Err(format!(
                "onnx runtime library {} does not exist; run `hiero semantic enable --runtime \
                 <lib>` with the runtime shared library",
                self.runtime.display()
            ));
        }
        let store = SemanticStore::open(config).map_err(|error| error.to_string())?;
        use hieronymus::semantic_model::ModelStatus;
        match store.model_status() {
            ModelStatus::Available => {}
            ModelStatus::Missing => {
                return Err(format!(
                    "no embedding model acquired at {}; run `hiero semantic enable`",
                    store.model_path().display()
                ));
            }
            ModelStatus::Invalid(reason) => return Err(reason),
        }
        match store.tokenizer_status() {
            ModelStatus::Available => Ok(()),
            ModelStatus::Missing => Err(format!(
                "no tokenizer asset acquired at {}; run `hiero semantic enable` (pinned-default \
                 mode) to fetch it alongside the model",
                store.tokenizer_path().display()
            )),
            ModelStatus::Invalid(reason) => Err(reason),
        }
    }

    fn arm(&self, config: &HieronymusConfig) -> Result<ArmedPair, String> {
        let store = SemanticStore::open(config).map_err(|error| error.to_string())?;
        let provider = store
            .load_embedding_provider(&self.runtime)
            .map_err(|error| error.to_string())?;
        let tokenizer = store
            .load_model_tokenizer()
            .map_err(|error| error.to_string())?;
        Ok(ArmedPair {
            provider: Box::new(provider),
            tokenizer: Box::new(tokenizer),
        })
    }
}

/// The unconfigured arm: no runtime location was persisted. Queues jobs under
/// the pinned identity (so a later-configured daemon rebuilds them) but never
/// arms, keeping the state honestly `Failed`.
struct UnconfiguredArm;

impl SemanticArm for UnconfiguredArm {
    fn identity(&self) -> EmbeddingIdentity {
        hieronymus::semantic_embeddings::OnnxEmbeddingProvider::static_identity()
    }

    fn precheck(&self, _config: &HieronymusConfig) -> Result<(), String> {
        Err(
            "onnx runtime library is not configured; run `hiero semantic enable --runtime <lib>` \
             once to persist its location"
                .to_string(),
        )
    }

    fn arm(&self, config: &HieronymusConfig) -> Result<ArmedPair, String> {
        Err(self
            .precheck(config)
            .expect_err("the unconfigured arm never prechecks clean"))
    }
}

/// Persist the runtime location for this data root (the `hiero semantic
/// enable --runtime <lib>` write). Exposed here so callers only need the one
/// module.
pub fn persist_runtime_library(config: &HieronymusConfig, runtime: &Path) -> Result<(), String> {
    save_runtime_library(config, runtime)
}

/// The per-data-root test seam: integration tests register a deterministic
/// arm for exactly their data root, so parallel daemons in one test binary
/// never observe each other's injection. Production data roots never appear
/// here; `resolve_arm` falls back to the persisted runtime configuration.
static TEST_ARMS: Mutex<Option<HashMap<PathBuf, Arc<dyn SemanticArm>>>> = Mutex::new(None);

/// Register a test arm for one data root (integration tests only).
#[doc(hidden)]
pub fn install_test_arm(data_root: &Path, arm: Arc<dyn SemanticArm>) {
    TEST_ARMS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(data_root.to_path_buf(), arm);
}

/// The arm one daemon instance runs with: the test seam for its data root
/// when registered, otherwise the persisted ONNX runtime configuration.
pub(crate) fn resolve_arm(config: &HieronymusConfig) -> Arc<dyn SemanticArm> {
    let registered = TEST_ARMS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|arms| arms.get(config.data_root()).cloned());
    if let Some(arm) = registered {
        return arm;
    }
    match load_runtime_library(config) {
        Some(runtime) => Arc::new(OnnxArm::new(runtime)),
        None => Arc::new(UnconfiguredArm),
    }
}

/// The durable job id returned when a rebuild was requested against an empty
/// corpus: nothing to index, the lane stays ready-for-ingest.
pub const EMPTY_CORPUS_JOB_ID: &str = "rebuild:empty-corpus";

/// How long the worker sleeps between poll ticks when no wakeup arrives.
const WORKER_POLL: Duration = Duration::from_millis(500);
/// How often a disarmed worker retries its (cheap) pre-checks.
const REARM_POLL: Duration = Duration::from_secs(5);

struct ControllerInner {
    config: HieronymusConfig,
    identity: EmbeddingIdentity,
    state: Mutex<RequiredSemanticState>,
    wake: std::sync::mpsc::Sender<()>,
}

/// Handle to the supervised semantic worker. Cheap to clone; every clone
/// observes the same state and can wake the worker.
#[derive(Clone)]
pub struct SemanticController {
    inner: Arc<ControllerInner>,
}

impl std::fmt::Debug for SemanticController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SemanticController")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl SemanticController {
    /// Production entry point (the S2 interface): start the worker over the
    /// pinned ONNX model and the given runtime library under the daemon's
    /// worker group.
    pub fn start(
        config: HieronymusConfig,
        runtime: PathBuf,
        workers: &mut WorkerGroup,
    ) -> Result<Self, String> {
        Self::start_with(
            config,
            workers,
            Arc::new(OnnxArm::new(runtime)),
            Box::new(|_lane| Ok(())),
        )
    }

    /// The injection point the daemon (and integration tests) use: an
    /// explicit arm plus the callback that installs a query-time lane into
    /// the application's recall service. Called once when the lane first arms
    /// and again on every verified generation activation, so queries always
    /// run on a coherent identity and generation.
    ///
    /// The callback returns a `Result` because its failure is a readiness
    /// fact, not a log line: a lane the application refused to accept means
    /// queries still run on the old (or no) lane, which is
    /// `ReadinessEvidence::query_installed == false` and therefore `Failed`.
    pub fn start_with(
        config: HieronymusConfig,
        workers: &mut WorkerGroup,
        arm: Arc<dyn SemanticArm>,
        install: Box<dyn Fn(SemanticLane) -> Result<(), String> + Send>,
    ) -> Result<Self, String> {
        let identity = arm.identity();
        let initial = match arm.precheck(&config) {
            Ok(()) => RequiredSemanticState::Acquiring,
            Err(reason) => RequiredSemanticState::Failed(reason),
        };
        let (wake, wake_rx) = std::sync::mpsc::channel::<()>();
        let inner = Arc::new(ControllerInner {
            identity,
            state: Mutex::new(initial),
            wake,
            config,
        });
        let worker_inner = Arc::clone(&inner);
        workers.spawn(Box::new(move |stop| {
            run_worker(stop, worker_inner, arm, install, wake_rx);
        }))?;
        Ok(Self { inner })
    }

    /// Queue a whole-corpus rebuild after an authoritative commit (RAG
    /// import). Returns the durable job id; a miss of the in-memory wakeup is
    /// recovered by startup/periodic reconciliation because the queueing
    /// itself is durable SQLite state.
    pub fn request_rebuild(&self, _series: &str) -> Result<String, String> {
        let outcome = queue_semantic_rebuild(&self.inner.config, &self.inner.identity)
            .map_err(|error| error.to_string())?;
        match outcome {
            hieronymus::semantic_recall::QueueOutcome::Enqueued(job_id)
            | hieronymus::semantic_recall::QueueOutcome::AlreadyQueued(job_id) => {
                // The queueing itself just made `rebuild_pending` true, so
                // close the window before the worker's next tick observes
                // it. This can only ever downgrade `Ready` — it never
                // upgrades anything, so the worker stays the sole source of
                // `Ready`.
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *state == RequiredSemanticState::Ready {
                    *state = RequiredSemanticState::Rebuilding;
                }
                drop(state);
                let _ = self.inner.wake.send(());
                Ok(job_id)
            }
            hieronymus::semantic_recall::QueueOutcome::EmptyCorpus => {
                Ok(EMPTY_CORPUS_JOB_ID.to_string())
            }
        }
    }

    /// The state `/status` serves and strict readiness gates consume.
    pub fn state(&self) -> RequiredSemanticState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn set_state(inner: &ControllerInner, state: RequiredSemanticState) {
    *inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
}

/// Publish the verdict [`readiness_from_evidence`] reached, optionally
/// carrying the actionable *cause* instead of the pure function's generic
/// symptom. `detail` can only ever refine a `Failed` verdict — it can never
/// turn a non-ready verdict into a ready one, which is what keeps
/// [`readiness_from_evidence`] the single gate.
fn publish(inner: &ControllerInner, verdict: RequiredSemanticState, detail: Option<&str>) {
    let state = match (&verdict, detail) {
        (RequiredSemanticState::Failed(_), Some(detail)) => {
            RequiredSemanticState::Failed(detail.to_string())
        }
        _ => verdict,
    };
    set_state(inner, state);
}

/// Publish the evidence's verdict and keep the remembered cause honest: a
/// service that actually serves has no outstanding cause, so `Ready` clears
/// it. Without that, a long-resolved job error would later be attached to an
/// unrelated failure.
fn publish_settling(
    inner: &ControllerInner,
    evidence: &ReadinessEvidence,
    detail: &mut Option<String>,
) {
    let verdict = readiness_from_evidence(evidence);
    if verdict == RequiredSemanticState::Ready {
        *detail = None;
    }
    publish(inner, verdict, detail.as_deref());
}

/// Publish an actionable failure and remember its cause for the ticks that
/// follow: the gathered evidence can only ever report the *symptom* ("no
/// valid current semantic generation"), never why the service got there.
fn fail(inner: &ControllerInner, detail: &mut Option<String>, reason: String) {
    set_state(inner, RequiredSemanticState::Failed(reason.clone()));
    *detail = Some(reason);
}

struct WorkerContext {
    inner: Arc<ControllerInner>,
    arm: Arc<dyn SemanticArm>,
    install: Box<dyn Fn(SemanticLane) -> Result<(), String> + Send>,
    wake_rx: std::sync::mpsc::Receiver<()>,
}

fn run_worker(
    stop: Arc<AtomicBool>,
    inner: Arc<ControllerInner>,
    arm: Arc<dyn SemanticArm>,
    install: Box<dyn Fn(SemanticLane) -> Result<(), String> + Send>,
    wake_rx: std::sync::mpsc::Receiver<()>,
) {
    let context = WorkerContext {
        inner,
        arm,
        install,
        wake_rx,
    };
    // The worker's own facts, i.e. everything a database read cannot see.
    // `pair` is the indexing pair; `query_installed` records that the *query*
    // lane armed and was accepted. The two are set together and cleared
    // together, so a published `Ready` can never outlive the lane that has to
    // answer for it.
    let mut pair: Option<ArmedPair> = None;
    let mut query_installed = false;
    // The actionable cause behind the current non-ready verdict, carried
    // across ticks until the service actually serves.
    let mut failure_detail: Option<String> = None;
    // Whether the one-shot startup recovery already ran. The durable queue is
    // what survives a restart, so re-queueing every tick would undo an
    // operator's cancellation.
    let mut recovery_attempted = false;
    // The exception to that: a generation this worker had to invalidate
    // uncovers the corpus through no operator action, so a recovery rebuild is
    // owed. Held until it is actually queued, because the invalidation may be
    // observed on a tick that cannot queue yet.
    let mut recovery_owed = false;
    let mut rearm_deadline = std::time::Instant::now();
    let stop_flag = Arc::clone(&stop);
    let rebuild = RebuildConfig {
        stop_check: Some(Arc::new(move || stop_flag.load(Ordering::Acquire))),
        ..RebuildConfig::default()
    };
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        // 1. Durable recovery first: reconcile crash residue (expired leases,
        //    terminal generations) before claiming anything new. This is also
        //    what recovers a queued job whose in-memory wakeup was missed.
        //    Opening the store here is what guarantees the `semantic_jobs`
        //    table the evidence gathering below reads. A store that cannot be
        //    opened or reconciled is a hard failure, not a quiet `continue`:
        //    nothing downstream could be trusted anyway.
        let jobs = match SemanticJobStore::open(&context.inner.config) {
            Ok(jobs) => jobs,
            Err(error) => {
                fail(
                    &context.inner,
                    &mut failure_detail,
                    format!("the durable semantic job store is unusable: {error}"),
                );
                wait_for_wakeup(&context, &stop);
                continue;
            }
        };
        if let Err(error) = jobs.reconcile() {
            fail(
                &context.inner,
                &mut failure_detail,
                format!("durable semantic job reconciliation failed: {error}"),
            );
            wait_for_wakeup(&context, &stop);
            continue;
        }

        // 2. Arming (with periodic retry: acquiring the assets later must
        //    heal the Failed state without a restart). BOTH lanes have to
        //    load and the query lane has to be accepted — see
        //    [`arm_both_lanes`].
        if pair.is_none() && rearm_deadline <= std::time::Instant::now() {
            set_state(&context.inner, RequiredSemanticState::Acquiring);
            match arm_both_lanes(&context) {
                Ok(indexing) => {
                    pair = Some(indexing);
                    query_installed = true;
                    failure_detail = None;
                }
                Err(reason) => {
                    query_installed = false;
                    fail(&context.inner, &mut failure_detail, reason);
                    rearm_deadline = std::time::Instant::now() + REARM_POLL;
                }
            }
        }

        // 3. Nothing armed: there is no lane to answer a query with, so the
        //    verdict is the not-installed one with its acquisition cause.
        //    Retry on the re-arm cadence.
        if pair.is_none() {
            publish(
                &context.inner,
                readiness_from_evidence(&ReadinessEvidence::default()),
                failure_detail.as_deref(),
            );
            wait_for_wakeup(&context, &stop);
            continue;
        }

        // 4. Evidence: what this service could actually answer with right
        //    now, read fresh every tick so a stale `Ready` cannot survive.
        let gathered = match gather_evidence(&context.inner, query_installed) {
            Ok(gathered) => gathered,
            Err(reason) => {
                fail(&context.inner, &mut failure_detail, reason);
                wait_for_wakeup(&context, &stop);
                continue;
            }
        };
        let mut evidence = gathered.evidence;
        if let Some(reason) = gathered.invalidated {
            failure_detail = Some(reason);
            recovery_owed = true;
        }

        // 5. Recovery queueing: a non-empty corpus with no valid generation
        //    and nothing queued needs a durable rebuild. Once at startup, and
        //    again for every generation this worker had to invalidate.
        if !evidence.corpus_empty
            && !evidence.generation_valid
            && !evidence.rebuild_pending
            && (!recovery_attempted || recovery_owed)
        {
            match queue_semantic_rebuild(&context.inner.config, &context.inner.identity) {
                Ok(
                    hieronymus::semantic_recall::QueueOutcome::Enqueued(_)
                    | hieronymus::semantic_recall::QueueOutcome::AlreadyQueued(_),
                ) => {
                    evidence.rebuild_pending = true;
                    recovery_owed = false;
                }
                // The corpus emptied between the count and the queueing;
                // ready-for-ingest, never an error.
                Ok(hieronymus::semantic_recall::QueueOutcome::EmptyCorpus) => {
                    evidence.corpus_empty = true;
                    recovery_owed = false;
                }
                Err(error) => {
                    fail(
                        &context.inner,
                        &mut failure_detail,
                        format!("queueing the semantic recovery rebuild failed: {error}"),
                    );
                    wait_for_wakeup(&context, &stop);
                    continue;
                }
            }
        }
        recovery_attempted = true;

        // 6. Publish before the (possibly long) rebuild, then drive whatever
        //    the durable queue offers.
        publish_settling(&context.inner, &evidence, &mut failure_detail);
        let pass = match pair.as_mut() {
            Some(armed) => drive_claimable_jobs(&context, armed, &jobs, &rebuild, &stop),
            // Unreachable: step 3 ended the tick when nothing is armed.
            None => DrivePass::default(),
        };

        // 7. Fold what the pass learned back into the worker's own facts.
        if pass.completed {
            failure_detail = None;
        }
        // Kept because step 8 must re-settle for a failure that was raised
        // *before* any job could be claimed (an unreadable queue), not only
        // for one raised while driving.
        let pass_failed = pass.failure.is_some();
        if let Some(reason) = pass.failure {
            failure_detail = Some(reason);
        }
        if let Some(reason) = pass.lane_lost {
            // A lane the application would not take means queries run on a
            // superseded lane (or none). Drop the pair so the next re-arm
            // rebuilds BOTH lanes rather than calling this ready.
            pair = None;
            query_installed = false;
            failure_detail = Some(reason);
            rearm_deadline = std::time::Instant::now() + REARM_POLL;
        }

        // 8. Re-settle from evidence after the pass changed anything: the
        //    verdict must describe the state the pass actually left behind,
        //    never the outcome a branch hoped for. (This is where a
        //    cancellation is judged: an older valid generation or an empty
        //    corpus is `Ready`, anything else is not.) A failure counts even
        //    when no job was claimed, so it surfaces on this tick rather than
        //    waiting out a poll interval behind the step-6 verdict.
        if !query_installed {
            // The pass lost the lane; no database read can change that
            // verdict, so do not spend one.
            publish(
                &context.inner,
                readiness_from_evidence(&ReadinessEvidence::default()),
                failure_detail.as_deref(),
            );
        } else if pass.drove_a_job || pass_failed {
            match gather_evidence(&context.inner, query_installed) {
                Ok(gathered) => {
                    if let Some(reason) = gathered.invalidated {
                        failure_detail = Some(reason);
                        recovery_owed = true;
                    }
                    publish_settling(&context.inner, &gathered.evidence, &mut failure_detail);
                }
                Err(reason) => fail(&context.inner, &mut failure_detail, reason),
            }
        }

        wait_for_wakeup(&context, &stop);
    }
}

/// Arm the indexing pair, arm a second pair for the query lane, and install
/// that lane into the application. All three steps propagate: readiness is a
/// claim that a query can be answered, so the *query* pair is what has to
/// load and be accepted — an armed indexing pair on its own proves nothing.
///
/// The pre-C3 code called `arm` a second time inside `if let Ok(..)` and
/// dropped the `Err`, so a provider that loaded for indexing but not for
/// querying produced a worker with zero query-lane installations that still
/// advertised `Ready`.
fn arm_both_lanes(context: &WorkerContext) -> Result<ArmedPair, String> {
    let indexing = context.arm.arm(&context.inner.config)?;
    install_query_lane(context)?;
    Ok(indexing)
}

/// Arm a fresh query pair and hand it to the application. Called on first
/// arming and again on every verified generation activation, so queries never
/// run against a superseded generation.
fn install_query_lane(context: &WorkerContext) -> Result<(), String> {
    let query = context
        .arm
        .arm(&context.inner.config)
        .map_err(|reason| format!("the semantic query lane could not be armed: {reason}"))?;
    (context.install)(SemanticLane::new(query.provider, query.tokenizer))
        .map_err(|reason| format!("the semantic query lane could not be installed: {reason}"))
}

/// What one job-driving pass changed about the worker's own facts. Everything
/// a database read can see is re-gathered afterwards instead of being
/// reported from here — this only carries what evidence cannot observe.
#[derive(Default)]
struct DrivePass {
    /// A job was claimed and run, so the tick must re-gather evidence.
    drove_a_job: bool,
    /// The actionable reason a job could not finish.
    failure: Option<String>,
    /// The query lane could not be replaced onto the freshly activated
    /// generation.
    lane_lost: Option<String>,
    /// A job completed and activated a generation.
    completed: bool,
}

/// Claim and drive the durable queue one job at a time. Deliberately silent
/// about readiness beyond the in-flight `Rebuilding` marker: the caller
/// re-derives the verdict from evidence once the pass is over.
fn drive_claimable_jobs(
    context: &WorkerContext,
    armed: &mut ArmedPair,
    jobs: &SemanticJobStore,
    rebuild: &RebuildConfig,
    stop: &AtomicBool,
) -> DrivePass {
    let mut pass = DrivePass::default();
    let claimable = match jobs.claimable_jobs() {
        Ok(claimable) => claimable,
        Err(error) => {
            pass.failure = Some(format!(
                "the durable semantic job queue is unreadable: {error}"
            ));
            return pass;
        }
    };
    for job_id in claimable {
        if stop.load(Ordering::Acquire) {
            break;
        }
        // A claimed job is in flight for the whole run; say so rather than
        // leaving the previous verdict standing over it.
        set_state(&context.inner, RequiredSemanticState::Rebuilding);
        pass.drove_a_job = true;
        let sample = match activation_sample(&context.inner.config, armed.tokenizer.as_mut()) {
            Ok(Some(sample)) => sample,
            // The corpus emptied underneath us: there is nothing to activate
            // against, and reconciliation settles the job on a later tick.
            Ok(None) => break,
            Err(reason) => {
                pass.failure = Some(reason);
                break;
            }
        };
        let result = jobs.run_rebuild(
            &job_id,
            RebuildInputs {
                provider: armed.provider.as_mut(),
                tokenizer: armed.tokenizer.as_mut(),
                sample,
            },
            rebuild,
        );
        match result {
            Ok(JobOutcome::Completed { .. }) => {
                pass.completed = true;
                pass.failure = None;
                // Verified activation: the query lane must move onto the new
                // generation, and failing to move it is a failure — the
                // pre-C3 code dropped that error and reported `Ready`.
                if let Err(reason) = install_query_lane(context) {
                    pass.lane_lost = Some(reason);
                    break;
                }
            }
            Ok(JobOutcome::Cancelled { .. }) => {
                // Nothing is fabricated here. Whether the service can still
                // serve — an older valid generation, or an empty corpus — is
                // decided by the evidence gathered after this pass, never by
                // the cancellation itself.
            }
            Ok(JobOutcome::Failed { error, .. }) => pass.failure = Some(error),
            Ok(JobOutcome::Busy { .. } | JobOutcome::LeaseLost { .. }) => {}
            Err(error) => pass.failure = Some(error.to_string()),
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
    }
    pass
}

/// Block until a wakeup arrives, the poll tick elapses, or cancellation is
/// observed.
fn wait_for_wakeup(context: &WorkerContext, _stop: &AtomicBool) {
    // recv_timeout bounds the wait regardless of the stop flag.
    match context.wake_rx.recv_timeout(WORKER_POLL) {
        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
    }
}

/// The gathered evidence plus the one thing the evidence itself cannot
/// express: that an unusable active generation had to be invalidated on this
/// tick.
struct GatheredEvidence {
    evidence: ReadinessEvidence,
    /// Why the active generation was invalidated, when it was. The corpus is
    /// now uncovered through no operator action, so the caller owes it a
    /// recovery rebuild even after the startup one already ran.
    invalidated: Option<String>,
}

/// Read the four readiness facts from real state.
///
/// Every hard read error is an `Err`, never a convenient `false`: an
/// unreadable database is not an empty corpus and not a quiet queue, and
/// papering over it is exactly how a broken store used to read as `Ready`.
/// The caller publishes the `Err` as `Failed` with its cause.
fn gather_evidence(
    inner: &ControllerInner,
    query_installed: bool,
) -> Result<GatheredEvidence, String> {
    let (chunk_count, rebuild_pending) = corpus_and_queue(&inner.config)?;
    let (generation_valid, invalidated) = assess_and_invalidate_active_generation(inner)?;
    Ok(GatheredEvidence {
        evidence: ReadinessEvidence {
            query_installed,
            corpus_empty: chunk_count == 0,
            generation_valid,
            rebuild_pending,
        },
        invalidated,
    })
}

/// The authoritative chunk count and whether a rebuild job is queued or in
/// flight, on one connection. `semantic_jobs` exists because the tick opened
/// the durable job store before gathering.
///
/// Every live job counts, not only one matching the running identity. That is
/// deliberate: `queue_semantic_rebuild` keeps exactly one rebuild in flight
/// and reconciliation terminates jobs whose generation went terminal, so a
/// live row means the current corpus is not covered yet — and over-reporting
/// `rebuild_pending` errs toward "not ready", which is the only safe
/// direction here.
fn corpus_and_queue(config: &HieronymusConfig) -> Result<(i64, bool), String> {
    let connection = hieronymus::db::open_migrated(&config.database_path())
        .map_err(|error| format!("the authoritative database is unreadable: {error}"))?;
    let chunk_count: i64 = connection
        .query_row("select count(*) from rag_chunks", [], |row| row.get(0))
        .map_err(|error| format!("the authoritative chunk count is unreadable: {error}"))?;
    let pending: i64 = connection
        .query_row(
            "select count(*) from semantic_jobs where status in ('queued', 'running')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("the durable semantic job queue is unreadable: {error}"))?;
    Ok((chunk_count, pending > 0))
}

/// Whether the active generation can actually serve queries right now —
/// and, when it cannot, INVALIDATE it and return the reason. This mutates
/// durable state on that path (the generation is driven to a terminal status
/// and out of the active slot); it is not a pure query.
///
/// The existence of a manifest row is explicitly NOT evidence. That row
/// survives a byte-fold-era identity (the `tokenizer` column was back-filled
/// with `byte-fold-v1`, which is a different embedding identity), a model or
/// runtime swap, and an index directory deleted underneath the daemon — every
/// one of which still answers `active_generation()` with `Ok(Some(_))` while
/// every query against it is empty or wrong. So the checks are: identity
/// equality against the arm this controller runs, then index integrity on
/// disk (both read in one pass by `probe_active_generation`).
///
/// An unusable generation is *invalidated*, not relabelled: it leaves the
/// active slot, so the corpus reads as uncovered and a rebuild is queued.
/// Never data loss — the authoritative chunks never left `rag_chunks`.
fn assess_and_invalidate_active_generation(
    inner: &ControllerInner,
) -> Result<(bool, Option<String>), String> {
    let (manifest, intact) = SemanticStore::probe_active_generation(&inner.config)
        .map_err(|error| format!("the semantic generation manifest is unreadable: {error}"))?;
    let Some(active) = manifest else {
        return Ok((false, None));
    };
    let unusable = if active.identity != inner.identity {
        Some(format!(
            "semantic generation {} was built under a different embedding identity ({} {}@{}, {} \
             dims, {} tokenizer) than the daemon runs; it was invalidated and must be rebuilt",
            active.generation_id,
            active.identity.provider(),
            active.identity.model(),
            active.identity.revision(),
            active.identity.dimensions(),
            active.identity.tokenizer(),
        ))
    } else if !intact {
        Some(format!(
            "the vector index of semantic generation {} did not survive on disk; it was \
             invalidated and must be rebuilt",
            active.generation_id
        ))
    } else {
        // C4: also compare the generation's recorded corpus revision against
        // the current authoritative revision here — a generation that is
        // internally intact can still be behind the corpus it claims to
        // cover.
        None
    };
    match unusable {
        None => Ok((true, None)),
        Some(reason) => {
            let store = SemanticStore::open(&inner.config).map_err(|error| {
                format!(
                    "{reason}; the semantic store could not be opened to invalidate it: {error}"
                )
            })?;
            store
                .invalidate_active_generation()
                .map_err(|error| format!("{reason}; invalidating it failed: {error}"))?;
            Ok((false, Some(reason)))
        }
    }
}

/// Task 7's activation sample: probe the series of the first authoritative
/// chunk with its own text, so the sample query is guaranteed rows.
/// `Ok(None)` means the corpus is empty (nothing to sample); a read or
/// tokenizer error surfaces so the caller can report it instead of silently
/// skipping the job.
fn activation_sample(
    config: &HieronymusConfig,
    tokenizer: &mut dyn ChunkTokenizer,
) -> Result<Option<SemanticSample>, String> {
    let connection = hieronymus::db::open_migrated(&config.database_path())
        .map_err(|error| format!("the authoritative database is unreadable: {error}"))?;
    let row = connection
        .query_row(
            "select series_slug, text from rag_chunks order by id limit 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("the activation sample chunk is unreadable: {error}"))?;
    let Some((series_slug, text)) = row else {
        return Ok(None);
    };
    let token_ids = tokenizer
        .tokenize(&AuthoritativeChunk {
            chunk_id: 0,
            series_slug: series_slug.clone(),
            text,
        })
        .map_err(|error| format!("the activation sample could not be tokenized: {error}"))?;
    Ok(Some(SemanticSample {
        series_slug,
        token_ids,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_semantic_ready_gate_is_exact() {
        assert!(require_semantic_ready(&RequiredSemanticState::Ready).is_ok());
        assert!(require_semantic_ready(&RequiredSemanticState::Acquiring).is_err());
        let message =
            require_semantic_ready(&RequiredSemanticState::Failed("boom".into())).unwrap_err();
        assert!(message.contains("boom"), "{message}");
    }
}

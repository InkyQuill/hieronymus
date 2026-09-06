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
            Box::new(|_lane| {}),
        )
    }

    /// The injection point the daemon (and integration tests) use: an
    /// explicit arm plus the callback that installs a query-time lane into
    /// the application's recall service. Called once when the lane first arms
    /// and again on every verified generation activation, so queries always
    /// run on a coherent identity and generation.
    pub fn start_with(
        config: HieronymusConfig,
        workers: &mut WorkerGroup,
        arm: Arc<dyn SemanticArm>,
        install: Box<dyn Fn(SemanticLane) + Send>,
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
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *state == RequiredSemanticState::Ready {
                    *state = RequiredSemanticState::Rebuilding;
                }
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

struct WorkerContext {
    inner: Arc<ControllerInner>,
    arm: Arc<dyn SemanticArm>,
    install: Box<dyn Fn(SemanticLane) + Send>,
    wake_rx: std::sync::mpsc::Receiver<()>,
}

fn run_worker(
    stop: Arc<AtomicBool>,
    inner: Arc<ControllerInner>,
    arm: Arc<dyn SemanticArm>,
    install: Box<dyn Fn(SemanticLane) + Send>,
    wake_rx: std::sync::mpsc::Receiver<()>,
) {
    let context = WorkerContext {
        inner,
        arm,
        install,
        wake_rx,
    };
    let mut pair: Option<ArmedPair> = None;
    let mut announced_ready = false;
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
        if let Ok(jobs) = SemanticJobStore::open(&context.inner.config) {
            let _ = jobs.reconcile();
        }

        // 2. Arming (with periodic retry: acquiring the assets later must
        //    heal the Failed state without a restart).
        if pair.is_none() && rearm_deadline <= std::time::Instant::now() {
            set_state(&context.inner, RequiredSemanticState::Acquiring);
            match context.arm.arm(&context.inner.config) {
                Ok(armed) => {
                    pair = Some(armed);
                    if let Ok(lane_pair) = context.arm.arm(&context.inner.config) {
                        (context.install)(SemanticLane::new(
                            lane_pair.provider,
                            lane_pair.tokenizer,
                        ));
                    }
                }
                Err(reason) => {
                    set_state(&context.inner, RequiredSemanticState::Failed(reason));
                    rearm_deadline = std::time::Instant::now() + REARM_POLL;
                }
            }
        }

        // 3. Once armed, decide the ready baseline: an active generation (or
        //    an empty corpus) is Ready; uncovered chunks queue a rebuild.
        if let Some(armed) = pair.as_mut() {
            if !announced_ready {
                announced_ready = announce_baseline(&context.inner);
            }
            // 4. Claim and drive durable jobs one at a time.
            let jobs = match SemanticJobStore::open(&context.inner.config) {
                Ok(jobs) => jobs,
                Err(_) => {
                    wait_for_wakeup(&context, &stop);
                    continue;
                }
            };
            let claimable = jobs.claimable_jobs().unwrap_or_default();
            for job_id in claimable {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let is_ready = context
                    .inner
                    .state
                    .lock()
                    .map(|state| *state == RequiredSemanticState::Ready)
                    .unwrap_or(false);
                if is_ready {
                    set_state(&context.inner, RequiredSemanticState::Rebuilding);
                }
                let Some(sample) =
                    activation_sample(&context.inner.config, armed.tokenizer.as_mut())
                else {
                    continue;
                };
                let result = jobs.run_rebuild(
                    &job_id,
                    RebuildInputs {
                        provider: armed.provider.as_mut(),
                        tokenizer: armed.tokenizer.as_mut(),
                        sample,
                    },
                    &rebuild,
                );
                match result {
                    Ok(JobOutcome::Completed { .. }) => {
                        // Verified activation: refresh the application's
                        // query lane onto the new generation.
                        if let Ok(lane_pair) = context.arm.arm(&context.inner.config) {
                            (context.install)(SemanticLane::new(
                                lane_pair.provider,
                                lane_pair.tokenizer,
                            ));
                        }
                        announced_ready = true;
                        set_state(&context.inner, RequiredSemanticState::Ready);
                    }
                    Ok(JobOutcome::Cancelled { .. }) => {
                        // The previous active generation (if any) keeps
                        // serving; a later import re-queues.
                        announced_ready = true;
                        set_state(&context.inner, RequiredSemanticState::Ready);
                    }
                    Ok(JobOutcome::Failed { error, .. }) => {
                        set_state(&context.inner, RequiredSemanticState::Failed(error));
                    }
                    Ok(JobOutcome::Busy { .. } | JobOutcome::LeaseLost { .. }) => {}
                    Err(error) => {
                        set_state(
                            &context.inner,
                            RequiredSemanticState::Failed(error.to_string()),
                        );
                    }
                }
                if stop.load(Ordering::Acquire) {
                    break;
                }
            }
            // Nothing left to claim: the durable queue is drained. Never
            // overwrite an honest Failed verdict with Ready.
            let currently_rebuilding = context
                .inner
                .state
                .lock()
                .map(|state| *state == RequiredSemanticState::Rebuilding)
                .unwrap_or(false);
            if announced_ready
                && currently_rebuilding
                && jobs
                    .claimable_jobs()
                    .map(|jobs| jobs.is_empty())
                    .unwrap_or(true)
            {
                set_state(&context.inner, RequiredSemanticState::Ready);
            }
        }

        wait_for_wakeup(&context, &stop);
    }
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

/// The post-arming baseline: `true` when the state is settled (Ready, or
/// Rebuilding because a job is queued/in flight). Uncovered chunks queue a
/// rebuild durably; an empty corpus is ready-for-ingest, never an error.
fn announce_baseline(inner: &ControllerInner) -> bool {
    let store = match SemanticStore::open(&inner.config) {
        Ok(store) => store,
        Err(error) => {
            set_state(inner, RequiredSemanticState::Failed(error.to_string()));
            return false;
        }
    };
    match store.active_generation() {
        Ok(Some(_)) => {
            set_state(inner, RequiredSemanticState::Ready);
            true
        }
        Ok(None) => {
            match queue_semantic_rebuild(&inner.config, &inner.identity) {
                Ok(hieronymus::semantic_recall::QueueOutcome::Enqueued(_))
                | Ok(hieronymus::semantic_recall::QueueOutcome::AlreadyQueued(_)) => {
                    let _ = inner.wake.send(());
                    set_state(inner, RequiredSemanticState::Rebuilding);
                    // Rebuilding settles through job outcomes; treat the
                    // baseline as settled so it is not re-queued every tick.
                    true
                }
                Ok(hieronymus::semantic_recall::QueueOutcome::EmptyCorpus) => {
                    set_state(inner, RequiredSemanticState::Ready);
                    true
                }
                Err(error) => {
                    set_state(inner, RequiredSemanticState::Failed(error.to_string()));
                    false
                }
            }
        }
        Err(error) => {
            set_state(inner, RequiredSemanticState::Failed(error.to_string()));
            false
        }
    }
}

/// Task 7's activation sample: probe the series of the first authoritative
/// chunk with its own text, so the sample query is guaranteed rows.
fn activation_sample(
    config: &HieronymusConfig,
    tokenizer: &mut dyn ChunkTokenizer,
) -> Option<SemanticSample> {
    let connection = hieronymus::db::open_migrated(&config.database_path()).ok()?;
    let row = connection
        .query_row(
            "select series_slug, text from rag_chunks order by id limit 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()?;
    let token_ids = tokenizer
        .tokenize(&AuthoritativeChunk {
            chunk_id: 0,
            series_slug: row.0.clone(),
            text: row.1,
        })
        .ok()?;
    Some(SemanticSample {
        series_slug: row.0,
        token_ids,
    })
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

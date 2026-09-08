//! The supervised dream controller (task D5, Astra 5 and 17): one supervised
//! worker owns every ordinary production dream run for a data root. Correction
//! consolidation has its own supervised worker sharing the root Dream lock. The configured
//! interval schedule (ADR 0005), the admin "Run Manual Dreaming" action, and
//! the MCP `hieronymus_dream` tool all coalesce into that worker through
//! [`DreamController::request`]; there is never a second concurrent
//! [`hieronymus::dreaming::DreamService`] run.
//!
//! Supervision (ADR 0009): the worker is admitted to the daemon's
//! [`WorkerGroup`](crate::daemon::workers::WorkerGroup), so a graceful stop joins it before discovery is removed
//! and root ownership is released. Scheduled sleeps are bounded slices on
//! the shared stop flag's condvar, and a run in progress at shutdown stops
//! at the next batch boundary — the batches that did run keep their durable
//! honest outcome, and the caller waiting on the run is woken with the
//! result instead of being abandoned.
//!
//! Providers are constructed inside the worker and never cross threads:
//! production builds a [`WorkflowResolver`] from the `provider.conf`
//! catalog on every run (so config changes apply without a restart), while
//! [`DreamController::start_with_provider_source`] is the explicit test
//! injection seam — a factory sampled inside the worker. No fixture-host
//! recognition exists anywhere; tests inject through construction only.
//!
//! The OS-level `dream_cycle_lock` still guards every batch
//! (`run_locked`), so exclusive CLI maintenance and other processes stay
//! exclusive; the controller's coalescing is the application-level layer on
//! top of it.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::load_dream_config;
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{
    DrainRecord, DreamError, DreamRunRecord, DreamService, ScheduledDecision,
    consecutive_not_enough_memories_skips, pending_short_term_memory_count, scheduled_decision,
};

use super::events::AdminEventHub;

/// How often the worker wakes to re-check the stop flag and any newly
/// arrived request. This bounds the shutdown join latency together with the
/// batch boundary.
const WORKER_SLICE: Duration = Duration::from_millis(250);

/// How often the scheduled-decision gate reads the actual config state
/// (enabled flag, interval, thresholds). The first gate fires immediately:
/// with no prior decision the interval has elapsed (Python autostart's
/// anchor semantics).
const SCHEDULE_GATE: Duration = Duration::from_secs(2);

/// Exponential outage delay; the scheduler's urgent gate cannot bypass it.
pub fn retry_delay(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs(
        (30_u64.saturating_mul(1_u64 << failures.saturating_sub(1).min(6))).min(1800),
    )
}

/// Wall clock injection keeps durable retry deadlines testable across restarts.
pub type RetryClock = Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>;

fn config_fingerprint(config: &HieronymusConfig) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for path in [config.dream_config_path(), config.provider_config_path()] {
        match std::fs::read(path) {
            Ok(bytes) => {
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                digest.update(b"missing");
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    // Credentials participate in repair detection, but only the digest is stored.
    Ok(format!("{:x}", digest.finalize()))
}

/// A request for dream work. `manual` bypasses the global automatic-
/// scheduling switch (`dream.conf` `enabled`) but never the per-workflow
/// assignments — the fail-closed `enabled_choices` gate stays authoritative.
/// `all` lifts the minimum-pending threshold (admin/MCP "dream all");
/// non-manual requests honor it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DreamRequest {
    pub all: bool,
    pub manual: bool,
}

/// A ticket identifying one controller run. Requests that arrive while a
/// run is active receive the active run's ticket: they coalesce into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamTicket {
    pub run_id: String,
}

/// The status of one controller run (the active one or the last finished).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamRunStatus {
    pub run_id: String,
    /// `"manual"`, `"scheduled"`, `"urgent"`, or `"backlog_escape"`.
    pub trigger: String,
    /// The overall drain outcome (`"completed"`, `"pending"`, `"skipped"`,
    /// `"failed"`), or `"interrupted"` when shutdown cut the drain short. `None` while
    /// the run is still active.
    pub outcome: Option<String>,
    /// Totals of the finished drain (`None` while active).
    pub batches: Option<usize>,
    pub input_count: Option<i64>,
}

/// The controller's current status: the active run, if any, and the last
/// finished one. This is the admin surface's honest view of the controller
/// (the durable run/phase registry remains the detailed record).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DreamStatus {
    pub active: Option<DreamRunStatus>,
    pub last: Option<DreamRunStatus>,
}

/// The provider source: sampled inside the worker per run, so the resolver
/// snapshot (and every provider it constructs) is worker-local. Production
/// samples the `provider.conf` catalog; tests inject loopback lanes here.
pub type ProviderSource = Arc<dyn Fn() -> WorkflowResolver + Send + Sync>;

/// One run's completion slot: the worker fills it exactly once, and every
/// waiter for the run (there can be many — requests coalesce) wakes with
/// the same answer.
struct RunSlot {
    outcome: Mutex<Option<Result<DrainRecord, String>>>,
    signal: Condvar,
}

impl RunSlot {
    fn new() -> Arc<RunSlot> {
        Arc::new(RunSlot {
            outcome: Mutex::new(None),
            signal: Condvar::new(),
        })
    }

    fn complete(&self, outcome: Result<DrainRecord, String>) {
        {
            let mut guard = self.lock();
            if guard.is_some() {
                return;
            }
            *guard = Some(outcome);
        }
        self.signal.notify_all();
    }

    fn wait(&self) -> Result<DrainRecord, String> {
        let mut guard = self.lock();
        loop {
            if let Some(outcome) = guard.clone() {
                return outcome;
            }
            guard = self
                .signal
                .wait(guard)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Result<DrainRecord, String>>> {
        self.outcome.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The work plan one queued entry carries. Manual requests map to the
/// `manual` trigger with `ignore_minimum` from [`DreamRequest::all`];
/// scheduled decisions map to their ADR 0005 trigger names (which pass
/// through `trigger_type_from_owner` unchanged, keeping the audited
/// threshold state truthful — `backlog_escape` sets `stale_cycle_override`).
struct RunPlan {
    trigger: &'static str,
    ignore_minimum: bool,
}

impl RunPlan {
    fn manual(all: bool) -> Self {
        RunPlan {
            trigger: "manual",
            ignore_minimum: all,
        }
    }
}

/// A request (manual or scheduled) waiting for the worker.
struct PendingRun {
    run_id: String,
    plan: RunPlan,
    slot: Arc<RunSlot>,
}

/// The run currently executing on the worker.
struct ActiveRun {
    run_id: String,
    trigger: String,
    slot: Arc<RunSlot>,
}

struct ControllerState {
    active: Option<ActiveRun>,
    pending: Option<PendingRun>,
    last: Option<DreamRunStatus>,
    next_run_id: u64,
}

struct ControllerInner {
    state: Mutex<ControllerState>,
    /// Wakes the worker for a newly queued request; the worker's scheduled
    /// waits are bounded slices on this condvar, so shutdown never waits on
    /// an unbounded sleep.
    signal: Condvar,
    /// The daemon's shared cancellation edge (ctrl-c handler, `POST
    /// /shutdown`): requests are refused once it is set, scheduled ticks
    /// stand down, and an in-flight drain stops at the next batch boundary.
    stop: Arc<AtomicBool>,
    events: Arc<AdminEventHub>,
    config: HieronymusConfig,
    source: ProviderSource,
    clock: RetryClock,
    retry_storage_failed: AtomicBool,
}

/// A cloneable handle to the dream controller. The worker is admitted to
/// the daemon's [`WorkerGroup`](crate::daemon::workers::WorkerGroup) at [`DreamController::start`]; every handle
/// shares the same worker, queue, and slots.
#[derive(Clone)]
pub struct DreamController {
    inner: Arc<ControllerInner>,
}

impl std::fmt::Debug for DreamController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DreamController")
            .field("stop", &self.inner.stop.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl DreamController {
    /// Start the controller over the real provider catalog (production):
    /// the resolver is sampled per run inside the worker from the current
    /// `provider.conf`, so configured lanes apply without a restart. A
    /// broken catalog file fails the affected run's workflow gate — fail
    /// closed, never a silent substitution.
    pub fn start(
        config: HieronymusConfig,
        workers: &mut super::workers::WorkerGroup,
    ) -> Result<Self, String> {
        let catalog_config = config.clone();
        Self::start_with_provider_source(
            config,
            workers,
            Arc::new(move || {
                WorkflowResolver::from_catalog(
                    hieronymus::provider_config::load_provider_catalog(&catalog_config)
                        .unwrap_or_default(),
                )
            }),
        )
    }

    /// The test injection seam: construct the controller over an explicit
    /// provider source. The factory is sampled inside the worker (providers
    /// never cross threads); production construction is
    /// [`DreamController::start`]. This is a constructor seam only — there
    /// is deliberately no fixture-host recognition in production paths.
    pub fn start_with_provider_source(
        config: HieronymusConfig,
        workers: &mut super::workers::WorkerGroup,
        source: ProviderSource,
    ) -> Result<Self, String> {
        Self::start_with_provider_source_and_clock(
            config,
            workers,
            source,
            Arc::new(chrono::Utc::now),
        )
    }

    /// Explicit test seam for persisted retry eligibility; production uses UTC.
    pub fn start_with_provider_source_and_clock(
        config: HieronymusConfig,
        workers: &mut super::workers::WorkerGroup,
        source: ProviderSource,
        clock: RetryClock,
    ) -> Result<Self, String> {
        let events = Arc::new(AdminEventHub::default());
        let inner = Arc::new(ControllerInner {
            state: Mutex::new(ControllerState {
                active: None,
                pending: None,
                last: None,
                next_run_id: 0,
            }),
            signal: Condvar::new(),
            stop: Arc::clone(workers.stop_flag()),
            events,
            config,
            source,
            clock,
            retry_storage_failed: AtomicBool::new(false),
        });
        let controller = DreamController { inner };
        let worker = controller.clone();
        // Admission inherits the daemon shutdown: `finish_shutdown` joins
        // the group before removing discovery and releasing root ownership,
        // so this thread can never outlive either.
        workers.spawn(Box::new(move |stop| worker.run_worker(stop)))?;
        Ok(controller)
    }

    /// The admin event hub the controller publishes dream lifecycle events
    /// on. The daemon serves it as its only hub, so the websocket event
    /// behavior is unchanged from the pre-controller surface.
    pub(crate) fn events(&self) -> Arc<AdminEventHub> {
        Arc::clone(&self.inner.events)
    }

    /// Request dream work. Returns a ticket for the run that will serve the
    /// request: a fresh one, or — when a run is active or already queued —
    /// that run's ticket (concurrent triggers coalesce into one run).
    /// Refused once the daemon is shutting down.
    pub fn request(&self, request: DreamRequest) -> Result<DreamTicket, String> {
        self.check_request(&request)?;
        Ok(self.submit(RunPlan::manual(request.all)).0)
    }

    /// Like [`Self::request`], but blocks until the serving run finishes
    /// and reports its drain outcome. This is the MCP `hieronymus_dream`
    /// path: the tool's answer is the finished run.
    pub fn request_and_wait(&self, request: DreamRequest) -> Result<DrainRecord, String> {
        self.check_request(&request)?;
        let (_, slot) = self.submit(RunPlan::manual(request.all));
        slot.wait()
    }

    /// Admission checks shared by [`Self::request`] and
    /// [`Self::request_and_wait`]: refuse once shutting down, and honor the
    /// global automatic-scheduling switch for non-manual requests (manual
    /// ones bypass it; the per-workflow gate is unaffected either way).
    fn check_request(&self, request: &DreamRequest) -> Result<(), String> {
        if self.inner.stop.load(Ordering::Acquire) {
            return Err("daemon is shutting down; dream work is not admitted".to_string());
        }
        if !request.manual {
            let dream_config =
                load_dream_config(&self.inner.config).map_err(|error| error.to_string())?;
            if !dream_config.enabled {
                return Err("automatic dreaming is disabled in dream.conf".to_string());
            }
        }
        Ok(())
    }

    /// Queue one manual plan, coalescing into the active or pending run.
    /// Returns the serving run's ticket and its completion slot.
    fn submit(&self, plan: RunPlan) -> (DreamTicket, Arc<RunSlot>) {
        let mut state = self.lock();
        if let Some(active) = &state.active {
            return (
                DreamTicket {
                    run_id: active.run_id.clone(),
                },
                Arc::clone(&active.slot),
            );
        }
        if let Some(pending) = &state.pending {
            // Coalesce: the queued drain will serve this request too.
            return (
                DreamTicket {
                    run_id: pending.run_id.clone(),
                },
                Arc::clone(&pending.slot),
            );
        }
        state.next_run_id += 1;
        let run_id = format!("dream-{}", state.next_run_id);
        let slot = RunSlot::new();
        state.pending = Some(PendingRun {
            run_id: run_id.clone(),
            plan,
            slot: Arc::clone(&slot),
        });
        drop(state);
        self.inner.signal.notify_all();
        (DreamTicket { run_id }, slot)
    }

    /// The controller's honest status: the active run and the last finished
    /// one. The admin surface can serve this directly.
    pub fn status(&self) -> DreamStatus {
        let state = self.lock();
        DreamStatus {
            active: state.active.as_ref().map(|active| DreamRunStatus {
                run_id: active.run_id.clone(),
                trigger: active.trigger.clone(),
                outcome: None,
                batches: None,
                input_count: None,
            }),
            last: state.last.clone(),
        }
    }

    /// Run one scheduled decision now, honoring the actual config state:
    /// the global `enabled` switch, the pending thresholds, and the
    /// consecutive-skip backlog escape (ADR 0005). The worker calls this on
    /// the configured interval; maintenance and tests may invoke it
    /// directly to force one decision. Returns the ticket of a run that was
    /// started (or coalesced into).
    pub fn run_scheduled_tick(&self) -> Option<DreamTicket> {
        if self.inner.stop.load(Ordering::Acquire) {
            return None;
        }
        let dream_config = match load_dream_config(&self.inner.config) {
            Ok(dream_config) => dream_config,
            Err(error) => {
                eprintln!(
                    "hiero dream worker: scheduled dreaming stands down, dream config failed to load: {error}"
                );
                return None;
            }
        };
        if !dream_config.enabled {
            return None;
        }
        if !self.retry_eligible().ok()? {
            return None;
        }
        let pending = pending_short_term_memory_count(&self.inner.config).ok()?;
        if pending == 0 || pending < dream_config.min_pending_short_term_memories {
            let service = DreamService::open(&self.inner.config, (self.inner.source)()).ok()?;
            if service.cycle_has_deterministic_work().ok()? {
                return self.submit_scheduled("scheduled", false);
            }
            if pending == 0 {
                return None;
            }
        }
        let skips = consecutive_not_enough_memories_skips(&self.inner.config).unwrap_or(0);
        match scheduled_decision(&dream_config, pending, skips) {
            ScheduledDecision::NothingPending => None,
            ScheduledDecision::NotEnoughMemories => {
                // Record the skip honestly on a durable run row; the
                // trailing count of these rows arms the backlog escape.
                let service = DreamService::open(&self.inner.config, (self.inner.source)()).ok()?;
                let _ = service.record_skipped_run(&format!(
                    "not_enough_memories: {pending} pending below minimum {}",
                    dream_config.min_pending_short_term_memories
                ));
                None
            }
            ScheduledDecision::Scheduled => self.submit_scheduled("scheduled", false),
            ScheduledDecision::Urgent => self.submit_scheduled("urgent", true),
            ScheduledDecision::BacklogEscape => self.submit_scheduled("backlog_escape", true),
        }
    }

    fn submit_scheduled(&self, trigger: &'static str, ignore_minimum: bool) -> Option<DreamTicket> {
        if !self.retry_eligible().unwrap_or(false) || self.inner.stop.load(Ordering::Acquire) {
            return None;
        }
        let (ticket, _) = self.submit(RunPlan {
            trigger,
            ignore_minimum,
        });
        Some(ticket)
    }

    /// Configuration repair resets the streak atomically; an unreadable store
    /// fails closed instead of turning an outage into a hot retry loop.
    fn retry_eligible(&self) -> Result<bool, String> {
        if self.inner.retry_storage_failed.load(Ordering::Acquire) {
            return Err(
                "automatic dreaming paused: retry state could not be persisted".to_string(),
            );
        }
        let fingerprint = config_fingerprint(&self.inner.config)?;
        let mut connection =
            open_migrated(&self.inner.config.database_path()).map_err(|e| e.to_string())?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        transaction.execute("insert into dream_retry_state(singleton, failures, next_attempt_at, config_fingerprint)
          values(1, 0, null, ?1) on conflict(singleton) do update set failures=0, next_attempt_at=null, config_fingerprint=excluded.config_fingerprint
          where dream_retry_state.config_fingerprint != excluded.config_fingerprint", [&fingerprint]).map_err(|e| e.to_string())?;
        let deadline: Option<String> = transaction
            .query_row(
                "select next_attempt_at from dream_retry_state where singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        match deadline {
            None => Ok(true),
            Some(value) => Ok(chrono::DateTime::parse_from_rfc3339(&value)
                .map_err(|e| e.to_string())?
                <= (self.inner.clock)()),
        }
    }

    fn retry_wakeup_due(&self) -> bool {
        use rusqlite::OptionalExtension;
        let Ok(connection) = open_migrated(&self.inner.config.database_path()) else {
            return false;
        };
        let row = connection.query_row("select failures, next_attempt_at, config_fingerprint from dream_retry_state where singleton=1", [], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?))).optional();
        let Ok(Some((failures, deadline, fingerprint))) = row else {
            return false;
        };
        if failures == 0 {
            return false;
        }
        config_fingerprint(&self.inner.config).is_ok_and(|current| current != fingerprint)
            || deadline
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                .is_some_and(|deadline| deadline <= (self.inner.clock)())
    }

    fn record_retry_outcome(&self, fingerprint: &str, failed: bool) -> Result<(), String> {
        let mut connection =
            open_migrated(&self.inner.config.database_path()).map_err(|e| e.to_string())?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        use rusqlite::OptionalExtension;
        let previous: u32 = transaction.query_row("select failures from dream_retry_state where singleton=1 and config_fingerprint=?1", [fingerprint], |row| row.get(0)).optional().map_err(|e| e.to_string())?.unwrap_or(0);
        let failures = if failed {
            previous.saturating_add(1)
        } else {
            0
        };
        // Positive bounded jitter (0–10%), capped at the maximum delay.
        let mut random = [0_u8; 1];
        let _ = getrandom::fill(&mut random);
        let seconds = retry_delay(failures).as_secs();
        let jittered = (seconds + seconds * u64::from(random[0] % 11) / 100).min(1800);
        let deadline = failed.then(|| {
            ((self.inner.clock)() + chrono::Duration::seconds(jittered as i64)).to_rfc3339()
        });
        transaction.execute("insert into dream_retry_state(singleton, failures, next_attempt_at, config_fingerprint)
          values(1, ?1, ?2, ?3) on conflict(singleton) do update set failures=excluded.failures, next_attempt_at=excluded.next_attempt_at, config_fingerprint=excluded.config_fingerprint",
          rusqlite::params![failures, deadline, fingerprint]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControllerState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // ------------------------------------------------------------------
    // The worker
    // ------------------------------------------------------------------

    fn run_worker(&self, stop: Arc<AtomicBool>) {
        // The interval anchor: the last scheduled decision time. `None`
        // means no decision has been made yet, so the interval has elapsed
        // (the first gate fires immediately, like Python's autostart).
        let mut anchor: Option<Instant> = None;
        let mut gate_due = Instant::now();
        loop {
            if stop.load(Ordering::Acquire) {
                // Shutdown stops accepting new runs: a request that slipped
                // in is woken with a clear refusal instead of hanging.
                self.refuse_pending("daemon is shutting down; the dream run never started");
                return;
            }
            if let Some(pending) = self.take_pending() {
                self.execute(&pending, &stop);
                continue;
            }
            if Instant::now() >= gate_due {
                gate_due = Instant::now() + SCHEDULE_GATE;
                anchor = self.scheduled_gate(anchor);
            }
            // Bounded interruptible wait: a queued request notifies, the
            // slice bounds shutdown latency, and every long operation in
            // between polls the stop flag itself.
            let state = self.lock();
            let _ = self
                .inner
                .signal
                .wait_timeout(state, WORKER_SLICE)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The scheduler gate: the urgent backlog trigger (ADR 0005) is
    /// evaluated at every gate independently of the interval — a backlog at
    /// `max_pending_short_term_memories` must never wait for the next
    /// scheduled firing. Scheduled and backlog-escape decisions run only
    /// when the configured interval has elapsed since the last decision (a
    /// decision is a run, a skip record, or an urgent run — all re-anchor
    /// the schedule); the urgent check leaves that anchor untouched. Urgent
    /// rides the same coalescing, OS lock, enabled state, and per-workflow
    /// rules as every other trigger.
    fn scheduled_gate(&self, anchor: Option<Instant>) -> Option<Instant> {
        if self.urgent_backlog_due() {
            self.submit_scheduled("urgent", true);
        }
        let now = Instant::now();
        let due = match anchor {
            None => true,
            Some(anchor) => {
                let interval = load_dream_config(&self.inner.config)
                    .map(|dream_config| {
                        Duration::from_secs(
                            (dream_config.schedule_interval_minutes.max(1) as u64) * 60,
                        )
                    })
                    .unwrap_or(SCHEDULE_GATE);
                now.duration_since(anchor) >= interval
            }
        };
        if due || self.retry_wakeup_due() {
            self.run_scheduled_tick();
            Some(now)
        } else {
            anchor
        }
    }

    /// Whether the urgent maximum-backlog trigger holds right now: automatic
    /// dreaming enabled and crystallization-eligible pending at the
    /// configured maximum.
    fn urgent_backlog_due(&self) -> bool {
        let dream_config = match load_dream_config(&self.inner.config) {
            Ok(dream_config) => dream_config,
            Err(error) => {
                eprintln!(
                    "hiero dream worker: urgent-backlog trigger stands down, dream config failed to load: {error}"
                );
                return false;
            }
        };
        if !dream_config.enabled {
            return false;
        }
        pending_short_term_memory_count(&self.inner.config)
            .map(|pending| pending >= dream_config.max_pending_short_term_memories)
            .unwrap_or(false)
    }

    fn take_pending(&self) -> Option<(String, RunPlan, Arc<RunSlot>)> {
        let mut state = self.lock();
        state.pending.take().map(|pending| {
            state.active = Some(ActiveRun {
                run_id: pending.run_id.clone(),
                trigger: pending.plan.trigger.to_string(),
                slot: Arc::clone(&pending.slot),
            });
            (pending.run_id, pending.plan, pending.slot)
        })
    }

    /// A queued request that will never be served (shutdown): wake its
    /// waiters with a clear refusal so nothing hangs on the join.
    fn refuse_pending(&self, message: &str) {
        let pending = {
            let mut state = self.lock();
            state.pending.take()
        };
        if let Some(pending) = pending {
            pending.slot.complete(Err(message.to_string()));
        }
    }

    /// Execute one queued run on the worker: publish the lifecycle events,
    /// run the drain, complete the slot, and record the status. Providers
    /// are constructed inside this call (worker-local by construction).
    fn execute(&self, pending: &(String, RunPlan, Arc<RunSlot>), stop: &AtomicBool) {
        let (run_id, plan, slot) = pending;
        let trigger = plan.trigger;
        let fingerprint = config_fingerprint(&self.inner.config);
        self.inner
            .events
            .publish("dream_started", json!({ "trigger": trigger }));

        // A panic must never strand a waiter (the serve thread blocking on
        // this slot would hang the shutdown join): catch it, log what
        // panicked, and fail the run honestly. The durable rows of completed
        // batches stay.
        let outcome = match std::panic::catch_unwind(AssertUnwindSafe(|| self.run_plan(plan, stop)))
        {
            Ok(outcome) => outcome,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string panic payload".to_string());
                eprintln!("hiero dream worker panicked during a {trigger} run: {detail}");
                Err("dream worker panicked; the run stopped at the current \
                     batch boundary"
                    .to_string())
            }
        };

        if let Ok(fingerprint) = &fingerprint
            && (outcome.is_err()
                || outcome
                    .as_ref()
                    .is_ok_and(|drain| drain.outcome == "completed"))
        {
            let failed = outcome.is_err();
            if let Err(error) = self.record_retry_outcome(fingerprint, failed) {
                self.inner
                    .retry_storage_failed
                    .store(true, Ordering::Release);
                eprintln!("hiero dream worker: could not persist retry state: {error}");
            } else {
                self.inner
                    .retry_storage_failed
                    .store(false, Ordering::Release);
            }
        }

        // The wait answer and the events tell the same story: a drain the
        // shutdown cut short is an interrupted run, not a completion.
        let (slot_outcome, final_outcome) = match outcome {
            Ok(drain) if drain.record.status == "skipped" => {
                self.inner.events.publish(
                    "dream_failed",
                    json!({ "trigger": trigger, "error": drain.record.error }),
                );
                (Ok(drain), "skipped".to_string())
            }
            Ok(drain) if matches!(drain.outcome.as_str(), "pending" | "interrupted") => {
                let state = drain.outcome.clone();
                self.inner.events.publish("dream_paused", json!({ "trigger": trigger, "status": state, "batches": drain.batches, "progress": drain.progress }));
                (Ok(drain), state)
            }
            Ok(drain) => {
                self.inner.events.publish(
                    "dream_completed",
                    json!({
                        "trigger": trigger,
                        "result": dream_run_payload(&drain.record),
                    }),
                );
                if trigger == "manual" {
                    // Port of AdminStore.run_manual_dreaming's audit entry.
                    self.write_manual_audit_entry(&drain.record);
                }
                (Ok(drain), "completed".to_string())
            }
            Err(error) => {
                self.inner.events.publish(
                    "dream_failed",
                    json!({ "trigger": trigger, "error": error }),
                );
                (Err(error), "failed".to_string())
            }
        };

        let finished = DreamRunStatus {
            run_id: run_id.clone(),
            trigger: trigger.to_string(),
            outcome: Some(final_outcome),
            batches: slot_outcome.as_ref().ok().map(|drain| drain.batches),
            input_count: slot_outcome.as_ref().ok().map(|drain| drain.input_count),
        };
        {
            let mut state = self.lock();
            state.active = None;
            state.last = Some(finished);
        }
        slot.complete(slot_outcome);
    }

    /// Open the service over a freshly sampled provider source and drain.
    /// The phase observer streams `dream_phase_progress` from this thread,
    /// replacing the old per-run polling monitor.
    fn run_plan(&self, plan: &RunPlan, stop: &AtomicBool) -> Result<DrainRecord, String> {
        let mut service = DreamService::open(&self.inner.config, (self.inner.source)())
            .map_err(|error| self.redact_error(&error))?;
        let events = Arc::clone(&self.inner.events);
        service.set_phase_observer(Arc::new(move |run_id, cycle_id, phase| {
            events.publish(
                "dream_phase_progress",
                json!({ "run_id": run_id, "cycle_id": cycle_id, "phase": phase }),
            );
        }));
        // The owner doubles as the audited trigger type: `manual` maps to
        // the manual trigger, and the scheduled trigger names pass through
        // `trigger_type_from_owner` unchanged (so a `backlog_escape` run
        // audits `stale_cycle_override`).
        service
            .run_draining(plan.trigger, plan.ignore_minimum, stop)
            .map_err(|error| self.redact_error(&error))
    }

    /// Error text for events, redacted exactly like the dreaming core so
    /// configured provider keys never leave the process.
    fn redact_error(&self, error: &DreamError) -> String {
        let message = error.to_string();
        hieronymus::provider_config::load_provider_catalog(&self.inner.config)
            .map(|catalog| {
                let keys: Vec<&str> = catalog
                    .providers
                    .values()
                    .map(|profile| profile.key().expose_secret().as_str())
                    .collect();
                hieronymus::secret::redact_values(&message, &keys)
            })
            .unwrap_or(message)
    }

    /// Port of AdminStore.run_manual_dreaming's audit entry: one `run`
    /// audit-log row per completed manual dream run.
    fn write_manual_audit_entry(&self, record: &DreamRunRecord) {
        if let Ok(connection) = open_migrated(&self.inner.config.database_path()) {
            let _ = connection.execute(
                "insert into audit_log(action, entity_type, entity_id, note, created_at)
                 values ('run', 'dream', ?1, ?2, ?3)",
                rusqlite::params![
                    record.id.to_string(),
                    format!(
                        "Manual dream run {} with provider {}",
                        record.cycle_id, record.provider
                    ),
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ],
            );
        }
    }
}

/// The `dream_completed` result payload: the serialized dream-run record
/// (Python `dataclass_to_json(run)`, field names verbatim).
fn dream_run_payload(record: &DreamRunRecord) -> serde_json::Value {
    json!({
        "id": record.id,
        "cycle_id": record.cycle_id,
        "status": record.status,
        "provider": record.provider,
        "input_count": record.input_count,
        "created_crystal_count": record.created_crystal_count,
        "proposal_count": record.proposal_count,
        "error": record.error,
    })
}

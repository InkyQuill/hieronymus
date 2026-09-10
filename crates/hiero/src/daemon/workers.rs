//! Supervised daemon workers (ADR 0009; security design spec §Lifecycle And
//! Discovery: "graceful shutdown stops admission, closes MCP/WebSocket
//! sessions, signals workers, waits for bounded work").
//!
//! [`WorkerGroup`] owns every mutating worker thread the daemon spawns and one
//! shared cancellation flag. Nothing may outlive it: the daemon joins the
//! group before it removes its discovery record and releases data-root
//! ownership, so no writer can still hold the database while another process
//! claims the root.
//!
//! Admission is guarded under the same mutex that owns the handles: once the
//! stop flag is set, [`WorkerGroup::spawn`] refuses instead of racing a join.
//! [`WorkerGroup::stop_and_join`] never holds that mutex while joining, so a
//! worker that itself spawns (or a refused admission) cannot deadlock the
//! shutdown.
//!
//! The group is also the daemon's per-connection thread pool, so it would grow
//! without bound over a long uptime if handles were only drained at shutdown.
//! [`WorkerGroup::reap_finished`] joins the handles that have already run to
//! completion, without touching live ones and without stopping admission; the
//! accept loop calls it on every tick.
//!
//! Future controllers (D5's `DreamController`, S2's `SemanticController`)
//! register their work here: they hand a `Send` closure to `spawn` and keep
//! their non-`Send` providers/sessions inside the worker they own.

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// A set of supervised worker threads sharing one cancellation flag.
#[derive(Debug)]
pub struct WorkerGroup {
    stop: Arc<AtomicBool>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    /// Serializes the *join* sections of [`WorkerGroup::reap_finished`] and
    /// [`WorkerGroup::stop_and_join`]. Both take handles out of `handles`
    /// before joining them (joining under that lock would deadlock a worker
    /// that spawns), so a shutdown racing a reap would otherwise take a list
    /// the reap had already shortened and report a completed drain while the
    /// reap was still working through its own share.
    ///
    /// That is an *accounting* hazard today, not a live-writer one: a reap
    /// only ever removes handles whose closure has already returned (and
    /// dropped everything it captured), so no thread is still doing work when
    /// a shutdown misses it. What the mutex buys is that `stop_and_join`'s
    /// verdict — and the `live_count()` a caller reads right after it — mean
    /// exactly what they say, instead of "everything except whatever a reap
    /// happens to be holding". It also keeps that guarantee if the reap ever
    /// grows to touch handles that have not finished, which is the point at
    /// which the ownership invariant would depend on it.
    ///
    /// Lock order is always `joining` → `handles`, and `handles` is never held
    /// across a join, so no cycle exists. `reap_finished` only ever *tries*
    /// this lock: a reap that loses to a shutdown does nothing at all (the
    /// shutdown is about to join everything anyway), which keeps the accept
    /// loop free of any wait.
    joining: Mutex<()>,
}

impl WorkerGroup {
    /// Build a group over the caller's cancellation flag. Sharing the flag is
    /// deliberate: the accept loop, the websocket sessions, and every worker
    /// observe exactly one "stop" edge.
    pub fn new(stop: Arc<AtomicBool>) -> WorkerGroup {
        WorkerGroup {
            stop,
            handles: Mutex::new(Vec::new()),
            joining: Mutex::new(()),
        }
    }

    /// The shared cancellation flag.
    pub fn stop_flag(&self) -> &Arc<AtomicBool> {
        &self.stop
    }

    /// Whether cancellation has been signalled.
    pub fn is_stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Admit one worker. The closure receives the shared stop flag and must
    /// return once it observes cancellation (or once its own bounded work
    /// finishes). Refused after [`WorkerGroup::stop_and_join`] has begun:
    /// admission and the handle list are guarded by the same lock, so a
    /// handle can never be pushed after the join drained the list.
    ///
    /// **Contract on `work`:** the closure MUST return within a bounded time
    /// of observing cancellation. The daemon never force-kills worker threads,
    /// and [`WorkerGroup::stop_and_join`] has no hard deadline — it waits
    /// indefinitely. A worker that ignores the stop flag, or blocks on I/O
    /// without a timeout, therefore hangs shutdown and (by design) keeps
    /// data-root ownership held. Every blocking operation inside `work` needs
    /// its own timeout, and long loops must poll the flag between iterations.
    pub fn spawn(&self, work: Box<dyn FnOnce(Arc<AtomicBool>) + Send>) -> Result<(), String> {
        let mut handles = self.handles.lock().unwrap_or_else(PoisonError::into_inner);
        if self.stop.load(Ordering::Acquire) {
            // Dropping `work` here also drops anything it captured (an
            // accepted socket, for instance), which is the intended
            // "stop admitting" behavior.
            return Err("daemon is shutting down; work is not admitted".to_string());
        }
        let stop = Arc::clone(&self.stop);
        handles.push(std::thread::spawn(move || work(stop)));
        Ok(())
    }

    /// Number of workers currently supervised (diagnostics and tests).
    pub fn live_count(&self) -> usize {
        self.handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Join the workers that have already finished, leaving live ones — and
    /// admission — untouched. Returns how many handles were joined.
    ///
    /// This is the retention half of the group's contract. The daemon serves
    /// every connection on a group thread, so without a periodic reap the
    /// handle list grows by one entry per request for the whole daemon
    /// lifetime. Reaping is pure maintenance: it never sets the stop flag,
    /// never refuses a later `spawn`, and never waits on a worker that is
    /// still running.
    ///
    /// Cheap and non-blocking by construction:
    ///
    /// - finished handles are collected under the `handles` mutex and joined
    ///   *after* it is released, so a worker that spawns (or a caller that
    ///   admits work) is never blocked behind a join;
    /// - the join section is only *tried*, never awaited, so a reap that
    ///   collides with `stop_and_join` reports `Ok(0)` and leaves the drain to
    ///   shutdown.
    ///
    /// A finished worker that panicked is still removed and still joined —
    /// losing the panic would hide a failed writer — but the call then reports
    /// `Err("worker panicked")`, matching [`WorkerGroup::stop_and_join`].
    /// Callers (the accept loop) log it and keep serving: one panicked
    /// connection worker must not take the daemon down.
    ///
    /// **Not callable from inside a supervised worker.** Like `stop_and_join`,
    /// this joins group threads; a worker that called it could wait on itself.
    pub fn reap_finished(&self) -> Result<usize, String> {
        // Try, never wait: a shutdown already holds this and is about to join
        // everything, so there is nothing useful (and nothing safe to block
        // the accept loop on) left to do here.
        let _joining = match self.joining.try_lock() {
            Ok(guard) => guard,
            // A panic in some other join section does not corrupt the `()`
            // behind this mutex; taking it anyway is better than never
            // reaping again for the daemon's whole lifetime.
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(0),
        };
        let finished = {
            let mut handles = self.handles.lock().unwrap_or_else(PoisonError::into_inner);
            // Partition in place: `swap_remove` is O(1) and order carries no
            // meaning here (handles are independent), so a busy daemon pays
            // one pass over a short list per accept tick.
            let mut finished = Vec::new();
            let mut index = 0;
            while index < handles.len() {
                if handles[index].is_finished() {
                    finished.push(handles.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            finished
        };
        // Joining outside the mutex: every handle here has already run to
        // completion, so this is bounded regardless.
        let mut joined = 0;
        let mut failure = None;
        for handle in finished {
            joined += 1;
            if handle.join().is_err() {
                failure = Some("worker panicked".to_string());
            }
        }
        match failure {
            Some(message) => Err(message),
            None => Ok(joined),
        }
    }

    /// Signal cancellation and wait for every admitted worker. Idempotent: a
    /// second call finds an empty handle list and returns `Ok`.
    ///
    /// A poisoned handle lock never aborts the drain. Refusing to join because
    /// some other thread panicked while holding this mutex would leave the
    /// workers running *and* wedge shutdown permanently (the error would
    /// repeat on every retry, so ownership would be held until the process
    /// exits). The `Vec<JoinHandle>` behind the lock is not made inconsistent
    /// by a panic, so the safe move is to take it anyway, join everything, and
    /// report the poison afterwards as a soft failure.
    pub fn stop_and_join(&self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        // Wait out any reap that is mid-join. `reap_finished` removes handles
        // from the list before joining them, so taking the list without this
        // would drain only what the reap left behind and still report a
        // complete drain — a verdict the caller acts on by releasing
        // data-root ownership. The handles in a reap's hands have already
        // finished, so this is about the accounting being exact rather than
        // about a writer still running; see the `joining` field. Bounded for
        // the same reason: a reap only ever joins work that is already done.
        let _joining = self.joining.lock().unwrap_or_else(PoisonError::into_inner);
        // Take the handles out from under the lock before joining: a worker
        // that is still running must be able to observe the refusal path
        // (and any nested `spawn`) without blocking on this join.
        let (handles, poisoned) = {
            let mut guard = match self.handles.lock() {
                Ok(guard) => (guard, false),
                Err(poison) => (poison.into_inner(), true),
            };
            (std::mem::take(&mut *guard.0), guard.1)
        };
        let mut failure = None;
        for handle in handles {
            if handle.join().is_err() {
                failure = Some("worker panicked".to_string());
            }
        }
        match (failure, poisoned) {
            (Some(message), _) => Err(message),
            (None, true) => Err("worker lock poisoned".to_string()),
            (None, false) => Ok(()),
        }
    }
}

/// Connections accepted but not yet dispatched: the daemon parks a clone of
/// each socket here while its worker blocks reading request headers.
///
/// This is the "close/wake socket reads" half of the shutdown order. A peer
/// that connects and sends nothing would otherwise pin its worker until the
/// socket's read timeout expires; shutting the socket down makes that read
/// return immediately, so `stop_and_join` completes promptly instead of merely
/// being bounded by the timeout.
///
/// The exact guarantee, because a mutating handler depends on it:
///
/// - a request whose head was fully read and whose [`release`](Self::release)
///   returned `true` is dispatched, and is never torn down mid-write;
/// - a request woken by [`wake_all`](Self::wake_all) *before* its `release`
///   ran — the narrow window between `read_request` returning and the release
///   taking the lock — is **not dispatched at all**.
///
/// That second case is why `release` reports whether the ticket was still
/// parked. Without it the socket would already be shut down while the handler
/// ran on regardless: the mutation would commit and only the response write
/// would fail, silently. A client retrying the interrupted `POST` against the
/// next daemon instance would then double-apply it. Losing the race means the
/// request is dropped before it takes effect, so a retry is the first and only
/// application.
///
/// Live websocket sessions are never parked here at all; they poll the stop
/// flag themselves.
#[derive(Debug, Default)]
pub struct IdleConnections {
    next_id: AtomicU64,
    parked: Mutex<Vec<(u64, TcpStream)>>,
}

impl IdleConnections {
    /// Park a clone of `stream`; returns the ticket to release it with. A
    /// socket that cannot be cloned is simply not parked (it stays bounded by
    /// its read timeout).
    pub fn park(&self, stream: &TcpStream) -> Option<u64> {
        let clone = stream.try_clone().ok()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        parked.push((id, clone));
        Some(id)
    }

    /// Release a parked socket once its request head has been read.
    ///
    /// Returns whether the ticket was still parked: `true` means this
    /// connection won the race against [`wake_all`](Self::wake_all) and may be
    /// dispatched; `false` means shutdown already shut the socket down and the
    /// caller must **not** run the handler. An un-parked connection (`None`
    /// ticket, i.e. the socket could not be cloned) reports `true` — it was
    /// never in the wake set, so nothing interrupted it.
    #[must_use]
    pub fn release(&self, ticket: Option<u64>) -> bool {
        let Some(ticket) = ticket else { return true };
        let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        let before = parked.len();
        parked.retain(|(id, _)| *id != ticket);
        parked.len() < before
    }

    /// Wake every parked read. Called once admission has stopped.
    pub fn wake_all(&self) {
        let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, stream) in parked.drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn stop_and_join_waits_for_admitted_work() {
        let stop = Arc::new(AtomicBool::new(false));
        let group = WorkerGroup::new(Arc::clone(&stop));
        let finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&finished);
        group
            .spawn(Box::new(move |_stop| {
                std::thread::sleep(Duration::from_millis(150));
                flag.store(true, Ordering::Release);
            }))
            .unwrap();
        let started = Instant::now();
        group.stop_and_join().unwrap();
        assert!(
            finished.load(Ordering::Acquire),
            "worker must have finished"
        );
        assert!(started.elapsed() >= Duration::from_millis(140));
        assert!(stop.load(Ordering::Acquire), "the shared flag is set");
    }

    #[test]
    fn admission_is_refused_once_stopping() {
        let group = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
        group.stop_and_join().unwrap();
        let error = group.spawn(Box::new(|_| {})).unwrap_err();
        assert!(error.contains("shutting down"), "{error}");
        // A second join is a clean no-op.
        group.stop_and_join().unwrap();
    }

    #[test]
    fn a_panicking_worker_is_reported() {
        let group = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
        group
            .spawn(Box::new(|_| panic!("worker under test")))
            .unwrap();
        let error = group.stop_and_join().unwrap_err();
        assert_eq!(error, "worker panicked");
    }

    #[test]
    fn release_reports_whether_the_connection_won_the_race_against_wake_all() {
        // The dispatch gate in `serve_connection` depends on this exact
        // signal: `false` means shutdown already shut the socket down, so the
        // handler must not run (otherwise a mutation would commit while its
        // response write silently failed, and the client's retry against the
        // next daemon would double-apply it).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (accepted, _) = listener.accept().unwrap();

        let idle = IdleConnections::default();

        // Won the race: released before any wake.
        let ticket = idle.park(&accepted);
        assert!(idle.release(ticket), "an unwoken ticket releases cleanly");
        // Releasing twice reports `false`; only the first release owns it.
        assert!(!idle.release(ticket));

        // Lost the race: `wake_all` drained the ticket first.
        let ticket = idle.park(&accepted);
        idle.wake_all();
        assert!(
            !idle.release(ticket),
            "a woken ticket must report that it was already torn down"
        );

        // A connection that was never parked was never in the wake set.
        assert!(idle.release(None));
        drop(client);
    }

    #[test]
    fn a_poisoned_handle_lock_still_joins_every_worker() {
        // A poisoned mutex must never abort the drain: refusing to join would
        // leave workers running AND wedge shutdown forever, since the error
        // would repeat on every retry and ownership is held until it succeeds.
        use std::sync::atomic::AtomicUsize;

        let group = Arc::new(WorkerGroup::new(Arc::new(AtomicBool::new(false))));
        let ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ran);
        group
            .spawn(Box::new(move |_| {
                std::thread::sleep(Duration::from_millis(50));
                counter.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap();

        // Poison the handles mutex from another thread.
        let poisoner = Arc::clone(&group);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.handles.lock().unwrap();
            panic!("poisoning the handle lock");
        })
        .join();
        assert!(group.handles.is_poisoned());

        // The worker is still joined, and the poison is reported as a soft
        // failure rather than skipping the drain.
        let error = group.stop_and_join().unwrap_err();
        assert_eq!(error, "worker lock poisoned");
        assert_eq!(ran.load(Ordering::SeqCst), 1, "the worker must be joined");
        assert_eq!(group.live_count(), 0, "the handle list was drained");
    }

    #[test]
    fn workers_observe_the_shared_cancellation_flag() {
        let stop = Arc::new(AtomicBool::new(false));
        let group = WorkerGroup::new(Arc::clone(&stop));
        group
            .spawn(Box::new(|stop| {
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }))
            .unwrap();
        assert_eq!(group.live_count(), 1);
        // Would hang forever if `stop_and_join` did not signal first.
        group.stop_and_join().unwrap();
        assert_eq!(group.live_count(), 0);
    }
}

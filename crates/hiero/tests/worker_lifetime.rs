//! Worker lifetime: nothing the daemon spawns may outlive data-root
//! ownership, and nothing it finishes may be retained forever.
//!
//! Two regressions live here (review findings A2 and A9):
//!
//! - **A2** — `Daemon::start` used to spawn the semantic worker *before* it
//!   bound the listener, so the common "port is occupied" refusal dropped a
//!   `WorkerGroup` that joins nothing (detaching a live writer) and a
//!   `RootOwnership` that releases immediately. The public constructor could
//!   therefore leave a writer running on a root another process was free to
//!   take over. Bind and credential validation now precede every spawn, and
//!   every fallible step after the first spawn unwinds through a guard that
//!   signals, joins, unpublishes, and only then releases the root.
//! - **A9** — the group appended one `JoinHandle` per accepted connection and
//!   only drained at shutdown, so a long-lived daemon retained a handle per
//!   request served. `WorkerGroup::reap_finished` retires completed work
//!   without stopping admission, and the accept loop calls it every tick.
//!
//! Real threads and real sockets throughout; every wait is bounded.

mod common;

use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hiero::daemon::semantic_worker::{ArmedPair, SemanticArm, install_test_arm};
use hiero::daemon::workers::WorkerGroup;
use hiero::daemon::{Daemon, DaemonOptions};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::ownership::RootOwnership;
use hieronymus::semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider};

use common::{send_request, start_daemon_on_ephemeral_port, wait_until};

// ------------------------------------------------------------------- reaping

#[test]
fn finished_work_is_reaped_without_stopping_admission() {
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    for _ in 0..100 {
        workers.spawn(Box::new(|_| {})).unwrap();
    }
    // Accumulate across ticks rather than demanding one exhaustive pass: a
    // reap only ever takes what has already finished, so on a loaded runner
    // the hundredth closure may still be starting up. The property under test
    // is that all of it is eventually retired — and that admission kept
    // working the whole time.
    let reaped = AtomicUsize::new(0);
    assert!(
        wait_until(
            || {
                let seen = workers.reap_finished().unwrap_or(0);
                reaped.fetch_add(seen, Ordering::SeqCst) + seen == 100
            },
            Duration::from_secs(10)
        ),
        "every finished worker must be reaped (reaped {})",
        reaped.load(Ordering::SeqCst)
    );
    assert_eq!(workers.live_count(), 0);
    workers.spawn(Box::new(|_| {})).unwrap();
    workers.stop_and_join().unwrap();
}

#[test]
fn a_live_worker_is_never_reaped_and_stays_supervised_until_the_join() {
    // Reaping must be able to run at any moment (the accept loop calls it on
    // every tick) without disturbing work in flight.
    let release = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    let gate = Arc::clone(&release);
    let done = Arc::clone(&finished);
    workers
        .spawn(Box::new(move |stop| {
            while !gate.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            done.store(true, Ordering::Release);
        }))
        .unwrap();
    // Two short workers alongside it, so the reap has something to remove.
    workers.spawn(Box::new(|_| {})).unwrap();
    workers.spawn(Box::new(|_| {})).unwrap();
    assert!(wait_until(
        || workers.live_count() == 3,
        Duration::from_secs(2)
    ));

    // Accumulate: the two short workers need not have finished by the same
    // reap tick, and a reap only ever takes what is already done.
    let reaped = AtomicUsize::new(0);
    assert!(
        wait_until(
            || {
                let seen = workers.reap_finished().unwrap_or(0);
                reaped.fetch_add(seen, Ordering::SeqCst) + seen == 2
            },
            Duration::from_secs(5)
        ),
        "the two completed workers must be reaped (reaped {})",
        reaped.load(Ordering::SeqCst)
    );
    assert_eq!(
        workers.live_count(),
        1,
        "the live worker is still supervised"
    );
    assert!(!finished.load(Ordering::Acquire));

    release.store(true, Ordering::Release);
    workers.stop_and_join().unwrap();
    assert!(finished.load(Ordering::Acquire));
    assert_eq!(workers.live_count(), 0);
}

#[test]
fn a_panicking_finished_worker_is_reported_and_the_rest_are_still_reaped() {
    // Panic diagnostics must survive the reap: a panicked worker is a failed
    // writer, and silently dropping its handle would hide that. The other
    // finished handles are removed all the same.
    let workers = WorkerGroup::new(Arc::new(AtomicBool::new(false)));
    workers
        .spawn(Box::new(|_| panic!("worker under test")))
        .unwrap();
    for _ in 0..4 {
        workers.spawn(Box::new(|_| {})).unwrap();
    }

    // Reap until the group is empty, remembering whether the panic was ever
    // reported. Nothing here may assume all five finished within one tick:
    // the panic must be reported by whichever pass joins it, and every other
    // handle must still be removed.
    let panic_reported = AtomicBool::new(false);
    assert!(
        wait_until(
            || {
                if let Err(error) = workers.reap_finished() {
                    assert_eq!(error, "worker panicked");
                    panic_reported.store(true, Ordering::Release);
                }
                workers.live_count() == 0
            },
            Duration::from_secs(10)
        ),
        "a panic must not leave the other finished handles behind"
    );
    assert!(
        panic_reported.load(Ordering::Acquire),
        "the panic diagnostics must survive the reap"
    );
    // A panicked worker never wedges admission.
    workers.spawn(Box::new(|_| {})).unwrap();
    workers.stop_and_join().unwrap();
}

#[test]
fn a_worker_that_spawns_another_is_reaped_without_deadlock() {
    // Nested admission is the case the handle mutex must never be held
    // across: the outer worker spawns while the reap is joining.
    let workers = Arc::new(WorkerGroup::new(Arc::new(AtomicBool::new(false))));
    let inner_ran = Arc::new(AtomicBool::new(false));
    let group = Arc::clone(&workers);
    let flag = Arc::clone(&inner_ran);
    workers
        .spawn(Box::new(move |_| {
            group
                .spawn(Box::new(move |_| {
                    std::thread::sleep(Duration::from_millis(30));
                    flag.store(true, Ordering::Release);
                }))
                .expect("a nested spawn is admitted while the group is running");
        }))
        .unwrap();

    let reaped = AtomicUsize::new(0);
    assert!(
        wait_until(
            || {
                let seen = workers.reap_finished().unwrap_or(0);
                reaped.fetch_add(seen, Ordering::SeqCst) + seen == 2
            },
            Duration::from_secs(5)
        ),
        "both the outer and the nested worker must be reaped (reaped {})",
        reaped.load(Ordering::SeqCst)
    );
    assert!(inner_ran.load(Ordering::Acquire));
    workers.stop_and_join().unwrap();
}

#[test]
fn a_reap_and_a_shutdown_may_run_at_the_same_time() {
    // What this proves: the two drains can run concurrently, 20 rounds in a
    // row, without deadlocking on the `joining`/`handles` lock pair, without
    // losing a worker between them, and without either call reporting a
    // failure. It does *not* isolate the accounting the `joining` mutex
    // protects — a reap only ever takes handles whose closure already
    // returned, so a shutdown that raced one still sees every worker's effect
    // either way. That ordering is argued at the mutex itself; this is the
    // stress test that the argument holds up under a real race.
    for _ in 0..20 {
        let workers = Arc::new(WorkerGroup::new(Arc::new(AtomicBool::new(false))));
        let completed = Arc::new(AtomicUsize::new(0));
        for index in 0..24 {
            let counter = Arc::clone(&completed);
            workers
                .spawn(Box::new(move |_| {
                    // Half finish immediately (reap fodder), half outlive the
                    // reap and must be joined by the shutdown.
                    if index % 2 == 0 {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                }))
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));

        let reaper = {
            let workers = Arc::clone(&workers);
            std::thread::spawn(move || workers.reap_finished())
        };
        let stopper = {
            let workers = Arc::clone(&workers);
            let counter = Arc::clone(&completed);
            std::thread::spawn(move || {
                let result = workers.stop_and_join();
                // Read the world as the shutdown caller sees it: this is the
                // point at which the daemon would release ownership.
                (result, counter.load(Ordering::SeqCst))
            })
        };
        let reaped = reaper.join().unwrap().expect("no worker panicked");
        let (stopped, observed) = stopper.join().unwrap();
        stopped.expect("no worker panicked");

        assert!(reaped <= 24);
        assert_eq!(
            observed, 24,
            "every worker must have run to completion before stop_and_join returned"
        );
        assert_eq!(completed.load(Ordering::SeqCst), 24);
        assert_eq!(workers.live_count(), 0, "no handle was left behind");
    }
}

// -------------------------------------------------- the counted semantic arm

/// A [`SemanticArm`] that reports how far the daemon's semantic worker got.
///
/// `arm` is the only part that runs on the worker thread, so it counts in a
/// short bounded loop: a *detached* worker keeps ticking after `Daemon::start`
/// returned, a joined one cannot. `precheck` runs on the startup thread and
/// therefore proves whether the controller was started at all.
struct CountingArm {
    prechecks: AtomicUsize,
    ticks: AtomicUsize,
}

/// How long `arm` stays busy: long enough that a detached worker is visibly
/// still running when startup fails, short enough to keep the join bounded.
const ARM_TICKS: usize = 30;
const ARM_TICK: Duration = Duration::from_millis(20);

impl CountingArm {
    fn new() -> Arc<CountingArm> {
        Arc::new(CountingArm {
            prechecks: AtomicUsize::new(0),
            ticks: AtomicUsize::new(0),
        })
    }

    fn prechecks(&self) -> usize {
        self.prechecks.load(Ordering::SeqCst)
    }

    fn ticks(&self) -> usize {
        self.ticks.load(Ordering::SeqCst)
    }
}

impl SemanticArm for CountingArm {
    fn identity(&self) -> hieronymus::semantic_embeddings::EmbeddingIdentity {
        FakeEmbeddingProvider::new(384).identity().clone()
    }

    fn precheck(&self, _config: &HieronymusConfig) -> Result<(), String> {
        self.prechecks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn arm(&self, _config: &HieronymusConfig) -> Result<ArmedPair, String> {
        for _ in 0..ARM_TICKS {
            self.ticks.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(ARM_TICK);
        }
        Err("the counted test arm never arms".to_string())
    }
}

fn options(root: &Path, port: u16) -> DaemonOptions {
    DaemonOptions {
        data_root: Some(root.to_path_buf()),
        port,
        ..Default::default()
    }
}

/// Assert the post-failure invariant: the root is free *and* no worker of the
/// failed daemon is still executing on it.
fn assert_no_writer_survived(root: &Path, arm: &CountingArm) {
    let config = HieronymusConfig::new(root);
    let guard = RootOwnership::acquire(&config, "probe")
        .expect("the root must be free the moment start() returned an error");
    let before = arm.ticks();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        arm.ticks(),
        before,
        "a semantic worker was still running after ownership was released"
    );
    drop(guard);
}

// ------------------------------------------------- failed startup boundaries

#[test]
fn an_occupied_port_never_spawns_a_worker() {
    // The A2 reproduction. Bind is decided before any thread exists, so the
    // most common startup refusal cannot detach a writer: the semantic
    // controller is never even constructed.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    let arm = CountingArm::new();
    install_test_arm(root.path(), Arc::clone(&arm) as Arc<dyn SemanticArm>);

    let error = Daemon::start(&options(root.path(), port)).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("cannot bind loopback endpoint"), "{text}");

    assert_eq!(
        arm.prechecks(),
        0,
        "the semantic controller must not start before the listener is bound"
    );
    assert_eq!(arm.ticks(), 0);
    assert_no_writer_survived(root.path(), &arm);
    assert!(
        !root.path().join("daemon.json").exists(),
        "a failed start publishes no readiness"
    );
    drop(occupied);
}

#[test]
fn an_unusable_credential_never_spawns_a_worker() {
    // Credential validation is the other pre-spawn gate: a token file that
    // cannot be used is a plain refusal, with no thread to clean up.
    let root = tempfile::tempdir().unwrap();
    let token = root.path().join("daemon.token");
    std::fs::write(&token, "ab".repeat(32)).unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
    let arm = CountingArm::new();
    install_test_arm(root.path(), Arc::clone(&arm) as Arc<dyn SemanticArm>);

    let error = Daemon::start(&options(root.path(), 0)).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("readable beyond its owner"), "{text}");

    assert_eq!(arm.prechecks(), 0, "no worker is started for a bad token");
    assert_no_writer_survived(root.path(), &arm);
}

#[test]
fn a_failed_discovery_publish_joins_its_workers_before_releasing_ownership() {
    // The post-spawn half: discovery publication happens *after* the semantic
    // worker exists, so this is the path the startup guard has to cover. A
    // directory where the record belongs makes the atomic rename fail.
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("daemon.json")).unwrap();
    let arm = CountingArm::new();
    install_test_arm(root.path(), Arc::clone(&arm) as Arc<dyn SemanticArm>);

    let error = Daemon::start(&options(root.path(), 0)).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("cannot write the discovery record"), "{text}");

    assert_eq!(
        arm.prechecks(),
        1,
        "the semantic worker must really have been started for this to be the guarded path"
    );
    assert!(
        arm.ticks() > 0,
        "the worker must really have been running when startup failed"
    );
    // The guard joined it: ownership is free and nothing is still ticking.
    assert_no_writer_survived(root.path(), &arm);
}

// The remaining post-spawn failure — `DreamController::start`, which runs
// after the discovery record is published — is only reachable when worker
// admission is already refused, so it has no external trigger. The startup
// guard's own drop order (join → unpublish → release) is covered directly by
// the unit tests in `daemon::mod`.

// ------------------------------------------------------- daemon-side retention

#[test]
fn a_served_request_is_not_retained_for_the_daemon_lifetime() {
    // The A9 reproduction over the real accept loop: 20 completed requests
    // must not leave 20 handles behind.
    let (root, mut daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let baseline = daemon.worker_count();

    for _ in 0..20 {
        let response = send_request(port, "GET", "/health", &[], b"");
        assert_eq!(response.status, 200);
    }

    assert!(
        wait_until(
            || daemon.worker_count() <= baseline + 1,
            Duration::from_secs(5)
        ),
        "completed connection workers must be reaped (baseline {baseline}, now {})",
        daemon.worker_count()
    );
    daemon.stop().unwrap();
    drop(root);
}

#[test]
fn reaping_never_interferes_with_a_request_in_flight() {
    // Admission and dispatch keep working while the accept loop reaps: a slow
    // request served across many reap ticks still completes.
    let (root, mut daemon) = start_daemon_on_ephemeral_port();
    let port = daemon.local_addr().port();
    let responses = Arc::new(Mutex::new(Vec::new()));
    let mut clients = Vec::new();
    for _ in 0..8 {
        let sink = Arc::clone(&responses);
        clients.push(std::thread::spawn(move || {
            let response = send_request(port, "GET", "/health", &[], b"");
            sink.lock().unwrap().push(response.status);
        }));
    }
    for client in clients {
        client.join().unwrap();
    }
    let statuses = responses.lock().unwrap().clone();
    assert_eq!(statuses.len(), 8);
    assert!(statuses.iter().all(|status| *status == 200), "{statuses:?}");
    daemon.stop().unwrap();
    drop(root);
}

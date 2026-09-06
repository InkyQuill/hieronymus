//! Task D5 process evidence: production dream runs execute on the daemon's
//! supervised dream controller — drained, scheduled, coalesced, and joined
//! on shutdown (Astra 5 and 17).
//!
//! Process-level proof per the task brief: a loopback provider with
//! completed sessions exceeding twice the batch cap. Verifies that each
//! input is archived exactly once (the drain), that disabled automatic
//! scheduling makes no provider calls, that manual execution works over the
//! admin action and the MCP tool, that a mid-run failure leaves the
//! remaining inputs pending, and that SIGTERM joins the worker. The pure
//! `drain_batches` regression (crates/hieronymus/tests/dream_drain.rs) does
//! not establish this wiring; these tests do.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use hiero::daemon::dream_worker::{DreamController, DreamRequest};
use hiero::daemon::workers::WorkerGroup;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::ownership::RootOwnership;
use hieronymus::provider_config::{
    ProviderCatalog, ProviderProfile, load_provider_catalog, save_provider_catalog,
};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

const PASS_TIMEOUT: Duration = Duration::from_secs(30);

fn wait_for(condition: impl Fn() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "condition never became true");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn scalar(root: &Path, sql: &str) -> i64 {
    let config = HieronymusConfig::new(root);
    open_migrated(&config.database_path())
        .unwrap()
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Loopback LLM with a mid-run failure switch and a request gate
// ---------------------------------------------------------------------------

/// Blocks one numbered request (after recording) until released, so a test
/// can hold a batch mid-flight and orchestrate shutdown around it.
#[derive(Default)]
struct GateState {
    block_at: Option<usize>,
    released: bool,
}

struct LoopbackLlm {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    gate: Arc<(Mutex<GateState>, Condvar)>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackLlm {
    fn start() -> Self {
        Self::start_with(std::sync::atomic::AtomicUsize::new(usize::MAX))
    }

    fn start_failing_after(seen: usize) -> Self {
        Self::start_with(std::sync::atomic::AtomicUsize::new(seen))
    }

    fn start_with(fail_after: std::sync::atomic::AtomicUsize) -> Self {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let gate = Arc::new((Mutex::new(GateState::default()), Condvar::new()));
        let fail_after = Arc::new(fail_after);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread_gate = Arc::clone(&gate);
        let thread_fail = Arc::clone(&fail_after);
        let accept_thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let requests = Arc::clone(&thread_requests);
                        let gate = Arc::clone(&thread_gate);
                        let fail_after = Arc::clone(&thread_fail);
                        std::thread::spawn(move || {
                            serve_connection(stream, &requests, &gate, &fail_after)
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            gate,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Hold request number `at` (1-based, after recording) until released.
    fn block_request(&self, at: usize) {
        let (state, _) = &*self.gate;
        let mut guard = state.lock().unwrap();
        guard.block_at = Some(at);
    }

    fn release(&self) {
        let (state, signal) = &*self.gate;
        let mut guard = state.lock().unwrap();
        guard.released = true;
        signal.notify_all();
    }
}

impl Drop for LoopbackLlm {
    fn drop(&mut self) {
        self.release();
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.accept_thread.take() {
            handle.join().unwrap();
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    requests: &Mutex<Vec<Value>>,
    gate: &(Mutex<GateState>, Condvar),
    fail_after: &AtomicUsize,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    // Record first and drop the lock: a held gate must never block the
    // test's request_count polling.
    let seen_count = {
        let mut seen = requests.lock().unwrap();
        seen.push(request.clone());
        seen.len()
    };
    // The gate: hold this request until released.
    {
        let (state, signal) = gate;
        let mut guard = state.lock().unwrap();
        while guard.block_at == Some(seen_count) && !guard.released {
            guard = signal.wait(guard).unwrap();
        }
    }
    let (status, content) = if seen_count > fail_after.load(Ordering::Acquire) {
        (500, "bad gateway".to_string())
    } else {
        (200, answer(&request))
    };
    let body = json!({ "choices": [{ "message": { "content": content } }] }).to_string();
    let response = format!(
        "HTTP/1.1 {status} dream\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// The dreaming answers, derived exactly like the deterministic provider so
/// downstream state (one crystal per memory) stays predictable.
fn answer(request: &Value) -> String {
    let instruction: Value = serde_json::from_str(
        request["messages"][0]["content"]
            .as_str()
            .unwrap_or_default(),
    )
    .unwrap_or(Value::Null);
    let pass = instruction["instruction"]
        .as_str()
        .unwrap_or_default()
        .split("Dream pass: ")
        .nth(1)
        .map(|rest| rest.split('.').next().unwrap_or_default())
        .unwrap_or_default()
        .to_string();
    let memories = instruction["memories"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let memory_ids: Vec<Value> = memories.iter().map(|memory| memory["id"].clone()).collect();
    if pass == "coverage_audit" {
        return json!({ "covered_memory_ids": memory_ids }).to_string();
    }
    if pass == "knowledge_crystals" {
        let crystals: Vec<Value> = memories
            .iter()
            .map(|memory| {
                let rule = memory["source_credibility"] == json!("user_rule")
                    || !memory["rule_intent"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .is_empty();
                json!({
                    "crystal_type": if rule { "rule" } else { "observation" },
                    "title": hieronymus::dreaming::title_from_kind(
                        memory["kind"].as_str().unwrap_or_default()),
                    "text": hieronymus::dreaming::normalize_candidate_text(
                        memory["text"].as_str().unwrap_or_default()),
                    "strength": 0.6,
                    "confidence": hieronymus::dreaming::source_credibility_confidence(
                        memory["source_credibility"].as_str().unwrap_or_default()),
                    "source_memory_ids": [memory["id"].clone()],
                })
            })
            .collect();
        return json!({ "crystals": crystals }).to_string();
    }
    "{}".to_string()
}

fn read_request(stream: &mut TcpStream) -> Option<Value> {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 8192];
    let separator = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(count) => raw.extend_from_slice(&buffer[..count]),
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&raw[..separator]).to_string();
    let content_length = head
        .split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = raw[separator + 4..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => body.extend_from_slice(&buffer[..count]),
            Err(_) => break,
        }
    }
    serde_json::from_slice(&body).ok()
}

// ---------------------------------------------------------------------------
// Data-root seeding and provider wiring
// ---------------------------------------------------------------------------

fn config(root: &Path) -> HieronymusConfig {
    HieronymusConfig::new(root)
}

/// `sessions` completed sessions with `memories_per_session` pending
/// short-term memories each.
fn seed_backlog(config: &HieronymusConfig, sessions: usize, memories_per_session: usize) {
    Registry::open(config)
        .unwrap()
        .create_series("book", "Book", "ja", "ru", None)
        .unwrap();
    for session_index in 0..sessions {
        let workspace = WorkspaceStore::open(config).unwrap();
        let context =
            hieronymus::memory_models::TranslationContext::new("book", "ja", "ru", "translate");
        let session = workspace.start_session(&context).unwrap();
        for memory_index in 0..memories_per_session {
            workspace
                .add_short_term_memory(
                    session.id,
                    &ShortTermMemoryInput::new(
                        "note",
                        format!("Memory {session_index}.{memory_index} awaits the dream drain."),
                    ),
                )
                .unwrap();
        }
        workspace.complete_session(session.id).unwrap();
    }
}

/// Wire the two required workflows to the loopback lane. `enabled` is the
/// global automatic-scheduling switch (manual dreaming bypasses it; the
/// per-workflow assignments stay enabled either way). `timeout_seconds`
/// must cover any test-orchestrated gate hold.
fn wire_lane(
    config: &HieronymusConfig,
    url: &str,
    enabled: bool,
    min_pending: i64,
    batch_cap: i64,
    timeout_seconds: f64,
) {
    let catalog = ProviderCatalog::default().with_provider(
        "loopback-lane",
        ProviderProfile::new(
            "Loopback Lane",
            "openai",
            url,
            "loopback-key",
            timeout_seconds,
        ),
    );
    save_provider_catalog(config, &catalog).unwrap();
    let mut dream_config = default_dream_config();
    dream_config.enabled = enabled;
    dream_config.min_pending_short_term_memories = min_pending;
    dream_config.max_pending_short_term_memories = min_pending.max(batch_cap);
    dream_config.max_short_term_memories_per_cycle = min_pending.max(batch_cap);
    dream_config.max_short_term_memories_per_run = batch_cap;
    for name in ["coverage_audit", "knowledge_crystals"] {
        let workflow = dream_config.workflows.get_mut(name).unwrap();
        workflow.provider = "loopback-lane".to_string();
        workflow.model = "test-model".to_string();
        workflow.enabled = true;
    }
    save_dream_config(config, &dream_config).unwrap();
}

/// The test injection seam in action: a provider source sampled inside the
/// worker from the data root's real catalog (the loopback lane).
fn start_controller(config: HieronymusConfig) -> (WorkerGroup, DreamController) {
    let source_config = config.clone();
    let source = Arc::new(move || {
        WorkflowResolver::from_catalog(load_provider_catalog(&source_config).unwrap())
    });
    let stop = Arc::new(AtomicBool::new(false));
    let mut workers = WorkerGroup::new(Arc::clone(&stop));
    let controller =
        DreamController::start_with_provider_source(config, &mut workers, source).unwrap();
    (workers, controller)
}

// ---------------------------------------------------------------------------
// Process-level evidence
// ---------------------------------------------------------------------------

/// Manual dreaming drains a backlog larger than twice the batch cap: every
/// input is archived exactly once across capped batches, the provider is
/// called exactly twice per non-empty batch, and the MCP tool answers from
/// the same controller without re-running pending work.
#[test]
fn manual_dream_drains_a_backlog_larger_than_two_caps_exactly_once() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    // 12 inputs at a batch cap of 4: three capped batches (> 2x the cap).
    seed_backlog(&config, 3, 4);
    let llm = LoopbackLlm::start();
    wire_lane(&config, &llm.url(), false, 1, 4, 5.0);

    let daemon = common::start_daemon(root.path());

    // Manual execution through the admin action (the frozen route shape).
    let (_grant, session) = common::browser_session(&daemon);
    let origin = format!("http://127.0.0.1:{}", daemon.local_addr().port());
    let response = common::send_request(
        daemon.local_addr().port(),
        "POST",
        "/api/admin/actions/run_manual_dreaming",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={session}"),
            ),
            ("Origin".to_string(), origin),
        ],
        br#"{}"#,
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body(),
        json!({"started": true, "status": "running"})
    );

    // The controller drains batch after batch until the backlog is gone.
    wait_for(
        || {
            scalar(
                root.path(),
                "select count(*) from short_term_memories where archived_at is null",
            ) == 0
        },
        PASS_TIMEOUT,
    );
    wait_for(
        || {
            scalar(
                root.path(),
                "select count(*) from dream_runs where status = 'completed'",
            ) >= 3
        },
        PASS_TIMEOUT,
    );
    // Give a coalescing bug an empty cycle to show up in, then assert the
    // drain stopped for real.
    std::thread::sleep(Duration::from_millis(500));

    // Each input archived exactly once: the totals of the three completed
    // capped batches equal the backlog, and nothing is left pending.
    assert_eq!(
        scalar(
            root.path(),
            "select coalesce(sum(input_count), 0) from dream_runs where status = 'completed'"
        ),
        12
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'completed'"
        ),
        3,
        "exactly three capped batches"
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from short_term_memories where archived_at is not null"
        ),
        12
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from task_sessions where status = 'dreamed'"
        ),
        3
    );
    // Two provider passes per non-empty batch, and never a fourth batch.
    assert_eq!(
        llm.request_count(),
        6,
        "capped batches re-select; no over-calls"
    );

    // The controller's honest status: the drain finished with three batches.
    let status = daemon.dream_status();
    assert!(status.active.is_none());
    let last = status.last.expect("a finished run is recorded");
    assert_eq!(last.outcome.as_deref(), Some("completed"));
    assert_eq!(last.batches, Some(3));
    assert_eq!(last.input_count, Some(12));

    // The MCP tool shares the controller: with nothing pending it completes
    // an honest empty cycle and dials no provider.
    let output = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "tool-call",
            "hieronymus_dream",
            "--args",
            "{}",
            "--data-root",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(payload["status"], json!("completed"));
    assert_eq!(payload["provider"], json!("openai"));
    assert_eq!(payload["input_count"], json!(0));
    assert_eq!(llm.request_count(), 6, "the MCP call dialed no provider");
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'completed'"
        ),
        4,
        "the MCP run is the fourth, empty batch"
    );

    daemon.shutdown().unwrap();
}

/// Disabled automatic scheduling makes no provider calls and records
/// nothing: the controller reads the actual config state and stands down.
#[test]
fn disabled_automatic_scheduling_makes_no_provider_calls() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 3, 4);
    let llm = LoopbackLlm::start();
    wire_lane(&config, &llm.url(), false, 1, 4, 5.0);

    let daemon = common::start_daemon(root.path());
    // The first scheduled decision fires immediately at controller start;
    // give the worker several gate ticks to prove it stands down.
    std::thread::sleep(Duration::from_secs(3));

    assert_eq!(llm.request_count(), 0, "no provider call while disabled");
    assert_eq!(
        scalar(root.path(), "select count(*) from dream_runs"),
        0,
        "a disabled schedule records neither runs nor skips"
    );
    let status = daemon.dream_status();
    assert!(status.active.is_none());
    assert!(status.last.is_none());
    daemon.shutdown().unwrap();
}

/// A mid-run provider failure fails the batch durably and leaves the
/// remaining inputs pending — the drain stops honestly instead of spinning
/// or claiming success.
#[test]
fn a_mid_run_failure_leaves_the_remaining_inputs_pending() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 3, 4);
    // Two requests succeed (the first batch), then every request fails.
    let llm = LoopbackLlm::start_failing_after(2);
    wire_lane(&config, &llm.url(), false, 1, 4, 5.0);

    let daemon = common::start_daemon(root.path());
    let (_grant, session) = common::browser_session(&daemon);
    let origin = format!("http://127.0.0.1:{}", daemon.local_addr().port());
    let response = common::send_request(
        daemon.local_addr().port(),
        "POST",
        "/api/admin/actions/run_manual_dreaming",
        &[
            (
                "Cookie".to_string(),
                format!("hieronymus_session={session}"),
            ),
            ("Origin".to_string(), origin),
        ],
        br#"{}"#,
    );
    assert_eq!(response.status, 200);

    wait_for(
        || {
            scalar(
                root.path(),
                "select count(*) from dream_runs where status = 'failed'",
            ) >= 1
        },
        PASS_TIMEOUT,
    );

    // The completed first batch is durable; the failing second batch is a
    // durable failed run; the remaining eight inputs stay pending.
    assert_eq!(
        scalar(
            root.path(),
            "select coalesce(sum(input_count), 0) from dream_runs where status = 'completed'"
        ),
        4
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from short_term_memories where archived_at is null"
        ),
        8,
        "the remaining inputs stay pending"
    );
    let requests_after_failure = llm.request_count();
    assert!(
        requests_after_failure >= 3,
        "the failing batch was attempted"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        llm.request_count(),
        requests_after_failure,
        "the drain stopped; no spin"
    );
    let status = daemon.dream_status();
    assert_eq!(
        status.last.expect("finished").outcome.as_deref(),
        Some("failed")
    );
    daemon.shutdown().unwrap();
}

/// SIGTERM joins the dream worker mid-run through the graceful shutdown:
/// the in-flight batch finishes durably, the drain stops at the batch
/// boundary, and the process releases the data root.
#[test]
fn a_sigterm_joins_the_dream_worker_mid_run() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 3, 4);
    let llm = LoopbackLlm::start();
    // Automatic dreaming enabled: the first scheduled decision fires at
    // startup (no prior decision, so the interval has elapsed). The second
    // provider request (the first batch's coverage pass) is held until the
    // test releases it, so SIGTERM lands mid-run.
    wire_lane(&config, &llm.url(), true, 1, 4, 120.0);
    llm.block_request(2);

    let mut child = Command::new(env!("CARGO_BIN_EXE_hiero"))
        .args([
            "daemon",
            "--data-root",
            root.path().to_str().unwrap(),
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // The ready line is printed only after the signal handler is installed.
    let mut ready = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut ready,
    )
    .expect("the daemon must print its ready line");
    assert!(ready.contains("listening"), "{ready}");

    // The scheduled run started and is held mid-batch.
    wait_for(|| llm.request_count() >= 2, PASS_TIMEOUT);

    // A watchdog keeps a hung join from hanging the suite forever.
    let child_pid = child.id();
    let killed = Arc::new(AtomicBool::new(false));
    let thread_killed = Arc::clone(&killed);
    let watchdog = std::thread::spawn(move || {
        for _ in 0..600 {
            if thread_killed.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = Command::new("kill")
            .args(["-9", &child_pid.to_string()])
            .status();
    });

    // SIGTERM, exactly like a service manager's stop.
    let signaled = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill must be available");
    assert!(signaled.success());
    // Release the held request: the in-flight batch finishes, the drain
    // observes cancellation at the batch boundary, and the worker exits.
    llm.release();

    let status = child.wait().unwrap();
    killed.store(true, Ordering::Release);
    let _ = watchdog.join();
    assert_eq!(
        status.code(),
        Some(0),
        "SIGTERM must exit through the graceful path"
    );

    // The daemon removed its discovery record and released the data root.
    assert!(!root.path().join("daemon.json").exists());
    let probe = RootOwnership::acquire(&config, "probe");
    assert!(probe.is_ok(), "SIGTERM releases data-root ownership");
    drop(probe);

    // The in-flight batch committed durably; the drain stopped honestly at
    // the boundary; nothing is left marked running.
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'completed'"
        ),
        1
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from short_term_memories where archived_at is null"
        ),
        8,
        "the remaining batches were not started"
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'running'"
        ),
        0
    );
}

// ---------------------------------------------------------------------------
// Controller-level evidence: coalescing, scheduling, and shutdown boundaries
// ---------------------------------------------------------------------------

/// Concurrent triggers coalesce into one active run: a request while a run
/// is active receives the active run's ticket, and only one drain executes.
#[test]
fn concurrent_requests_coalesce_into_one_active_run() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 1, 1);
    let llm = LoopbackLlm::start();
    wire_lane(&config, &llm.url(), false, 1, 10, 120.0);
    llm.block_request(1);

    let (workers, controller) = start_controller(config);

    let first = controller
        .request(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap();
    wait_for(|| controller.status().active.is_some(), PASS_TIMEOUT);

    // Two more triggers while the run is active: same ticket, same run.
    let second = controller
        .request(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap();
    assert_eq!(first.run_id, second.run_id, "requests coalesce");
    let waiter = {
        let controller = controller.clone();
        std::thread::spawn(move || {
            controller.request_and_wait(DreamRequest {
                all: true,
                manual: true,
            })
        })
    };

    llm.release();
    let drain = waiter.join().unwrap().expect("the coalesced run completes");
    assert_eq!(drain.batches, 1);
    assert_eq!(drain.input_count, 1);
    // One batch, two passes — the coalesced requests never ran a second
    // DreamService.
    assert_eq!(llm.request_count(), 2);

    let status = controller.status();
    assert!(status.active.is_none());
    assert_eq!(status.last.expect("finished").run_id, first.run_id);

    // A request after completion starts a new run with its own ticket.
    let third = controller
        .request(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap();
    assert_ne!(third.run_id, first.run_id);
    let drain = controller
        .request_and_wait(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap();
    assert_eq!(drain.record.status, "completed");
    assert_eq!(drain.input_count, 0, "nothing left to drain");
    assert_eq!(llm.request_count(), 2, "the empty cycle dialed no provider");

    workers.stop_and_join().unwrap();
}

/// Scheduled ticks read the actual config state: below the minimum they
/// record honest `not_enough_memories` skip rows, and after enough
/// consecutive skips the backlog escape processes the small leftover
/// (ADR 0005's default shape).
#[test]
fn scheduled_ticks_record_honest_skips_then_escape_the_backlog() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 1, 2);
    let llm = LoopbackLlm::start();
    wire_lane(&config, &llm.url(), true, 5, 10, 5.0);
    // Backlog escape after two consecutive scheduled skips.
    let mut dream = hieronymus::dream_config::load_dream_config(&config).unwrap();
    dream.not_enough_memories_cycle_threshold = 2;
    save_dream_config(&config, &dream).unwrap();

    let (workers, controller) = start_controller(config.clone());

    // The worker's own first gate fires immediately and records the first
    // skip (2 pending below the minimum of 5).
    wait_for(
        || {
            scalar(
                root.path(),
                "select count(*) from dream_runs where status = 'skipped'",
            ) >= 1
        },
        PASS_TIMEOUT,
    );

    // One direct tick below the escape threshold: another honest skip.
    assert!(controller.run_scheduled_tick().is_none());
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'skipped' and error like 'not_enough_memories%'"
        ),
        2
    );
    assert_eq!(llm.request_count(), 0, "skipped ticks dial no provider");

    // The next consecutive skip crosses the threshold: the escape runs and
    // drains the small leftover with the minimum lifted.
    let ticket = controller
        .run_scheduled_tick()
        .expect("the backlog escape runs");
    assert!(!ticket.run_id.is_empty());
    // The escape drains the small leftover: wait for the run's durable
    // completion, not merely for the backlog to reach zero mid-drain.
    wait_for(
        || {
            scalar(
                root.path(),
                "select count(*) from dream_runs where status = 'completed'",
            ) >= 1
        },
        PASS_TIMEOUT,
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from short_term_memories where archived_at is null"
        ),
        0
    );
    assert_eq!(
        llm.request_count(),
        2,
        "one batch, two passes over two inputs"
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'completed' and input_count = 2"
        ),
        1
    );
    // With nothing pending, a further tick is an honest stand-down.
    assert!(controller.run_scheduled_tick().is_none());

    workers.stop_and_join().unwrap();
}

/// A shutdown while a drain is in flight stops at the next batch boundary:
/// the in-flight batch keeps its durable completed outcome, the remaining
/// inputs stay pending, and the waiting caller is woken with the honest
/// interruption instead of being abandoned.
#[test]
fn shutdown_stops_the_drain_at_a_batch_boundary_with_honest_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    seed_backlog(&config, 2, 4);
    let llm = LoopbackLlm::start();
    wire_lane(&config, &llm.url(), false, 1, 4, 120.0);
    // Hold the first batch's second pass so shutdown lands mid-batch.
    llm.block_request(2);

    let (workers, controller) = start_controller(config);

    let waiter = {
        let controller = controller.clone();
        std::thread::spawn(move || {
            controller.request_and_wait(DreamRequest {
                all: true,
                manual: true,
            })
        })
    };
    wait_for(|| llm.request_count() >= 2, PASS_TIMEOUT);

    // Shutdown joins the worker; the held request completes first.
    let joiner = std::thread::spawn(move || workers.stop_and_join());
    llm.release();
    let (stop_result, waiter_result) = (joiner.join().unwrap(), waiter.join().unwrap());
    stop_result.expect("the worker joins");
    let error = waiter_result.expect_err("the interrupted drain is an honest failure");
    assert!(error.contains("interrupted"), "{error}");

    // The in-flight batch committed; the rest stays pending; no row is left
    // running.
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'completed'"
        ),
        1
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from short_term_memories where archived_at is null"
        ),
        4
    );
    assert_eq!(
        scalar(
            root.path(),
            "select count(*) from dream_runs where status = 'running'"
        ),
        0
    );
}

/// Once the worker group is stopping, dream work is refused instead of
/// queued behind a shutdown.
#[test]
fn requests_are_refused_once_shutting_down() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let (workers, controller) = start_controller(config);
    workers.stop_and_join().unwrap();

    let error = controller
        .request(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap_err();
    assert!(error.contains("shutting down"), "{error}");
    let error = controller
        .request_and_wait(DreamRequest {
            all: true,
            manual: true,
        })
        .unwrap_err();
    assert!(error.contains("shutting down"), "{error}");
}

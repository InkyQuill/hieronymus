//! Durable recovery, lease, cancellation, and transaction-boundary proofs for
//! the semantic qualification scenario runner (Task 11).
//!
//! Every helper operates only beneath the supplied temporary directory: the
//! SQLite state database and the LanceDB store live under `<root>/work`. The
//! crash probe is an owned, core-disabled child process group whose allowed
//! work root is relocated into the temporary directory through
//! `SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT`.

use anyhow::{Context, ensure};
use hieronymus_semantic_native_qualification::scenario::{
    ScenarioMode, ScenarioOptions, TransactionProbe, run_scenario,
};
use rusqlite::Connection;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// SIGABRT on Linux: the crash probe must die through `std::process::abort`,
/// which raises exactly this signal after the batch receipt is committed.
const SIGABRT: i32 = 6;
/// SIGKILL on Linux: the supervision timeout terminates a hung probe.
const SIGKILL: i32 = 9;
/// Mandated population of a complete generation.
const EXPECTED_ROW_COUNT: i64 = 10_000;
/// Upper bound for a real crash probe (42 batches) before the owned process
/// group is terminated and reaped instead of being left to hang.
const CRASH_PROBE_TIMEOUT: Duration = Duration::from_secs(300);
/// Supervision timeout for the controlled hung-child test (seconds, not
/// minutes: the sleeper would otherwise outlive the whole suite).
const HANG_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn work_dir(root: &Path) -> PathBuf {
    root.join("work")
}

fn state_db(root: &Path) -> PathBuf {
    work_dir(root).join("state.sqlite3")
}

fn options_for(root: &Path, generation: &str, mode: ScenarioMode) -> ScenarioOptions {
    ScenarioOptions {
        work_dir: work_dir(root),
        state_db: state_db(root),
        generation: generation.to_string(),
        mode,
        probe: TransactionProbe::new(),
    }
}

pub async fn run_complete(root: &Path, generation: &str) -> anyhow::Result<()> {
    run_scenario(options_for(root, generation, ScenarioMode::Complete))
        .await
        .map(|_| ())
}

/// A resume is a complete run that takes over the expired lease of a dead
/// builder and continues from the persisted batch cursor, exactly once.
pub async fn resume(root: &Path, generation: &str) -> anyhow::Result<()> {
    run_complete(root, generation).await
}

pub async fn run_cancel(root: &Path, generation: &str, stop_after: u64) -> anyhow::Result<()> {
    run_scenario(options_for(
        root,
        generation,
        ScenarioMode::Cancel { stop_after },
    ))
    .await
    .map(|_| ())
}

pub async fn run_complete_with_probe(probe: &TransactionProbe) -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let options = ScenarioOptions {
        work_dir: root.path().join("work"),
        state_db: root.path().join("work").join("state.sqlite3"),
        generation: "generation-a".to_string(),
        mode: ScenarioMode::Complete,
        probe: probe.clone(),
    };
    run_scenario(options).await.map(|_| ())
}

/// Outcome of a supervised probe run.
struct SupervisedOutcome {
    status: std::process::ExitStatus,
    killed_on_timeout: bool,
}

/// Spawns `script` as an owned, core-disabled child process group (the script
/// `exec`s, so the spawned pid *is* the probe binary and the group's only
/// member) and supervises it: if it has not exited within `timeout`, the
/// runner terminates it (`start_kill`, i.e. SIGKILL to the group leader) and
/// reaps it. `kill_on_drop` is the final safety net so no code path can
/// orphan the probe.
async fn supervised_probe(
    script: &str,
    work_root: &Path,
    timeout: Duration,
) -> anyhow::Result<SupervisedOutcome> {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg(script);
    // Owned process group: the probe leads its own group, so nothing it could
    // ever spawn escapes the runner's terminate-and-reap responsibility.
    command.process_group(0);
    // The mandated `finally` discipline as a drop guard: a probe dropped on
    // any unforeseen path is killed rather than orphaned.
    command.kill_on_drop(true);
    command.env("SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT", work_root);
    // stdio is inherited: the probe's own evidence (e.g. the receipt commit
    // line before the abort) surfaces directly in the test run's output.
    let mut child = command
        .spawn()
        .context("could not spawn the supervised probe")?;
    let mut wait = Box::pin(async { child.wait().await.context("probe wait failed") });
    match tokio::time::timeout(timeout, wait.as_mut()).await {
        Ok(status) => Ok(SupervisedOutcome {
            status: status?,
            killed_on_timeout: false,
        }),
        Err(_elapsed) => {
            // Terminate and reap: the exec'ed probe is the group leader, so
            // SIGKILL to its pid takes down the whole (single-member) group.
            drop(wait);
            child
                .start_kill()
                .context("could not terminate the hung probe process group")?;
            let status = child
                .wait()
                .await
                .context("could not reap the terminated probe")?;
            Ok(SupervisedOutcome {
                status,
                killed_on_timeout: true,
            })
        }
    }
}

/// Spawns the scenario CLI as an owned, core-disabled child process group and
/// supervises it to completion. The mandated crash outcome is a SIGABRT
/// raised only after the `stop_after` batch receipt has been committed; that
/// outcome is reported as `Err`. Any other outcome — including a probe that
/// hangs past the supervision timeout and is terminated and reaped — panics,
/// so a broken probe can never masquerade as a designed crash.
pub async fn run_crash(root: &Path, generation: &str, stop_after: u64) -> anyhow::Result<()> {
    let binary = std::env::var("CARGO_BIN_EXE_semantic-native")
        .expect("the semantic-native binary target must be built for the crash probe");
    let script = format!(
        "ulimit -c 0; exec '{}' scenario --work-dir '{}' --state-db '{}' --mode crash --generation '{}' --stop-after '{}'",
        binary,
        work_dir(root).display(),
        state_db(root).display(),
        generation,
        stop_after,
    );
    let outcome = supervised_probe(&script, root, CRASH_PROBE_TIMEOUT)
        .await
        .with_context(|| format!("could not supervise the crash probe for {generation}"))?;
    if outcome.killed_on_timeout {
        panic!(
            "crash probe for {generation} hung past the {CRASH_PROBE_TIMEOUT:?} supervision \
             timeout and was terminated and reapped instead of aborting"
        );
    }
    if outcome.status.signal() != Some(SIGABRT) {
        panic!(
            "crash probe did not abort through SIGABRT (status: {}); \
             the probe's own output is on this test's stderr",
            outcome.status
        );
    }
    Err(anyhow::anyhow!(
        "crash probe aborted as designed after committing the {stop_after}-row receipt \
         (status: {})",
        outcome.status
    ))
}

fn state_connection(root: &Path) -> anyhow::Result<Connection> {
    let connection = Connection::open(state_db(root)).with_context(|| {
        format!(
            "could not open the state database {}",
            state_db(root).display()
        )
    })?;
    connection.pragma_update(None, "busy_timeout", 5_000)?;
    Ok(connection)
}

pub fn read_active_generation(root: &Path) -> anyhow::Result<String> {
    let connection = state_connection(root)?;
    let mut statement =
        connection.prepare("select generation_id from semantic_generations where active = 1")?;
    let mut rows = statement.query([])?;
    let mut found: Option<String> = None;
    while let Some(row) = rows.next()? {
        let generation: String = row.get(0)?;
        ensure!(
            found.is_none(),
            "more than one active semantic generation in the state database"
        );
        found = Some(generation);
    }
    found.context("no active semantic generation in the state database")
}

pub fn written_count(root: &Path, generation: &str) -> anyhow::Result<i64> {
    let connection = state_connection(root)?;
    connection
        .query_row(
            "select written_count from semantic_generations where generation_id = ?1",
            [generation],
            |row| row.get(0),
        )
        .with_context(|| format!("no generation row for {generation}"))
}

pub fn generation_status(root: &Path, generation: &str) -> anyhow::Result<String> {
    let connection = state_connection(root)?;
    connection
        .query_row(
            "select status from semantic_generations where generation_id = ?1",
            [generation],
            |row| row.get(0),
        )
        .with_context(|| format!("no generation row for {generation}"))
}

#[tokio::test]
async fn crash_leaves_previous_generation_active_and_resume_is_exact() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    let crash = run_crash(root.path(), "generation-b", 4_200).await;
    assert!(crash.is_err());
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    resume(root.path(), "generation-b").await?;
    assert_eq!(written_count(root.path(), "generation-b")?, 10_000);
    Ok(())
}

#[tokio::test]
async fn cancellation_never_activates_cancelled_generation() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    run_cancel(root.path(), "generation-b", 3_200).await?;
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    assert_eq!(generation_status(root.path(), "generation-b")?, "cancelled");
    Ok(())
}

#[tokio::test]
async fn native_io_never_runs_inside_sqlite_write_transaction() -> anyhow::Result<()> {
    let probe = TransactionProbe::new();
    run_complete_with_probe(&probe).await?;
    assert_eq!(probe.native_io_while_write_transaction(), 0);
    Ok(())
}

#[test]
fn transaction_probe_rejects_native_io_inside_write_transactions() -> anyhow::Result<()> {
    let probe = TransactionProbe::new();
    let mut connection = Connection::open_in_memory()?;
    let error = probe
        .write_transaction(&mut connection, "tripwire", |_| {
            probe.enter_native("tripwire-native").map(|_guard| ())
        })
        .expect_err("native I/O inside a write transaction must fail hard");
    ensure!(
        error.to_string().contains("write transaction"),
        "unexpected tripwire error: {error}"
    );
    assert_eq!(probe.native_io_while_write_transaction(), 1);
    Ok(())
}

#[tokio::test]
async fn crash_receipt_is_durable_at_batch_boundary() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    let _ = run_crash(root.path(), "generation-b", 4_200).await;
    // The abort happened after the receipt commit: the persisted counters must
    // sit exactly on the mandated 4,200-row boundary with generation-b still
    // building and generation-a still the only active generation.
    assert_eq!(written_count(root.path(), "generation-b")?, 4_200);
    assert_eq!(generation_status(root.path(), "generation-b")?, "building");
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    Ok(())
}

#[tokio::test]
async fn complete_run_holds_expected_row_count_and_single_active_generation() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    run_complete(root.path(), "generation-a").await?;
    assert_eq!(
        written_count(root.path(), "generation-a")?,
        EXPECTED_ROW_COUNT
    );
    assert_eq!(generation_status(root.path(), "generation-a")?, "active");
    assert_eq!(read_active_generation(root.path())?, "generation-a");
    Ok(())
}

#[tokio::test]
async fn crash_probe_terminates_and_reaps_a_hung_child_group() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    // A probe that hangs (here: a controlled sleeper standing in for a stuck
    // ONNX/LanceDB call) must be terminated and reaped by the supervision
    // timeout, never orphaned. The sleeper would sleep 300 seconds; the
    // supervision timeout is 2 seconds.
    let started = Instant::now();
    let outcome = supervised_probe(
        "ulimit -c 0; exec sleep 300",
        root.path(),
        HANG_PROBE_TIMEOUT,
    )
    .await
    .expect("the supervised sleeper must be handled without error");
    let elapsed = started.elapsed();
    assert!(
        outcome.killed_on_timeout,
        "a probe past its supervision timeout must be terminated, not waited on"
    );
    assert_eq!(
        outcome.status.signal(),
        Some(SIGKILL),
        "the terminated probe must have been killed and reaped, got {}",
        outcome.status
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "supervision must not wait out the sleeper, took {elapsed:?}"
    );
    Ok(())
}

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
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// SIGABRT on Linux: the crash probe must die through `std::process::abort`,
/// which raises exactly this signal after the batch receipt is committed.
const SIGABRT: i32 = 6;
/// Mandated population of a complete generation.
const EXPECTED_ROW_COUNT: i64 = 10_000;

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

/// Spawns the scenario CLI as an owned, core-disabled child process group and
/// waits for it. The mandated crash outcome is a SIGABRT raised only after the
/// `stop_after` batch receipt has been committed; that outcome is reported as
/// `Err`. Any other failure panics with the captured output so a broken probe
/// can never masquerade as a designed crash.
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
    let mut command = std::process::Command::new("/bin/sh");
    command.arg("-c").arg(script);
    // Owned process group: the probe leads its own group, so nothing it could
    // ever spawn escapes the runner's terminate-and-reap responsibility.
    command.process_group(0);
    command.env("SEMANTIC_NATIVE_QUALIFICATION_WORK_ROOT", root);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command
        .spawn()
        .with_context(|| format!("could not spawn the crash probe for {generation}"))?;
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .context("crash probe task join failed")?
        .context("could not wait for the crash probe")?;
    let status = output.status;
    if status.signal() != Some(SIGABRT) {
        panic!(
            "crash probe did not abort through SIGABRT (status: {status}); stdout: {}; stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Err(anyhow::anyhow!(
        "crash probe aborted as designed after committing the {stop_after}-row receipt; stderr: {}",
        String::from_utf8_lossy(&output.stderr).trim(),
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

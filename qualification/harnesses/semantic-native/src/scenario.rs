//! Durable SQLite job/generation recovery for the semantic qualification
//! scenario runner.
//!
//! SQLite is the only recovery state: leases, batch cursors, cancellation
//! flags, written counters, and generation activation all live in the state
//! database. No filesystem progress file participates in recovery or
//! activation. Every SQLite write transaction is short and committed before
//! any ONNX or LanceDB I/O runs; [`TransactionProbe`] turns a violation of
//! that boundary into a hard failure with a recorded counter.
//!
//! Batch flow per generation batch:
//! 1. a short write transaction claims/renews the lease and reads the cursor;
//! 2. the transaction commits and the probe guard releases;
//! 3. ONNX embedding and the LanceDB append run with no transaction open;
//! 4. a fresh short write transaction checks the lease owner, persists the
//!    written counters, and advances the cursor.
//!
//! Evidence printed anywhere in this module carries counts, ids, checksums,
//! and digests only — never raw vectors.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use tokio::time::sleep;

use crate::corpus::{self, Chunk};
use crate::index::{
    EMBEDDING_DIMENSIONS, GenerationIndex, IndexRow, ModelIdentity, RowFingerprint,
    validate_series_slug,
};
use crate::model::OnnxEmbeddingProvider;

/// Identity of the pinned qualification embedding model.
pub const MODEL_NAME: &str = "all-MiniLM-L6-v2";
/// SHA-256 of the acquired `model.onnx`; matches
/// `qualification/prerequisites.json` and is verified before every load.
pub const MODEL_SHA256: &str = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";
/// Chunks embedded and appended per lease-protected batch. 100 divides the
/// mandated stop-after marks (4,200 / 3,200) and the full 10,000-row
/// population, so crash and cancellation always land on committed receipts.
pub const BATCH_SIZE: usize = 100;
/// Rows a complete generation must hold before activation.
pub const EXPECTED_ROW_COUNT: usize = 10_000;

/// A foreign lease must expire before it can be taken over; the dead builder
/// of the crash scenario never renews, so a resume waits at most this long.
const LEASE_TTL_MS: i64 = 5_000;
const LEASE_POLL: Duration = Duration::from_millis(250);
const LEASE_DEADLINE: Duration = Duration::from_secs(120);

/// Query length used by the sample searches that gate activation.
const SAMPLE_TOP_K: usize = 10;

/// Observed-fact witness over the transaction boundary.
///
/// Every SQLite write transaction in the runner goes through
/// [`TransactionProbe::write_transaction`], and every ONNX/LanceDB section is
/// entered through [`TransactionProbe::enter_native`]. Entering native code
/// while a write transaction is open is a hard failure and increments the
/// recorded violation counter exposed by
/// [`TransactionProbe::native_io_while_write_transaction`].
#[derive(Clone, Debug, Default)]
pub struct TransactionProbe {
    state: Arc<ProbeState>,
}

#[derive(Debug, Default)]
struct ProbeState {
    write_depth: AtomicUsize,
    write_transactions: AtomicUsize,
    native_calls: AtomicUsize,
    violations: AtomicUsize,
}

impl TransactionProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one short SQLite write transaction. The transaction opens in
    /// immediate mode, the body runs to completion, and the commit happens
    /// before the depth guard is released, so native code can observe the
    /// exact window in which SQLite holds a write transaction.
    pub fn write_transaction<T>(
        &self,
        connection: &mut Connection,
        label: &str,
        body: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        if self.state.write_depth.fetch_add(1, Ordering::SeqCst) != 0 {
            self.state.write_depth.fetch_sub(1, Ordering::SeqCst);
            self.state.violations.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("nested SQLite write transaction ({label})");
        }
        let outcome = (|| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .with_context(|| format!("could not open the {label} write transaction"))?;
            let value = body(&transaction)?;
            transaction
                .commit()
                .with_context(|| format!("could not commit the {label} write transaction"))?;
            Ok(value)
        })();
        self.state.write_depth.fetch_sub(1, Ordering::SeqCst);
        self.state.write_transactions.fetch_add(1, Ordering::SeqCst);
        outcome
    }

    /// Enters a native (ONNX/LanceDB) section. Fails hard — after recording
    /// the violation — if any SQLite write transaction is currently open.
    pub fn enter_native(&self, label: &str) -> anyhow::Result<NativeGuard<'_>> {
        self.state.native_calls.fetch_add(1, Ordering::SeqCst);
        if self.state.write_depth.load(Ordering::SeqCst) != 0 {
            self.state.violations.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("native I/O ({label}) attempted inside a SQLite write transaction");
        }
        Ok(NativeGuard { _probe: self })
    }

    /// How many native-I/O violations were recorded. A clean run reports 0.
    pub fn native_io_while_write_transaction(&self) -> usize {
        self.state.violations.load(Ordering::SeqCst)
    }

    /// Number of write transactions the probe supervised (diagnostic).
    pub fn write_transaction_count(&self) -> usize {
        self.state.write_transactions.load(Ordering::SeqCst)
    }

    /// Number of native sections the probe supervised (diagnostic).
    pub fn native_call_count(&self) -> usize {
        self.state.native_calls.load(Ordering::SeqCst)
    }
}

/// Scoped witness that a native section is open.
#[derive(Debug)]
pub struct NativeGuard<'a> {
    _probe: &'a TransactionProbe,
}

/// What the runner should do after the receipt at `stop_after` commits.
#[derive(Clone, Debug)]
pub enum ScenarioMode {
    /// Build, verify, sample-query, and activate the generation.
    Complete,
    /// Abort the process with SIGABRT once the receipt committing
    /// `stop_after` rows has been committed. The abort therefore always
    /// happens exactly on a durable batch boundary.
    Crash { stop_after: u64 },
    /// Commit a cancellation request once the receipt committing
    /// `stop_after` rows has been committed; the loop observes the durable
    /// flag between batches and finishes the generation as `cancelled`.
    Cancel { stop_after: u64 },
}

impl ScenarioMode {
    pub fn label(&self) -> &'static str {
        match self {
            ScenarioMode::Complete => "complete",
            ScenarioMode::Crash { .. } => "crash",
            ScenarioMode::Cancel { .. } => "cancel",
        }
    }
}

/// Inputs for one scenario run. Tests supply temporary paths directly; the
/// CLI validates its own work-directory policy before constructing these.
#[derive(Clone, Debug)]
pub struct ScenarioOptions {
    pub work_dir: PathBuf,
    pub state_db: PathBuf,
    pub generation: String,
    pub mode: ScenarioMode,
    pub probe: TransactionProbe,
}

/// Normalized outcome of one scenario run. No vectors, ever.
#[derive(Clone, Debug, Serialize)]
pub struct ScenarioSummary {
    pub generation_id: String,
    pub mode: String,
    pub final_status: String,
    pub active_generation: String,
    pub written_count: i64,
    pub expected_count: i64,
    pub completed_batches: i64,
    pub resumed_from_batch: Option<i64>,
    pub ann_index_type: Option<String>,
    pub ann_indexed_rows: Option<usize>,
    pub native_io_violations: usize,
    pub write_transactions: usize,
    pub native_calls: usize,
}

/// Result of the `query` subcommand: which generation served the query and
/// which corpus ids came back. Counts and ids only — never vectors.
#[derive(Clone, Debug, Serialize)]
pub struct QueryReceipt {
    pub query_id: String,
    pub generation_id: String,
    pub series_slug: String,
    pub returned_ids: Vec<String>,
    pub eligible_count: usize,
    pub cross_series_count: usize,
}

const SCHEMA_SQL: &str = "
create table if not exists semantic_generations (
    generation_id text primary key,
    status text not null check (status in ('building','ready','active','superseded','cancelled','failed')),
    model_digest text not null,
    expected_count integer not null,
    written_count integer not null default 0,
    active integer not null default 0 check (active in (0,1))
);
create unique index if not exists one_active_semantic_generation
    on semantic_generations(active) where active = 1;
create table if not exists semantic_jobs (
    job_id text primary key,
    generation_id text not null references semantic_generations(generation_id),
    status text not null check (status in ('queued','running','cancel_requested','cancelled','complete','failed')),
    next_batch integer not null,
    completed_batches integer not null,
    lease_owner text,
    lease_expires_unix_ms integer,
    cancel_requested integer not null default 0 check (cancel_requested in (0,1))
);
";

fn qualification_root() -> anyhow::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .context("the harness lives two levels below the qualification root")
}

/// Paths of the acquired, verified runtime and model artifacts, resolved
/// repository-relative from the crate manifest (never from `$HOME`).
pub fn artifact_paths() -> (PathBuf, PathBuf) {
    let root = qualification_root()
        .expect("qualification root")
        .join(".artifacts/models");
    (
        root.join("onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"),
        root.join("all-MiniLM-L6-v2/model.onnx"),
    )
}

fn open_state_db(state_db: &Path) -> anyhow::Result<Connection> {
    let connection = Connection::open(state_db)
        .with_context(|| format!("could not open the state database {}", state_db.display()))?;
    connection.execute_batch(
        "pragma journal_mode = wal;
         pragma synchronous = full;
         pragma busy_timeout = 5000;
         pragma foreign_keys = on;",
    )?;
    // Idempotent migration of the qualification-only state schema.
    connection.execute_batch(SCHEMA_SQL).with_context(|| {
        format!(
            "could not migrate the state database {}",
            state_db.display()
        )
    })?;
    Ok(connection)
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_millis() as i64
}

/// Lease owner identity for this process.
fn process_owner() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    format!("{}:{nanos}", std::process::id())
}

fn job_id_for(generation: &str) -> String {
    format!("build:{generation}")
}

/// Durable per-job state read inside a write transaction.
#[derive(Clone, Copy, Debug)]
struct JobState {
    next_batch: i64,
    written_count: i64,
    completed_batches: i64,
    cancel_requested: bool,
}

enum LeaseClaim {
    Claimed(JobState),
    /// A foreign, unexpired lease belongs to a live builder.
    Held,
}

fn claim_lease(
    transaction: &Transaction<'_>,
    job_id: &str,
    generation: &str,
    owner: &str,
) -> anyhow::Result<LeaseClaim> {
    let row = transaction
        .query_row(
            "select lease_owner, lease_expires_unix_ms, next_batch,
                    cancel_requested, completed_batches
             from semantic_jobs where job_id = ?1",
            [job_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .context("could not read the job row")?;
    let Some((lease_owner, lease_expires, next_batch, cancel_requested, completed_batches)) = row
    else {
        anyhow::bail!("job row {job_id} disappeared before the lease claim");
    };
    if let (Some(holder), Some(expires)) = (&lease_owner, lease_expires)
        && holder != owner
        && expires > now_unix_ms()
    {
        return Ok(LeaseClaim::Held);
    }
    let expires_at = now_unix_ms() + LEASE_TTL_MS;
    transaction.execute(
        "update semantic_jobs set status = 'running', lease_owner = ?1,
                lease_expires_unix_ms = ?2 where job_id = ?3",
        rusqlite::params![owner, expires_at, job_id],
    )?;
    let written_count = transaction.query_row(
        "select written_count from semantic_generations where generation_id = ?1",
        [generation],
        |row| row.get(0),
    )?;
    Ok(LeaseClaim::Claimed(JobState {
        next_batch,
        written_count,
        completed_batches,
        cancel_requested,
    }))
}

/// Commits one batch receipt: verifies the caller still owns the lease,
/// advances the cursor and batch counters, renews the lease, and adds to the
/// generation's written counter. Returns the new durable written count.
fn commit_batch_receipt(
    transaction: &Transaction<'_>,
    job_id: &str,
    generation: &str,
    owner: &str,
    appended: usize,
) -> anyhow::Result<i64> {
    let updated = transaction.execute(
        "update semantic_jobs set next_batch = next_batch + 1,
                completed_batches = completed_batches + 1,
                lease_owner = ?2, lease_expires_unix_ms = ?3
          where job_id = ?1 and lease_owner = ?2",
        rusqlite::params![job_id, owner, now_unix_ms() + LEASE_TTL_MS],
    )?;
    ensure!(
        updated == 1,
        "lease for job {job_id} was lost before the receipt could be committed"
    );
    transaction.execute(
        "update semantic_generations set written_count = written_count + ?1
          where generation_id = ?2",
        rusqlite::params![appended as i64, generation],
    )?;
    let written = transaction.query_row(
        "select written_count from semantic_generations where generation_id = ?1",
        [generation],
        |row| row.get(0),
    )?;
    Ok(written)
}

/// Registers (or re-admits) the generation and its build job, rejecting
/// terminal states and mismatched model identities before any native work.
fn register_generation_and_job(
    probe: &TransactionProbe,
    connection: &mut Connection,
    generation: &str,
    model_digest: &str,
    expected_count: i64,
) -> anyhow::Result<()> {
    probe.write_transaction(connection, "register generation", |transaction| {
        transaction.execute(
            "insert or ignore into semantic_generations
                    (generation_id, status, model_digest, expected_count, written_count, active)
             values (?1, 'building', ?2, ?3, 0, 0)",
            rusqlite::params![generation, model_digest, expected_count],
        )?;
        let (status, digest, expected, active): (String, String, i64, i64) = transaction
            .query_row(
                "select status, model_digest, expected_count, active
                 from semantic_generations where generation_id = ?1",
                [generation],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .with_context(|| format!("no generation row for {generation}"))?;
        ensure!(
            digest == model_digest,
            "generation {generation} was built against model digest {digest}, expected {model_digest}"
        );
        ensure!(
            expected == expected_count,
            "generation {generation} expects {expected} rows, this scenario builds {expected_count}"
        );
        ensure!(
            active == 0 && matches!(status.as_str(), "building" | "ready"),
            "generation {generation} is in terminal state {status:?} and cannot be rebuilt"
        );
        transaction.execute(
            "insert or ignore into semantic_jobs
                    (job_id, generation_id, status, next_batch, completed_batches)
             values (?1, ?2, 'queued', 0, 0)",
            rusqlite::params![job_id_for(generation), generation],
        )?;
        let job_status: String = transaction
            .query_row(
                "select status from semantic_jobs where job_id = ?1",
                [job_id_for(generation)],
                |row| row.get(0),
            )
            .with_context(|| format!("no job row for {generation}"))?;
        ensure!(
            matches!(job_status.as_str(), "queued" | "running"),
            "build job for {generation} is {job_status:?}; a finished build resumes exactly never"
        );
        Ok(())
    })
}

/// Polls until this process owns the lease, waiting out any foreign unexpired
/// lease instead of stealing it from a possibly-live builder.
async fn take_lease(
    probe: &TransactionProbe,
    connection: &mut Connection,
    job_id: &str,
    generation: &str,
    owner: &str,
) -> anyhow::Result<JobState> {
    let deadline = Instant::now() + LEASE_DEADLINE;
    loop {
        let attempt = probe.write_transaction(connection, "claim lease", |transaction| {
            claim_lease(transaction, job_id, generation, owner)
        })?;
        match attempt {
            LeaseClaim::Claimed(state) => return Ok(state),
            LeaseClaim::Held => {
                ensure!(
                    Instant::now() < deadline,
                    "timed out waiting for the expired lease of job {job_id}"
                );
                sleep(LEASE_POLL).await;
            }
        }
    }
}

/// Reconciles the persisted SQLite counters against the rows physically
/// stored in LanceDB: same count, same (chunk_id, checksum) multiset.
async fn verify_persisted_counters(
    probe: &TransactionProbe,
    index: &GenerationIndex,
    generation: &str,
    chunks: &[Chunk],
    written_count: i64,
) -> anyhow::Result<()> {
    ensure!(written_count >= 0, "negative persisted written count");
    ensure!(
        written_count as usize <= chunks.len(),
        "persisted written count {} exceeds the corpus population",
        written_count
    );
    let expected: HashSet<(i64, String)> = chunks[..written_count as usize]
        .iter()
        .map(|chunk| {
            (
                IndexRow::numeric_chunk_id(&chunk.id),
                chunk.checksum.clone(),
            )
        })
        .collect();
    let stored: Vec<RowFingerprint> = {
        let _native = probe.enter_native("lancedb snapshot rows")?;
        index.snapshot_rows(EXPECTED_ROW_COUNT * 2).await?
    };
    ensure!(
        stored.len() == written_count as usize,
        "LanceDB holds {} rows for {generation} but SQLite counted {written_count}",
        stored.len()
    );
    let stored_set: HashSet<(i64, String)> = stored
        .iter()
        .map(|row| (row.chunk_id, row.checksum.clone()))
        .collect();
    ensure!(
        stored_set == expected,
        "persisted counters disagree with LanceDB ids/checksums for {generation}"
    );
    let foreign: Vec<&RowFingerprint> = stored
        .iter()
        .filter(|row| row.generation_id != generation)
        .collect();
    ensure!(
        foreign.is_empty(),
        "generation {generation} stores rows from another generation"
    );
    Ok(())
}

/// Runs one scenario to completion (or to its crash/cancel boundary).
pub async fn run_scenario(options: ScenarioOptions) -> anyhow::Result<ScenarioSummary> {
    validate_series_slug(&options.generation).with_context(|| {
        format!(
            "generation id {:?} is not a normalized slug",
            options.generation
        )
    })?;
    let generation = options.generation.clone();

    let (chunks, queries) = {
        let spec = corpus::load_default_spec().context("could not load the corpus recipe")?;
        corpus::generate_corpus(&spec)?
    };
    ensure!(
        chunks.len() == EXPECTED_ROW_COUNT,
        "corpus recipe produced {} chunks, expected {EXPECTED_ROW_COUNT}",
        chunks.len()
    );
    ensure!(!queries.is_empty(), "corpus recipe produced no queries");

    std::fs::create_dir_all(&options.work_dir).with_context(|| {
        format!(
            "could not create the work directory {}",
            options.work_dir.display()
        )
    })?;
    if let Some(parent) = options.state_db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = open_state_db(&options.state_db)?;
    let job_id = job_id_for(&generation);

    register_generation_and_job(
        &options.probe,
        &mut connection,
        &generation,
        MODEL_SHA256,
        EXPECTED_ROW_COUNT as i64,
    )?;

    // Native: verified model + runtime load. Nothing above this line touched
    // a model, an index, or the network.
    let (runtime_path, model_path) = artifact_paths();
    let mut provider = {
        let _native = options.probe.enter_native("onnx provider load")?;
        OnnxEmbeddingProvider::load(&runtime_path, &model_path, MODEL_SHA256)
            .context("could not load the verified embedding provider")?
    };
    let identity = ModelIdentity::new(MODEL_NAME, EMBEDDING_DIMENSIONS, MODEL_SHA256)?;

    // Native: open (or reopen) this generation's LanceDB table.
    let index_root = options.work_dir.join("index");
    let mut index = {
        let _native = options.probe.enter_native("lancedb open generation")?;
        GenerationIndex::create(&index_root, identity.clone(), &generation).await?
    };

    let owner = process_owner();
    let mut state = take_lease(
        &options.probe,
        &mut connection,
        &job_id,
        &generation,
        &owner,
    )
    .await?;
    let resumed_from_batch = if state.next_batch > 0 {
        // A crashed builder left durable state: reconcile before continuing.
        verify_persisted_counters(
            &options.probe,
            &index,
            &generation,
            &chunks,
            state.written_count,
        )
        .await?;
        Some(state.next_batch)
    } else {
        None
    };

    let mut cancelled = false;
    while (state.next_batch as usize) * BATCH_SIZE < EXPECTED_ROW_COUNT {
        state = take_lease(
            &options.probe,
            &mut connection,
            &job_id,
            &generation,
            &owner,
        )
        .await?;
        if state.cancel_requested {
            cancelled = true;
            break;
        }
        let start = (state.next_batch as usize) * BATCH_SIZE;
        let end = (start + BATCH_SIZE).min(EXPECTED_ROW_COUNT);
        let batch = &chunks[start..end];

        // Native section: no write transaction is open here.
        {
            let _native = options
                .probe
                .enter_native("onnx embed + lancedb append batch")?;
            let mut rows = Vec::with_capacity(batch.len());
            for chunk in batch {
                let vector = provider.embed(&chunk.token_ids)?;
                let mut row = IndexRow::from_corpus_chunk(chunk);
                row.vector = vector;
                // The generation being built owns every row it stores; mixed
                // generation ids are rejected by the index validator.
                row.generation_id = generation.clone();
                rows.push(row);
            }
            index.append(rows).await?;
        }

        let written = options
            .probe
            .write_transaction(&mut connection, "batch receipt", {
                let job_id = job_id.clone();
                let generation = generation.clone();
                let owner = owner.clone();
                move |transaction| {
                    commit_batch_receipt(transaction, &job_id, &generation, &owner, batch.len())
                }
            })?;
        state.written_count = written;
        state.next_batch += 1;
        state.completed_batches += 1;

        match options.mode {
            ScenarioMode::Crash { stop_after } if state.written_count as u64 >= stop_after => {
                eprintln!(
                    "crash probe: receipt committing {stop_after} rows is durable; aborting process"
                );
                std::process::abort();
            }
            ScenarioMode::Cancel { stop_after } if state.written_count as u64 >= stop_after => {
                options.probe.write_transaction(
                    &mut connection,
                    "request cancellation",
                    |transaction| {
                        transaction.execute(
                            "update semantic_jobs set cancel_requested = 1 where job_id = ?1",
                            rusqlite::params![job_id],
                        )?;
                        Ok(())
                    },
                )?;
            }
            _ => {}
        }
    }

    if cancelled {
        // Close the native handles before recording the terminal state.
        drop(index);
        drop(provider);
        options
            .probe
            .write_transaction(&mut connection, "commit cancellation", {
                let job_id = job_id.clone();
                let generation = generation.clone();
                move |transaction| {
                    let updated = transaction.execute(
                        "update semantic_jobs set status = 'cancelled', lease_owner = null,
                            lease_expires_unix_ms = null where job_id = ?1",
                        rusqlite::params![job_id],
                    )?;
                    ensure!(updated == 1, "job row vanished before cancellation");
                    transaction.execute(
                    "update semantic_generations set status = 'cancelled' where generation_id = ?1",
                    rusqlite::params![generation],
                )?;
                    Ok(())
                }
            })?;
        return finish_summary(
            &options,
            &mut connection,
            &generation,
            "cancelled",
            resumed_from_batch,
            None,
            None,
        )
        .await;
    }

    // Native: ANN index over the complete population, then verification and
    // sample queries. All must succeed before activation.
    let (ann_index_type, ann_indexed_rows) = {
        let _native = options.probe.enter_native("lancedb ann index + verify")?;
        let evidence = index.create_ann_index().await?;
        let verified = index.verify().await?;
        ensure!(
            verified.num_unindexed_rows == 0 && verified.num_indexed_rows == EXPECTED_ROW_COUNT,
            "ANN verification reported {} indexed / {} unindexed rows",
            verified.num_indexed_rows,
            verified.num_unindexed_rows
        );
        (Some(evidence.index_type), Some(verified.num_indexed_rows))
    };
    {
        let _native = options.probe.enter_native("sample queries")?;
        for query_index in [0usize, queries.len() - 1] {
            let query = &queries[query_index];
            let vector = provider.embed(&query.token_ids)?;
            let hits = index
                .search(&query.eligible_series_slug, &vector, SAMPLE_TOP_K)
                .await?;
            ensure!(
                !hits.is_empty(),
                "sample query {} returned no rows",
                query.id
            );
            ensure!(
                hits.iter()
                    .all(|hit| hit.series_slug == query.eligible_series_slug),
                "sample query {} leaked rows across series",
                query.id
            );
        }
    }
    // Native handles closed before activation.
    drop(index);
    drop(provider);

    // Activation is exactly one SQLite transaction.
    options
        .probe
        .write_transaction(&mut connection, "activate generation", {
            let job_id = job_id.clone();
            let generation = generation.clone();
            move |transaction| {
                transaction.execute(
                    "update semantic_generations set active = 0, status = 'superseded'
                 where active = 1",
                    [],
                )?;
                transaction.execute(
                    "update semantic_generations set status = 'active', active = 1
                 where generation_id = ?1",
                    rusqlite::params![generation],
                )?;
                transaction.execute(
                    "update semantic_jobs set status = 'complete', lease_owner = null,
                        lease_expires_unix_ms = null where job_id = ?1",
                    rusqlite::params![job_id],
                )?;
                Ok(())
            }
        })?;

    finish_summary(
        &options,
        &mut connection,
        &generation,
        "active",
        resumed_from_batch,
        ann_index_type,
        ann_indexed_rows,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish_summary(
    options: &ScenarioOptions,
    connection: &mut Connection,
    generation: &str,
    final_status: &str,
    resumed_from_batch: Option<i64>,
    ann_index_type: Option<String>,
    ann_indexed_rows: Option<usize>,
) -> anyhow::Result<ScenarioSummary> {
    ensure!(
        options.probe.native_io_while_write_transaction() == 0,
        "native I/O violated the transaction boundary {} time(s)",
        options.probe.native_io_while_write_transaction()
    );
    let (written_count, active_generation): (i64, Option<String>) = connection.query_row(
        "select written_count,
                (select generation_id from semantic_generations where active = 1)
         from semantic_generations where generation_id = ?1",
        [generation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let completed_batches: i64 = connection.query_row(
        "select completed_batches from semantic_jobs where job_id = ?1",
        [job_id_for(generation)],
        |row| row.get(0),
    )?;
    let active_generation = active_generation
        .with_context(|| format!("no active semantic generation after the {generation} run"))?;
    Ok(ScenarioSummary {
        generation_id: generation.to_string(),
        mode: options.mode.label().to_string(),
        final_status: final_status.to_string(),
        active_generation,
        written_count,
        expected_count: EXPECTED_ROW_COUNT as i64,
        completed_batches,
        resumed_from_batch,
        ann_index_type,
        ann_indexed_rows,
        native_io_violations: options.probe.native_io_while_write_transaction(),
        write_transactions: options.probe.write_transaction_count(),
        native_calls: options.probe.native_call_count(),
    })
}

/// Runs one corpus query against the currently active generation and returns
/// a vector-free receipt. Requires nonempty, series-isolated results.
pub async fn run_active_query(
    index_root: &Path,
    state_db: &Path,
    series_slug: &str,
    query_index: usize,
) -> anyhow::Result<QueryReceipt> {
    validate_series_slug(series_slug)
        .with_context(|| format!("series slug {series_slug:?} is not a normalized slug"))?;
    let (chunks, queries) = {
        let spec = corpus::load_default_spec().context("could not load the corpus recipe")?;
        corpus::generate_corpus(&spec)?
    };
    ensure!(
        query_index < queries.len(),
        "query index {query_index} is outside the 0..{} corpus query range",
        queries.len()
    );
    let query = &queries[query_index];

    let connection = open_state_db(state_db)?;
    let active: String = connection
        .query_row(
            "select generation_id from semantic_generations where active = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .context("no active semantic generation; run a complete scenario first")?;

    let (runtime_path, model_path) = artifact_paths();
    let mut provider = OnnxEmbeddingProvider::load(&runtime_path, &model_path, MODEL_SHA256)
        .context("could not load the verified embedding provider")?;
    let identity = ModelIdentity::new(MODEL_NAME, EMBEDDING_DIMENSIONS, MODEL_SHA256)?;
    let index = GenerationIndex::create(index_root, identity, &active).await?;

    let vector = provider.embed(&query.token_ids)?;
    let hits = index.search(series_slug, &vector, SAMPLE_TOP_K).await?;

    // Map numeric ids back onto corpus string ids for the receipt.
    let mut id_by_numeric = std::collections::HashMap::with_capacity(chunks.len());
    for chunk in &chunks {
        id_by_numeric.insert(IndexRow::numeric_chunk_id(&chunk.id), chunk.id.clone());
    }
    let mut returned_ids = Vec::with_capacity(hits.len());
    let mut cross_series_count = 0usize;
    for hit in &hits {
        if hit.series_slug != series_slug {
            cross_series_count += 1;
            continue;
        }
        if let Some(corpus_id) = id_by_numeric.get(&hit.chunk_id) {
            returned_ids.push(corpus_id.clone());
        }
    }
    let eligible_count = returned_ids.len();
    ensure!(
        !returned_ids.is_empty(),
        "query {} returned no eligible rows for series {series_slug}",
        query.id
    );
    ensure!(
        cross_series_count == 0,
        "query {} leaked {cross_series_count} cross-series rows",
        query.id
    );
    Ok(QueryReceipt {
        query_id: query.id.clone(),
        generation_id: active,
        series_slug: series_slug.to_string(),
        returned_ids,
        eligible_count,
        cross_series_count,
    })
}

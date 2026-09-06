//! Durable semantic rebuild jobs: SQLite job records, transactional batch
//! leases, cancellation, and crash recovery driving Task 7's generation
//! lifecycle. Shapes are lifted from the qualified harness's scenario runner
//! (`qualification/harnesses/semantic-native`): SQLite is the only recovery
//! state, every write transaction is short, and native I/O (inference and
//! LanceDB) never runs inside one.
//!
//! Scope decisions (documented per the design's §Durable Jobs):
//! - One job drives one whole-corpus generation rebuild. Task 7's generations
//!   are corpus-wide (`begin_generation` freezes the full `rag_chunks` count),
//!   so per-series targeting has no data-plane surface yet; the job id is
//!   `rebuild:<generation_id>` and the unique index admits exactly one job per
//!   generation.
//! - Batch selection always consumes [`SemanticStore::pending_chunk_ids`]
//!   verbatim, which is Task 7's strict cursor contract: pending ids are the
//!   authoritative ids past the manifest cursor and must never be skipped, or
//!   the generation would wedge below its expected count.
//! - The runner never holds a SQLite write transaction across inference or
//!   LanceDB I/O: the claim transaction commits first, [`SemanticStore::
//!   write_batch`] embeds and appends with no transaction open and then
//!   commits its own short manifest receipt, and the job receipt is a third
//!   short transaction. The manifest is the authoritative cursor; every job
//!   receipt resyncs the job's cursor and counters from it, so a takeover
//!   after a crash cannot double-count progress.
//! - A resumed build verifies the persisted candidate rows against the
//!   manifest (row count plus the chunk-id/checksum multiset behind the
//!   cursor) before writing anything. A batch whose append landed but whose
//!   receipt was lost leaves the counts disagreeing; the takeover fails the
//!   job and the candidate generation (GC-able) instead of duplicating rows.
//!   The prior active generation is never touched by any of this.
//! - Cancellation is a durable flag observed at every batch boundary: the
//!   batch in flight finishes, the job and its generation are marked
//!   cancelled, and the candidate can never activate. Task 7's GC semantics
//!   then collect it.
//! - Reconciliation policy: a building generation with an expired job lease is
//!   resumable work — reconcile only resyncs its progress from the manifest
//!   and reports it reclaimable; the next claim resumes it. Only disagreeing
//!   or terminal-adjacent state is marked failed (and GC-able). In-process
//!   Task 7 builds with no job row at all are outside the jobs contract and
//!   are never touched.
//! - `attempts` counts consecutive failed batch attempts (the bounded-retry
//!   counter, reset by every successful receipt); `failed_batches` is the
//!   cumulative count. `last_error` keeps the most recent error even after a
//!   later success, as history.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider};
use crate::semantic_error::SemanticError;
use crate::semantic_index::{VectorIndex, generation_table_exists};
use crate::semantic_store::{GenerationManifest, SemanticChunk, SemanticSample, SemanticStore};

/// The only job type in this slice: driving one generation rebuild.
pub const JOB_TYPE_REBUILD: &str = "rebuild";

/// Deterministic job id for a generation rebuild.
pub fn rebuild_job_id(generation_id: &str) -> String {
    format!("{JOB_TYPE_REBUILD}:{generation_id}")
}

/// Ensure the durable-jobs schema exists through the caller's connection
/// (idempotent, transaction-safe; the upgrade protocol runs this inside its
/// single transaction).
pub(crate) fn ensure_jobs_schema(connection: &Connection) -> Result<(), SemanticError> {
    connection.execute_batch(SEMANTIC_JOBS_SCHEMA_SQL)?;
    Ok(())
}

/// The upgrade protocol's in-transaction variant of
/// [`SemanticJobStore::enqueue_rebuild`]: same manifest checks and row shape,
/// but through the caller's connection so the durable `queued` (pending)
/// rebuild job commits with everything else in the upgrade's one transaction.
pub(crate) fn enqueue_rebuild_in_transaction(
    connection: &Connection,
    generation_id: &str,
    identity: &EmbeddingIdentity,
) -> Result<JobRecord, SemanticError> {
    if connection.is_autocommit() {
        return Err(SemanticError::InvalidState(
            "job creation requires a caller-owned transaction".to_string(),
        ));
    }
    ensure_jobs_schema(connection)?;
    let manifest = generation_manifest_in_transaction(connection, generation_id)?;
    if manifest.status != "building" {
        return Err(SemanticError::InvalidState(format!(
            "generation {generation_id} is {} and cannot enqueue a rebuild",
            manifest.status
        )));
    }
    if manifest.identity != *identity {
        return Err(SemanticError::IdentityMismatch {
            expected: describe_identity(&manifest.identity),
            actual: describe_identity(identity),
        });
    }

    let job_id = rebuild_job_id(generation_id);
    let existing: Option<String> = connection
        .query_row(
            "select job_id from semantic_jobs where generation_id = ?1",
            params![generation_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(SemanticError::InvalidState(format!(
            "generation {generation_id} already has a rebuild job"
        )));
    }
    let now = now_iso8601();
    connection.execute(
        "insert into semantic_jobs(
           job_id, job_type, generation_id, status, provider, model, model_revision,
           dimensions, cursor_chunk_id, total_chunks, completed_chunks, completed_batches,
           failed_batches, attempts, last_error, cancel_requested, lease_owner,
           lease_expires_unix_ms, created_at, updated_at
         )
         values (?1, 'rebuild', ?2, 'queued', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0,
                 null, 0, null, null, ?10, ?10)",
        params![
            job_id,
            generation_id,
            identity.provider(),
            identity.model(),
            identity.revision(),
            identity.dimensions() as i64,
            manifest.last_chunk_id,
            manifest.expected_count as i64,
            manifest.written_count as i64,
            now,
        ],
    )?;
    Ok(JobRecord {
        job_id,
        job_type: JOB_TYPE_REBUILD.to_string(),
        generation_id: generation_id.to_string(),
        status: "queued".to_string(),
        identity: JobIdentity::from_embedding(identity),
        cursor_chunk_id: manifest.last_chunk_id,
        total_chunks: manifest.expected_count as i64,
        completed_chunks: manifest.written_count as i64,
        completed_batches: 0,
        failed_batches: 0,
        attempts: 0,
        last_error: None,
        cancel_requested: false,
        lease_owner: None,
        lease_expires_unix_ms: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Reads a generation manifest through an existing connection (the in-
/// transaction enqueue cannot open its own store).
fn generation_manifest_in_transaction(
    connection: &Connection,
    generation_id: &str,
) -> Result<GenerationManifest, SemanticError> {
    let row = connection
        .query_row(
            "select generation_id, status, provider, model, model_revision,
                    dimensions, normalization, tokenizer, max_input_tokens, max_batch_inputs,
                    expected_count, written_count, last_chunk_id, active,
                    created_at, updated_at
             from semantic_generations where generation_id = ?1",
            params![generation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .optional()?;
    let Some((
        generation_id,
        status,
        provider,
        model,
        revision,
        dimensions,
        normalization,
        tokenizer,
        max_input_tokens,
        max_batch_inputs,
        expected_count,
        written_count,
        last_chunk_id,
        active,
        created_at,
        updated_at,
    )) = row
    else {
        return Err(SemanticError::NotFound(format!(
            "generation {generation_id}"
        )));
    };
    let dimensions = dimensions
        .try_into()
        .map_err(|_| SemanticError::InvalidState("dimensions must be positive".to_string()))?;
    let max_input_tokens = max_input_tokens.try_into().map_err(|_| {
        SemanticError::InvalidState("max_input_tokens must be positive".to_string())
    })?;
    let max_batch_inputs = max_batch_inputs.try_into().map_err(|_| {
        SemanticError::InvalidState("max_batch_inputs must be positive".to_string())
    })?;
    let identity = EmbeddingIdentity::new(
        provider,
        model,
        revision,
        dimensions,
        normalization,
        tokenizer,
        max_input_tokens,
        max_batch_inputs,
    )?;
    Ok(GenerationManifest {
        generation_id,
        status,
        identity,
        expected_count: expected_count.max(0) as u64,
        written_count: written_count.max(0) as u64,
        last_chunk_id,
        active: active != 0,
        created_at,
        updated_at,
    })
}

const SEMANTIC_JOBS_SCHEMA_SQL: &str = "
create table if not exists semantic_jobs (
    job_id text primary key,
    job_type text not null,
    generation_id text not null references semantic_generations(generation_id)
        on delete cascade,
    status text not null check (status in ('queued','running','completed','cancelled','failed')),
    provider text not null,
    model text not null,
    model_revision text not null,
    dimensions integer not null,
    cursor_chunk_id integer not null default 0,
    total_chunks integer not null default 0,
    completed_chunks integer not null default 0,
    completed_batches integer not null default 0,
    failed_batches integer not null default 0,
    attempts integer not null default 0,
    last_error text,
    cancel_requested integer not null default 0 check (cancel_requested in (0,1)),
    lease_owner text,
    lease_expires_unix_ms integer,
    created_at text not null,
    updated_at text not null
);
create unique index if not exists one_semantic_job_per_generation
    on semantic_jobs(generation_id);
create index if not exists semantic_jobs_status_idx on semantic_jobs(status);
";

/// The four model-identity facts a job pins. Task 7's full identity checks
/// (normalization, limits, dimensions) still gate every write and the
/// activation; this record pins enough to detect a swapped provider early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobIdentity {
    pub provider: String,
    pub model: String,
    pub model_revision: String,
    pub dimensions: usize,
}

impl JobIdentity {
    fn from_embedding(identity: &EmbeddingIdentity) -> Self {
        Self {
            provider: identity.provider().to_string(),
            model: identity.model().to_string(),
            model_revision: identity.revision().to_string(),
            dimensions: identity.dimensions(),
        }
    }

    fn matches(&self, identity: &EmbeddingIdentity) -> bool {
        self.provider == identity.provider()
            && self.model == identity.model()
            && self.model_revision == identity.revision()
            && self.dimensions == identity.dimensions()
    }

    fn describe(&self) -> String {
        format!(
            "{} {}@{} ({} dims)",
            self.provider, self.model, self.model_revision, self.dimensions
        )
    }
}

/// One durable job row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: String,
    pub job_type: String,
    pub generation_id: String,
    pub status: String,
    pub identity: JobIdentity,
    pub cursor_chunk_id: i64,
    pub total_chunks: i64,
    pub completed_chunks: i64,
    pub completed_batches: i64,
    pub failed_batches: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub cancel_requested: bool,
    pub lease_owner: Option<String>,
    pub lease_expires_unix_ms: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Bounded-batch and lease parameters for one runner. Keep `batch_size` within
/// the provider's `max_batch_inputs`; [`SemanticStore::write_batch`] enforces
/// the provider limit regardless.
#[derive(Clone)]
pub struct RebuildConfig {
    /// Authoritative chunks claimed, embedded, and written per lease-held
    /// batch.
    pub batch_size: usize,
    /// How long a claimed lease stays live before another worker may take it
    /// over.
    pub lease_ttl: Duration,
    /// Consecutive failed batch attempts allowed before the job and its
    /// candidate generation are marked failed (GC-able).
    pub max_batch_attempts: u32,
    /// Supervised-shutdown hook (Task S2): checked at every batch boundary.
    /// When it reports `true`, the runner stops claiming further batches and
    /// returns [`JobOutcome::Busy`] — the lease expires naturally and
    /// reconciliation resumes the durable job after the restart. `None` for
    /// unsupervised (test) runs.
    pub stop_check: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        Self {
            batch_size: 16,
            lease_ttl: Duration::from_secs(60),
            max_batch_attempts: 3,
            stop_check: None,
        }
    }
}

/// One authoritative chunk row handed to the tokenizer. Tokenization is a
/// separate concern owned by the recall integration; jobs only need the token
/// streams, so the real tokenizer plugs in here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeChunk {
    pub chunk_id: i64,
    pub series_slug: String,
    pub text: String,
}

/// Turns authoritative chunk text into the token stream the provider embeds.
pub trait ChunkTokenizer: Send {
    fn tokenize(&mut self, chunk: &AuthoritativeChunk) -> Result<Vec<u32>, SemanticError>;
}

/// Everything a rebuild run needs beyond durable state: the embedding
/// provider, the chunk tokenizer, and the activation sample (Task 7's
/// sample-query check). The sample must probe a series that has chunks in the
/// corpus, or activation fails its empty-result check.
pub struct RebuildInputs<'a> {
    pub provider: &'a mut dyn EmbeddingProvider,
    pub tokenizer: &'a mut dyn ChunkTokenizer,
    pub sample: SemanticSample,
}

/// Result of one lease claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseClaim {
    /// The lease is ours; the job is running under this owner.
    Claimed(ClaimedJob),
    /// A foreign, unexpired lease belongs to a live worker.
    Held,
}

/// Durable facts read under the claim transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedJob {
    pub job_id: String,
    pub generation_id: String,
    pub cancel_requested: bool,
}

/// How a [`SemanticJobStore::run_rebuild`] call ended. Infrastructural
/// failures (database, missing or terminal job) propagate as `Err`; everything
/// job-shaped is an outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobOutcome {
    /// The candidate completed and activated through Task 7's checks.
    Completed { generation_id: String },
    /// The cancellation flag was honored at a batch boundary.
    Cancelled { generation_id: String },
    /// The job failed within its attempt bound; the candidate is failed and
    /// GC-able. The prior active generation is untouched.
    Failed {
        generation_id: String,
        error: String,
    },
    /// A foreign lease is still live; retry after it expires.
    Busy { generation_id: String },
    /// Another worker took the lease over mid-run; this worker stopped
    /// without recording further progress.
    LeaseLost { generation_id: String },
}

/// What [`SemanticJobStore::reconcile`] found and fixed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Running jobs whose lease expired; reclaimable by the next claim.
    pub reclaimable_jobs: Vec<String>,
    /// Running jobs whose generation already activated; completed in place.
    pub completed_jobs: Vec<String>,
    /// Running or queued jobs whose generation is cancelled.
    pub cancelled_jobs: Vec<String>,
    /// Jobs failed because their generation vanished or is terminal.
    pub failed_jobs: Vec<String>,
    /// Building generations marked failed (GC-able) behind a terminal job.
    pub failed_generations: Vec<String>,
    /// Building generations marked cancelled behind a cancelled job.
    pub cancelled_generations: Vec<String>,
}

/// What the manifest says about a claimed job's generation.
enum ManifestVerdict {
    Building(Box<GenerationManifest>),
    AlreadyActive,
    Cancelled,
    Unrecoverable(String),
}

/// Outcome of recording one failed batch attempt.
enum BatchFailure {
    /// The attempt bound is not reached; the loop may claim again.
    Retry,
    /// The bound is reached; the job and candidate are failed (GC-able).
    Exhausted,
    /// The lease was lost; another worker owns the job now.
    LeaseLost,
}

/// Durable rebuild jobs over the data root. Every method opens its own short
/// connection; workers coordinate exclusively through SQLite rows.
pub struct SemanticJobStore {
    config: HieronymusConfig,
}

impl SemanticJobStore {
    /// Opens the store, ensuring the semantic schema and the jobs table exist.
    /// This never downloads a model and never opens the vector store.
    pub fn open(config: &HieronymusConfig) -> Result<Self, SemanticError> {
        let connection = open_migrated(&config.database_path())?;
        crate::semantic_store::ensure_semantic_schema(&connection)?;
        connection.execute_batch(SEMANTIC_JOBS_SCHEMA_SQL)?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Registers the rebuild job for an existing, still-building generation.
    /// The manifest must exist, be building, and match the given identity; the
    /// job's counters start at the manifest's current progress so a
    /// partially-built generation resumes instead of restarting.
    pub fn enqueue_rebuild(
        &self,
        generation_id: &str,
        identity: &EmbeddingIdentity,
    ) -> Result<JobRecord, SemanticError> {
        let store = SemanticStore::open(&self.config)?;
        let manifest = store
            .generation_manifest(generation_id)?
            .ok_or_else(|| SemanticError::NotFound(format!("generation {generation_id}")))?;
        if manifest.status != "building" {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} is {} and cannot enqueue a rebuild",
                manifest.status
            )));
        }
        if manifest.identity != *identity {
            return Err(SemanticError::IdentityMismatch {
                expected: describe_identity(&manifest.identity),
                actual: describe_identity(identity),
            });
        }

        let job_id = rebuild_job_id(generation_id);
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "select job_id from semantic_jobs where generation_id = ?1",
                params![generation_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} already has a rebuild job"
            )));
        }
        let now = now_iso8601();
        transaction.execute(
            "insert into semantic_jobs(
               job_id, job_type, generation_id, status, provider, model, model_revision,
               dimensions, cursor_chunk_id, total_chunks, completed_chunks, completed_batches,
               failed_batches, attempts, last_error, cancel_requested, lease_owner,
               lease_expires_unix_ms, created_at, updated_at
             )
             values (?1, 'rebuild', ?2, 'queued', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0,
                     null, 0, null, null, ?10, ?10)",
            params![
                job_id,
                generation_id,
                identity.provider(),
                identity.model(),
                identity.revision(),
                identity.dimensions() as i64,
                manifest.last_chunk_id,
                manifest.expected_count as i64,
                manifest.written_count as i64,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(JobRecord {
            job_id,
            job_type: JOB_TYPE_REBUILD.to_string(),
            generation_id: generation_id.to_string(),
            status: "queued".to_string(),
            identity: JobIdentity::from_embedding(identity),
            cursor_chunk_id: manifest.last_chunk_id,
            total_chunks: manifest.expected_count as i64,
            completed_chunks: manifest.written_count as i64,
            completed_batches: 0,
            failed_batches: 0,
            attempts: 0,
            last_error: None,
            cancel_requested: false,
            lease_owner: None,
            lease_expires_unix_ms: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// The job record, if present.
    pub fn job(&self, job_id: &str) -> Result<Option<JobRecord>, SemanticError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "select job_id, job_type, generation_id, status, provider, model,
                        model_revision, dimensions, cursor_chunk_id, total_chunks,
                        completed_chunks, completed_batches, failed_batches, attempts,
                        last_error, cancel_requested, lease_owner, lease_expires_unix_ms,
                        created_at, updated_at
                 from semantic_jobs where job_id = ?1",
                params![job_id],
                job_from_row,
            )
            .optional()?;
        Ok(row)
    }

    /// Job ids a worker may claim right now, oldest first: every `queued`
    /// job plus `running` jobs whose lease expired (crash residue made
    /// reclaimable by [`Self::reconcile`]).
    pub fn claimable_jobs(&self) -> Result<Vec<String>, SemanticError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select job_id from semantic_jobs
             where status = 'queued'
                or (status = 'running'
                    and (lease_expires_unix_ms is null
                         or lease_expires_unix_ms <= ?1))
             order by created_at, job_id",
        )?;
        let rows = statement.query_map(params![now_unix_ms()], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(SemanticError::from)
    }

    /// Sets the durable cancellation flag. The current bounded batch finishes;
    /// the runner stops at the next boundary and never activates the
    /// candidate generation.
    pub fn request_cancel(&self, job_id: &str) -> Result<(), SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "update semantic_jobs set cancel_requested = 1, updated_at = ?2 where job_id = ?1",
            params![job_id, now_iso8601()],
        )?;
        transaction.commit()?;
        if updated == 0 {
            return Err(SemanticError::NotFound(format!("semantic job {job_id}")));
        }
        Ok(())
    }

    /// Claims (or renews) the job's lease in one immediate transaction. A
    /// foreign, unexpired lease is held off; an expired one is taken over
    /// without touching any generation state — the resumed run reconciles
    /// against the manifest before its next write.
    pub fn claim_lease(
        &self,
        job_id: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<LeaseClaim, SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "select generation_id, status, cancel_requested, lease_owner,
                        lease_expires_unix_ms
                 from semantic_jobs where job_id = ?1",
                params![job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((generation_id, status, cancel_requested, holder, expires)) = row else {
            return Err(SemanticError::NotFound(format!("semantic job {job_id}")));
        };
        if !matches!(status.as_str(), "queued" | "running") {
            return Err(SemanticError::InvalidState(format!(
                "semantic job {job_id} is {status} and cannot be claimed"
            )));
        }
        if let (Some(holder), Some(expires)) = (&holder, expires)
            && holder != owner
            && expires > now_unix_ms()
        {
            return Ok(LeaseClaim::Held);
        }
        transaction.execute(
            "update semantic_jobs set status = 'running', lease_owner = ?2,
                    lease_expires_unix_ms = ?3, updated_at = ?4
             where job_id = ?1",
            params![
                job_id,
                owner,
                now_unix_ms() + ttl.as_millis() as i64,
                now_iso8601()
            ],
        )?;
        transaction.commit()?;
        Ok(LeaseClaim::Claimed(ClaimedJob {
            job_id: job_id.to_string(),
            generation_id,
            cancel_requested,
        }))
    }

    /// Drives one rebuild job to a terminal outcome: claim, read the
    /// authoritative pending window, embed and write the bounded batch with no
    /// transaction open, record progress transactionally, and repeat until the
    /// candidate is complete — then activate through Task 7's atomic checks.
    /// Honors the cancellation flag at every boundary and stops with
    /// [`JobOutcome::Busy`] when a foreign lease is live.
    pub fn run_rebuild(
        &self,
        job_id: &str,
        mut inputs: RebuildInputs<'_>,
        config: &RebuildConfig,
    ) -> Result<JobOutcome, SemanticError> {
        let record = self
            .job(job_id)?
            .ok_or_else(|| SemanticError::NotFound(format!("semantic job {job_id}")))?;
        if !record.identity.matches(inputs.provider.identity()) {
            return Err(SemanticError::IdentityMismatch {
                expected: record.identity.describe(),
                actual: describe_identity(inputs.provider.identity()),
            });
        }
        let owner = process_owner();
        let store = SemanticStore::open(&self.config)?;
        // The deep persisted-state check runs once per call, on the first
        // claim: cross-call takeovers reconcile before writing anything,
        // while a live worker's own renewals stay cheap.
        let mut first_claim = true;
        loop {
            // Supervised shutdown: stop at a batch boundary. The lease lapses
            // and reconciliation resumes the durable job later.
            if let Some(stop) = &config.stop_check
                && stop()
            {
                return Ok(JobOutcome::Busy {
                    generation_id: record.generation_id.clone(),
                });
            }
            let claimed = match self.claim_lease(job_id, &owner, config.lease_ttl)? {
                LeaseClaim::Claimed(claimed) => claimed,
                LeaseClaim::Held => {
                    return Ok(JobOutcome::Busy {
                        generation_id: record.generation_id.clone(),
                    });
                }
            };
            let generation_id = claimed.generation_id;
            let manifest = match evaluate_manifest(&store, &generation_id)? {
                ManifestVerdict::Building(manifest) => manifest,
                ManifestVerdict::AlreadyActive => {
                    self.mark_job_terminal(job_id, "completed", None)?;
                    return Ok(JobOutcome::Completed { generation_id });
                }
                ManifestVerdict::Cancelled => {
                    self.mark_job_terminal(job_id, "cancelled", None)?;
                    return Ok(JobOutcome::Cancelled { generation_id });
                }
                ManifestVerdict::Unrecoverable(reason) => {
                    self.mark_job_terminal(job_id, "failed", Some(&reason))?;
                    return Ok(JobOutcome::Failed {
                        generation_id,
                        error: reason,
                    });
                }
            };
            if claimed.cancel_requested {
                return self.cancel_build(job_id, &generation_id, &store);
            }
            if first_claim {
                first_claim = false;
                if let Err(error) = self.verify_candidate(&store, &manifest) {
                    let error = error.to_string();
                    self.fail_job_and_generation(job_id, &generation_id, &error)?;
                    return Ok(JobOutcome::Failed {
                        generation_id,
                        error,
                    });
                }
            }

            let pending = store.pending_chunk_ids(&generation_id, config.batch_size)?;
            if pending.is_empty() {
                return self.activate_candidate(job_id, &generation_id, &store, &mut inputs);
            }
            match self.embed_and_write(&store, &generation_id, &pending, &mut inputs) {
                Ok(()) => {
                    if !self.commit_batch_receipt(
                        job_id,
                        &generation_id,
                        &owner,
                        config.lease_ttl,
                    )? {
                        return Ok(JobOutcome::LeaseLost { generation_id });
                    }
                }
                Err(error) => {
                    match self.record_batch_failure(
                        job_id,
                        &generation_id,
                        &owner,
                        &error.to_string(),
                        config,
                    )? {
                        BatchFailure::Retry => {}
                        BatchFailure::LeaseLost => {
                            return Ok(JobOutcome::LeaseLost { generation_id });
                        }
                        BatchFailure::Exhausted => {
                            return Ok(JobOutcome::Failed {
                                generation_id,
                                error: error.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Reconciles crash residue without destroying resumable work:
    ///
    /// - a queued or running job whose generation is missing or terminal gets
    ///   the matching terminal status;
    /// - a running job with an expired lease is reported reclaimable and its
    ///   progress is resynced from the manifest (the next claim resumes it);
    /// - a building generation behind a terminal job is marked failed (or
    ///   cancelled, matching the job) and becomes GC-able;
    /// - building generations with no job row are in-process Task 7 builds
    ///   and are never touched.
    ///
    /// Reconcile performs no LanceDB I/O; the deep persisted-state check runs
    /// at takeover.
    pub fn reconcile(&self) -> Result<ReconcileReport, SemanticError> {
        let store = SemanticStore::open(&self.config)?;
        let mut connection = self.connection()?;
        let job_ids: Vec<String> = {
            let mut statement =
                connection.prepare("select job_id from semantic_jobs order by job_id")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut report = ReconcileReport::default();
        for job_id in job_ids {
            let row = connection
                .query_row(
                    "select status, generation_id, lease_owner, lease_expires_unix_ms
                     from semantic_jobs where job_id = ?1",
                    params![job_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
                .optional()?;
            let Some((status, generation_id, holder, expires)) = row else {
                continue;
            };
            if !matches!(status.as_str(), "queued" | "running") {
                continue;
            }
            let lease_live =
                matches!((&holder, expires), (Some(_), Some(expires)) if expires > now_unix_ms());
            let Some(manifest) = store.generation_manifest(&generation_id)? else {
                let reason = format!("generation {generation_id} has no manifest");
                self.mark_job_terminal(&job_id, "failed", Some(&reason))?;
                report.failed_jobs.push(job_id);
                continue;
            };
            match manifest.status.as_str() {
                "active" => {
                    self.mark_job_terminal(&job_id, "completed", None)?;
                    report.completed_jobs.push(job_id);
                }
                "cancelled" => {
                    self.mark_job_terminal(&job_id, "cancelled", None)?;
                    report.cancelled_jobs.push(job_id);
                }
                "building" => {
                    if status == "queued" || lease_live {
                        continue;
                    }
                    // Expired lease: resumable work, not a failure. Resync the
                    // job's progress from the authoritative manifest so
                    // observers see current counters; the next claim resumes.
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    transaction.execute(
                        "update semantic_jobs set
                            cursor_chunk_id = (select last_chunk_id from semantic_generations
                                               where generation_id = ?2),
                            completed_chunks = (select written_count from semantic_generations
                                                 where generation_id = ?2),
                            updated_at = ?3
                         where job_id = ?1 and status = 'running'",
                        params![job_id, generation_id, now_iso8601()],
                    )?;
                    transaction.commit()?;
                    report.reclaimable_jobs.push(job_id);
                }
                other => {
                    let reason = format!("generation {generation_id} is {other}");
                    self.mark_job_terminal(&job_id, "failed", Some(&reason))?;
                    report.failed_jobs.push(job_id);
                }
            }
        }

        // Building generations whose only job reached a terminal state cannot
        // resume: resolve them to the matching terminal status so Task 7's GC
        // rules can collect them.
        let stuck: Vec<(String, String)> = {
            let mut statement = connection.prepare(
                "select j.generation_id, j.status from semantic_jobs j
                 join semantic_generations g on g.generation_id = j.generation_id
                 where g.status = 'building'
                   and j.status in ('completed', 'cancelled', 'failed')
                 order by j.generation_id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (generation_id, job_status) in stuck {
            let resolved = if job_status == "cancelled" {
                "cancelled"
            } else {
                "failed"
            };
            connection.execute(
                "update semantic_generations set status = ?2, updated_at = ?3
                 where generation_id = ?1 and status = 'building'",
                params![generation_id, resolved, now_iso8601()],
            )?;
            if resolved == "cancelled" {
                report.cancelled_generations.push(generation_id);
            } else {
                report.failed_generations.push(generation_id);
            }
        }
        Ok(report)
    }

    fn connection(&self) -> Result<Connection, SemanticError> {
        let connection = open_migrated(&self.config.database_path())?;
        // Workers coordinate through row-level lease transactions; the busy
        // timeout lets contending claimants serialize instead of failing.
        connection.pragma_update(None, "busy_timeout", 5_000)?;
        Ok(connection)
    }

    /// Cancels the candidate through Task 7 (which refuses anything but a
    /// building generation), then records the job's terminal state.
    fn cancel_build(
        &self,
        job_id: &str,
        generation_id: &str,
        store: &SemanticStore,
    ) -> Result<JobOutcome, SemanticError> {
        if let Err(error) = store.cancel_generation(generation_id) {
            if !matches!(error, SemanticError::InvalidState(_)) {
                return Err(error);
            }
            // The candidate reached a terminal state concurrently; resolve the
            // job against whatever the manifest now says.
            return self.resolve_against_manifest(job_id, generation_id, store);
        }
        self.mark_job_terminal(job_id, "cancelled", None)?;
        Ok(JobOutcome::Cancelled {
            generation_id: generation_id.to_string(),
        })
    }

    /// Activates the completed candidate through Task 7's atomic checks
    /// (count, checksums, dimension, sample query) and records completion.
    fn activate_candidate(
        &self,
        job_id: &str,
        generation_id: &str,
        store: &SemanticStore,
        inputs: &mut RebuildInputs<'_>,
    ) -> Result<JobOutcome, SemanticError> {
        if let Err(error) =
            store.activate_generation(generation_id, &mut *inputs.provider, &inputs.sample)
        {
            if matches!(error, SemanticError::InvalidState(_)) {
                // Another worker's activation won the exactly-once race;
                // resolve the job against the resulting manifest state.
                return self.resolve_against_manifest(job_id, generation_id, store);
            }
            let error = error.to_string();
            self.fail_job_and_generation(job_id, generation_id, &error)?;
            return Ok(JobOutcome::Failed {
                generation_id: generation_id.to_string(),
                error,
            });
        }
        self.mark_job_terminal(job_id, "completed", None)?;
        Ok(JobOutcome::Completed {
            generation_id: generation_id.to_string(),
        })
    }

    /// Maps the current manifest state onto the job's terminal outcome.
    fn resolve_against_manifest(
        &self,
        job_id: &str,
        generation_id: &str,
        store: &SemanticStore,
    ) -> Result<JobOutcome, SemanticError> {
        let owned = generation_id.to_string();
        match evaluate_manifest(store, generation_id)? {
            ManifestVerdict::Building(_) => {
                let error = format!("generation {generation_id} stayed building unexpectedly");
                self.fail_job_and_generation(job_id, generation_id, &error)?;
                Ok(JobOutcome::Failed {
                    generation_id: owned,
                    error,
                })
            }
            ManifestVerdict::AlreadyActive => {
                self.mark_job_terminal(job_id, "completed", None)?;
                Ok(JobOutcome::Completed {
                    generation_id: owned,
                })
            }
            ManifestVerdict::Cancelled => {
                self.mark_job_terminal(job_id, "cancelled", None)?;
                Ok(JobOutcome::Cancelled {
                    generation_id: owned,
                })
            }
            ManifestVerdict::Unrecoverable(reason) => {
                self.mark_job_terminal(job_id, "failed", Some(&reason))?;
                Ok(JobOutcome::Failed {
                    generation_id: owned,
                    error: reason,
                })
            }
        }
    }

    /// Reads the authoritative rows, tokenizes them, and writes one bounded
    /// batch. This is the only native section of the loop: no SQLite write
    /// transaction is open while inference or LanceDB I/O runs, and the batch
    /// is exactly the pending window Task 7's cursor contract mandates.
    fn embed_and_write(
        &self,
        store: &SemanticStore,
        generation_id: &str,
        pending: &[i64],
        inputs: &mut RebuildInputs<'_>,
    ) -> Result<(), SemanticError> {
        let mut batch = Vec::with_capacity(pending.len());
        for chunk_id in pending {
            let Some((series_slug, text)) = store.chunk_row(*chunk_id)? else {
                return Err(SemanticError::NotFound(format!("rag chunk {chunk_id}")));
            };
            let token_ids = inputs.tokenizer.tokenize(&AuthoritativeChunk {
                chunk_id: *chunk_id,
                series_slug: series_slug.clone(),
                text,
            })?;
            batch.push(SemanticChunk {
                chunk_id: *chunk_id,
                series_slug,
                token_ids,
            });
        }
        store.write_batch(generation_id, &mut *inputs.provider, &batch)?;
        Ok(())
    }

    /// Before a resumed run writes anything, reconciles the physically stored
    /// candidate rows against the manifest: same row count, same chunk
    /// id/checksum multiset behind the cursor, no foreign generations.
    /// Disagreement means a batch receipt was lost mid-write; the caller fails
    /// the job and the candidate instead of duplicating rows.
    fn verify_candidate(
        &self,
        store: &SemanticStore,
        manifest: &GenerationManifest,
    ) -> Result<(), SemanticError> {
        let generation_id = manifest.generation_id.as_str();
        let root = store.index_root();
        if !generation_table_exists(&root, generation_id) {
            if manifest.written_count > 0 {
                return Err(SemanticError::ValidationFailed(format!(
                    "generation {generation_id} has no candidate index on disk although its \
                     manifest counted {} written rows",
                    manifest.written_count
                )));
            }
            return Ok(());
        }
        let index = VectorIndex::open(&root, manifest.identity.clone(), generation_id)?;
        let stored_rows = index.count_rows()?;
        if stored_rows != manifest.written_count as usize {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {generation_id} holds {stored_rows} candidate rows while its \
                 manifest counted {} written rows; a batch receipt was lost mid-write",
                manifest.written_count
            )));
        }
        if stored_rows == 0 {
            return Ok(());
        }
        let stored = index.snapshot_rows(stored_rows)?;
        let authoritative = store.chunk_checksums_behind_cursor(manifest.last_chunk_id)?;
        if stored.len() != authoritative.len() {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {generation_id} holds {} rows behind cursor {} while the \
                 authoritative store counts {}",
                stored.len(),
                manifest.last_chunk_id,
                authoritative.len()
            )));
        }
        for row in &stored {
            if row.generation_id != generation_id {
                return Err(SemanticError::ValidationFailed(format!(
                    "generation {generation_id} stores chunk {} from generation {}",
                    row.chunk_id, row.generation_id
                )));
            }
            match authoritative.get(&row.chunk_id) {
                None => {
                    return Err(SemanticError::ValidationFailed(format!(
                        "generation {generation_id} stores chunk {} which is ahead of the \
                         manifest cursor {}",
                        row.chunk_id, manifest.last_chunk_id
                    )));
                }
                Some(checksum) if *checksum != row.checksum => {
                    return Err(SemanticError::ValidationFailed(format!(
                        "generation {generation_id} holds a stale checksum for chunk {}; the \
                         authoritative rows changed since the write",
                        row.chunk_id
                    )));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Commits one batch receipt: verifies the caller still owns the lease,
    /// resyncs cursor/counters from the authoritative manifest, and renews the
    /// lease. Returns `false` when the lease was taken over mid-batch.
    fn commit_batch_receipt(
        &self,
        job_id: &str,
        generation_id: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<bool, SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "update semantic_jobs set
                cursor_chunk_id = (select last_chunk_id from semantic_generations
                                   where generation_id = ?2),
                completed_chunks = (select written_count from semantic_generations
                                     where generation_id = ?2),
                completed_batches = completed_batches + 1,
                attempts = 0,
                lease_owner = ?3, lease_expires_unix_ms = ?4, updated_at = ?5
             where job_id = ?1 and lease_owner = ?3 and status = 'running'",
            params![
                job_id,
                generation_id,
                owner,
                now_unix_ms() + ttl.as_millis() as i64,
                now_iso8601(),
            ],
        )?;
        transaction.commit()?;
        Ok(updated == 1)
    }

    /// Records one failed batch attempt under the lease; when the consecutive
    /// attempt bound is reached, fails the job and its building candidate in
    /// the same transaction.
    fn record_batch_failure(
        &self,
        job_id: &str,
        generation_id: &str,
        owner: &str,
        error: &str,
        config: &RebuildConfig,
    ) -> Result<BatchFailure, SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            "update semantic_jobs set failed_batches = failed_batches + 1,
                    attempts = attempts + 1, last_error = ?2,
                    lease_owner = ?3, lease_expires_unix_ms = ?4, updated_at = ?5
             where job_id = ?1 and lease_owner = ?3 and status = 'running'",
            params![
                job_id,
                error,
                owner,
                now_unix_ms() + config.lease_ttl.as_millis() as i64,
                now_iso8601(),
            ],
        )?;
        if updated == 0 {
            transaction.commit()?;
            return Ok(BatchFailure::LeaseLost);
        }
        let attempts: i64 = transaction.query_row(
            "select attempts from semantic_jobs where job_id = ?1",
            params![job_id],
            |row| row.get(0),
        )?;
        let exhausted = attempts >= i64::from(config.max_batch_attempts);
        if exhausted {
            transaction.execute(
                "update semantic_jobs set status = 'failed', lease_owner = null,
                        lease_expires_unix_ms = null, updated_at = ?2
                 where job_id = ?1 and status = 'running'",
                params![job_id, now_iso8601()],
            )?;
            transaction.execute(
                "update semantic_generations set status = 'failed', updated_at = ?2
                 where generation_id = ?1 and status = 'building'",
                params![generation_id, now_iso8601()],
            )?;
        }
        transaction.commit()?;
        Ok(if exhausted {
            BatchFailure::Exhausted
        } else {
            BatchFailure::Retry
        })
    }

    /// Moves a queued or running job to a terminal state and releases the
    /// lease. A no-op when the job is already terminal.
    fn mark_job_terminal(
        &self,
        job_id: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> Result<(), SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "update semantic_jobs set status = ?2, last_error = coalesce(?3, last_error),
                    lease_owner = null, lease_expires_unix_ms = null, updated_at = ?4
             where job_id = ?1 and status in ('queued', 'running')",
            params![job_id, status, last_error, now_iso8601()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Fails the job and its building candidate generation together. The
    /// generation update is guarded by `status = 'building'`, so an
    /// already-active generation can never be failed by a losing worker.
    fn fail_job_and_generation(
        &self,
        job_id: &str,
        generation_id: &str,
        error: &str,
    ) -> Result<(), SemanticError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "update semantic_jobs set status = 'failed', last_error = ?2,
                    lease_owner = null, lease_expires_unix_ms = null, updated_at = ?3
             where job_id = ?1 and status in ('queued', 'running')",
            params![job_id, error, now_iso8601()],
        )?;
        transaction.execute(
            "update semantic_generations set status = 'failed', updated_at = ?3
             where generation_id = ?2 and status = 'building'",
            params![job_id, generation_id, now_iso8601()],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

/// Reads the manifest and classifies it against what a claimed job can still
/// do with it.
fn evaluate_manifest(
    store: &SemanticStore,
    generation_id: &str,
) -> Result<ManifestVerdict, SemanticError> {
    let Some(manifest) = store.generation_manifest(generation_id)? else {
        return Ok(ManifestVerdict::Unrecoverable(format!(
            "generation {generation_id} has no manifest"
        )));
    };
    match manifest.status.as_str() {
        "building" => Ok(ManifestVerdict::Building(Box::new(manifest))),
        "active" => Ok(ManifestVerdict::AlreadyActive),
        "cancelled" => Ok(ManifestVerdict::Cancelled),
        other => Ok(ManifestVerdict::Unrecoverable(format!(
            "generation {generation_id} is {other}"
        ))),
    }
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    Ok(JobRecord {
        job_id: row.get(0)?,
        job_type: row.get(1)?,
        generation_id: row.get(2)?,
        status: row.get(3)?,
        identity: JobIdentity {
            provider: row.get(4)?,
            model: row.get(5)?,
            model_revision: row.get(6)?,
            dimensions: row.get::<_, i64>(7)? as usize,
        },
        cursor_chunk_id: row.get(8)?,
        total_chunks: row.get(9)?,
        completed_chunks: row.get(10)?,
        completed_batches: row.get(11)?,
        failed_batches: row.get(12)?,
        attempts: row.get(13)?,
        last_error: row.get(14)?,
        cancel_requested: row.get::<_, i64>(15)? != 0,
        lease_owner: row.get(16)?,
        lease_expires_unix_ms: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

/// Lease owner identity for one runner instance: pid, per-instance counter,
/// and subsecond nanos, so two runners in one process never collide.
fn process_owner() -> String {
    static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
    let instance = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    format!("{}:{instance}:{nanos}", std::process::id())
}

fn describe_identity(identity: &EmbeddingIdentity) -> String {
    format!(
        "{} {}@{} ({} dims)",
        identity.provider(),
        identity.model(),
        identity.revision(),
        identity.dimensions()
    )
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

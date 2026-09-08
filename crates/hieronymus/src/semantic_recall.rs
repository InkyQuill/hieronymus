//! Query-time semantic lane and reciprocal-rank fusion (design §Search And
//! Fusion): the lane embeds the query with the same provider identity the
//! active generation was built under, searches the active generation with the
//! series predicate inside the ANN query (never a post-filtered over-fetch),
//! and fuses the FTS and semantic chunk lanes by RANK, never by raw scores.
//!
//! Degraded discipline: any lane failure (no active generation, model or
//! identity unavailable, index loss, search error) returns an empty semantic
//! lane and a structured [`RecallWarning`] on the response — FTS results are
//! never silently mixed with a half-running lane. Corrupt hits (stale
//! checksum, deleted chunk, foreign series or generation) are excluded and a
//! Task 8 rebuild job is scheduled over a fresh generation; one repair runs at
//! a time. Tokenization is the pinned Unigram
//! [`ModelTokenizer`](crate::semantic_tokenizer::ModelTokenizer) used identically for documents (rebuild
//! jobs) and queries (this lane), so document and query embeddings stay one
//! identity.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::OptionalExtension;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::memory_models::TranslationContext;
use crate::rag::RagStore;
use crate::rag_models::{RagChunkRecord, RagSearchHit};
use crate::recall::{
    RecallWarning, SemanticAvailability, WARNING_REPAIR_FAILED, WARNING_REPAIR_SCHEDULED,
    WARNING_SEMANTIC_UNAVAILABLE,
};
use crate::semantic_embeddings::EmbeddingProvider;
use crate::semantic_error::SemanticError;
use crate::semantic_index::VectorIndex;
use crate::semantic_jobs::{AuthoritativeChunk, ChunkTokenizer, SemanticJobStore};
use crate::semantic_store::SemanticStore;
use crate::terminology::ContractTerm;

/// Reciprocal rank fusion constant (Cormack et al. 2009, the standard k=60).
/// RRF scores are `sum(1 / (k + rank))` over the lanes a chunk appears in;
/// k=60 dampens the top-rank advantage so a single lane's rank-1 hit cannot
/// dominate the fused order, and it is large enough that rank differences
/// between the tail positions stay comparable. The value is frozen here so
/// fused orderings stay stable across releases.
pub const RRF_K: f64 = 60.0;

/// Reason carried by advisory hits that only the semantic lane surfaced.
pub const SEMANTIC_MATCH_REASON: &str = "rag semantic match";

/// Reciprocal rank fusion of the two advisory chunk lanes. Inputs are chunk
/// ids in lane rank order (position 0 is rank 1); the output is the fused
/// order with each chunk's RRF score. A chunk present in both lanes sums both
/// contributions. Ties (identical lane-membership patterns produce
/// bit-identical sums) break by chunk id ascending, so the fusion is a pure,
/// stable function of its inputs.
pub fn rrf_fuse(fts_ranked: &[i64], semantic_ranked: &[i64]) -> Vec<(i64, f64)> {
    fn accumulate(ranked: &[i64], scores: &mut HashMap<i64, f64>) {
        for (position, chunk_id) in ranked.iter().enumerate() {
            *scores.entry(*chunk_id).or_insert(0.0) += 1.0 / (RRF_K + position as f64 + 1.0);
        }
    }
    let mut scores: HashMap<i64, f64> = HashMap::new();
    accumulate(fts_ranked, &mut scores);
    accumulate(semantic_ranked, &mut scores);
    let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
    fused.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(&right.0))
    });
    fused
}

/// The reason carried when no query-time lane is armed at all: required
/// semantics did not run, so whatever came back is the lexical half only.
pub const NO_SEMANTIC_LANE_REASON: &str =
    "no semantic lane is armed for this data root; required semantic retrieval did not run";

/// Fuse the two advisory chunk lanes into one ranked hit list (design §Search
/// And Fusion). Inputs are the FTS lane's hits in rank order and the semantic
/// lane's eligible records in rank order; the output is the reciprocal-rank
/// fused order, every hit carrying its FUSED score.
///
/// The FTS hit is the preferred carrier — it holds the boost context and the
/// lane reason the frozen payloads expose — so a chunk both lanes returned
/// keeps its FTS `reason`; a semantic-only chunk carries
/// [`SEMANTIC_MATCH_REASON`], which is how callers tell the lanes apart in a
/// fused response.
///
/// One function so both hybrid entry points fuse identically: the mixed
/// `RecallService::recall` and the session-less
/// `RecallService::search_series` behind `hieronymus_rag_search` (task C5).
/// Two copies of this loop is exactly how the public search drifted onto a
/// lexical-only path in the first place.
pub fn fuse_chunk_lanes(
    fts_hits: Vec<RagSearchHit>,
    semantic_records: Vec<RagChunkRecord>,
) -> Vec<RagSearchHit> {
    let fts_ranked: Vec<i64> = fts_hits.iter().map(|hit| hit.chunk.id).collect();
    let semantic_ranked: Vec<i64> = semantic_records.iter().map(|record| record.id).collect();
    let mut carriers: HashMap<i64, RagSearchHit> = fts_hits
        .into_iter()
        .map(|hit| (hit.chunk.id, hit))
        .collect();
    for record in semantic_records {
        carriers.entry(record.id).or_insert(RagSearchHit {
            chunk: record,
            score: 0.0,
            reason: SEMANTIC_MATCH_REASON.to_string(),
        });
    }
    rrf_fuse(&fts_ranked, &semantic_ranked)
        .into_iter()
        .map(|(chunk_id, score)| {
            let hit = carriers
                .remove(&chunk_id)
                .expect("fused ids always have a lane carrier");
            RagSearchHit { score, ..hit }
        })
        .collect()
}

/// Whether a mixed recall response must carry
/// [`WARNING_SEMANTIC_UNAVAILABLE`], and with what reason (task C5, review
/// finding A5).
///
/// Both working memory and semantic RAG are mandatory, so "the semantic half
/// did not run" is a fact about the RESPONSE, not a configuration detail the
/// caller can be left to guess at. The two inputs are the only two ways to
/// learn it:
///
/// - `lane_executed` — whether this service's own query lane actually ran and
///   searched (an unarmed lane never runs; an armed one that cannot reach its
///   generation degrades, and already carries its own warning);
/// - `availability` — what the owner of the shared semantic service says
///   about it, when one is attached. A lane that ran clean over a generation
///   that does not cover the current corpus (`Rebuilding`) is still an
///   incomplete answer, and only the service knows that.
///
/// The silent case is deliberately narrow: the lane ran clean AND nothing
/// contradicts it. Before C5 an unarmed lane was treated as a "supported
/// degraded mode" and said nothing at all, which let a cold, misconfigured,
/// or failed semantic runtime coexist with ordinary-looking successful
/// results.
pub fn incomplete_semantic_reason(
    lane_executed: bool,
    availability: Option<&SemanticAvailability>,
) -> Option<String> {
    match (lane_executed, availability) {
        // Ran clean, and either nobody owns a service-level verdict or the
        // owner agrees it is serving: the response is complete.
        (true, None | Some(SemanticAvailability::Ready)) => None,
        // The service's own verdict is the most actionable reason there is,
        // whether or not this service's lane managed to run.
        (_, Some(SemanticAvailability::Unavailable(reason))) => Some(reason.clone()),
        (false, None) => Some(NO_SEMANTIC_LANE_REASON.to_string()),
        // A service claiming readiness while nothing can answer a query is a
        // contradiction; report it rather than pick a side silently.
        (false, Some(SemanticAvailability::Ready)) => Some(format!(
            "the semantic service reports ready, but {NO_SEMANTIC_LANE_REASON}"
        )),
    }
}

/// Rule ids of the contract terms whose forbidden variants occur in `text`.
/// Case-sensitive on purpose: this mirrors the forbidden-variant check in
/// `Termbase::validate`. The markers are advisory metadata only — they never
/// remove, reorder into, or satisfy the deterministic contract itself.
pub fn conflicting_rule_ids(text: &str, contract: &[ContractTerm]) -> Vec<i64> {
    let mut ids: Vec<i64> = contract
        .iter()
        .filter(|term| {
            term.forbidden_variants
                .iter()
                .any(|variant| text.contains(variant))
        })
        .map(|term| term.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Re-exported at the lane level: the retired tokenizer id. Generations
/// persisted under it predate the pinned WordPiece tokenizer and are rejected
/// by the identity check until rebuilt.
pub use crate::semantic_embeddings::BYTE_FOLD_TOKENIZER_ID;

/// The armed query-time semantic lane: one embedding provider plus the
/// tokenizer matching the document-side mapping. Both live behind one mutex
/// because the provider trait takes `&mut self` (inference state) while the
/// recall service is shared as `&self`.
pub struct SemanticLane {
    inner: std::sync::Mutex<LaneInner>,
}

struct LaneInner {
    provider: Box<dyn EmbeddingProvider>,
    tokenizer: Box<dyn ChunkTokenizer>,
}

/// One lane run: whether it ran, bounded authoritative candidate records
/// (corrupt hits excluded), and structured warnings. Current candidates precede
/// non-current source candidates; RecallService applies context disclosure and
/// separates their result sections before publication.
pub struct LaneRun {
    /// False when the lane degraded and never searched: the caller must then
    /// return the FTS results untouched (plus the warning) instead of fusing.
    pub degraded: bool,
    pub records: Vec<RagChunkRecord>,
    pub warnings: Vec<RecallWarning>,
}

enum RepairOutcome {
    Scheduled(String),
    AlreadyBuilding,
}

impl SemanticLane {
    pub fn new(provider: Box<dyn EmbeddingProvider>, tokenizer: Box<dyn ChunkTokenizer>) -> Self {
        Self {
            inner: std::sync::Mutex::new(LaneInner {
                provider,
                tokenizer,
            }),
        }
    }

    /// Runs the lane. Never fails: every failure degrades to an empty lane
    /// with a structured `semantic_lane_unavailable` warning, leaving the FTS
    /// results untouched on the response.
    pub fn run(
        &self,
        config: &HieronymusConfig,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> LaneRun {
        let result: Result<_, crate::coherent_reads::CoherentReadError> =
            crate::coherent_reads::stable_read(config, &context.series_slug, None, |db| {
                Ok(self.run_with_connection(db, config, context, query, limit))
            });
        match result {
            Ok(observed) => {
                let mut run = observed.value;
                self.publish_repairs(config, &mut run.warnings);
                run
            }
            Err(error) => LaneRun {
                degraded: true,
                records: vec![],
                warnings: vec![RecallWarning {
                    kind: WARNING_SEMANTIC_UNAVAILABLE.into(),
                    reason: error.to_string(),
                }],
            },
        }
    }

    pub(crate) fn run_with_connection(
        &self,
        connection: &rusqlite::Connection,
        config: &HieronymusConfig,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> LaneRun {
        match self.try_run(connection, config, context, query, limit) {
            Ok(run) => run,
            Err(reason) => LaneRun {
                degraded: true,
                records: Vec::new(),
                warnings: vec![RecallWarning {
                    kind: WARNING_SEMANTIC_UNAVAILABLE.to_string(),
                    reason,
                }],
            },
        }
    }

    pub(crate) fn publish_repairs(
        &self,
        config: &HieronymusConfig,
        warnings: &mut [RecallWarning],
    ) {
        for warning in warnings
            .iter_mut()
            .filter(|w| w.kind == "semantic_repair_required")
        {
            let identity = self
                .inner
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .provider
                .identity()
                .clone();
            let outcome = schedule_repair(config, &identity);
            match outcome {
                Ok(RepairOutcome::Scheduled(id)) => {
                    warning.kind = WARNING_REPAIR_SCHEDULED.into();
                    warning
                        .reason
                        .push_str(&format!("; rebuild generation {id} scheduled"));
                }
                Ok(RepairOutcome::AlreadyBuilding) => {
                    warning.kind = WARNING_REPAIR_SCHEDULED.into();
                    warning
                        .reason
                        .push_str("; a rebuild is already in progress");
                }
                Err(error) => {
                    warning.kind = WARNING_REPAIR_FAILED.into();
                    warning
                        .reason
                        .push_str(&format!("; scheduling the rebuild failed: {error}"));
                }
            }
        }
    }

    fn try_run(
        &self,
        connection: &rusqlite::Connection,
        config: &HieronymusConfig,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<LaneRun, String> {
        let authoritative = RagStore::for_read(config);
        let revision_before =
            crate::rag::current_corpus_revision(connection).map_err(|error| error.to_string())?;
        let store = SemanticStore::for_read(config);
        let manifest = match store.active_generation_with_connection(connection) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                return Err("no active semantic generation; the response is FTS-only".to_string());
            }
            Err(error) => return Err(error.to_string()),
        };
        // Lock once for the identity check and the query embedding. A
        // poisoned lock is recovered from: the guarded state is the boxed
        // provider itself.
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if manifest.identity != *guard.provider.identity() {
            let actual = describe_identity(guard.provider.identity());
            return Err(format!(
                "lane identity {actual} cannot query the generation built as {}; model identity, \
                 dimension, and generation mixing is impossible",
                describe_identity(&manifest.identity),
            ));
        }
        if !store
            .active_generation_intact_with_connection(connection)
            .map_err(|error| error.to_string())?
        {
            return Err(
                "the active generation's index is missing from disk; a rebuild is required"
                    .to_string(),
            );
        }

        // Query embedding: the tokenizer only sees text, so queries and
        // documents share one token mapping by construction.
        let synthetic = AuthoritativeChunk {
            chunk_id: 0,
            series_slug: context.series_slug.clone(),
            text: query.to_string(),
        };
        let token_ids = guard
            .tokenizer
            .tokenize(&synthetic)
            .map_err(|error| error.to_string())?;
        let vector = guard
            .provider
            .embed_query(&token_ids)
            .map_err(|error| error.to_string())?;
        drop(guard);

        let index = VectorIndex::open(
            &store.index_root(),
            manifest.identity.clone(),
            &manifest.generation_id,
        )
        .map_err(|error| error.to_string())?;
        // Same candidate depth as the FTS lane, capped identically.
        let depth = limit.min(crate::rag::MAX_RAG_SEARCH_LIMIT);
        let story_query =
            crate::story_applicability::StoryApplicability::resolve_context(connection, context)
                .map_err(|e| e.to_string())?;
        let mut candidate_depth = depth.max(1);
        let (eligible, corrupt) = loop {
            let hits = index
                .search(&context.series_slug, &vector, candidate_depth)
                .map_err(|error| format!("semantic search failed: {error}"))?;
            let chunk_ids: Vec<i64> = hits.iter().map(|hit| hit.chunk_id).collect();
            let by_id: HashMap<i64, RagChunkRecord> = authoritative
                .chunks_by_ids_with_connection(connection, &chunk_ids)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|record| (record.id, record))
                .collect();
            let mut current = vec![];
            let mut outside = vec![];
            let mut corrupt = 0;
            for hit in &hits {
                let Some(record) = by_id.get(&hit.chunk_id) else {
                    corrupt += 1;
                    continue;
                };
                if record.series_slug != context.series_slug
                    || hit.series_slug != context.series_slug
                    || hit.generation_id != manifest.generation_id
                    || crate::semantic_store::sha256_text(&record.text) != hit.checksum
                {
                    corrupt += 1;
                    continue;
                }
                let disposition = crate::claim_reads::rehydrate_claims(
                    connection,
                    crate::claim_reads::ClaimTarget::RagChunk(record.id),
                    &story_query,
                )
                .map_err(|e| e.to_string())?;
                if matches!(
                    disposition,
                    crate::claim_reads::ClaimDisposition::Current
                        | crate::claim_reads::ClaimDisposition::Qualified(_)
                ) {
                    current.push(record.clone());
                } else {
                    outside.push(record.clone());
                }
            }
            if current.len() >= depth
                || hits.len() < candidate_depth
                || candidate_depth >= crate::claim_reads::CANDIDATE_BUDGET
            {
                current.truncate(depth);
                outside.truncate(depth);
                current.extend(outside);
                break (current, corrupt);
            }
            candidate_depth = (candidate_depth * 2).min(crate::claim_reads::CANDIDATE_BUDGET);
        };
        drop(index);
        let revision_after =
            crate::rag::current_corpus_revision(connection).map_err(|error| error.to_string())?;
        let mut warnings = Vec::new();
        if manifest.corpus_revision != revision_before || manifest.corpus_revision != revision_after
        {
            // The old index remains useful to mixed recall. Strict search
            // refuses this warning because its bare array cannot report gaps.
            warnings.push(RecallWarning {
                kind: WARNING_SEMANTIC_UNAVAILABLE.to_string(),
                reason: format!("semantic generation {} covers corpus revision {}, but the query observed revisions {revision_before} through {revision_after}; a current rebuild is required", manifest.generation_id, manifest.corpus_revision),
            });
        }

        if corrupt > 0 {
            warnings.push(RecallWarning {
                kind: "semantic_repair_required".into(),
                reason: format!("{corrupt} corrupt semantic hit(s) excluded"),
            });
        }
        // Both current and non-current records are rehydrated by the enclosing
        // recall snapshot before disclosure; outside records never take current slots.
        Ok(LaneRun {
            degraded: false,
            records: eligible,
            warnings,
        })
    }
}

/// Outcome of queueing a whole-corpus rebuild ([`queue_semantic_rebuild`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueOutcome {
    /// A fresh generation was begun and its rebuild job enqueued; the id is
    /// the durable job id.
    Enqueued(String),
    /// A building generation with a live job already covers exactly the
    /// current authoritative corpus; its job id is returned.
    AlreadyQueued(String),
    /// The corpus has no chunks: nothing to index, the lane stays
    /// ready-for-ingest (never an error).
    EmptyCorpus,
}

/// Whether a queued rebuild already covers exactly the build the caller is
/// about to request (task C4, review finding A4).
///
/// This replaces the pre-C4 equivalence, which compared nothing but the frozen
/// `expected_count` against the current authoritative chunk count. That test
/// is unsound in both directions a real corpus moves:
///
/// - **replacement at equal count.** Editing a document, or swapping one for
///   another of the same length, leaves the chunk count untouched while
///   changing the text of every chunk it owns. The count-only check called
///   that a dedup hit, so the queued generation — built from the OLD text —
///   was allowed to activate and the edit was never semantically retrievable.
/// - **a changed embedding identity.** A model, revision, dimension, or
///   tokenizer swap makes the queued vectors unusable for this daemon's
///   queries no matter how many chunks they cover.
///
/// Corpus revision plus identity is exact on both counts: the revision is
/// bumped by every text-affecting authoritative write, and the identity is the
/// one the vectors will actually be built under. Both must match.
pub fn same_build_request(
    current_revision: i64,
    queued_revision: i64,
    current_identity: &crate::semantic_embeddings::EmbeddingIdentity,
    queued_identity: &crate::semantic_embeddings::EmbeddingIdentity,
) -> bool {
    current_revision == queued_revision && current_identity == queued_identity
}

/// Queues a whole-corpus rebuild generation plus its durable job (Task S2's
/// post-commit queueing, also the startup recovery path for chunks with no
/// active generation). One rebuild is in flight at a time:
///
/// - a live building job that already covers exactly this build request —
///   same corpus revision, same embedding identity
///   ([`same_build_request`]) — is returned as-is (dedup);
/// - a live building job that does not is durably cancelled (its candidate can
///   never cover the current corpus) and a fresh generation is queued over the
///   whole corpus.
///
/// Either way the durable work intent is retired up to the revision now
/// covered, so reconciliation stops re-queueing work that is queued or done
/// while a LATER import's intent (raised after this transaction read the
/// revision) still survives to be honoured.
///
/// The check, the cancel, the candidate generation, the job insert, and the
/// intent retirement all happen inside ONE `BEGIN IMMEDIATE` transaction, so
/// two imports racing in this window serialize: the loser re-reads the
/// winner's fresh job and dedups (or supersedes it) instead of queueing a
/// second generation. Any failure rolls the whole thing back — no cancelled
/// flag, unqueued candidate, or prematurely retired intent can survive a
/// partial queueing.
pub fn queue_semantic_rebuild(
    config: &HieronymusConfig,
    identity: &crate::semantic_embeddings::EmbeddingIdentity,
) -> Result<QueueOutcome, SemanticError> {
    use rusqlite::{TransactionBehavior, params};

    // Schema bootstrap only (idempotent, autocommit); the decision below runs
    // in its own immediate transaction.
    SemanticStore::open(config)?;
    SemanticJobStore::open(config)?;
    let mut connection = open_migrated(Path::new(&config.database_path()))?;
    // Two racing importers contend for the same write lock; the busy timeout
    // lets them serialize instead of failing.
    connection.pragma_update(None, "busy_timeout", 5_000)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let chunk_count: i64 =
        transaction.query_row("select count(*) from rag_chunks", [], |row| row.get(0))?;
    let corpus_revision = crate::rag::current_corpus_revision(&transaction)?;
    if chunk_count == 0 {
        // Nothing to index: the request IS satisfied, so the owed-work record
        // must be retired too or reconciliation would ask forever.
        crate::rag::clear_semantic_work_intent_through(&transaction, corpus_revision)?;
        transaction.commit()?;
        return Ok(QueueOutcome::EmptyCorpus);
    }

    let live = live_building_job(&transaction)?;
    if let Some((job_id, queued_revision, queued_identity)) = &live
        && same_build_request(corpus_revision, *queued_revision, identity, queued_identity)
    {
        crate::rag::clear_semantic_work_intent_through(&transaction, corpus_revision)?;
        let job_id = job_id.clone();
        transaction.commit()?;
        return Ok(QueueOutcome::AlreadyQueued(job_id));
    }
    if let Some((job_id, _, _)) = &live {
        transaction.execute(
            "update semantic_jobs set cancel_requested = 1, updated_at = ?2 where job_id = ?1",
            params![job_id, chrono::Utc::now().to_rfc3339()],
        )?;
    }

    // Unique across racers and repeated calls: wall-clock nanos plus a
    // process-local sequence (two serialized racers can share a nanosecond).
    static QUEUE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation_id = format!(
        "rebuild-{}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        QUEUE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    crate::semantic_store::begin_generation_in_transaction(&transaction, &generation_id, identity)?;
    let record = crate::semantic_jobs::enqueue_rebuild_in_transaction(
        &transaction,
        &generation_id,
        identity,
    )?;
    crate::rag::clear_semantic_work_intent_through(&transaction, corpus_revision)?;
    transaction.commit()?;
    Ok(QueueOutcome::Enqueued(record.job_id))
}

/// The one live building job, with the corpus revision and full embedding
/// identity its candidate generation was begun under. `None` means no rebuild
/// is in flight, so nothing can be deduped against.
///
/// The identity is read in full (not just provider/model/dimensions like
/// `JobIdentity`): normalization, tokenizer, and the input limits are part of
/// what makes two sets of vectors mutually queryable, and [`same_build_request`]
/// compares all of it.
fn live_building_job(
    connection: &rusqlite::Connection,
) -> Result<Option<(String, i64, crate::semantic_embeddings::EmbeddingIdentity)>, SemanticError> {
    let row = connection
        .query_row(
            "select j.job_id, g.corpus_revision, g.provider, g.model, g.model_revision,
                    g.dimensions, g.normalization, g.tokenizer, g.max_input_tokens,
                    g.max_batch_inputs
             from semantic_jobs j
             join semantic_generations g on g.generation_id = j.generation_id
             where j.status in ('queued', 'running')
               and g.status = 'building'
             order by j.created_at
             limit 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        job_id,
        corpus_revision,
        provider,
        model,
        revision,
        dimensions,
        normalization,
        tokenizer,
        max_input_tokens,
        max_batch_inputs,
    )) = row
    else {
        return Ok(None);
    };
    let identity = crate::semantic_embeddings::EmbeddingIdentity::new(
        provider,
        model,
        revision,
        dimensions.max(0) as usize,
        normalization,
        tokenizer,
        max_input_tokens.max(0) as usize,
        max_batch_inputs.max(0) as usize,
    )?;
    Ok(Some((job_id, corpus_revision, identity)))
}

/// Schedules the repair for corrupt hits: a fresh generation plus the Task 8
/// rebuild job that drives it to activation. This is the smallest repair path
/// that reuses the durable job system verbatim — Task 8 jobs attach to a
/// building candidate, never to the active generation, so the repair is a new
/// generation. One repair at a time: any building generation (of any identity)
/// means a rebuild is already in flight and nothing new is scheduled.
fn schedule_repair(
    config: &HieronymusConfig,
    identity: &crate::semantic_embeddings::EmbeddingIdentity,
) -> Result<RepairOutcome, SemanticError> {
    {
        let connection = open_migrated(Path::new(&config.database_path()))?;
        let building: i64 = connection.query_row(
            "select count(*) from semantic_generations where status = 'building'",
            [],
            |row| row.get(0),
        )?;
        if building > 0 {
            return Ok(RepairOutcome::AlreadyBuilding);
        }
    }
    let generation_id = format!(
        "repair-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let store = SemanticStore::open(config)?;
    store.begin_generation(&generation_id, identity)?;
    let jobs = SemanticJobStore::open(config)?;
    match jobs.enqueue_rebuild(&generation_id, identity) {
        Ok(_) => Ok(RepairOutcome::Scheduled(generation_id)),
        Err(error) => {
            // Never leave an unqueued candidate generation behind.
            let _ = store.cancel_generation(&generation_id);
            Err(error)
        }
    }
}

fn describe_identity(identity: &crate::semantic_embeddings::EmbeddingIdentity) -> String {
    format!(
        "{} {}@{} ({} dims)",
        identity.provider(),
        identity.model(),
        identity.revision(),
        identity.dimensions()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_term(id: i64, forbidden: &[&str]) -> ContractTerm {
        ContractTerm {
            id,
            category: "rule".to_string(),
            source_text: "source".to_string(),
            canonical_translation: "canonical".to_string(),
            forbidden_variants: forbidden
                .iter()
                .map(|variant| variant.to_string())
                .collect(),
            tags: Vec::new(),
            notes: String::new(),
        }
    }

    #[test]
    fn rrf_combines_ranks_and_breaks_ties_by_chunk_id() {
        // Chunk 1 is FTS rank 1 + semantic rank 2; chunk 2 is FTS rank 2 +
        // semantic rank 1: identical membership patterns, so a tie broken by
        // chunk id ascending. Chunks 3 and 4 tie on a single lane rank too.
        let fused = rrf_fuse(&[1, 2, 3], &[2, 1, 4]);
        let ids: Vec<i64> = fused.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        let both = 1.0 / (RRF_K + 1.0) + 1.0 / (RRF_K + 2.0);
        assert!((fused[0].1 - both).abs() < 1e-12);
        assert!((fused[1].1 - both).abs() < 1e-12);
        let single = 1.0 / (RRF_K + 3.0);
        assert!((fused[2].1 - single).abs() < 1e-12);
        assert!((fused[3].1 - single).abs() < 1e-12);
    }

    #[test]
    fn rrf_with_one_empty_lane_preserves_the_other_lane_and_ignores_duplicates() {
        let fused = rrf_fuse(&[5, 6], &[]);
        assert_eq!(
            fused,
            vec![(5, 1.0 / (RRF_K + 1.0)), (6, 1.0 / (RRF_K + 2.0))]
        );
        assert!(rrf_fuse(&[], &[]).is_empty());
        // A chunk repeated within one lane accumulates both rank contributions
        // into its single fused entry.
        let fused = rrf_fuse(&[7, 7], &[]);
        assert_eq!(fused, vec![(7, 1.0 / (RRF_K + 1.0) + 1.0 / (RRF_K + 2.0))]);
    }

    fn chunk(id: i64, text: &str) -> RagChunkRecord {
        RagChunkRecord {
            claim_annotation: Default::default(),
            id,
            source_id: 1,
            series_slug: "demo".to_string(),
            source_ref: "chapter.txt".to_string(),
            chunk_kind: "text".to_string(),
            text: text.to_string(),
            display_text: text.to_string(),
            location: "paragraph 1".to_string(),
            metadata: serde_json::Map::new(),
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
        }
    }

    #[test]
    fn fusion_keeps_the_fts_carrier_and_marks_semantic_only_hits() {
        let fts = vec![RagSearchHit {
            chunk: chunk(1, "lexical"),
            score: 2.5,
            reason: "rag project text match".to_string(),
        }];
        let semantic = vec![chunk(1, "lexical"), chunk(2, "paraphrase")];
        let fused = fuse_chunk_lanes(fts, semantic);

        let ids: Vec<i64> = fused.iter().map(|hit| hit.chunk.id).collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "both-lane hit ranks above the semantic-only"
        );
        // The carrier's reason survives; the fused score replaces the lane
        // score (RRF is over ranks, never raw lane scores).
        assert_eq!(fused[0].reason, "rag project text match");
        assert!((fused[0].score - (1.0 / (RRF_K + 1.0)) * 2.0).abs() < 1e-12);
        assert_eq!(fused[1].reason, SEMANTIC_MATCH_REASON);
        assert!((fused[1].score - 1.0 / (RRF_K + 2.0)).abs() < 1e-12);
    }

    /// The C5 truth table: the only silent combination is a lane that ran
    /// clean with nothing contradicting it.
    #[test]
    fn incomplete_semantics_is_silent_only_when_the_lane_actually_ran() {
        let unavailable = SemanticAvailability::Unavailable("assets are acquiring".to_string());

        assert_eq!(incomplete_semantic_reason(true, None), None);
        assert_eq!(
            incomplete_semantic_reason(true, Some(&SemanticAvailability::Ready)),
            None
        );
        assert_eq!(
            incomplete_semantic_reason(true, Some(&unavailable)).as_deref(),
            Some("assets are acquiring"),
            "a clean lane over a service that is not serving is still incomplete"
        );
        assert_eq!(
            incomplete_semantic_reason(false, Some(&unavailable)).as_deref(),
            Some("assets are acquiring")
        );
        assert_eq!(
            incomplete_semantic_reason(false, None).as_deref(),
            Some(NO_SEMANTIC_LANE_REASON)
        );
        assert!(
            incomplete_semantic_reason(false, Some(&SemanticAvailability::Ready))
                .is_some_and(|reason| reason.contains(NO_SEMANTIC_LANE_REASON)),
            "a ready service with no lane is a contradiction, not silence"
        );
    }

    #[test]
    fn conflicting_rule_ids_mark_forbidden_variants_only() {
        let contract = vec![contract_term(7, &["sorcery"]), contract_term(9, &["wyrd"])];
        assert_eq!(
            conflicting_rule_ids("the sorcery and wyrd text", &contract),
            vec![7, 9]
        );
        // Case-sensitive, mirroring the forbidden-variant check in validate.
        assert!(conflicting_rule_ids("Sorcery is capitalized", &contract).is_empty());
        // Canonical renderings alone never mark.
        assert!(conflicting_rule_ids("plain canonical text", &contract).is_empty());
        let duplicated = vec![
            contract_term(7, &["sorcery"]),
            contract_term(7, &["sorcery"]),
        ];
        assert_eq!(conflicting_rule_ids("sorcery", &duplicated), vec![7]);
    }

    /// The full truth table of the dedup predicate: revision match/mismatch
    /// crossed with identity match/mismatch. Only the both-match corner is a
    /// dedup hit — the pre-C4 count comparison said "already queued" for three
    /// of these four.
    #[test]
    fn same_build_request_needs_both_the_revision_and_the_identity() {
        use crate::semantic_embeddings::{EmbeddingProvider, FakeEmbeddingProvider};
        let identity = FakeEmbeddingProvider::new(384).identity().clone();
        let other = FakeEmbeddingProvider::new(256).identity().clone();
        assert_ne!(identity, other);

        assert!(same_build_request(9, 9, &identity, &identity));
        assert!(!same_build_request(9, 8, &identity, &identity));
        assert!(!same_build_request(9, 9, &identity, &other));
        assert!(!same_build_request(9, 8, &identity, &other));

        // The pre-revision sentinel is behind everything, revision 0 included.
        assert!(!same_build_request(
            0,
            crate::semantic_store::UNKNOWN_CORPUS_REVISION,
            &identity,
            &identity
        ));
    }
}

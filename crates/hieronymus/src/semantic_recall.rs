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
//! a time. Tokenization is the pinned WordPiece [`ModelTokenizer`](crate::
//! semantic_tokenizer::ModelTokenizer) used identically for documents (rebuild
//! jobs) and queries (this lane), so document and query embeddings stay one
//! identity.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::OptionalExtension;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::memory_models::TranslationContext;
use crate::rag::RagStore;
use crate::rag_models::RagChunkRecord;
use crate::recall::{
    RecallWarning, WARNING_REPAIR_FAILED, WARNING_REPAIR_SCHEDULED, WARNING_SEMANTIC_UNAVAILABLE,
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

/// One lane run: whether the lane could run at all, its eligible hits in rank
/// order (corrupt hits already excluded), and any structured warnings the run
/// produced.
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
        match self.try_run(config, context, query, limit) {
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

    fn try_run(
        &self,
        config: &HieronymusConfig,
        context: &TranslationContext,
        query: &str,
        limit: usize,
    ) -> Result<LaneRun, String> {
        let store = SemanticStore::open(config).map_err(|error| error.to_string())?;
        let manifest = match store.active_generation() {
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
            .active_generation_intact()
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
        let lane_identity = guard.provider.identity().clone();
        drop(guard);

        let index = VectorIndex::open(
            &store.index_root(),
            manifest.identity.clone(),
            &manifest.generation_id,
        )
        .map_err(|error| error.to_string())?;
        // Same candidate depth as the FTS lane, capped identically.
        let depth = limit.min(crate::rag::MAX_RAG_SEARCH_LIMIT);
        let hits = index
            .search(&context.series_slug, &vector, depth)
            .map_err(|error| format!("semantic search failed: {error}"))?;
        drop(index);

        // Hit integrity: every hit must still match its authoritative row.
        // Stale checksums, deleted chunks, and foreign series or generations
        // are corrupt; they are excluded and schedule a rebuild.
        let chunk_ids: Vec<i64> = hits.iter().map(|hit| hit.chunk_id).collect();
        let hydrated = RagStore::open(config)
            .map_err(|error| error.to_string())?
            .chunks_by_ids(&chunk_ids)
            .map_err(|error| error.to_string())?;
        let by_id: HashMap<i64, RagChunkRecord> = hydrated
            .into_iter()
            .map(|record| (record.id, record))
            .collect();
        let mut eligible = Vec::with_capacity(hits.len());
        let mut corrupt = 0usize;
        for hit in hits {
            let Some(record) = by_id.get(&hit.chunk_id) else {
                corrupt += 1;
                continue;
            };
            let intact = record.series_slug == context.series_slug
                && hit.series_slug == context.series_slug
                && hit.generation_id == manifest.generation_id
                && crate::semantic_store::sha256_text(&record.text) == hit.checksum;
            if intact {
                eligible.push(record.clone());
            } else {
                corrupt += 1;
            }
        }

        let mut warnings = Vec::new();
        if corrupt > 0 {
            match schedule_repair(config, &lane_identity) {
                Ok(RepairOutcome::Scheduled(generation_id)) => warnings.push(RecallWarning {
                    kind: WARNING_REPAIR_SCHEDULED.to_string(),
                    reason: format!(
                        "{corrupt} corrupt semantic hit(s) excluded; rebuild generation \
                         {generation_id} scheduled"
                    ),
                }),
                Ok(RepairOutcome::AlreadyBuilding) => warnings.push(RecallWarning {
                    kind: WARNING_REPAIR_SCHEDULED.to_string(),
                    reason: format!(
                        "{corrupt} corrupt semantic hit(s) excluded; a rebuild is already \
                         in progress"
                    ),
                }),
                Err(error) => warnings.push(RecallWarning {
                    kind: WARNING_REPAIR_FAILED.to_string(),
                    reason: format!(
                        "{corrupt} corrupt semantic hit(s) excluded; scheduling the rebuild \
                         failed: {error}"
                    ),
                }),
            }
        }
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

/// Queues a whole-corpus rebuild generation plus its durable job (Task S2's
/// post-commit queueing, also the startup recovery path for chunks with no
/// active generation). One rebuild is in flight at a time:
///
/// - a live building job whose frozen `expected_count` matches the current
///   authoritative count is returned as-is (dedup);
/// - a live building job whose count went stale (a concurrent import landed
///   after the generation froze its expectation) is durably cancelled — its
///   candidate can never cover the new chunks — and a fresh generation is
///   queued over the whole corpus.
pub fn queue_semantic_rebuild(
    config: &HieronymusConfig,
    identity: &crate::semantic_embeddings::EmbeddingIdentity,
) -> Result<QueueOutcome, SemanticError> {
    let store = SemanticStore::open(config)?;
    let jobs = SemanticJobStore::open(config)?;
    let chunk_count = {
        let connection = open_migrated(Path::new(&config.database_path()))?;
        connection.query_row("select count(*) from rag_chunks", [], |row| {
            row.get::<_, i64>(0)
        })?
    };
    if chunk_count == 0 {
        return Ok(QueueOutcome::EmptyCorpus);
    }

    let live: Option<(String, i64)> = {
        let connection = open_migrated(Path::new(&config.database_path()))?;
        connection
            .query_row(
                "select j.job_id, g.expected_count
                 from semantic_jobs j
                 join semantic_generations g on g.generation_id = j.generation_id
                 where j.status in ('queued', 'running')
                   and g.status = 'building'
                 order by j.created_at
                 limit 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(SemanticError::from)?
    };
    if let Some((job_id, expected_count)) = live {
        if expected_count == chunk_count {
            return Ok(QueueOutcome::AlreadyQueued(job_id));
        }
        jobs.request_cancel(&job_id)?;
    }

    let generation_id = format!(
        "rebuild-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    store.begin_generation(&generation_id, identity)?;
    match jobs.enqueue_rebuild(&generation_id, identity) {
        Ok(record) => Ok(QueueOutcome::Enqueued(record.job_id)),
        Err(error) => {
            // Never leave an unqueued candidate generation behind.
            let _ = store.cancel_generation(&generation_id);
            Err(error)
        }
    }
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
}

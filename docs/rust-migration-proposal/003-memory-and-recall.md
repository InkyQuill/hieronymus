# Rust Migration Proposal: 003 - Memory & Recall System

Phase 3 of the Rust migration (depends on 001 for `HieronymusConfig`, 002 for `SqlitePool`,
row structs, and trigger-owned FTS). Covers domain models, every memory/concept/termbase store,
the reconsolidation-based recall system, the RAG pipeline, and local semantic (vector) retrieval.

**Parity target:** every store/service under `hiero-core::domain`, `hiero-core::rag`,
`hiero-core::semantic`, and `hiero-core::ingest`. **Legacy not carried forward:** `MemoryStore`
as a standalone wrapper class (its two methods collapse into direct calls below),
`strict_terms`-based term storage (rule-intent crystals only — see §4), the Python
`TranslationContext` pattern of normalizing tuples via `object.__setattr__` inside a frozen
dataclass, and the "active rule crystals are a mandatory, unbeatable recall lane" invariant from
this proposal's first draft — verified against `recall.py:268-269`, the real implementation is a
flat `+0.20` score boost (`_ACTIVE_RULE_BOOST`), not an unbeatable lane; see §3 for the corrected
model, formalized in `docs/superpowers/specs/2026-07-18-memory-reconsolidation-design.md`.

---

## 1. Domain Models

```rust
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationContext {
    pub series_slug: String,
    pub scope_key: String,           // computed at construction time, not a lazy property
    pub source_language: String,
    pub target_language: String,
    pub language_tags: Vec<String>,  // normalized via values::normalize_tuple at construction
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemorySource { LongTerm, ShortTerm, Rag }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub source: MemorySource,
    pub rank: usize,
    pub score: f64,
    pub text: String,
    pub reason: String,
    pub id: i64,
    pub metadata: serde_json::Value,
}
```

`CrystalRecord`, `CrystalActivationRecord`, `CrystalLinkRecord`, `ConceptRecord`,
`ConceptFacetRecord`, `ConceptProposalRecord`, `TaskSessionRecord`, `ShortTermMemoryRecord`,
`RagChunkRecord`, `MemoryEventRecord` are all defined in 002 §5 — every store below returns
those types, not local redefinitions. `CrystalType` (002 §5) enumerates
`{Lesson, Rule, Thought, Observation, ConceptNote, Concept, Erudition}` — this categorizes what
kind of memory a crystal is; it is **independent** of `rule_intent` (a freeform text field
signaling deterministic intent that can be set on *any* crystal type, not just `Rule`) and of
`source_credibility` (a freeform text label from a fixed small set — see §3.1).

---

## 2. Store Implementations

Every store holds `&SqlitePool` via construction, not passed per-call — matching the current
Python pattern (`FeedbackStore.__init__` stores `self.config`/connection state;
`scoring.py:49-53`) and avoiding the "pool passed both implicitly and explicitly" shape.

### 2.1 `CrystalStore`

```rust
pub struct AddCrystalInput {
    pub crystal_type: String, pub title: String, pub text: String,
    pub scope_type: String, pub scope_key: String, pub series_slug: String,
    pub source_language: String, pub target_language: String,
    pub source_credibility: String, pub rule_intent: String,
}

pub struct CrystalStore<'a> { pool: &'a SqlitePool }
impl<'a> CrystalStore<'a> {
    pub async fn add(&self, input: AddCrystalInput) -> Result<i64>;
    pub async fn get(&self, id: i64) -> Result<CrystalRecord>;
    pub async fn list_rule_intent(&self, filter: RuleFilter) -> Result<Vec<CrystalRecord>>;  // crystals with non-empty rule_intent, not crystal_type == 'rule'
    pub async fn archive(&self, id: i64) -> Result<CrystalRecord>;                             // general archive, any crystal_type — atomic, transaction-safe
    pub async fn supersede(&self, old_id: i64, new: AddCrystalInput) -> Result<CrystalRecord>;  // inserts new crystal with supersedes_crystal_id = old_id, flips old to 'superseded'
    pub async fn link(&self, source_id: i64, target_id: i64, link_type: &str) -> Result<CrystalLinkRecord>;  // crystal_links row, insert-or-ignore semantics
    pub async fn linked(&self, crystal_id: i64) -> Result<Vec<(CrystalLinkRecord, f64)>>;      // 1-hop neighbors with a derived link_weight, see §3
    pub async fn validate_rule(&self, id: i64) -> Result<ValidationReport>;
    pub async fn set_story_scopes(&self, id: i64, scopes: &[String]) -> Result<CrystalRecord>;
    pub async fn set_semantic_tags(&self, id: i64, tags: &[String]) -> Result<CrystalRecord>;
    pub async fn lowest_confidence(&self, ids: &[i64], limit: usize) -> Result<Vec<i64>>;
    pub async fn search(&self, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<CrystalRecord>>;               // FTS5 via crystals_fts
    pub async fn search_scored(&self, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<(CrystalRecord, f64)>>; // weighted BM25 + strength + confidence, before the recall-time boost in §3
}
pub fn search_expression(query: &str) -> String;  // tokenizes and quotes for FTS5, standalone fn

pub struct ValidationReport { pub ok: bool, pub findings: Vec<String> }
```

`link_type` values include at minimum `"related"` (general association, populated by
`LinkReinforcer` — §3.4) and any dreaming-assigned relation types; `linked()`'s derived
`link_weight` starts uniform (e.g. `1.0` per link) and is not yet a tunable strength column —
`crystal_links` has no weight column in the current schema (002 §2), so weight is derived from
link count/recency at query time rather than stored, unless a future migration adds one.

### 2.2 `WorkspaceStore` (sessions + short-term memory)

```rust
pub struct AddMemoryInput { pub source_role: String, pub kind: String, pub text: String, pub source_ref: String }

pub struct WorkspaceStore<'a> { pool: &'a SqlitePool }
impl<'a> WorkspaceStore<'a> {
    pub async fn start_session(&self, ctx: &TranslationContext, task_type: &str, volume: &str, chapter: &str) -> Result<TaskSessionRecord>;
    pub async fn get_session(&self, id: i64) -> Result<TaskSessionRecord>;
    pub async fn complete_session(&self, id: i64) -> Result<bool>;
    pub async fn complete_inactive(&self, cutoff: DateTime<Utc>) -> Result<Vec<i64>>;   // runs in 004's interval loop, not a separate thread
    pub async fn add_short_term(&self, session_id: i64, input: AddMemoryInput) -> Result<i64>;              // FTS trigger handles index (002 §3)
    pub async fn add_short_term_batch(&self, session_id: i64, items: &[AddMemoryInput]) -> Result<Vec<i64>>; // batch up to 500
    pub async fn list_short_term(&self, session_id: i64) -> Result<Vec<ShortTermMemoryRecord>>;
    pub async fn search_short_term(&self, session_id: i64, query: &str, limit: usize) -> Result<Vec<ShortTermMemoryRecord>>;

    /// Reconsolidation working-copy dedup, called from RecallService (§3), not exposed as a
    /// standalone CLI/MCP operation. Looks up an existing row with source_crystal_id = crystal_id
    /// for this session; inserts one seeded from the crystal's text if absent.
    pub(crate) async fn get_or_create_working_copy(&self, session_id: i64, crystal: &CrystalRecord) -> Result<(ShortTermMemoryRecord, bool)>;  // bool = was newly created
    pub async fn archive(&self, id: i64) -> Result<()>;  // sets archived_at; used by 004's Reconsolidator once a working copy is processed
}

/// Standalone fn, not a method — called from 004's background loop, not a dedicated thread.
pub async fn complete_stale_sessions(pool: &SqlitePool, cutoff: DateTime<Utc>) -> Result<Vec<i64>>;
```

### 2.3 `ConceptStore` / concept proposals

Concepts scope via `scope_type`/`scope_key` (`global` scope has empty `scope_key`; any other
scope type — e.g. `series`, `story`) requires a non-empty `scope_key`, matching the `CHECK`
constraint in 002 §2), **not** a direct series foreign key.

```rust
pub struct CreateConceptInput { pub canonical_name: String, pub scope_type: String, pub scope_key: String }
pub struct ConceptFilter { pub scope_type: String, pub scope_key: String, pub status: Option<String> }

pub struct ConceptStore<'a> { pool: &'a SqlitePool }
impl<'a> ConceptStore<'a> {
    pub async fn create(&self, input: CreateConceptInput) -> Result<ConceptRecord>;
    pub async fn get(&self, id: i64) -> Result<ConceptRecord>;
    pub async fn search(&self, query: &str, filter: ConceptFilter) -> Result<Vec<ConceptRecord>>;
    pub async fn add_facet(&self, concept_id: i64, language: &str, facet_type: &str, value: &str) -> Result<ConceptFacetRecord>;
    pub async fn update_facet(&self, facet_id: i64, value: &str) -> Result<ConceptFacetRecord>;
    pub async fn set_canonical_facet(&self, facet_id: i64) -> Result<()>;
    pub async fn merge_concepts(&self, source: i64, target: i64, reason: &str) -> Result<()>;    // sets source.merged_into_concept_id, unions facets
    pub async fn rename_concept(&self, id: i64, new_name: &str, reason: &str) -> Result<ConceptRecord>;  // writes a concept_renames row
    pub async fn link_crystal(&self, crystal_id: i64, concept_id: i64, link_type: &str, confidence: f64) -> Result<()>;  // crystal_concepts row
    pub async fn recall_boosts(&self, crystal_ids: &[i64], query: &str, story_scopes: &[String]) -> Result<HashMap<i64, f64>>;
}

pub struct CreateProposalInput { pub series_slug: String, pub source_language: String, pub target_language: String, pub concept_text: String, pub source_form: String, pub canonical_rendering: String }

pub struct ConceptProposalStore<'a> { pool: &'a SqlitePool }
impl<'a> ConceptProposalStore<'a> {
    pub async fn create(&self, input: CreateProposalInput) -> Result<i64>;
    pub async fn get(&self, id: i64) -> Result<ConceptProposalRecord>;
    pub async fn list_pending(&self) -> Result<Vec<ConceptProposalRecord>>;
    pub async fn approve(&self, id: i64) -> Result<()>;  // atomic: BEGIN IMMEDIATE, validates JSON before mutation, never leaves an orphaned concept
    pub async fn reject(&self, id: i64) -> Result<()>;
}
```

### 2.4 `Termbase` and rule-intent crystals

Terminology constraints are rule-intent crystals — `crystal_type` can be `"rule"` for a
dedicated terminology entry, but the recall-time and decay-time special-casing (§3, §4) keys off
`rule_intent` being non-empty, not `crystal_type`. This lets any crystal type carry rule intent
(e.g. an `observation` that later gets promoted to a hard constraint without changing its type).

```rust
pub struct TermProposal { pub series_slug: String, pub source_language: String, pub target_language: String, pub category: String, pub source_text: String, pub canonical_translation: String, pub tags: Vec<String> }

pub struct Termbase<'a> { pool: &'a SqlitePool }
impl<'a> Termbase<'a> {
    pub async fn propose(&self, input: TermProposal) -> Result<i64>;   // writes a crystal_type="rule" crystal with source_credibility="user_rule", rule_intent=category — see 002 §4's migration 0005 for the equivalent bulk conversion
    pub async fn approve_term(&self, id: i64) -> Result<()>;           // idempotent via BEGIN IMMEDIATE, flips status to 'active'
    pub async fn contract(&self, raw_text: &str) -> Result<Vec<ContractTerm>>;                                              // matches active rule-intent crystals' text against raw_text
    pub async fn validate(&self, translated: &str, raw: Option<&str>, source: Option<&str>) -> Result<Vec<ValidationFinding>>;  // reports forbidden/missing canonical renderings
}

pub struct ContractTerm { pub crystal_id: i64, pub source_text: String, pub canonical_translation: String }
pub struct ValidationFinding { pub crystal_id: i64, pub kind: String, pub detail: String }  // kind: "forbidden_variant_used" | "canonical_missing"

pub struct ParsedRule { pub source_text: String, pub canonical: String, pub forbidden: Vec<String> }
pub fn parse_rule(text: &str) -> Option<ParsedRule>;  // regex: "X is translated as Y[, not Z]" — for free-text agent-authored rules; NOT used for strict_terms migration (002 §4), which maps structured fields directly
```

### 2.5 `FeedbackStore` (scoring)

```rust
pub struct FeedbackEvent { pub crystal_id: i64, pub event_type: String, pub source_role: String, pub evidence: Option<String>, pub session_id: Option<i64> }

pub struct FeedbackStore<'a> { pool: &'a SqlitePool }
impl<'a> FeedbackStore<'a> {
    pub async fn record(&self, event: FeedbackEvent) -> Result<i64>;  // single canonical scoring authority
    pub async fn record_recall_outcome(&self, session_id: i64, useful: &[i64], miss: &[i64]) -> Result<()>;  // see §3
}

pub struct ScoreDelta { pub strength: f64, pub confidence: f64 }

/// Canonical in hiero-core::values (002 §6), imported everywhere — not reimplemented per-store.
/// rule_intent (non-empty) dampens decay deltas by a flat factor (default 0.5), scaled further
/// by SOURCE_CREDIBILITY_CONFIDENCE[crystal.source_credibility] — a well-established rule fades
/// slower under disuse, but nothing is permanently exempt (no archive immunity — see the
/// reconsolidation design doc for why this replaces the old "rules never archive" rule).
pub fn apply_score_delta(crystal: &CrystalRecord, delta: ScoreDelta) -> (f64, f64, String);  // -> (new_strength, new_confidence, new_status)

pub static IMMEDIATE_EVENT_DELTAS: phf::Map<&str, (f64, f64)> = phf::phf_map! {
    "confirmed_by_user" => (0.15, 0.20),
    "contradicted_by_user" => (-0.20, -0.25),
    "deleted_by_user" => (-0.50, -0.35),
    "recalled_useful" => (0.06, 0.04),   // new — see §3, one notch below confirmed_by_user
    "recalled_miss" => (-0.05, -0.03),   // new — deliberately softer than contradicted_by_user
};
pub static PASSIVE_EVENT_DELTAS: phf::Map<&str, (f64, f64)> = phf::phf_map! {
    "cited" => (0.03, 0.02),
    "used_in_translation" => (0.05, 0.02),
    "passed_review" => (0.07, 0.05),
    "caused_correction" => (-0.10, -0.12),
    "superseded" => (-0.12, -0.05),
    "recalled_again" => (0.02, 0.0),     // new — see §3, strength-only: resurfacing implies nothing about correctness
};
```

---

## 3. Recall Service — Reconsolidation Model

Full behavioral spec:
`docs/superpowers/specs/2026-07-18-memory-reconsolidation-design.md`. Summary here for
implementability without cross-referencing that doc for every field.

```rust
pub struct RecallService<'a> { pool: &'a SqlitePool }
impl<'a> RecallService<'a> {
    pub async fn recall(&self, session_id: i64, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<RecallResult>>;
}

// Named constants (module-level, hiero-core::domain::recall) — not config fields, matching how
// _ACTIVE_RULE_BOOST is a hardcoded Python module constant today.
pub const RULE_INTENT_BOOST: f64 = 0.20;
pub const SPREADING_ACTIVATION_THRESHOLD: f64 = 0.55;
pub const SPREADING_ATTENUATION: f64 = 0.5;
```

For each crystal candidate from `CrystalStore::search_scored`, `recall()`:

1. **Score boost.** `score += SOURCE_CREDIBILITY_CONFIDENCE.get(&crystal.source_credibility).copied().unwrap_or(0.35)` (0.35 = `observation`'s weight, the schema's own default), then `score += RULE_INTENT_BOOST` if `!crystal.rule_intent.trim().is_empty()`. A high-credibility rule-intent crystal typically ranks first; a strong enough non-rule match can still outrank it.
2. **Working-copy dedup.** `WorkspaceStore::get_or_create_working_copy(session_id, &crystal)`. If it already existed (not newly created), insert a `memory_events` row (`event_type = "recalled_again"`).
3. **Activation logging.** Insert one `crystal_activations` row per candidate (`session_id, crystal_id, recall_query, rank, score, outcome = NULL`) — every candidate, not deduped, unlike step 2's working copy.
4. **Spreading activation.** For any candidate with `score > SPREADING_ACTIVATION_THRESHOLD`, call `CrystalStore::linked(crystal.id)` and fold neighbors into the result set at `linked_score = score * link_weight * SPREADING_ATTENUATION`. One hop only, one bounded query per triggering crystal.

Then RAG chunks (§4-5) are merged in via RRF (§5.4) as before. Active-rule-first ordering is
**not** an invariant — final ordering is purely score-based after step 1's boost.

`FeedbackStore::record_recall_outcome(session_id, useful, miss)`: applies
`IMMEDIATE_EVENT_DELTAS["recalled_useful"]`/`["recalled_miss"]` to each id via
`FeedbackStore::record`, and sets `outcome = 'useful' | 'miss'` on the matching
`crystal_activations` rows for that session — this is what 004 §4's `LinkReinforcer` phase reads.

---

## 4. RAG Pipeline

```rust
pub struct ImportOptions { pub source_type: Option<String> }
pub struct RagImportResult { pub source_id: i64, pub chunks_created: usize }
pub struct SearchOptions { pub limit: usize }
pub struct RagSearchResult { pub chunk: RagChunkRecord, pub score: f64, pub source: MemorySource }

pub struct RagStore<'a> { pool: &'a SqlitePool }
impl<'a> RagStore<'a> {
    pub async fn import_file(&self, series_slug: &str, path: &Path, opts: ImportOptions) -> Result<RagImportResult>;  // SQLite commit before LanceDB indexing — see §5.3
    pub async fn search(&self, series_slug: &str, query: &str, opts: SearchOptions) -> Result<Vec<RagSearchResult>>;  // dual-lane hybrid, see §5.4
}

pub const MAX_RAG_CHUNK_CHARS: usize = 1200;
pub struct ParsedRagChunk { pub chunk_kind: String, pub text: String, pub display_text: String, pub location: String }
pub struct ParsedRagFile { pub chunks: Vec<ParsedRagChunk> }

pub enum SourceType { Markdown, Text, Csv, Yaml, Json, Docx, Html, Pdf }
pub fn load_rag_file(path: &Path, source_type: SourceType) -> Result<ParsedRagFile>;  // pulldown-cmark, serde_json, serde_yaml, csv

pub struct NormalizedRagSource { pub text: String, pub source_type: SourceType }
pub fn normalize_rag_source(path: &Path, managed_root: &Path) -> Result<NormalizedRagSource>;  // DOCX (docx-rs), HTML (scraper), PDF (pdf-extract)
```

---

## 5. Semantic Retrieval (Local Vector Search)

SQLite stores all document metadata, chunks, and source texts; LanceDB is a rebuildable derived
index. Deleting `lancedb/` and re-running `IndexRebuild` (001 §4 `RagCommand::IndexRebuild`)
never loses data.

### 5.1 Embedding provider

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn provider(&self) -> &str;
    fn model(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}
```

Default implementation: `OrtEmbeddingProvider` using the `ort` crate, lazily loading
`paraphrase-multilingual-MiniLM-L12-v2` (384 dimensions, ~470MB — downloads on first `embed_*`
call, never during startup/config load/`hiero doctor`). Remote providers (OpenAI, Gemini,
Ollama) are optional overrides via `semantic.conf`.

### 5.2 Vector index

```rust
#[async_trait]
pub trait SemanticIndex: Send + Sync {
    async fn health(&self) -> Result<IndexHealth>;
    async fn active_generation(&self) -> Result<Option<GenerationInfo>>;
    async fn begin_rebuild(&self) -> Result<GenerationId>;
    async fn upsert_vectors(&self, gen: GenerationId, vectors: Vec<VectorRecord>) -> Result<usize>;
    async fn search(&self, query: &[f32], filter: &SearchFilter, limit: usize) -> Result<Vec<SearchHit>>;
    async fn delete_vectors(&self, chunk_ids: &[i64]) -> Result<usize>;
    async fn activate_generation(&self, gen: GenerationId) -> Result<()>;   // atomic pointer swap
    async fn cancel_generation(&self, gen: GenerationId) -> Result<()>;
    async fn close(&self) -> Result<()>;
}

pub struct IndexHealth { pub ok: bool, pub vector_count: usize, pub active_generation: Option<GenerationId> }
pub struct GenerationInfo { pub id: GenerationId, pub created_at: DateTime<Utc>, pub vector_count: usize }
pub struct VectorRecord { pub chunk_id: i64, pub embedding: Vec<f32> }
pub struct SearchFilter { pub series_slug: String }
pub struct SearchHit { pub chunk_id: i64, pub distance: f32 }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenerationId(pub uuid::Uuid);
```

Only implementation: `LanceDbIndex` (crate `lancedb`). Generations are isolated
tables/directories so a rebuild-in-progress never affects the currently-active index; the trait
is intentionally narrow enough that a `Qdrant` implementation could be added later without
touching call sites.

### 5.3 Job queue

```rust
pub struct SemanticJobQueue<'a> { pool: &'a SqlitePool }
impl<'a> SemanticJobQueue<'a> {
    pub async fn enqueue_rebuild(&self) -> Result<i64>;
    pub async fn claim_next_batch(&self, batch_size: usize) -> Result<Vec<RagChunkRecord>>;  // orders by chunk id, validates checksum against semantic_chunk_state before each batch
    pub async fn mark_indexed(&self, chunk_ids: &[i64], generation_id: &str) -> Result<()>;
    pub async fn mark_failed(&self, job_id: i64, error: &str) -> Result<()>;
}
```

Durable state lives in `semantic_index_jobs` / `semantic_chunk_state` (002 §2). The worker
never holds a SQLite write transaction while embedding or writing LanceDB.

### 5.4 Hybrid ranking

```rust
pub fn reciprocal_rank_fusion(
    fts_results: &[(i64, f64)],       // (chunk_id, bm25_score)
    vector_results: &[(i64, f32)],    // (chunk_id, cosine_distance)
    k: f64,                            // constant, default 60
) -> Vec<(i64, f64)>                   // (chunk_id, rrf_score)
```

Invariants: identical chunk IDs merge via RRF, not raw score comparison; missing lanes handled
gracefully (FTS-only or vector-only both work); stale generation/checksum vectors excluded from
search; LanceDB unavailability falls back to FTS5-only automatically.

---

## 6. Ingestion Service

```rust
pub struct LearnInput { pub text: String, pub source_role: String, pub source_ref: Option<String>, pub kind: String }
pub struct LearnResult { pub memory_ids: Vec<i64> }
pub struct ReadInput { pub text: String, pub source_ref: Option<String>, pub store_observation: bool }
pub struct ReadResult { pub candidate_terms: Vec<String>, pub findings: Vec<ValidationFinding> }
pub struct LearningBlock { pub text: String }

pub struct IngestionService<'a> { pool: &'a SqlitePool }
impl<'a> IngestionService<'a> {
    pub async fn learn(&self, session_id: i64, input: LearnInput) -> Result<LearnResult>;   // splits text into blocks, stores as short-term memories
    pub async fn read(&self, session_id: i64, input: ReadInput) -> Result<ReadResult>;       // extracts candidate terms, validates against Termbase; when store_observation is true, persists the read as a short-term memory (source_role = "observation") in addition to returning findings — this wiring did not exist in the original Python ReadInput either and is added here, not carried over as a gap
}

pub fn split_blocks(text: &str, max_chars: usize) -> Vec<LearningBlock>;
pub fn extract_terms(text: &str) -> Vec<String>;  // capitalized-term extraction regex
```

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

impl TranslationContext {
    pub fn new(series_slug: impl Into<String>, source_language: impl Into<String>, target_language: impl Into<String>) -> Self;
    pub fn with_metadata(self, language_tags: &[String], story_scopes: &[String], semantic_tags: &[String], tags: &[String]) -> Self;
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

`CrystalRecord` remains the raw schema-row contract defined by 002. Public crystal reads return an
enriched domain view so callers never issue side-table queries themselves:

```rust
pub struct Crystal {
    pub record: CrystalRecord,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub concept_ids: Vec<i64>,
}
```

`Crystal` dereferences to its `CrystalRecord` for field access. Store hydration batches each
side-table query over bounded ID chunks; query count is proportional to chunks, never records.

Sessions and short-term memories follow the same raw-row/enriched-view boundary. Public workspace
reads return `TaskSession { record, language_tags, story_scopes, semantic_tags }` and
`ShortTermMemory { record, metadata, language_tags, story_scopes, semantic_tags }`; both views
deref to their raw schema record. `metadata` is a validated JSON object. This makes every public
side-table field observable without leaking hydration queries to callers, and the dreaming
contracts in 004 consume `ShortTermMemory`, not raw `ShortTermMemoryRecord` rows.

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
    pub async fn get(&self, id: i64) -> Result<Crystal>;
    pub async fn list_rule_intent(&self, filter: RuleFilter) -> Result<Vec<Crystal>>;  // crystals with non-empty rule_intent, not crystal_type == 'rule'
    pub async fn archive(&self, id: i64) -> Result<CrystalRecord>;                             // general archive, any crystal_type — atomic, transaction-safe
    pub async fn supersede(&self, old_id: i64, new: AddCrystalInput) -> Result<CrystalRecord>;  // inserts new crystal with supersedes_crystal_id = old_id, flips old to 'superseded'
    pub async fn link(&self, source_id: i64, target_id: i64, link_type: &str) -> Result<CrystalLinkRecord>;  // crystal_links row, insert-or-ignore semantics
    pub async fn linked(&self, crystal_id: i64) -> Result<Vec<(CrystalLinkRecord, f64)>>;      // 1-hop neighbors with a derived link_weight, see §3
    pub async fn validate_rule(&self, id: i64) -> Result<ValidationReport>;
    pub async fn set_story_scopes(&self, id: i64, scopes: &[String]) -> Result<CrystalRecord>;
    pub async fn set_semantic_tags(&self, id: i64, tags: &[String]) -> Result<CrystalRecord>;
    pub async fn lowest_confidence(&self, ids: &[i64], limit: usize) -> Result<Vec<i64>>;
    pub async fn search(&self, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<Crystal>>;               // FTS5 via crystals_fts
    pub async fn search_scored(&self, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<(Crystal, f64)>>; // weighted BM25 + strength + confidence, before the recall-time boost in §3
    pub async fn search_expression(&self, ctx: &TranslationContext, expression: &str, limit: usize) -> Result<Vec<(Crystal, f64)>>; // intentional raw FTS5 boundary; parser errors are typed
}
pub fn search_expression(query: &str) -> String;  // tokenizes and quotes for FTS5, standalone fn

pub struct ValidationReport { pub ok: bool, pub findings: Vec<String> }
```

User-created crystals support exactly two coherent base scopes: global means empty
`scope_key`/`series_slug`; series means a canonical registry slug and
`scope_key == format!("series:{series_slug}")`. Reads exclude incoherent legacy rows from list and
search visibility. All read-before-write paths use SQLx tracked `BEGIN IMMEDIATE` transactions so
cancellation/drop rolls back before a pooled connection can be reused.

`link_type` values include at minimum `"related"` (general association, populated by
`LinkReinforcer` — §3.4) and any dreaming-assigned relation types; `linked()`'s derived
`link_weight` starts uniform (e.g. `1.0` per link) and is not yet a tunable strength column —
`crystal_links` has no weight column in the current schema (002 §2), so weight is derived from
link count/recency at query time rather than stored, unless a future migration adds one.
Recall's crate-private bounded linked-crystal read executes separate source-first and target-first
queries. Both apply status, coherent global/exact-series scope, and language visibility before
ordering/`LIMIT`; migration `0006` supplies the covering target-first index. The two bounded lanes
are merged by neighbor/link identity, then truncated deterministically, so invisible or opposite-
direction links cannot starve a visible neighbor.

### 2.2 `WorkspaceStore` (sessions + short-term memory)

```rust
pub struct AddMemoryInput {
    pub source_role: String, pub kind: String, pub text: String, pub source_ref: String,
    pub metadata: serde_json::Value,
    pub language_tags: Vec<String>, pub story_scopes: Vec<String>, pub semantic_tags: Vec<String>,
    pub source_credibility: String, pub rule_intent: String, pub soft_origin: Option<String>,
}
pub struct ShortMemoryLimits {
    pub warning_sentence_count: usize,   // default 6
    pub rejection_sentence_count: usize, // default 30
    pub warning_symbol_count: usize,     // default 0 = disabled
    pub rejection_symbol_count: usize,   // default 0 = disabled
}
pub struct AddMemoryResult { pub memory: ShortTermMemory, pub warnings: Vec<String> }

pub struct WorkspaceStore<'a> { pool: &'a SqlitePool, limits: ShortMemoryLimits }
impl<'a> WorkspaceStore<'a> {
    pub const fn new(pool: &'a SqlitePool) -> Self; // Python-compatible default limits
    pub fn with_limits(pool: &'a SqlitePool, limits: ShortMemoryLimits) -> Result<Self>;
    pub async fn start_session(&self, ctx: &TranslationContext, task_type: &str, volume: &str, chapter: &str) -> Result<TaskSession>;
    pub async fn get_session(&self, id: i64) -> Result<TaskSession>;
    pub async fn complete_session(&self, id: i64) -> Result<bool>;
    pub async fn complete_inactive(&self, cutoff: DateTime<Utc>) -> Result<Vec<i64>>;   // runs in 004's interval loop, not a separate thread
    pub async fn add_short_term(&self, session_id: i64, input: AddMemoryInput) -> Result<AddMemoryResult>;              // FTS trigger handles index (002 §3)
    pub async fn add_short_term_batch(&self, session_id: i64, items: &[AddMemoryInput]) -> Result<Vec<AddMemoryResult>>; // batch up to 500, input order
    pub async fn list_short_term(&self, session_id: i64) -> Result<Vec<ShortTermMemory>>;
    pub async fn search_short_term(&self, session_id: i64, query: &str, limit: usize) -> Result<Vec<ShortTermMemory>>;

    /// Reconsolidation working-copy dedup, called from RecallService (§3), not exposed as a
    /// standalone CLI/MCP operation. Looks up an existing row with source_crystal_id = crystal_id
    /// for this session; inserts one seeded from the crystal's text if absent.
    pub(crate) async fn get_or_create_working_copy(&self, session_id: i64, crystal: &Crystal) -> Result<(ShortTermMemory, bool)>;  // bool = was newly created
    pub async fn archive(&self, id: i64) -> Result<()>;  // sets archived_at; used by 004's Reconsolidator once a working copy is processed
}

/// Standalone fn, not a method — called from 004's background loop, not a dedicated thread.
pub async fn complete_stale_sessions(pool: &SqlitePool, cutoff: DateTime<Utc>) -> Result<Vec<i64>>;
```

Short-memory validation trims before counting, rejects empty text, counts grouped runs of
`[.!?。！？]+` plus any non-empty trailing fragment as sentences, and counts Unicode scalar values
(Rust `char`, Python `len`) as symbols. Rejection checks run sentence then symbol; warning checks run
in the same order. Warnings are both returned in `AddMemoryResult` and joined into
`metadata.validation_warning`. Task 8 loads/validates persisted config and passes these already-
defined limits to `with_limits`; it does not redefine validation behavior.

Those limits apply only to external `add_short_term` and `add_short_term_batch` inputs. RecallService
creates a trusted derived working copy from a crystal that has already passed crystal validation, so
that path bypasses short-memory size warnings and rejection. It still performs the same trimming,
normalization, metadata projection, sentence/symbol counting, and exact source-field persistence, but
does not fabricate `validation_warning`; otherwise a valid long-term crystal could become impossible
to recall solely because the short-memory input policy is stricter.

Every read-before-write workspace path uses a SQLx-tracked `BEGIN IMMEDIATE` transaction. Working
copy identity is the logical pair `(session_id, source_crystal_id)`: lookup and insert remain in
one immediate transaction. Tests prove serialization across independently constructed pools to the
same file; SQLite's process-level writer lock provides the same safety to cooperating processes,
though this task does not claim a separate process integration test. The Phase 002 schema
intentionally remains unchanged; direct SQL writes
outside `WorkspaceStore` are not part of this domain invariant. An existing row is reused only when
its persisted source-derived fields are exactly equivalent; otherwise the store returns a typed
conflict. Deleting a source crystal preserves the working copy and clears its marker through the
schema's `ON DELETE SET NULL` behavior.

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

pub struct Termbase<'a> { pool: &'a SqlitePool, context: TranslationContext }
impl<'a> Termbase<'a> {
    pub fn new(pool: &'a SqlitePool, context: TranslationContext) -> Self;
    pub async fn propose(&self, input: TermProposal) -> Result<i64>;   // writes a crystal_type="rule" crystal with source_credibility="user_rule", rule_intent=category — see 002 §4's migration 0005 for the equivalent bulk conversion
    pub async fn approve_term(&self, id: i64) -> Result<()>;           // candidate approval is atomic; active reapproval validates the existing graph without repairing/replacing it
    pub async fn contract(&self, raw_text: &str) -> Result<Vec<ContractTerm>>;
    pub async fn validate(&self, translated: &str, raw: Option<&str>, source: Option<&str>) -> Result<Vec<ValidationFinding>>;
}

pub struct ContractTerm { pub crystal_id: i64, pub source_text: String, pub canonical_translation: String }
pub struct ValidationFinding {
    pub crystal_id: i64,
    pub kind: String,
    pub severity: String,
    pub expected: String,
    pub observed: String,
    pub detail: String,
}

pub struct ParsedRule { pub source_text: String, pub canonical: String, pub forbidden: Vec<String> }
pub fn parse_rule(text: &str) -> Option<ParsedRule>;
```

`Termbase` is never context-free. Callers construct it with `TranslationContext::new(...)` and
optionally `with_metadata(...)`. Proposal and approval dimensions must exactly match that context.
Contract and validation load coherent global fallback rules plus the exact series and source/target
language direction; story-scoped rules require an intersecting context story scope. Multilingual
name/alias/former-label facets are source forms only under the Python-compatible source-vs-target
language rule. Ambiguous concepts are resolved only by matching story, semantic, or extra-language
metadata; unresolved ambiguity is reported rather than guessed.

`ValidationFinding` has `crystal_id` plus five diagnostic fields. Its complete stable contract is:

| `kind` | `severity` | `expected` / `observed` meaning |
|---|---|---|
| `ambiguous_source` | `warning` | sorted candidate renderings / exact observed source slice |
| `conflicting_active_rules` | `warning` | sorted conflicting renderings / exact observed source slice |
| `forbidden_variant_used` | `high` | canonical rendering / exact observed forbidden slice |
| `canonical_missing` | `medium` | canonical rendering / empty string |

Source-form deduplication, matching, ambiguity grouping, canonical recognition, and forbidden
variant recognition use full Unicode default case folding (non-Turkic), with no NFC/NFKC
normalization. Fold expansions are mapped back to original UTF-8 spans, so diagnostics preserve the
exact observed spelling. The parser grammar is the case-sensitive ASCII
`X is translated as Y[, not Z][.]`: it removes at most one optional ASCII full stop, then rejects
a target or forbidden rendering ending in any Unicode General Category Punctuation code point.
Internal punctuation remains valid. Structured `strict_terms` migration (002 §4) maps typed fields
directly and never calls this free-text parser.

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
/// rule_intent (non-empty) dampens each negative delta by the exact factor
/// `1.0 - 0.5 * SOURCE_CREDIBILITY_CONFIDENCE[crystal.source_credibility].clamp(0.0, 1.0)`;
/// unknown forward-compatible credibility labels use the observation fallback `0.35`. Higher
/// credibility therefore loses less, while the factor remains in `[0.5, 1.0]`, so nothing is
/// permanently exempt (no archive immunity — see the reconsolidation design doc for why this
/// replaces the old "rules never archive" rule). Positive deltas and crystals without rule
/// intent are unchanged.
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
pub struct ClaimedSemanticBatch { /* private claim token, job/generation binding, chunks */ }
pub struct SemanticLeaseGuard { /* token-scoped renewable lease and write fence */ }
impl<'a> SemanticJobQueue<'a> {
    pub async fn enqueue_rebuild(&self) -> Result<i64>;
    pub async fn claim_next_batch(&self, batch_size: usize) -> Result<ClaimedSemanticBatch>;
    pub async fn lease_guard(&self, batch: &ClaimedSemanticBatch) -> Result<SemanticLeaseGuard>;
    pub async fn mark_indexed(&self, batch: &ClaimedSemanticBatch, generation_id: &str) -> Result<usize>;
    pub async fn mark_failed(&self, batch: &ClaimedSemanticBatch, error: &str) -> Result<()>;
}
impl SemanticLeaseGuard {
    pub async fn ensure_owned(&self) -> Result<()>;
    pub async fn renew(&self) -> Result<()>;
    pub async fn run_with_lease<F, T>(&self, future: F) -> Result<T>;
    pub async fn fence_write<F, T>(&self, future: F) -> Result<T>;
}
```

Durable state lives in `semantic_index_jobs`, `semantic_batch_claims`, and
`semantic_chunk_state` (002 §2). Each claim has a unique ownership token, finite lease, and
persisted attempt count. A live lease cannot be stolen; an expired lease is atomically re-owned
with a new token. After three delivered attempts the job becomes terminal `failed` with
`last_error`/`failed_at` diagnostics. `mark_failed` is also terminal: it records the bounded
error and releases the job's claims. Late or expired-token success/failure is rejected. A job is
complete only when no stale/pending chunks and no live claims remain. Claim hydration is
database-only and stays inside the short `BEGIN IMMEDIATE` claim transaction, so decode errors or
cancellation roll back ownership; embedding and LanceDB work never run under that transaction.

The default lease is 15 minutes, deliberately longer than the provider's 600-second total download
timeout, while `SemanticLeaseGuard::run_with_lease` renews at one third of the lease so legitimate
download, embedding, and indexing work can continue without a fixed upper bound. Renewal succeeds
only while every row remains owned by the same token, its lease is live, and the job is `running`;
expired or replaced tokens receive a typed lost-ownership error. Dropping or cancelling the wrapper
stops renewal, and process death therefore leaves the claim to expire naturally without consuming an
attempt until another worker actually reclaims it. Worker orchestration must keep all external work
inside `run_with_lease` and must wrap every Lance upsert and activation in `fence_write`, which calls
`ensure_owned` immediately before and after the mutation. Task 7 exposes this mandatory safe API but
does not yet contain a concrete job-to-index worker loop.

The active generation's stored series/checksum metadata is compared directly with authoritative
SQLite rows during search. `semantic_chunk_state.generation_id` tracks rebuild progress only:
partially indexing generation B must not invalidate unchanged results still served from active
generation A. All active-pointer/table reads hold a shared lifecycle lock through the Lance query;
generation mutations hold the corresponding exclusive cross-process lock.

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
    pub async fn read(&self, session_id: i64, input: ReadInput) -> Result<ReadResult>;       // loads the session's complete TranslationContext, constructs Termbase::new(pool, context), extracts candidate terms, and validates; when store_observation is true, persists the read as a short-term memory (source_role = "observation") in addition to returning findings
}

pub fn split_blocks(text: &str, max_chars: usize) -> Vec<LearningBlock>;
pub fn extract_terms(text: &str) -> Vec<String>;  // capitalized-term extraction regex
```

`IngestionService::read` must derive the exact series, language direction, and metadata context from
the stored session before termbase validation. It must not construct a context-free termbase or
reconstruct a partial context from `ReadInput`.

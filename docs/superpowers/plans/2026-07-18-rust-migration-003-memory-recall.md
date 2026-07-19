# Rust Migration Phase 003 Memory and Recall Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port all memory stores, deterministic termbase, recall, RAG ingestion/search, semantic indexing, and hybrid ranking to `hiero-core`.

**Architecture:** Focused stores own SQL transactions; `RecallService` composes lexical, graph, and optional semantic evidence without letting semantic results override rule-intent crystals. SQLite owns source text and index jobs; LanceDB and ONNX state are replaceable adapters behind small traits.

**Tech Stack:** Rust 2024, SQLx/FTS5, Tokio, ICU4X `icu_casemap`, `unicode-properties`, LanceDB 0.30, ort 2.0.0-rc.12, Arrow, pulldown-cmark, docx-rs, pdf-extract, scraper.

## Global Constraints

- Port every public behavior covered by current crystal, workspace, concept, termbase, recall, RAG, and scoring tests.
- Active non-empty `rule_intent` crystals form a mandatory deterministic recall lane.
- SQLite is authoritative; deleting LanceDB/model caches loses no domain data.
- Semantic failure degrades to FTS5-only and is observable; it never fails recall.
- Use bounded channels and `spawn_blocking` for ONNX inference/document parsing that blocks or is CPU-heavy.
- All multi-row mutations use explicit write transactions; no N+1 tag/facet loading.

---

## File Map

- `crates/hiero-core/src/domain/`: models and focused stores for crystals, workspace, concepts, termbase, feedback.
- `crates/hiero-core/src/recall/`: candidate lanes, graph expansion, scoring, activation logging.
- `crates/hiero-core/src/rag/`: source import, parsing, chunking, conversion, FTS search.
- `crates/hiero-core/src/semantic/`: embedding/index traits, LanceDB/ORT adapters, jobs, RRF.
- `crates/hiero-core/src/ingest/`: validated ingestion configuration and orchestration.

**Focused commands:** Tasks 1-8 respectively use `cargo test -p hiero-core --test test_crystals`, `--test test_memory`, `--test test_concepts --test test_rule_crystals --test test_termbase`, `--test test_scoring`, `--test test_recall`, `--test test_rag`, `--test test_semantic`, and `--test test_ingest`. RED means the named contract fails for the absent behavior; GREEN means exit 0 with all named tests passed.

### Task 1: Port Domain Inputs and Crystal Store

**Files:** Create `domain/{mod.rs,models.rs,crystals.rs}`, `tests/test_crystals.rs`.

**Interfaces:** Produce `AddCrystalInput`, enriched `Crystal` reads over raw `CrystalRecord`, `CrystalStore<'a> { pool: &'a SqlitePool }`, `add`, `get`, `list_rule_intent`, `archive`, `supersede`, `link`, `linked`, `validate_rule`, `set_story_scopes`, `set_semantic_tags`, `lowest_confidence`, `search`, `search_scored`, and safe/plain plus intentional/raw `search_expression` boundaries exactly as proposal 003 §1/§2.1.

- [ ] Port representative tests from `tests/test_crystals.py`, `test_memory_search.py`, and rule-crystal tests, including invalid score/type/status, FTS updates/deletes, supersession, batched metadata, and rule-intent filtering.
- [ ] Run `cargo test -p hiero-core --test test_crystals`; expect RED.
- [ ] Implement bound queries and SQLx tracked immediate transactions; normalize/deduplicate tags in first-seen order before writes; batch public metadata hydration by bounded ID chunks; enforce coherent global/series scopes; clamp only at the explicit scoring boundary and reject invalid user inputs elsewhere.
- [ ] Run focused tests; expect GREEN. Commit `feat: port crystal domain store`.

### Task 2: Port Sessions and Short-Term Memory

**Files:** Create `domain/workspace.rs`, `tests/test_memory.rs`.

**Interfaces:** Produce the proposal's complete `AddMemoryInput`, `ShortMemoryLimits`, `AddMemoryResult`, enriched `TaskSession` and `ShortTermMemory` views over raw rows, default/configured `WorkspaceStore::{new,with_limits,start_session,get_session,complete_session,complete_inactive,add_short_term,add_short_term_batch,list_short_term,search_short_term,archive}`, and transaction-serialized working-copy creation keyed by `(session_id, source_crystal_id)`. External adds enforce configured size limits; trusted crystal-derived copies retain normalization and source projection while bypassing those external warnings/rejections.

- [ ] Port `test_workspace.py`, `test_short_memory.py`, `test_short_term_metadata.py`, session language/story/semantic tag behavior, batch atomicity, and working-copy deduplication.
- [ ] Run focused test; expect RED. Implement session and memory transactions with base-table-only writes; expect GREEN.
- [ ] Commit `feat: port workspace memory store`.

### Task 3: Port Concepts and Deterministic Termbase

**Files:** Create `domain/{concepts.rs,termbase.rs,rule_parser.rs}`, `tests/test_concepts.rs`, `tests/test_rule_crystals.rs`, `tests/test_termbase.rs`.

**Interfaces:** Produce all proposal 003 §2.3 concept/facet/link/merge/rename methods; `ParsedRule`; `parse_rule(&str) -> Option<ParsedRule>`; six-field `ValidationFinding`; and contextual `Termbase::new(&SqlitePool, TranslationContext)` plus `{propose,approve_term,contract,validate}` over rule-intent crystals only.

- [ ] Port all concept lifecycle/facet/multilingual tests and termbase contract/validation fixtures. Add ambiguous/conflicting source warnings, forbidden variant, concept-specific rule, archived/corrupt active reapproval, cross-series/language/story isolation, Unicode default-casefold expansion and non-normalization, and Unicode terminal-punctuation cases.
- [ ] Run three focused tests; expect RED.
- [ ] Implement concept mutations transactionally and free-text parsing as the exact 003 §2.4 pure grammar. Structured legacy conversion must not call this parser. Termbase contract queries only context-compatible active non-empty `rule_intent` crystals; every consumer must supply a complete `TranslationContext`.
- [ ] Run focused tests; expect GREEN. Commit `feat: port concepts and deterministic termbase`.

### Task 4: Centralize Feedback and Scoring

**Files:** Create `domain/feedback.rs`, `domain/scoring.rs`, `tests/test_scoring.rs`.

**Interfaces:** Produce `FeedbackEvent`, `ScoreDelta`, `FeedbackStore::{record,record_recall_outcome}`, and pure `apply_score_delta(&CrystalRecord, ScoreDelta) -> (f64, f64, String)`.

- [ ] Port immediate/passive delta, confidence floor/archive, rule-intent decay dampening, idempotent activation outcomes, and audit-event tests.
- [ ] Run focused test; expect RED. Implement one scoring path shared by future CLI/MCP/admin consumers; expect GREEN.
- [ ] Commit `feat: centralize memory feedback scoring`.

### Task 5: Implement Recall and Activation Logging

**Files:** Create `recall/{mod.rs,candidates.rs,graph.rs,ranking.rs}`, `tests/test_recall.rs`.

**Interfaces:** Produce `TranslationContext`, `MemorySource`, `RecallResult`, and `RecallService::recall(session_id: i64, ctx: &TranslationContext, query: &str, limit: usize) -> Result<Vec<RecallResult>>` from proposal 003 §3.

- [ ] Port combined recall/enriched memory/RAG tests. Assert rule lane inclusion before limit truncation, source-credibility boost, bounded graph expansion, stable tie-breaking, deduplication, and one activation row per returned crystal.
- [ ] Run focused test; expect RED.
- [ ] Implement lane queries concurrently only when independent, fuse in a synchronous pure ranker, then log returned activations transactionally. Never hold a lock/transaction across semantic inference.
- [ ] Run focused tests; expect GREEN. Commit `feat: port deterministic recall service`.

### Task 6: Port RAG Parsing, Import, and FTS Search

**Files:** Create `rag/{mod.rs,models.rs,parsing.rs,conversion.rs,chunking.rs,store.rs}`, `tests/test_rag.rs`, `tests/fixtures/rag/`.

**Interfaces:** Produce proposal 003 §4 `RagStore::{import_file,search}`, `ImportOptions`, `RagImportResult`, `SearchOptions`, `RagSearchResult`, stable checksum/idempotence, and typed parser/chunker inputs and outputs.

- [ ] Port every current RAG parsing, conversion, payload, schema, CLI-store behavior fixture, including Markdown, text, PDF, DOCX, unsupported/broken file, reimport, rollback, and chunk boundaries.
- [ ] Run focused test; expect RED.
- [ ] Keep parsing/chunking pure; dispatch blocking format parsers via `spawn_blocking`; persist source/chunks/tags in one transaction and let triggers own FTS.
- [ ] Run focused tests; expect GREEN. Commit `feat: port RAG ingestion and search`.

### Task 7: Add Rebuildable Semantic Retrieval

**Files:** Create `semantic/{mod.rs,embedding.rs,index.rs,lancedb.rs,jobs.rs,rrf.rs}`, `tests/test_semantic.rs`.

**Interfaces:** Produce `EmbeddingProvider`, `SemanticIndex`, `IndexHealth`, `GenerationId`, `GenerationInfo`, `VectorRecord`, `SearchFilter`, `SearchHit`, `SemanticJobQueue`, ownership-bearing `ClaimedSemanticBatch`, test-only `FakeEmbeddingProvider`/`FakeSemanticIndex`, and `reciprocal_rank_fusion` from proposal 003 §5. Queue completion/failure must accept a live leased batch claim rather than caller-supplied chunk/job IDs; expired claims are re-owned up to a bounded terminal-attempt limit with persisted diagnostics.

- [ ] Write contract tests that run the same insert/delete/search corpus against fake and temporary LanceDB indexes; test deterministic embeddings, generation swap, checksum skip, cancellation, missing model, corrupt index, and FTS fallback.
- [ ] Run focused test; expect RED.
- [ ] Implement bounded job claiming in SQLite, ORT inference behind `spawn_blocking`, LanceDB generation directories, and atomic active-generation switch only after complete indexing. Pin `ort` explicitly because it is a release candidate and record its Rust 1.88 floor under the workspace's 1.94 floor.
- [ ] Add hybrid RRF to recall with rule-lane preservation and stable ID tie-break. Run tests; expect GREEN. Commit `feat: add rebuildable semantic recall`.

### Task 8: Implement Ingestion Configuration and Phase Gate

**Files:** Create `ingest/{mod.rs,config.rs,service.rs}`, `tests/test_ingest.rs`; modify `lib.rs` exports.

**Interfaces:** Produce `IngestConfig::load/save/validate`, consuming Task 2's existing `ShortMemoryLimits` through `WorkspaceStore::with_limits`, plus `LearnInput`, `LearnResult`, `ReadInput`, `ReadResult`, `LearningBlock`, `IngestionService::{learn,read}`, `split_blocks`, and `extract_terms`. `read(session_id, ..)` must load the session's complete `TranslationContext` and call `Termbase::new(pool, context)`; a context-free or request-only partial termbase is forbidden.

- [ ] Port ingest config/default/invalid-value and agent-ingestion tests; expect RED.
- [ ] Implement atomic config persistence and orchestration without duplicating store logic; expect GREEN.
- [ ] Run all Phase 003 tests plus workspace clippy/doc. Commit `feat: complete Rust memory and recall core`.

## Phase Acceptance

- [ ] All eight Rust integration tests pass with no network/model download.
- [ ] Deleting the temporary LanceDB directory and rerunning rebuild reproduces semantic results from SQLite.
- [ ] Termbase and recall tests prove semantic evidence cannot displace an active rule-intent result.
- [ ] Every Python parity file named in proposal 006 §1.2 maps to at least one Rust assertion.

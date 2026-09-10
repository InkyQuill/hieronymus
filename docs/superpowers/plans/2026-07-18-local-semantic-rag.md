# Local Semantic RAG Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`, `superpowers:test-driven-development`, and `superpowers:verification-before-completion`. Keep all tests offline by injecting fake embedders; never download a real model in CI.

**Goal:** Add multilingual local semantic retrieval and deterministic hybrid ranking while preserving SQLite as the authoritative corpus and active rule crystals as mandatory constraints.

**Architecture:** SQLite stores sources/chunks/metadata/FTS and semantic-index state. A rebuildable LanceDB generation stores vectors keyed by current chunk checksum. FastEmbed provides lazy local embeddings by default. `SemanticIndex` and `EmbeddingProvider` protocols isolate storage and model choices.

**Tech stack:** Python 3.12, SQLite/FTS5, LanceDB 0.34.x, FastEmbed 0.8.x, pytest, Svelte 5/Vitest.

**Design:** `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`, Plan 4.

**Depends on:** Complete Plans 1-3, including versioned migrations and ASGI lifecycle.

**Worktree safety:** Model caches and LanceDB data are runtime artifacts and must never be added to Git. Tests use temporary roots and fake vectors.

---

## Task 1: Add Semantic Configuration And Embedding Provider Boundary

**Files:**

- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Create: `src/hieronymus/semantic_config.py`
- Create: `src/hieronymus/embeddings.py`
- Create: `tests/test_semantic_config.py`
- Create: `tests/test_embeddings.py`
- Modify: `src/hieronymus/config.py`

- [ ] **Step 1: Add bounded dependencies**

  Add `lancedb>=0.34,<0.35` and `fastembed>=0.8,<0.9`, then regenerate the lockfile. Confirm supported Python/platform wheels for the project's CI matrix before accepting the lock delta.

- [ ] **Step 2: Write configuration contracts**

  Define defaults: semantic enabled, provider `local`, model `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`, bounded batch size, candidate multiplier, and fusion constant. Cover config file, environment overrides, validation, and disabled/FTS-only mode.

- [ ] **Step 3: Write provider contracts with a fake**

  `EmbeddingProvider` exposes identity (provider/model/revision/dimensions), bounded document embedding, and query embedding. Assert stable vector dimensions, empty-input handling, batch bounds, and typed failures.

- [ ] **Step 4: Verify RED**

  Run: `uv run pytest tests/test_semantic_config.py tests/test_embeddings.py -q`

- [ ] **Step 5: Implement lazy FastEmbed loading**

  Construct the real model only on first embedding request. Never load/download during import, package installation, config parsing, daemon startup, status, or FTS-only search. Provide a progress callback boundary for CLI/web callers. Cache the instantiated model per provider object, not globally across incompatible configs.

- [ ] **Step 6: Add optional remote-provider registration**

  Define the registry/config seam only. Implement remote providers only if an existing official SDK in the project exposes a stable embeddings API with clear tests; otherwise return a precise unsupported-provider error and leave the protocol ready. Local remains the default.

- [ ] **Step 7: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_semantic_config.py tests/test_embeddings.py -q
  uv run ruff check src/hieronymus/semantic_config.py src/hieronymus/embeddings.py tests/test_semantic_config.py tests/test_embeddings.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add pyproject.toml uv.lock src/hieronymus/config.py src/hieronymus/semantic_config.py src/hieronymus/embeddings.py tests/test_semantic_config.py tests/test_embeddings.py
  git commit -m "feat: add local embedding provider boundary"
  ```

## Task 2: Implement A Rebuildable LanceDB Semantic Index

**Files:**

- Create: `src/hieronymus/semantic_index.py`
- Create: `src/hieronymus/lance_index.py`
- Create: `tests/test_semantic_index_contract.py`
- Create: `tests/test_lance_index.py`
- Modify: `.gitignore`

- [ ] **Step 1: Define the storage protocol and records**

  `SemanticIndex` supports health, active generation, begin rebuild, bounded upsert, delete chunk IDs, search with filters, activate generation, cancel/discard generation, and close. Search results include chunk ID, distance, generation, and index identity.

- [ ] **Step 2: Write a reusable contract suite**

  Run the same behavior assertions against an in-memory fake and a temporary LanceDB implementation: idempotent upsert, replacement, deletion, filters, generation isolation, cancellation, corrupt/missing directory, and deterministic ordering for equal distances.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_semantic_index_contract.py tests/test_lance_index.py -q`

- [ ] **Step 4: Implement generation directories/tables**

  Store the active-generation pointer in SQLite later; the Lance adapter must accept an explicit generation and never infer currentness from directory names. Rows include `chunk_id`, `series_slug`, language/story/semantic scopes, checksum, provider/model/revision/dimensions, and vector.

- [ ] **Step 5: Make activation atomic**

  A generation is searchable only after all expected rows are present and its manifest checksum/count is verified. Activation updates one authoritative pointer transactionally. Old generations may be garbage-collected only after no search can select them.

- [ ] **Step 6: Protect the repository**

  Ignore runtime Lance/model-cache directories by precise project-relative patterns without ignoring fixtures or source.

- [ ] **Step 7: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_semantic_index_contract.py tests/test_lance_index.py -q
  uv run ruff check src/hieronymus/semantic_index.py src/hieronymus/lance_index.py tests/test_semantic_index_contract.py tests/test_lance_index.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add .gitignore src/hieronymus/semantic_index.py src/hieronymus/lance_index.py tests/test_semantic_index_contract.py tests/test_lance_index.py
  git commit -m "feat: add rebuildable LanceDB index"
  ```

## Task 3: Track Index State And Queue Bounded Work

**Files:**

- Modify: `src/hieronymus/migrations/global.sql`
- Create: `src/hieronymus/migrations/versions/0004_semantic_index_state.sql`
- Create: `src/hieronymus/semantic_jobs.py`
- Create: `tests/test_semantic_jobs.py`
- Modify: `src/hieronymus/rag_store.py`
- Modify: `tests/test_rag_store.py`
- Modify: `src/hieronymus/service_app.py`
- Modify: `src/hieronymus/service_daemon.py`
- Modify: `tests/test_service_daemon.py`

- [ ] **Step 1: Write schema/state tests**

  Track active generation, desired index identity, job status, expected/processed counts, error, cancellation request, created/updated timestamps, and per-chunk indexed checksum. Ensure job rows survive daemon restart.

- [ ] **Step 2: Write authoritative-first import tests**

  Inject embedding/index failure after RAG import. Assert SQLite source/chunks/FTS commit successfully, a retryable failed job is visible, and lexical search works. Source replacement/deletion must enqueue removal of obsolete vectors.

- [ ] **Step 3: Write bounded worker tests**

  Cover batch size, restart/resume, cancellation between batches, model identity change, stale checksum exclusion, source deletion, generation activation, and failure redaction.

- [ ] **Step 4: Verify RED**

  Run: `uv run pytest tests/test_semantic_jobs.py tests/test_rag_store.py tests/test_service_daemon.py -q`

- [ ] **Step 5: Implement the durable job store**

  Keep job metadata in SQLite. Select pending chunks in bounded ID order. Recheck checksum immediately before writing each batch. Do not hold an SQLite write transaction while embedding or writing LanceDB.

- [ ] **Step 6: Attach the worker to daemon lifespan**

  Start one worker after schema readiness and stop it through the shared bounded shutdown coordinator. Wake it through an event when imports enqueue work; use a low-frequency fallback wait, not busy polling.

- [ ] **Step 7: Implement safe degradation**

  Worker/model/index errors update status and logs but do not make the daemon or lexical RAG unavailable.

- [ ] **Step 8: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_semantic_jobs.py tests/test_rag_store.py tests/test_service_daemon.py -q
  uv run ruff check src/hieronymus/semantic_jobs.py src/hieronymus/rag_store.py src/hieronymus/service_daemon.py
  ```

- [ ] **Step 9: Commit**

  ```bash
  git add src/hieronymus/migrations/global.sql src/hieronymus/migrations/versions/0004_semantic_index_state.sql src/hieronymus/semantic_jobs.py src/hieronymus/rag_store.py src/hieronymus/service_app.py src/hieronymus/service_daemon.py tests/test_semantic_jobs.py tests/test_rag_store.py tests/test_service_daemon.py
  git commit -m "feat: queue semantic indexing jobs"
  ```

## Task 4: Add Deterministic Hybrid Retrieval

**Files:**

- Create: `src/hieronymus/hybrid_ranking.py`
- Create: `tests/test_hybrid_ranking.py`
- Modify: `src/hieronymus/rag_models.py`
- Modify: `src/hieronymus/rag_store.py`
- Modify: `src/hieronymus/recall.py`
- Modify: `src/hieronymus/rag_payloads.py`
- Modify: `tests/test_rag_store.py`
- Modify: `tests/test_recall_rag.py`
- Modify: `tests/test_memory_search.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Write pure rank-fusion tests**

  Cover lexical-only, vector-only, overlap, ties, missing lanes, expanded candidate budgets, deterministic chunk-ID tiebreaking, and reciprocal-rank fusion contributions. Do not normalize or compare raw BM25 and cosine values.

- [ ] **Step 2: Add multilingual retrieval fixtures**

  Use deterministic fake vectors for paraphrases and cross-language equivalents that FTS cannot match. Include exact terminology/glossary cases where lexical retrieval must remain strong.

- [ ] **Step 3: Add rule-precedence regression**

  Create semantic evidence conflicting with an active rule crystal. Assert recall returns the rule in its protected mandatory lane and semantic RAG cannot replace or outrank that constraint.

- [ ] **Step 4: Verify RED**

  Run: `uv run pytest tests/test_hybrid_ranking.py tests/test_rag_store.py tests/test_recall_rag.py tests/test_memory_search.py -q`

- [ ] **Step 5: Implement dual-lane candidate retrieval**

  Query FTS and the active semantic generation independently with the same series/language/story/semantic filters and expanded limits. Reject vector hits whose current SQLite checksum or index identity no longer matches.

- [ ] **Step 6: Fuse then boost**

  Apply RRF first, then existing glossary and metadata boosts, then truncate. Preserve stable behavior for semantic-disabled and unavailable-index modes.

- [ ] **Step 7: Extend explanations compatibly**

  Add optional lexical rank, vector rank, fusion contribution, model identity, index generation, and fallback reason fields without removing existing payload fields.

- [ ] **Step 8: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_hybrid_ranking.py tests/test_rag_store.py tests/test_recall_rag.py tests/test_memory_search.py tests/test_mcp_server.py -q
  uv run ruff check src/hieronymus/hybrid_ranking.py src/hieronymus/rag_store.py src/hieronymus/recall.py
  ```

- [ ] **Step 9: Commit**

  ```bash
  git add src/hieronymus/hybrid_ranking.py src/hieronymus/rag_models.py src/hieronymus/rag_store.py src/hieronymus/recall.py src/hieronymus/rag_payloads.py tests/test_hybrid_ranking.py tests/test_rag_store.py tests/test_recall_rag.py tests/test_memory_search.py tests/test_mcp_server.py
  git commit -m "feat: add hybrid semantic RAG ranking"
  ```

## Task 5: Add CLI, Status, And Doctor Controls

**Files:**

- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/doctor.py`
- Modify: `src/hieronymus/service_app.py`
- Modify: `src/hieronymus/mcp_server.py` or `src/hieronymus/mcp_tools.py`
- Modify: `tests/test_rag_cli.py`
- Modify: `tests/test_doctor.py`
- Modify: `tests/test_service_app.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Write command/status contracts**

  Add `hiero rag index status`, `hiero rag index rebuild`, and `hiero rag index cancel`, each with human and JSON output. Define exit behavior for healthy, pending, degraded, failed, and disabled states.

- [ ] **Step 2: Add no-download status tests**

  Patch FastEmbed construction to fail if called. Status, doctor, ordinary daemon startup, and FTS-only search must not instantiate/download the model.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_rag_cli.py tests/test_doctor.py tests/test_service_app.py tests/test_mcp_server.py -q`

- [ ] **Step 4: Implement controls through one service**

  CLI, HTTP status, doctor, and MCP diagnostics call the same semantic-index status service. Rebuild creates a new generation job; cancel requests bounded worker cancellation. Expose errors without leaking provider credentials.

- [ ] **Step 5: Provide visible lazy-download progress**

  Foreground rebuild can stream/log model download and batch progress. Background daemon jobs publish console events. Never write progress to stdio MCP stdout.

- [ ] **Step 6: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_rag_cli.py tests/test_doctor.py tests/test_service_app.py tests/test_mcp_server.py -q
  uv run ruff check src/hieronymus/cli.py src/hieronymus/doctor.py src/hieronymus/service_app.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/cli.py src/hieronymus/doctor.py src/hieronymus/service_app.py src/hieronymus/mcp_server.py src/hieronymus/mcp_tools.py tests/test_rag_cli.py tests/test_doctor.py tests/test_service_app.py tests/test_mcp_server.py
  git commit -m "feat: manage semantic RAG indexing"
  ```

## Task 6: Add Minimal Web Index Management

**Files:**

- Create: `frontend/src/web/components/RagIndexEditor.svelte`
- Create: `frontend/src/web/components/RagIndexEditor.test.ts`
- Modify: `frontend/src/web/components/IngestEditor.svelte`
- Modify: `frontend/src/web/lib/api.ts`
- Modify: `frontend/src/web/lib/types.ts`
- Modify: `src/hieronymus/service_app.py`
- Modify: `tests/test_service_app.py`

- [ ] **Step 1: Write rendered UI contracts**

  Cover healthy/pending/degraded/failed/disabled states, progress counts, rebuild confirmation, cancel, lazy-model warning/download progress, and FTS fallback messaging. Avoid source-string assertions for behavior.

- [ ] **Step 2: Write HTTP endpoint contracts**

  Add bounded status, rebuild, and cancel routes under the existing ingest/settings API family. Rebuild is idempotent while an equivalent job is active.

- [ ] **Step 3: Verify RED**

  Run:

  ```bash
  uv run pytest tests/test_service_app.py -k semantic -q
  bun run --cwd frontend test -- RagIndexEditor.test.ts
  ```

- [ ] **Step 4: Implement the panel**

  Mount it in the ingest configuration experience rather than adding a new top-level navigation area. Use existing semantic tokens, 44px targets, error treatment, timeout-aware API helper, and toast conventions.

- [ ] **Step 5: Verify GREEN and accessibility**

  Run:

  ```bash
  uv run pytest tests/test_service_app.py -k semantic -q
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/web/components/RagIndexEditor.svelte frontend/src/web/components/RagIndexEditor.test.ts frontend/src/web/components/IngestEditor.svelte frontend/src/web/lib/api.ts frontend/src/web/lib/types.ts src/hieronymus/service_app.py tests/test_service_app.py
  git commit -m "feat: manage semantic index in the console"
  ```

## Task 7: Semantic RAG Verification And Evaluation

**Files:**

- Create: `tests/fixtures/rag-semantic-evaluation.json`
- Create: `tests/test_semantic_evaluation.py`
- Modify only in-scope production files if evaluation reveals a defect.

- [ ] **Step 1: Add a small deterministic evaluation corpus**

  Include English/Russian and at least one additional project-relevant language pair, paraphrases, named entities, exact terminology, distractors, scope filters, and conflicting rule evidence. Use fake deterministic vectors in CI.

- [ ] **Step 2: Assert quality invariants, not benchmark theater**

  Require hybrid recall to recover designated semantic-only hits, preserve exact lexical hits, obey filters, remain deterministic, and preserve rule precedence. Do not encode fragile global score thresholds.

- [ ] **Step 3: Run full verification**

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  uv build
  ```

- [ ] **Step 4: Test installed offline fallback**

  In an isolated wheel install with network/model cache unavailable, import a RAG source and perform search. Confirm FTS works, status explains the missing semantic model, and startup does not attempt a download.

- [ ] **Step 5: Test rebuild recovery**

  Interrupt a disposable real LanceDB rebuild, restart, and verify the old generation remains active until the new one completes. Delete/corrupt the derived index and verify clean rebuild from SQLite.

- [ ] **Step 6: Commit evaluation fixtures/tests**

  ```bash
  git add tests/fixtures/rag-semantic-evaluation.json tests/test_semantic_evaluation.py
  git commit -m "test: verify semantic RAG quality invariants"
  ```

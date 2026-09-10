# Correctness And Data Integrity Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task by task. Use `superpowers:test-driven-development` for each behavior change and `superpowers:verification-before-completion` before every completion claim.

**Goal:** Repair the confirmed scoring, proposal, FTS, value-normalization, installer, and frontend request defects without changing unrelated public contracts.

**Architecture:** Domain services remain synchronous and SQLite-backed. Shared value rules move to one utility module, `FeedbackStore` becomes the sole scoring authority, proposal approval acquires its write transaction before resolving concepts, and SQLite triggers become the sole FTS synchronization mechanism.

**Tech stack:** Python 3.12, SQLite/FTS5, pytest, Svelte 5, TypeScript, Vitest, Bun.

**Design:** `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`, Plan 1.

**Depends on:** Nothing. Execute this plan first.

**Worktree safety:** Preserve the user's `uv.lock`, `report.md`, `improvements.md`, and unrelated untracked files. Stage only files named by the current task.

---

## Task 1: Centralize Canonical Value Helpers

**Files:**

- Create: `src/hieronymus/values.py`
- Create: `tests/test_values.py`
- Modify: `src/hieronymus/workspace.py`
- Modify: `src/hieronymus/scoring.py`
- Modify: `src/hieronymus/recall.py`
- Modify: `src/hieronymus/rag_store.py`
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/dream_audit.py`
- Modify: `src/hieronymus/crystals.py`
- Modify: `src/hieronymus/concepts.py`
- Modify: `src/hieronymus/registry.py`
- Modify: `src/hieronymus/termbase.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/memory_models.py`
- Test: existing focused modules importing those helpers

- [ ] **Step 1: Write failing helper contracts**

  Cover `utc_now()` ending in `Z`, `clamp_score()` bounds, stable order plus case-sensitive deduplication for normalized tuples, and `json_object()` rejecting arrays/scalars with `ValueError`.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_values.py -q`

  Expected: import failure because `hieronymus.values` does not exist.

- [ ] **Step 3: Implement the minimal shared helpers**

  Keep the module dependency-free. Accept an injectable `datetime` only if tests require it; do not add a clock abstraction.

- [ ] **Step 4: Replace duplicated helpers incrementally**

  Replace only equivalent implementations. Preserve specialized database-time behavior in `memory_migration.py` until its versioned migration work in Plan 3. Normalize all newly written timestamps to UTC `Z`; continue parsing historical `+00:00` rows.

- [ ] **Step 5: Remove the redundant admin catch-all**

  In `_dream_audit_detail`, retain the expected JSON decoding fallback and remove the unnecessary broader branch. Do not remove broad catches that intentionally isolate daemon loops, RPC boundaries, or provider calls.

- [ ] **Step 6: Verify GREEN and affected suites**

  Run:

  ```bash
  uv run pytest tests/test_values.py tests/test_workspace.py tests/test_scoring.py tests/test_recall.py tests/test_rag_store.py tests/test_admin_store.py -q
  uv run ruff check src/hieronymus/values.py tests/test_values.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/values.py tests/test_values.py src/hieronymus/workspace.py src/hieronymus/scoring.py src/hieronymus/recall.py src/hieronymus/rag_store.py src/hieronymus/dreaming.py src/hieronymus/dream_audit.py src/hieronymus/crystals.py src/hieronymus/concepts.py src/hieronymus/registry.py src/hieronymus/termbase.py src/hieronymus/admin.py src/hieronymus/memory_models.py
  git commit -m "refactor: centralize canonical value helpers"
  ```

## Task 2: Make Feedback Scoring Canonical

**Files:**

- Modify: `src/hieronymus/scoring.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `tests/test_scoring.py`
- Modify: `tests/test_admin_actions.py`

- [ ] **Step 1: Write failing parity regressions**

  Add parameterized tests proving direct `FeedbackStore.record()` and each admin feedback action produce identical strength, confidence, status, event deltas, and applied state. Include confidence reaching exactly zero, active-rule immunity, and delete-threshold archival.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_scoring.py tests/test_admin_actions.py -q`

  Expected: the admin confidence-zero case diverges.

- [ ] **Step 3: Add a transaction-compatible scoring operation**

  Let `FeedbackStore` accept an existing SQLite connection for the internal operation while preserving the public convenience method that opens and commits its own connection. Keep event creation and crystal mutation in the caller's transaction.

- [ ] **Step 4: Delegate admin feedback**

  Replace `_record_immediate_feedback` arithmetic with the canonical scoring operation. Remove admin-local delta, threshold, and clamp constants that are no longer needed.

- [ ] **Step 5: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_scoring.py tests/test_admin_actions.py tests/test_tui_bridge_admin.py -q
  uv run ruff check src/hieronymus/scoring.py src/hieronymus/admin.py
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/hieronymus/scoring.py src/hieronymus/admin.py tests/test_scoring.py tests/test_admin_actions.py
  git commit -m "fix: unify crystal feedback scoring"
  ```

## Task 3: Make Proposal Approval Atomic And Race-Safe

**Files:**

- Modify: `src/hieronymus/admin.py`
- Modify: `tests/test_admin_actions.py`
- Modify: `tests/test_admin_store.py`

- [ ] **Step 1: Add malformed-payload rollback tests**

  Insert pending proposals with malformed approved/forbidden JSON. Assert approval raises a clear `ValueError` and leaves concepts, facets, tags, proposal status, migration ledger, and audit rows unchanged.

- [ ] **Step 2: Add a concurrent-approval regression**

  Use two real SQLite connections and a synchronization barrier around approval. Assert only one approval transition occurs and no duplicate concept/facet graph is created.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_admin_actions.py tests/test_admin_store.py -q`

- [ ] **Step 4: Validate before mutation**

  Parse both variant arrays before opening the write phase. Require arrays of nonblank strings; normalize once and pass typed tuples into the approval helpers instead of reparsing stored JSON after writes begin.

- [ ] **Step 5: Acquire the write transaction before resolution**

  Execute `BEGIN IMMEDIATE`, reread the proposal in that transaction, verify it is pending, then resolve/create the concept, facets, tags, status, and audit row. Roll back on every exception. Do not add a uniqueness constraint that would incorrectly forbid deliberately duplicated concept names with different semantic tags.

- [ ] **Step 6: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_admin_actions.py tests/test_admin_store.py -q
  uv run ruff check src/hieronymus/admin.py tests/test_admin_actions.py tests/test_admin_store.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/admin.py tests/test_admin_actions.py tests/test_admin_store.py
  git commit -m "fix: make proposal approval atomic"
  ```

## Task 4: Make SQLite Triggers Own Memory FTS

**Files:**

- Modify: `src/hieronymus/migrations/global.sql`
- Modify: `src/hieronymus/db.py`
- Modify: `src/hieronymus/workspace.py`
- Modify: `src/hieronymus/crystals.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/memory_migration.py`
- Modify: `tests/test_config.py`
- Modify: `tests/test_workspace.py`
- Modify: `tests/test_crystals.py`
- Modify: `tests/test_admin_actions.py`
- Modify: `tests/test_memory_graph_migration.py`

- [ ] **Step 1: Write trigger and drift regressions**

  Cover raw insert/update/delete, application mutations, session cascade deletion, crystal update/delete, and external-content FTS query results. Assert every content row has exactly one searchable FTS row and deleted rows produce no hit.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_config.py tests/test_workspace.py tests/test_crystals.py tests/test_admin_actions.py -q`

- [ ] **Step 3: Add idempotent triggers and one-time repair**

  Add `_ai`, `_ad`, and `_au` triggers for both content tables. During global compatibility, detect missing/drifted indexes and issue the FTS5 `rebuild` command before trigger-owned behavior is assumed.

- [ ] **Step 4: Remove every manual write**

  Use `rg` to find insert/delete operations targeting `short_term_memories_fts` and `crystals_fts`. Remove application-owned synchronization but retain FTS query SQL and migration rebuild commands.

  Run: `rg -n "insert into (short_term_memories_fts|crystals_fts)|insert into (short_term_memories_fts|crystals_fts)\(" src/hieronymus`

  Expected: only migration/repair SQL remains.

- [ ] **Step 5: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_config.py tests/test_workspace.py tests/test_crystals.py tests/test_admin_actions.py tests/test_memory_graph_migration.py -q
  uv run ruff check src/hieronymus tests
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/hieronymus/migrations/global.sql src/hieronymus/db.py src/hieronymus/workspace.py src/hieronymus/crystals.py src/hieronymus/admin.py src/hieronymus/dreaming.py src/hieronymus/memory_migration.py tests/test_config.py tests/test_workspace.py tests/test_crystals.py tests/test_admin_actions.py tests/test_memory_graph_migration.py
  git commit -m "fix: make triggers own memory FTS indexes"
  ```

## Task 5: Correct Installer TTY Detection

**Files:**

- Modify: `install.sh`
- Modify: `tests/test_release_scripts.py`

- [ ] **Step 1: Add redirected-stdio regressions**

  Cover an accessible `/dev/tty` with redirected stdin or stdout. Assert noninteractive defaults are used and no prompt read occurs. Retain the positive interactive path.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_release_scripts.py -k tty -q`

- [ ] **Step 3: Tighten `has_tty()`**

  Require `[ -t 0 ] && [ -t 1 ]` before the existing `/dev/tty` readability/writability probes.

- [ ] **Step 4: Verify GREEN and shell syntax**

  Run:

  ```bash
  sh -n install.sh
  uv run pytest tests/test_release_scripts.py -q
  ```

- [ ] **Step 5: Commit**

  ```bash
  git add install.sh tests/test_release_scripts.py
  git commit -m "fix: detect interactive installer terminals"
  ```

## Task 6: Restore Memory Removal And Bound Frontend Requests

**Files:**

- Modify: `frontend/src/web/components/MemoryViews.svelte`
- Modify: `frontend/src/web/components/MemoryViews.test.ts`
- Modify: `frontend/src/web/lib/api.ts`
- Create: `frontend/src/web/lib/api.test.ts`

- [ ] **Step 1: Add a rendered removal-action test**

  Render the Short-Term Memory view with one selected memory. Assert Remove appears, requires confirmation, calls `remove_short_term_memory`, and refreshes the view after success.

- [ ] **Step 2: Add API timeout tests**

  Use fake timers and a mocked never-settling fetch. Assert the request aborts at the shared deadline, reports a stable timeout message, and still honors a caller-provided abort signal.

- [ ] **Step 3: Verify RED**

  Run: `bun run --cwd frontend test -- MemoryViews.test.ts api.test.ts`

- [ ] **Step 4: Expose the action and implement composed cancellation**

  Add the action only to the `Short-Term Memory` view. In the API helper, create a timeout controller and compose it with `init.signal` without leaking listeners or timers. Preserve existing request options and error parsing.

- [ ] **Step 5: Verify GREEN and frontend quality**

  Run:

  ```bash
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/web/components/MemoryViews.svelte frontend/src/web/components/MemoryViews.test.ts frontend/src/web/lib/api.ts frontend/src/web/lib/api.test.ts
  git commit -m "fix: restore memory removal and request timeouts"
  ```

## Task 7: Plan-Level Verification

**Files:**

- Modify only if failures expose an in-scope regression.

- [ ] **Step 1: Run required verification**

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  ```

- [ ] **Step 2: Recheck finding-specific invariants**

  Confirm no duplicate canonical value helpers remain, no manual memory FTS writes remain, the removal action is mapped to the correct view, and `git diff --check` passes.

- [ ] **Step 3: Record durable documentation changes**

  Do not delete this plan yet. Add any implementation facts needed by Plan 5's traceability pass to its future review-disposition document or commit notes.

- [ ] **Step 4: Final commit only if verification required fixes**

  Use a narrow conventional commit naming the verified correction. Do not create an empty commit.

# Memory Schema And Internal Boundaries Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`, `superpowers:test-driven-development`, and `superpowers:verification-before-completion`. Treat legacy-database migration as destructive-risk work: create and verify backups before dropping tables.

**Goal:** Replace startup compatibility rewrites and legacy strict-term storage with versioned, lossless migrations; align internal names with current responsibilities; and split/bound dreaming internals without changing memory semantics.

**Architecture:** `global.sql` defines a fresh schema and ordered migrations upgrade existing databases. Rule crystals become the only terminology authority. Console APIs move out of the obsolete `tui_bridge` namespace. `DreamService` remains the facade over focused collaborators.

**Tech stack:** Python 3.12, SQLite/FTS5, pytest, JSON backups, Ruff.

**Design:** `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`, Plan 3.

**Depends on:** Complete Plans 1 and 2. Use the ASGI-era module names when they differ from this plan's starting paths.

**Worktree safety:** Migration tests must use temporary databases. Never run the drop migration against the user's real Hieronymus data during development. Preserve unrelated worktree files.

---

## Task 1: Introduce Ordered Schema Migrations

**Files:**

- Create: `src/hieronymus/migrations/versions/__init__.py`
- Create: `src/hieronymus/migrations/versions/0001_memory_fts_triggers.sql` or mark the equivalent Plan 1 state as baseline
- Modify: `src/hieronymus/db.py`
- Modify: `src/hieronymus/migrations/global.sql`
- Create: `tests/test_schema_migrations.py`
- Modify: `tests/test_config.py`

- [ ] **Step 1: Write migration-ledger tests**

  Cover a fresh database, an existing pre-ledger database, ordered application, idempotent rerun, checksum/name mismatch, failure rollback, and a migration that cannot run inside an existing transaction.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_schema_migrations.py -q`

- [ ] **Step 3: Implement the runner**

  Create `schema_migrations(version, name, checksum, applied_at)`. Discover packaged version files in lexical numeric order. Apply each migration transactionally and record it only after success. Fail if an already-recorded migration's checksum/name differs.

- [ ] **Step 4: Establish the baseline safely**

  Fresh databases execute `global.sql`, create the ledger, and record migrations already represented by that schema without replaying destructive upgrade SQL. Existing databases execute compatibility-safe schema creation, then pending upgrades. Keep Plan 1's FTS repair behavior represented exactly once.

- [ ] **Step 5: Route all global initialization through one API**

  Replace store-specific `apply_migration(conn, "global.sql")` assumptions with one `ensure_schema(conn)` boundary while retaining `apply_migration` only for focused tests or non-global scripts.

- [ ] **Step 6: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_schema_migrations.py tests/test_config.py -q
  uv run ruff check src/hieronymus/db.py tests/test_schema_migrations.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/db.py src/hieronymus/migrations/global.sql src/hieronymus/migrations/versions tests/test_schema_migrations.py tests/test_config.py
  git commit -m "feat: add ordered schema migrations"
  ```

## Task 2: Build A Lossless Strict-Term Retirement Migration

**Files:**

- Create: `src/hieronymus/legacy_terms.py`
- Create: `src/hieronymus/migrations/versions/0002_retire_strict_terms.py`
- Modify: `src/hieronymus/memory_migration.py`
- Create: `tests/test_strict_term_retirement.py`
- Modify: `tests/test_memory_graph_migration.py`

- [ ] **Step 1: Create representative legacy fixtures**

  Include approved/active terms, rejected/inactive terms, aliases, forbidden variants, semantic tags, already-migrated ledger rows, duplicate reruns, unsupported alias shapes, and an injected mid-migration failure.

- [ ] **Step 2: Write backup contracts**

  Require a timestamped, versioned JSON backup containing all source rows and relationships plus a checksum. Write to a temporary file, fsync where supported, atomically rename, and verify it can be parsed before database mutation begins.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_strict_term_retirement.py -q`

- [ ] **Step 4: Extract reusable conversion logic**

  Move strict-term-to-rule-graph conversion out of the broad migrator into focused functions that accept an existing connection and return a typed coverage report. Preserve provenance/ledger identity so reruns reconcile instead of duplicate.

- [ ] **Step 5: Implement blocking parity checks**

  Every approved/active row must resolve to an active rule crystal, concept, source/rendering facets, variants, tags, and provenance. If any required row is unsupported or incomplete, raise with its term ID and leave tables intact. Rejected/inactive history must exist in the verified backup and audit record.

- [ ] **Step 6: Keep table dropping disabled initially**

  The migration module may expose the drop phase, but do not register/activate it until Task 3 has removed runtime reads and writes.

- [ ] **Step 7: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py -q
  uv run ruff check src/hieronymus/legacy_terms.py src/hieronymus/migrations/versions/0002_retire_strict_terms.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add src/hieronymus/legacy_terms.py src/hieronymus/migrations/versions/0002_retire_strict_terms.py src/hieronymus/memory_migration.py tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py
  git commit -m "feat: prepare lossless strict term retirement"
  ```

## Task 3: Cut Termbase And Admin Views Over To Rule Crystals

**Files:**

- Modify: `src/hieronymus/termbase.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/mcp_server.py` or `src/hieronymus/mcp_tools.py`
- Modify: `src/hieronymus/memory_migration.py`
- Modify: `src/hieronymus/migrations/global.sql`
- Modify: `src/hieronymus/migrations/versions/0002_retire_strict_terms.py`
- Modify: `tests/test_termbase_contract.py`
- Modify: `tests/test_termbase_validate.py`
- Modify: `tests/test_admin_actions.py`
- Modify: `tests/test_admin_store.py`
- Modify: `tests/test_mcp_server.py`
- Modify: `tests/test_strict_term_retirement.py`

- [ ] **Step 1: Add rule-only termbase contracts**

  Start from a fresh schema with no strict-term tables. Cover propose/approve/import, aliases, forbidden variants, canonical rendering, validation, recall precedence, admin rendering rows, and MCP payload parity.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_termbase_contract.py tests/test_termbase_validate.py tests/test_admin_store.py tests/test_mcp_server.py -q`

- [ ] **Step 3: Write rule crystals natively**

  Refactor termbase mutations to create/update the rule crystal plus concept graph in one transaction. Do not dual-write legacy tables. Preserve deterministic approved-rule precedence.

- [ ] **Step 4: Replace the rendering view query**

  Build admin rendering rows from rule crystals and their concept/facet metadata using bounded set-based queries. Remove `_list_strict_terms` and its per-row tag lookup.

- [ ] **Step 5: Remove legacy schema from fresh databases**

  Delete strict-term tables/FTS declarations from `global.sql`. Activate the upgrade migration's verified drop phase for existing databases. Drop child tables and FTS in a foreign-key-safe order only after parity checks pass.

- [ ] **Step 6: Remove obsolete migration-on-read behavior**

  Keep only compatibility code needed to execute the one-time versioned upgrade. Remove routine startup checks that repeatedly scan `strict_terms`.

- [ ] **Step 7: Verify GREEN and absence**

  Run:

  ```bash
  uv run pytest tests/test_termbase_contract.py tests/test_termbase_validate.py tests/test_admin_actions.py tests/test_admin_store.py tests/test_mcp_server.py tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py -q
  rg -n "from strict_terms|into strict_terms|update strict_terms|strict_terms_fts" src/hieronymus --glob '!migrations/versions/0002_retire_strict_terms.py' --glob '!legacy_terms.py'
  ```

  Expected: no runtime legacy-table access outside the one-time retirement implementation.

- [ ] **Step 8: Commit**

  ```bash
  git add src/hieronymus/termbase.py src/hieronymus/admin.py src/hieronymus/mcp_server.py src/hieronymus/mcp_tools.py src/hieronymus/memory_migration.py src/hieronymus/migrations/global.sql src/hieronymus/migrations/versions/0002_retire_strict_terms.py tests/test_termbase_contract.py tests/test_termbase_validate.py tests/test_admin_actions.py tests/test_admin_store.py tests/test_mcp_server.py tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py
  git commit -m "refactor: make rule crystals the termbase authority"
  ```

## Task 4: Rename The Console API Boundary

**Files:**

- Move: `src/hieronymus/tui_bridge/` to `src/hieronymus/console_api/`
- Delete: obsolete `run_stdio` and line-dispatch support no longer used by ASGI
- Modify: imports throughout `src/hieronymus/`
- Move/rename: `tests/test_tui_bridge_admin.py`
- Move/rename: `tests/test_tui_bridge_config.py`
- Move/rename or delete: `tests/test_tui_bridge_protocol.py`
- Modify: remaining tests importing `tui_bridge`

- [ ] **Step 1: Record the current public/internal import surface**

  Use `rg` and package tests to distinguish internal imports from documented public contracts. The package has no promised external `tui_bridge` API; do not add a permanent alias merely to preserve obsolete naming.

- [ ] **Step 2: Add import-boundary tests for `console_api`**

  Require typed admin/config services and error payloads from the new namespace. Assert no `run_stdio` export exists.

- [ ] **Step 3: Move modules and imports mechanically**

  Preserve behavior and avoid combining this rename with API redesign. Delete protocol/stdio modules only after `rg` proves the ASGI service and tests do not use them.

- [ ] **Step 4: Verify focused parity**

  Run:

  ```bash
  uv run pytest tests/test_console_api_admin.py tests/test_console_api_config.py tests/test_service_app.py -q
  rg -n "tui_bridge|run_stdio" src tests
  uv run ruff check src/hieronymus/console_api tests/test_console_api_admin.py tests/test_console_api_config.py
  ```

  Expected `rg`: no live references; historical docs are handled in Plan 5.

- [ ] **Step 5: Commit**

  Stage the moved/deleted files explicitly and commit:

  ```bash
  git commit -m "refactor: rename console API boundary"
  ```

## Task 5: Canonicalize Data-Root And Cache Paths

**Files:**

- Modify: `src/hieronymus/config.py`
- Modify: modules using `.config_root`
- Modify: `src/hieronymus/llm_cache.py`
- Modify: `src/hieronymus/doctor.py`
- Modify: relevant config/cache/service tests
- Create or modify: `tests/test_config.py`
- Modify: `tests/test_llm_cache.py`
- Modify: `tests/test_doctor.py`

- [ ] **Step 1: Add path-precedence tests**

  Assert explicit argument > `HIERONYMUS_DATA_ROOT` > `XDG_CONFIG_HOME/hieronymus` > `~/.config/hieronymus`. Cover relative/expanded paths and a blank XDG value.

- [ ] **Step 2: Add cache-adoption tests**

  Cover: old only (atomic adoption), new only (unchanged), both (new wins and old is reported), malformed old file, and interrupted rename behavior.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_config.py tests/test_llm_cache.py tests/test_doctor.py -q`

- [ ] **Step 4: Remove the alias**

  Rename all internal `config_root` use to `data_root`, including runtime path records and JSON status fields where compatibility permits. If a user-facing JSON field must remain for one release, derive it explicitly and mark it deprecated rather than retaining the property alias.

- [ ] **Step 5: Honor XDG and adopt the cache**

  Resolve the default root through `XDG_CONFIG_HOME`. Rename `llmcache.tmp` to `llm-cache.json` with atomic one-time adoption and doctor diagnostics for conflicts.

- [ ] **Step 6: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_config.py tests/test_llm_cache.py tests/test_doctor.py tests/test_cli_service.py -q
  rg -n "config_root|llmcache\.tmp" src tests
  uv run ruff check src/hieronymus tests
  ```

- [ ] **Step 7: Commit**

  Stage only path-related files and commit:

  ```bash
  git commit -m "refactor: canonicalize Hieronymus data paths"
  ```

## Task 6: Extract Dream Validation And Persistence Collaborators

**Files:**

- Create: `src/hieronymus/dream_validation.py`
- Create: `src/hieronymus/dream_persistence.py`
- Create: `tests/test_dream_validation.py`
- Create: `tests/test_dream_persistence.py`
- Modify: `src/hieronymus/dreaming.py`
- Modify: `tests/test_dreaming.py`

- [ ] **Step 1: Freeze characterization coverage**

  Add tests around normalization, malformed candidates, allowed IDs, rule invariants, provenance, graph writes, rollback, audit entries, and provider-output contracts before moving code.

- [ ] **Step 2: Record the focused baseline**

  Run: `uv run pytest tests/test_dreaming.py -q`

- [ ] **Step 3: Extract validation without behavior changes**

  Move normalization/validation dataclasses and pure functions first. Keep exceptions and messages stable. Run the characterization suite.

- [ ] **Step 4: Extract persistence without behavior changes**

  Move transaction-scoped insert/update/link/audit helpers behind a collaborator receiving an existing connection. Do not let it commit independently.

- [ ] **Step 5: Keep `DreamService` as facade**

  Its public constructor/methods and MCP/CLI results remain unchanged. Avoid a generic framework or dependency-injection container.

- [ ] **Step 6: Verify parity**

  Run:

  ```bash
  uv run pytest tests/test_dream_validation.py tests/test_dream_persistence.py tests/test_dreaming.py tests/test_dream_workflows.py tests/test_dream_evidence_passes.py -q
  uv run ruff check src/hieronymus/dream_validation.py src/hieronymus/dream_persistence.py src/hieronymus/dreaming.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/dream_validation.py src/hieronymus/dream_persistence.py src/hieronymus/dreaming.py tests/test_dream_validation.py tests/test_dream_persistence.py tests/test_dreaming.py
  git commit -m "refactor: extract dream validation and persistence"
  ```

## Task 7: Extract And Bound Dream Maintenance

**Files:**

- Create: `src/hieronymus/dream_maintenance.py`
- Create: `tests/test_dream_maintenance.py`
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/migrations/global.sql`
- Create: `src/hieronymus/migrations/versions/0003_index_dream_maintenance.sql`
- Modify: `tests/test_dreaming.py`
- Modify: `tests/test_config.py`

- [ ] **Step 1: Add bounded-selection regressions**

  Create thousands of eligible crystals. Assert selection touches at most the configured cap plus one sentinel, excludes current-cycle activated/reinforced rows and active rules, and progresses deterministically without a full `count(*)` or large offset.

- [ ] **Step 2: Add query-plan assertions**

  Use `EXPLAIN QUERY PLAN` to require the maintenance composite index for the candidate query. Avoid assertions on SQLite's exact prose beyond the selected index name.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_dream_maintenance.py -q`

- [ ] **Step 4: Add the composite index**

  Cover status, crystal type, reinforcement cycle, activation cycle, and deterministic ID ordering in the smallest index or pair of indexes justified by the query plan. Add it to fresh schema and an ordered upgrade migration.

- [ ] **Step 5: Implement `limit + 1` selection**

  Remove the full candidate count and offset query. Return the first bounded candidates plus a boolean/magnitude-safe skipped indicator. Preserve the existing changed-crystal cap and audit semantics.

- [ ] **Step 6: Extract maintenance**

  Move passive-event application, activation marking, and cycle decay into `DreamMaintenance` operating on the caller's transaction. Preserve active-rule immunity and canonical scoring helpers.

- [ ] **Step 7: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_dream_maintenance.py tests/test_dreaming.py tests/test_config.py -q
  uv run ruff check src/hieronymus/dream_maintenance.py src/hieronymus/dreaming.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add src/hieronymus/dream_maintenance.py src/hieronymus/dreaming.py src/hieronymus/migrations/global.sql src/hieronymus/migrations/versions/0003_index_dream_maintenance.sql tests/test_dream_maintenance.py tests/test_dreaming.py tests/test_config.py
  git commit -m "perf: bound dream maintenance selection"
  ```

## Task 8: Migration And Plan-Level Verification

**Files:**

- Modify only for in-scope failures.

- [ ] **Step 1: Run required verification**

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  ```

- [ ] **Step 2: Exercise real upgrade copies**

  Create disposable copies of representative old database fixtures. Run doctor/dry-run, perform upgrade, verify backup checksum, verify no strict-term tables, and compare termbase validation/recall results before and after.

- [ ] **Step 3: Exercise failure recovery**

  Inject a blocking approved term and a mid-migration failure. Confirm legacy tables remain, no migration ledger row is recorded, and the backup is readable.

- [ ] **Step 4: Inspect boundaries**

  Confirm no runtime strict-term SQL remains, no `tui_bridge` imports remain, no `config_root` alias remains, no old cache filename remains, and the decay query uses its intended index.

- [ ] **Step 5: Commit only necessary verification fixes**

  Do not create an empty commit. Plan 5 will publish durable migration and memory-model documentation.

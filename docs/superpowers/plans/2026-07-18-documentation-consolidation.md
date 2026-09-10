# Documentation Consolidation Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` and `superpowers:verification-before-completion`. This plan deletes fulfilled planning artifacts only after their durable decisions and implementation evidence are mapped.

**Goal:** Replace accumulated completed/superseded specs and plans with concise current documentation, record the disposition of every review finding, and leave `docs/superpowers` containing only genuinely unimplemented work.

**Architecture:** Durable documentation describes the system that exists after Plans 1-4. A temporary traceability matrix proves every fulfilled artifact has a destination before deletion. Historical step lists are not archived verbatim.

**Tech stack:** Markdown, project boundary tests, `rg`, Git link/path verification.

**Design:** `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`, Plan 5.

**Depends on:** Execute after Plans 1-4 so documentation describes verified implementation rather than intention.

**Worktree safety:** Preserve `report.md`, `improvements.md`, `uv.lock` changes not caused by Plans 1-4, and `docs/rust-migration-proposal/` unless the user separately puts them in scope. Do not delete a Superpowers artifact merely because its checkboxes are checked or unchecked; verify current code and commits.

---

## Task 1: Build Traceability And Review Disposition

**Files:**

- Create: `docs/review-disposition-2026-07-18.md`
- Create temporarily: `docs/superpowers/traceability-2026-07-18.md`
- Read: every file under `docs/superpowers/specs/` and `docs/superpowers/plans/`
- Read: `report.md`
- Read: `improvements.md`
- Inspect: current implementation and Git history

- [ ] **Step 1: Inventory every planning artifact**

  Record path, stated goal, implementation evidence, current status (`fulfilled`, `superseded`, or `pending`), durable destination, and deletion eligibility. Checkbox state is evidence only, not authority.

- [ ] **Step 2: Verify implementation evidence**

  For each artifact, cite current files/tests and the implementing commit(s). If any supposedly completed behavior is absent, mark that artifact pending and keep it in `docs/superpowers`; do not paper over the gap.

- [ ] **Step 3: Write the finding disposition document**

  Include every heading from `report.md` and `improvements.md`, its verified status, implementing plan/commit, final code/document location, and concise reason for intentional non-change. Specifically retain the stale/intentional dispositions approved in the design.

- [ ] **Step 4: Cross-check completeness mechanically**

  Compare report/improvement headings against disposition anchors/table rows. Add a test or small checked script if a simple heading count is insufficient. Ensure no finding is silently omitted.

- [ ] **Step 5: Commit the audit before deletion**

  ```bash
  git add docs/review-disposition-2026-07-18.md docs/superpowers/traceability-2026-07-18.md
  git commit -m "docs: map completed design history"
  ```

## Task 2: Document Architecture, Service/MCP, And Web Console

**Files:**

- Create: `docs/architecture.md`
- Create: `docs/service-and-mcp.md`
- Create: `docs/web-console.md`
- Merge/replace: `docs/current-baseline.md`
- Merge/replace: `docs/service-toolkit.md`
- Merge relevant content: `docs/usage.md`
- Merge relevant content: `docs/opentui-conventions.md`
- Modify: `tests/test_cli_boundaries.py`
- Create: `tests/test_documentation.py`

- [ ] **Step 1: Write current architecture from code**

  Document process ownership, SQLite and LanceDB roles, console/API/MCP boundaries, deterministic terminology precedence, runtime paths, and lifecycle. Clearly distinguish authoritative storage from rebuildable indexes.

- [ ] **Step 2: Document the fixed service contract**

  Include default `127.0.0.1:9768`, override precedence, occupied-port failure, logs, start/status/stop/restart, `/mcp`, stdio compatibility window, no-auth exposure model, Origin behavior, and explicit warning for non-loopback binding.

- [ ] **Step 3: Document the web console**

  Cover ASGI routes, packaged assets, development override, Svelte/Tailwind architecture, responsive/accessibility rules, API timeout behavior, WebSocket events, and semantic-index management.

- [ ] **Step 4: Remove obsolete TUI guidance**

  Preserve only general interaction/accessibility conventions still used by the web console. Remove OpenTUI/runtime instructions and rename all user-facing “management TUI” references.

- [ ] **Step 5: Update boundary tests to durable paths**

  Move assertions currently reading `service-toolkit.md` or `memory-dreaming.md` to the new canonical documents. Add tests for the fixed port, exact `/mcp` URL, no token instructions, and non-loopback warning.

- [ ] **Step 6: Verify**

  ```bash
  uv run pytest tests/test_cli_boundaries.py tests/test_documentation.py -q
  rg -n "random port|ephemeral port|\?token=|hieronymus_token|Management TUI|OpenTUI" README.md docs --glob '!superpowers/**'
  ```

  Expected: no obsolete current guidance; historical review disposition may mention removed terms when clearly labelled.

- [ ] **Step 7: Commit**

  ```bash
  git add docs/architecture.md docs/service-and-mcp.md docs/web-console.md docs/current-baseline.md docs/service-toolkit.md docs/usage.md docs/opentui-conventions.md tests/test_cli_boundaries.py tests/test_documentation.py
  git commit -m "docs: consolidate service and console architecture"
  ```

## Task 3: Document Memory, Dreaming, RAG, And Providers

**Files:**

- Create: `docs/memory-model.md`
- Create: `docs/rag.md`
- Create: `docs/providers.md`
- Merge/replace: `docs/memory-dreaming.md`
- Modify: `docs/translation-workspace-integration.md`
- Modify: `tests/test_documentation.py`
- Modify: `tests/test_cli_boundaries.py`

- [ ] **Step 1: Document the current memory model**

  Cover task sessions, short-term memory, concepts/facets, rule/lesson crystals, scoring, feedback, activation, dreaming phases, bounded maintenance, provenance, and audit. State that rule crystals are the only terminology authority and explain active-rule precedence.

- [ ] **Step 2: Document legacy migration and recovery**

  Explain versioned schema migrations, strict-term backup/parity/drop behavior, blocking failures, backup location, doctor checks, and restoration. Do not document removed legacy tables as current APIs.

- [ ] **Step 3: Document RAG as implemented**

  Cover parsing/import, authoritative SQLite corpus/FTS, local FastEmbed default, LanceDB derived generations, lazy model download, bounded indexing jobs, hybrid RRF, filters/boosts, explanations, offline fallback, rebuild/cancel, and the future `SemanticIndex` backend seam.

- [ ] **Step 4: Document provider boundaries**

  Separate chat/dream providers from embedding providers. Document local default, model identity/cache behavior, optional remote registration, secret handling, and failure degradation.

- [ ] **Step 5: Update translation-workspace integration**

  Use current MCP URL/stdio compatibility examples and explain how RAG evidence complements but cannot override approved terminology.

- [ ] **Step 6: Verify terminology consistency**

  ```bash
  uv run pytest tests/test_cli_boundaries.py tests/test_documentation.py -q
  rg -n "strict_terms|strict term table|SQLite-only RAG|FTS-only RAG|unbounded decay" docs --glob '!superpowers/**' --glob '!review-disposition-2026-07-18.md'
  ```

  Expected: no removed architecture described as current.

- [ ] **Step 7: Commit**

  ```bash
  git add docs/memory-model.md docs/rag.md docs/providers.md docs/memory-dreaming.md docs/translation-workspace-integration.md tests/test_documentation.py tests/test_cli_boundaries.py
  git commit -m "docs: consolidate memory and retrieval architecture"
  ```

## Task 4: Document Installation, Agents, And Releases

**Files:**

- Create: `docs/installation-and-updates.md`
- Create: `docs/agent-integration.md`
- Create: `docs/release-process.md`
- Merge/replace: `docs/agent-workflows.md`
- Modify: `docs/project-conventions.md`
- Modify: `docs/usage.md`
- Modify: `tests/test_documentation.py`
- Modify: `tests/test_release_workflow.py`
- Modify: `tests/test_agent_assets.py`

- [ ] **Step 1: Document install/update behavior**

  Cover prerequisites, Bun validation, Hatch-owned frontend build, stable/dev channels, XDG/data-root precedence, cache adoption, install/uninstall, diagnostics, and noninteractive installer behavior.

- [ ] **Step 2: Document each agent adapter exactly**

  For every supported client, show the generated configuration as implemented: HTTP URL for confirmed HTTP-capable clients and `hieronymus-mcp` for compatibility clients. Document hooks/project skills separately from MCP transport.

- [ ] **Step 3: Document release ownership**

  Cover zero-major semantic-release behavior, version sources, `uv build` ownership, Hatch/Bun coupling, CI frontend/backend tradeoff, artifact checks, and the MCP `<2` compatibility bound.

- [ ] **Step 4: Remove superseded commands and paths**

  Check all examples against `--help`, installed console scripts, generated assets, workflows, and current file locations.

- [ ] **Step 5: Verify**

  ```bash
  uv run pytest tests/test_documentation.py tests/test_release_workflow.py tests/test_agent_assets.py -q
  uv run hiero --help
  uv run hiero rag --help
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add docs/installation-and-updates.md docs/agent-integration.md docs/release-process.md docs/agent-workflows.md docs/project-conventions.md docs/usage.md tests/test_documentation.py tests/test_release_workflow.py tests/test_agent_assets.py
  git commit -m "docs: consolidate installation and integration guidance"
  ```

## Task 5: Update Navigation, Baseline, And Roadmap

**Files:**

- Modify: `README.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/usage.md`
- Delete only after merged: superseded top-level docs replaced in Tasks 2-4
- Modify: `tests/test_documentation.py`

- [ ] **Step 1: Replace the README document index**

  Link the new canonical documents, remove TUI/current-baseline/service-toolkit links that no longer exist, and keep the quickstart concise.

- [ ] **Step 2: Rewrite the roadmap from current state**

  Mark implemented service, MCP, schema, and semantic RAG capabilities as baseline. Keep only genuinely pending work, including potential Qdrant support and eventual stdio/private-bridge removal.

- [ ] **Step 3: Remove duplicate top-level documents**

  Delete a replaced document only after every unique current instruction is merged and inbound links are updated. Prefer deletion over leaving “see new file” stubs unless an external public link is known to require compatibility.

- [ ] **Step 4: Add link/path validation**

  In `tests/test_documentation.py`, parse relative Markdown links under README/docs, ignore external URLs/anchors appropriately, and assert local targets exist. Also assert documented scripts and canonical files exist.

- [ ] **Step 5: Verify**

  ```bash
  uv run pytest tests/test_documentation.py tests/test_cli_boundaries.py -q
  rg -n "current-baseline\.md|service-toolkit\.md|memory-dreaming\.md|agent-workflows\.md|opentui-conventions\.md" README.md docs tests --glob '!superpowers/**' --glob '!review-disposition-2026-07-18.md'
  git diff --check
  ```

- [ ] **Step 6: Commit**

  Stage the README, roadmap, usage guide, link test, and explicitly verified top-level deletions:

  ```bash
  git commit -m "docs: publish the current Hieronymus baseline"
  ```

## Task 6: Remove Fulfilled Superpowers Artifacts

**Files:**

- Delete: fulfilled files under `docs/superpowers/specs/`
- Delete: fulfilled files under `docs/superpowers/plans/`
- Delete: `docs/superpowers/traceability-2026-07-18.md` after its mappings are represented in durable docs/review disposition
- Preserve: any artifact proven genuinely pending
- Preserve: `report.md`, `improvements.md`, and unrelated user files

- [ ] **Step 1: Re-run the traceability audit**

  For every file, confirm destination sections exist, implementation evidence is current, and all findings/tasks are covered. Any pending artifact stays and is listed in the roadmap.

- [ ] **Step 2: Remove fulfilled old artifacts**

  Delete completed/superseded Tailwind, frontend redesign, daemon, web console, admin views, provider SDK, project skills, RAG MVP/completion, deep-reading, release, and TUI planning artifacts after mapping verification.

- [ ] **Step 3: Remove this completed program's artifacts**

  Once Plans 1-4 and Tasks 1-5 of this plan are verified and documented, delete:

  - `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`
  - `docs/superpowers/plans/2026-07-18-correctness-and-data-integrity.md`
  - `docs/superpowers/plans/2026-07-18-stable-daemon-and-web-mcp.md`
  - `docs/superpowers/plans/2026-07-18-memory-schema-and-boundaries.md`
  - `docs/superpowers/plans/2026-07-18-local-semantic-rag.md`
  - `docs/superpowers/plans/2026-07-18-documentation-consolidation.md`

- [ ] **Step 4: Verify the directory invariant**

  Run: `find docs/superpowers -type f -print | sort`

  Expected: no files, or only artifacts explicitly proven pending and linked from `docs/roadmap.md`.

- [ ] **Step 5: Run full repository verification**

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  uv build
  git diff --check
  ```

- [ ] **Step 6: Commit the cleanup**

  Stage only verified documentation, tests, and Superpowers deletions. Confirm `report.md`, `improvements.md`, `docs/rust-migration-proposal/`, and unrelated `uv.lock` work are not accidentally staged.

  ```bash
  git commit -m "docs: retire fulfilled implementation plans"
  ```

# Strict-Term Validation Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining ownership-schema and alias-binding validation gaps before strict-term mutation.

**Architecture:** Require both ordered uniqueness contracts on structured relationship ownership. Add structured alias provenance that binds source IDs, term IDs, alias IDs, target kinds, and ledger targets; existing aliases must belong to the encoded term, while removed aliases require a durable migration-created provenance row.

**Tech Stack:** Python 3.12, SQLite, pytest, Ruff.

## Global Constraints

- Every invalid schema or alias binding blocks with all affected active term IDs before mutation.
- Canonical source parsing rejects leading-zero, case, kind, and target-table variants.
- Shared/user graph data and legacy tables remain intact; drop remains disabled.

---

### Task 1: RED validation matrix

**Files:**
- Modify: `tests/test_strict_term_retirement.py`

**Interfaces:**
- Consumes: `prepare_strict_term_retirement(conn, backup_dir)`.
- Produces: five relationship-uniqueness fixtures plus cross-term and stale-alias regressions.

- [x] **Step 1: Add uniqueness matrix tests**

Recreate relationship ownership with neither key, only the source-scoped key, only the global edge
key, wrong column order, and both exact keys. Assert only the exact schema proceeds.

- [x] **Step 2: Add alias-binding tests**

Forge a term-one alias source using an alias row owned by term two and assert pre-mutation blocking;
remove a genuinely migrated alias and assert its structured provenance authorizes safe cleanup.

- [x] **Step 3: Run RED**

Run the exact new tests and confirm the partial uniqueness schemas and cross-term alias currently
pass validation incorrectly.

### Task 2: Exact uniqueness and durable alias provenance

**Files:**
- Modify: `src/hieronymus/memory_migration.py`
- Modify: `src/hieronymus/legacy_terms.py`
- Modify: `.superpowers/sdd/plan3-task-2-report.md`

**Interfaces:**
- Produces: exact ordered uniqueness validation and
  `strict_term_retirement_alias_provenance(source_table, source_id, term_id, alias_id,
  target_table, target_id, alias_kind, created_at)`.

- [x] **Step 1: Require both uniqueness keys**

Validate the exact source-scoped primary key and exact global edge unique key independently with
logical AND; any missing/reordered key raises `StrictTermConversionBlocked` with active IDs.

- [x] **Step 2: Record and validate alias provenance**

Record one structured row for every current alias ledger projection. Validate canonical source
encoding, target table/kind, ledger identity, existing alias term ownership, and stale same-source
provenance before conversion writes or cleanup.

- [x] **Step 3: Verify and commit**

Run focused ownership/retirement/graph tests, full pytest, Ruff check, Ruff format check, and diff
check; update the report, self-review all source variants, and create a separate commit.

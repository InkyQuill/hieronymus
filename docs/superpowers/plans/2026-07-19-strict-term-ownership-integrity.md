# Strict-Term Ownership Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make strict-term cleanup incapable of deleting shared or spoof-selected graph data.

**Architecture:** Replace opaque relationship keys with structured provenance columns and validate the complete ownership graph before conversion writes. Cleanup consumes only validated, term-bound ownership rows; shared facets lose only the stale source edge and remain reachable.

**Tech Stack:** Python 3.12, SQLite, pytest, Ruff.

## Global Constraints

- Backup verification and all schema/ownership preflights occur before graph mutation.
- Stored ownership metadata is evidence to validate, never a raw deletion instruction.
- Shared/user graph data is preserved; destructive table retirement remains disabled.

---

### Task 1: RED ownership-integrity regressions

**Files:**
- Modify: `tests/test_strict_term_retirement.py`

**Interfaces:**
- Consumes: `prepare_strict_term_retirement(conn, backup_dir)`.
- Produces: regressions for cross-term relationship spoofing and shared-facet cleanup.

- [x] **Step 1: Add a cross-term structured-ownership test**

Seed two converted terms, rebind a migration-owned relationship row to the other term, change the
first source projection, and assert retirement raises `LegacyTermRetirementBlocked` naming both
active IDs before any tag or ledger mutation.

- [x] **Step 2: Add a shared-facet cleanup test**

Point a second ledger row and ownership row at an alias facet, remove the first alias, rerun, and
assert the facet and second target remain while only the first stale edges disappear.

- [x] **Step 3: Run RED**

Run `uv run pytest tests/test_strict_term_retirement.py -q`; expect both new tests to fail against
opaque relationship keys and unconditional stale-facet deletion.

### Task 2: Structured provenance and preflight

**Files:**
- Modify: `src/hieronymus/memory_migration.py`
- Modify: `tests/test_strict_term_retirement.py`

**Interfaces:**
- Produces: `_validate_retirement_ownership_integrity(conn, active_ids)` and structured ownership
  rows containing source, relationship type, owner type/ID, related type/ID, and value.

- [x] **Step 1: Replace opaque keys**

Create the private relationship-ownership table with typed structured columns and constraints;
sync tag/language/link rows through exact structured predicates.

- [x] **Step 2: Validate before mutation**

For every active term, verify object targets exist and match ledger identity, relationship owners
match the term's owned concept/facet/crystal, values match actual rows, and no object has conflicting
source ownership. Raise one domain error containing every affected active term ID.

- [x] **Step 3: Make stale facet cleanup reference-safe**

Delete a stale facet only when its object ownership is exclusive and no other ledger, ownership,
relationship, or current projection references it; otherwise remove only this source edge.

- [x] **Step 4: Run GREEN and complete verification**

Run focused retirement/graph tests, `uv run pytest`, `uv run ruff check .`,
`uv run ruff format --check .`, and `git diff --check`; update the task report and commit separately.

# Plan 3 Task 3 Report

## Outcome

Runtime termbase, admin rendering, and MCP behavior now use rule crystals and the concept graph
without reading or writing legacy strict-term tables. Fresh databases do not declare the legacy
parent, child, or FTS tables. Existing databases execute Python migration `0002` once, write and
verify the Task 2 backup, prove parity and ownership, then drop FTS, aliases, tags, and the parent
inside the migration runner's transaction.

## TDD evidence

- Fresh no-legacy RED: `test_fresh_termbase_schema_has_no_legacy_term_tables` initially failed
  because `global.sql` still declared all four legacy objects.
- Native-write RED: proposal graph and crystals FTS tests initially failed because `propose()`
  only inserted strict-term rows.
- Alias RED: source/search alias approval tests failed while the old parser restriction remained;
  source/search aliases are now native concept alias facets.
- Upgrade RED: Python `0002` discovery/drop test initially failed with `DROP_PHASE_ENABLED=False`.
- GREEN targeted command: termbase, validation, admin actions/store, MCP, retirement, and memory
  migration suites: `223 passed, 5 skipped`.

## Native graph mutations and authority

- `propose()` uses `BEGIN IMMEDIATE` and creates one pending rule crystal, its selected/new
  concept, canonical source/rendering facets, semantic tags, and the defining edge in one
  transaction. It never dual-writes strict tables.
- `add_alias()` adds pending graph facets; forbidden aliases are folded into canonical rule text
  at approval. Unsupported lossless shapes fail before activation and SQLite rolls back.
- `approve()` validates the graph projection and atomically activates the existing crystal;
  repeated approval is idempotent.
- Contract and validation load only active, sufficiently strong/confident, linked rule crystals.
  The existing deterministic-rule thresholds and recall protected-rule precedence remain intact.
- Public termbase/admin/MCP identifiers and payload fields remain crystal-backed equivalents of
  the former term identifiers.

## Admin query bound

`AdminStore._list_rule_renderings()` uses one bounded CTE query for up to 200 rules, rendering
facets, and aggregated semantic tags. `test_rendering_rows_use_bounded_set_queries` records the
SQLite trace and proves four rows still execute exactly one rendering-list query; the former
per-row tag query and `_list_strict_terms` are gone.

## Existing database retirement

- `MigrationContext` is supplied to Python migrations and derives the backup directory as
  `<database parent>/backups/strict-terms`, keeping it below the data/database root.
- Fresh databases baseline `0002` only after its verifier proves all legacy objects absent.
- Existing databases call `prepare_strict_term_retirement()` once from pending `0002`, using its
  backup, checksum, inactive audit, parity, and ownership checks. The finalizer drops
  `strict_terms_fts`, aliases, tags, then `strict_terms` in the same controlled transaction.
- Idempotency test proves the second `ensure_schema()` creates no second backup.
- Injected mid-drop failure proves FTS/child/parent schema and row data roll back intact and 0002
  remains unledgered. Backup/parity failure coverage remains in the 59-test retirement suite.
- Shared/user graph preservation continues to be enforced by Task 2 ownership validation before
  the drop finalizer runs.

## Absence and verification

The following search is empty outside the three one-time compatibility locations:

```text
rg -n "from strict_terms|into strict_terms|update strict_terms|strict_terms_fts" src/hieronymus \
  --glob '!**/migrations/versions/0002_retire_strict_terms.py' \
  --glob '!**/legacy_terms.py' --glob '!**/memory_migration.py'
```

Verification completed before commit:

- `uv run ruff check .` — clean.
- `uv run ruff format --check .` — 180 files formatted.
- Full suite first clean run after the cutover: `1424 passed, 5 skipped`.
- A later full rerun after native alias refinement had one unrelated websocket disconnect timing
  failure (`subscriber_count` remained 1 for one second); it passed immediately in isolation.
- Required final full rerun: `1424 passed, 5 skipped in 146.86s`.

The five skips are obsolete tests that expected runtime strict-term reconciliation after 0002 has
already removed the source tables. Equivalent conversion, reconciliation, rollback, parity, and
ownership behavior remains exercised in `test_strict_term_retirement.py`.

## Self-review

- Confirmed no fresh strict schema and no runtime legacy SQL in termbase/admin/MCP.
- Confirmed mutation IDs are crystal IDs, matching MCP payload parity.
- Confirmed target-language rendering selection is constrained by facet language.
- Confirmed admin list work is set-based and bounded.
- Confirmed migration backup side effects can remain after a database rollback by design, while
  the database and migration ledger remain retryable.
- Concern: the backup JSON necessarily survives a failed migration attempt; retry creates a new
  verified backup rather than overwriting the first, which favors recovery over cleanup.

## Commit

Implementation: `e5d30bd` (`refactor: make rule crystals the termbase authority`).

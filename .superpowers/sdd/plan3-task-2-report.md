# Plan 3 Task 2 — Lossless Strict-Term Retirement Preparation

## Status

Implemented as a prepared, importable migration module. It is not discovered by the
SQL-only ordered runner, `DROP_PHASE_ENABLED` is false, and the legacy tables remain intact.

## Fixture matrix and TDD evidence

| Fixture | Contract exercised | Result |
| --- | --- | --- |
| approved + active term | active rule, concept, source/rendering facets | mapped with parity |
| source/search alias | alias facet and ledger identity | mapped losslessly |
| approved alias | rendering facet and distinct ledger identity | mapped losslessly |
| forbidden alias | rule text and alias-to-crystal ledger identity | mapped losslessly |
| semantic tag | concept and crystal side tables | exact subset parity |
| rejected/inactive term | backup and idempotent audit snapshot | preserved with relationships |
| existing ledger/rerun | provenance reconciliation | graph/audit counts unchanged |
| case-insensitive alias | unsupported shape | blocks with term ID before mutation |
| injected post-term failure | transaction atomicity | all DB writes roll back; backup remains valid |
| caller-active transaction | ownership ambiguity | rejected before backup or mutation |
| changed canonical/aliases/tags | exact rerun convergence | reconciled without duplicates |
| archived 0.99-confidence concept | active graph parity | reactivated; confidence preserved |
| alias kind change/removal | relationship parity | stale ledger/facet removed |
| four concurrent same-second backups | collision safety | four unique verified files |
| unsafe timestamp/symlink | path containment | rejected before publication |
| directory fsync ENOTSUP/EIO | durability boundary | unsupported ignored; EIO propagated |
| stale/malformed inactive audit | complete audit parity | reconciled in place, deduplicated |
| incomplete graph/ledger/alias | actionable blocking | every affected term ID reported |
| incomplete inactive-audit schema | pre-mutation validation | inactive term IDs reported |

RED was established with `uv run pytest tests/test_strict_term_retirement.py -q`:
collection failed with `ModuleNotFoundError: No module named 'hieronymus.legacy_terms'`.
After implementation, the focused suite passed 44/44.

Reviewer remediation established a second RED run: 11/19 tests failed on the reported
reconciliation, collision, path, fsync, audit, and error-message gaps. Three additional
self-review regressions then failed for alias-kind relationship replacement and blank alias
fields. All now pass.

## Backup format and durability

- Filename: `strict-terms-v1-<validated UTC timestamp>-<128-bit unique ID>.json`.
- Root identity: `hieronymus.strict-terms-backup`, version `1`.
- Contents: every ordered row from `strict_terms`, `strict_term_tags`, and
  `strict_term_aliases`, including inactive/rejected terms and their relationships.
- Integrity: SHA-256 over canonical compact JSON excluding the `checksum` field; the
  published file is parsed and its checksum recomputed before conversion starts.
- Publication: the timestamp accepts only `YYYYMMDDTHHMMSSZ`; resolved directory containment
  rejects symlink/path escapes; each final name has a collision-resistant UUID. A
  same-directory exclusive temporary file is flushed and file-fsynced before `os.replace`,
  followed by directory fsync and parse/checksum verification. Only concrete unsupported
  directory-fsync errnos are suppressed; EIO and other durability failures propagate before
  conversion. Tests spy on fsync/replace, run four concurrent same-timestamp writers, and
  prove no temporary file remains.

## Conversion, coverage, and parity

`convert_strict_terms(conn, ...)` accepts an existing SQLite connection, never commits or
rolls back, and returns `StrictTermConversionReport`. The preparation orchestrator rejects
an already-active transaction, acquires `BEGIN IMMEDIATE`, creates and verifies the backup,
runs conversion and parity, and owns commit/rollback explicitly.

The representative parity fixture reports `total=2`, `active=1`, `migrated=1`,
`inactive_audited=1`, `blocked=()`, `complete=true`. Blocking parity checks resolve ledger
targets and verify active crystal status/text, concept link, source/rendering/alias facets,
approved/forbidden alias provenance, semantic tags, and inactive audit coverage.

Rerunning against the same source reconciles ledger targets and leaves concept, facet,
crystal, and audit counts unchanged. The injected failure occurs after the first term has
written its graph; rollback leaves all graph and ledger counts at zero while the verified
two-term backup remains readable.

Rerun reconciliation is now an exact strict-term provenance projection: source/rendering and
alias facets reconcile value, type, language, canonical flag, and minimum confidence; rule
crystals reconcile active status and canonical text while preserving stronger confidence and
strength; archived ledger concepts are restored to `established` while preserving stronger
confidence. Stale alias relationships/facets and obsolete semantic/language tags are removed.
Parity checks exact concept/crystal activity and identity, facets, languages, tags, links, and
alias target-table relationships. Inactive audit `before_json` is recomputed from the complete
term plus tags and aliases, reconciled in place, and checked for exact single-row equality.

`pytest-cov`/`coverage.py` are not project dependencies (`pytest --cov` is unrecognized and
`python -m coverage` is unavailable), so no synthetic package coverage percentage is
reported. Behavioral retirement coverage is represented by the typed coverage report and
the fixture matrix above.

## Inactive drop proof

`hieronymus.migrations.versions.0002_retire_strict_terms` imports successfully, exposes
the backup/conversion/drop phases, sets `DROP_PHASE_ENABLED = False`, and its drop function
raises a Task-3-specific disabled error. `discover_schema_migrations()` still returns only
SQL migration `0001`; all three legacy tables remain present after preparation.

## Verification

- `uv run pytest tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py -q`
  — 60 passed.
- `uv run pytest` — 1391 passed in 137.85s.
- `uv run ruff check .` — passed.
- `uv run ruff format --check .` — 180 files already formatted.
- `git diff --check` — passed.

## Self-review and concerns

Reviewed the complete diff for mutation ordering, transaction ownership, backup contents,
checksum scope, resolved path containment, errno handling, alias provenance (including kind
changes and shared-facet splitting), exact rerun behavior, and accidental runner activation. The
runtime still reads legacy tables, so the destructive phase correctly remains unavailable.
No implementation blocker remains. Task 3 must remove runtime legacy access before enabling
or implementing table drops.

## Commit

Implementation and verification report: `10bf714 feat: prepare lossless strict term retirement`.
Reviewer Important findings are fixed in the subsequent dedicated remediation commit.

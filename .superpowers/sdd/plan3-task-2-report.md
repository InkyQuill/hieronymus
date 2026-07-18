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
| unowned ledger target | ownership boundary | preserved; ledger repointed to dedicated node |
| owned graph rerun | provenance reconciliation | only marked nodes/relationships reconciled |
| case-insensitive alias | unsupported shape | blocks with term ID before mutation |
| injected post-term failure | transaction atomicity | all DB writes roll back; backup remains valid |
| caller-active transaction | ownership ambiguity | rejected before backup or mutation |
| changed canonical/aliases/tags | exact rerun convergence | reconciled without duplicates |
| archived 0.99-confidence concept | active graph parity | reactivated; confidence preserved |
| alias kind change/removal | relationship parity | stale ledger/facet removed |
| four concurrent same-second backups | collision safety | four unique verified files |
| unsafe timestamp/symlink | path containment | rejected before publication |
| directory fsync ENOTSUP/EIO | durability boundary | unsupported ignored; EIO propagated |
| stale/malformed inactive audit | append-only history | prior event preserved; corrected event appended |
| malformed tag/alias/ledger/provenance | actionable blocking | every affected term ID reported |
| user tags/languages/links | shared graph safety | preserved while owned projection converges |
| directory symlink swap / renamed backup | descriptor and identity pinning | rejected safely |
| incomplete inactive-audit schema | pre-mutation validation | inactive term IDs reported |
| cross-term/spoofed ownership | deletion authorization | blocks all active IDs before mutation |
| two-ledger shared facet | exclusive deletion proof | stale edge released; facet/reference preserved |
| missing owned relationship | ownership integrity and parity | blocks; coverage remains incomplete |

RED was established with `uv run pytest tests/test_strict_term_retirement.py -q`:
collection failed with `ModuleNotFoundError: No module named 'hieronymus.legacy_terms'`.
After implementation, the focused suite passed 44/44.

Reviewer remediation established a second RED run: 11/19 tests failed on the reported
reconciliation, collision, path, fsync, audit, and error-message gaps. Three additional
self-review regressions then failed for alias-kind relationship replacement and blank alias
fields. All now pass.

The rereview established a third RED run: 9/30 focused tests failed on unowned target reuse,
user-relationship deletion, audit mutation, backup identity/symlink races, relationship schema
diagnostics, and ledger uniqueness. After ownership and descriptor-pinning remediation, the
focused retirement suite passes 32/32.

The acceptance review established a fourth RED run: both new critical regressions failed. A
cross-term relationship row selected another term's tag for deletion and stale cleanup deleted a
facet still targeted by another ledger. Relationship provenance is now structured rather than
JSON-encoded, the complete ownership graph is validated before retirement writes, and focused
retirement/ownership/graph verification passes 77/77.

## Backup format and durability

- Filename: `strict-terms-v1-<validated UTC timestamp>-<128-bit unique ID>.json`.
- Root identity: `hieronymus.strict-terms-backup`, version `1`.
- Contents: every ordered row from `strict_terms`, `strict_term_tags`, and
  `strict_term_aliases`, including inactive/rejected terms and their relationships.
- Integrity: SHA-256 over canonical compact JSON excluding the `checksum` field. The payload
  authenticates both the timestamp and 128-bit backup ID against the filename; rename tampering
  is rejected before conversion.
- Publication: the timestamp accepts only `YYYYMMDDTHHMMSSZ`; resolved directory containment
  rejects symlink/path escapes; each final name has a collision-resistant UUID. A
  pinned directory descriptor is opened with `O_DIRECTORY`/`O_NOFOLLOW`; the relative
  same-directory temporary file uses `O_EXCL`/`O_NOFOLLOW`, is flushed and file-fsynced, and is
  atomically replaced relative to that same descriptor. Directory fsync and descriptor-relative
  parse/checksum verification follow. Only concrete unsupported directory-fsync errnos are
  suppressed; EIO and other durability failures propagate before conversion. Tests cover four
  concurrent same-timestamp writers, path rename tampering, and a directory symlink swap.

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

Rerun reconciliation is an ownership-bounded strict-term projection. Migration-private object
provenance uses typed object IDs; relationship provenance stores structured owner type/ID,
related type/ID, and value columns rather than executable opaque keys. Before any retirement
mutation, integrity validation proves canonical source IDs, term binding, ledger identity, owned
foreign targets, facet-to-concept membership, and exact relationship existence. Any malformed,
missing, spoofed, or cross-term edge blocks every affected active term; parity independently runs
the same audit and cannot report complete against corrupt ownership metadata.

Ledger and natural matches without valid ownership are adopted/user data: they are preserved and
the ledger is repointed to a dedicated owned node. Tag and language deletion requires the exact
structured edge, its actual relationship, and its current migration-owned node. Stale facet
deletion additionally requires exclusive ownership: no other ledger/ownership/current projection,
source crystal, user language tag, story scope, or semantic tag may reference it. Otherwise only
the stale ledger and ownership metadata are released, leaving every external target resolvable.
Inactive audit events remain append-only: an exact snapshot is reused; changed or malformed
history is preserved and a corrected event is appended.

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

- `uv run pytest tests/test_strict_term_retirement.py tests/test_memory_graph_migration.py tests/test_memory_schema_metadata.py -q` — 77 passed.
- `uv run pytest` — 1403 passed in 146.50s.
- `uv run ruff check .` — passed.
- `uv run ruff format --check .` — 180 files already formatted.
- `git diff --check` — passed.

## Self-review and concerns

Reviewed the complete diff for mutation ordering, transaction ownership, canonical term binding,
structured ownership constraints, foreign-target and actual-edge validation, cross-term conflict
blocking, deletion exclusivity, stranded ledger targets, best-effort graph repair compatibility,
shared relationship preservation, append-only audit history, backup identity/checksum scope,
descriptor-relative publication, schema diagnostics, alias provenance, and accidental runner
activation. The runtime still reads legacy tables, so destructive retirement remains unavailable.
No implementation blocker remains. Task 3 must remove runtime legacy access before enabling
or implementing table drops.

## Commit

Implementation and verification report: `10bf714 feat: prepare lossless strict term retirement`.
Reviewer Important findings are fixed in the subsequent dedicated remediation commit.
Rereview findings are fixed in a separate ownership/backup hardening commit.
Critical acceptance findings are fixed in a subsequent structured-ownership safety commit.

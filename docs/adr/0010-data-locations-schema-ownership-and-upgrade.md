# Preserve Existing Data Through Versioned Import And Upgrade

## Status

Proposed.

## Context

Existing installations use a Python-created SQLite database and configuration
layout. The Rust proposal changes both the default data location and database
filename while describing SQLx migrations that assume a fresh SQLx ledger.
Applying a fresh-schema migration directly to an existing Python database would
collide with existing tables, and structured legacy terminology cannot be
safely converted by SQL alone.

## Decision

Treat the SQLite database and local configuration as user data that must survive
the reimplementation.

Keep `--data-root` and `HIERONYMUS_DATA_ROOT` as the single isolation boundary.
Within a selected root, data and configuration retain their existing relative
locations during the Rust cutover. A future XDG split requires a separate ADR
and migration. The Rust binary must auto-detect the current database filename;
it must not silently create a new empty database beside a legacy database.

The retained root contains `hieronymus.sqlite`, `provider.conf`, `dream.conf`,
`ingest.conf`, `release.conf`, `llmcache.tmp`, `backups/`, and generated
`agent-plugins/`. The Rust migration does not rename, split, or relocate these
paths. Unchanged authoritative TOML files remain byte-identical. Files that
require conversion use `toml_edit` so comments, ordering, and unknown supported
keys survive; file ownership and user-only permissions for credentials are
preserved. `llmcache.tmp` is derived and may be invalidated; it is never treated
as configuration authority. Detailed file conversion is owned by the
[data-root/config migration spec](../superpowers/specs/2026-08-31-rust-data-root-config-migration-design.md).

Use an application-owned schema identity and monotonically increasing schema
version independent of `_sqlx_migrations`. Database opening follows this state
machine:

1. identify an empty, supported Python, supported Rust, newer, or unknown schema;
2. refuse mutation for newer or unknown schemas;
3. for Python schemas, run read-only preflight and produce a conversion report;
4. create a timestamped sibling backup and fsync it before conversion;
5. convert in one exclusive upgrade operation with foreign keys enabled;
6. rebuild derived FTS and semantic indexes;
7. verify row accounting, foreign keys, FTS integrity, and deterministic
   terminology equivalence;
8. atomically mark the new schema version only after verification succeeds.

Fresh databases are created directly at the current Rust schema. Existing
databases use a Rust upgrade runner that executes ordered SQL and typed
converters inside one exclusive SQLite transaction after the durable backup is
complete. A typed converter may read and write only through the transaction
handle; it cannot commit. FTS rebuild and all authoritative row verification run
inside that transaction. Filesystem config conversion and semantic-index
rebuild run only after the database commit and use their own atomic promotion
protocols. SQLx migrations alone are not the import protocol.

The existing memory-graph migration ledger is preserved or explicitly mapped;
its provenance is never discarded. Structured strict-term aliases retain
language, alias kind, case sensitivity, canonical rendering, forbidden forms,
and concept linkage. Human-readable crystal text is a presentation field, not
the source for reconstructing deterministic terminology.

The database-upgrade track owns creation and population of the structured rule
tables defined by ADR 0011. Its typed strict-term converter maps every supported
legacy rule and records source ids plus target rule/form/projection ids in the
migration ledger. The terminology track owns runtime semantics after conversion;
it does not perform opportunistic migration on read.

## One-Way Cutover And Failure Recovery

Upgrade failure leaves the original database untouched or restores it from the
verified backup before releasing the exclusive lock. Before the target schema
version is committed, recovery returns to the pre-upgrade state. After a
successful commit, downgrade and Python runtime rollback are unsupported.

The immutable backup remains for Rust recovery tooling, data export, and manual
forensics. It is never opened in place. A recovery operation creates a new work
copy, reruns the current Rust importer, verifies it, and atomically promotes the
recovered Rust database.

## Consequences

The Rust schema may be cleaner than the Python schema, but compatibility is
implemented as an explicit import/upgrade boundary. Test fixtures must cover
every supported source schema and corrupted/partial states.

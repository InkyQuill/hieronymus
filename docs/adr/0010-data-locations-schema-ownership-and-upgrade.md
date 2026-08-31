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
databases use a Rust upgrade runner that may execute typed conversion code
between SQL migration steps. SQLx migrations alone are not the import protocol.

The existing memory-graph migration ledger is preserved or explicitly mapped;
its provenance is never discarded. Structured strict-term aliases retain
language, alias kind, case sensitivity, canonical rendering, forbidden forms,
and concept linkage. Human-readable crystal text is a presentation field, not
the source for reconstructing deterministic terminology.

## Rollback

Upgrade failure leaves the original database untouched or restores it from the
verified backup before releasing the exclusive lock. After a successful
upgrade, automatic downgrade is not attempted. Rollback means stopping Rust,
restoring the backup, and reinstalling the recorded Python version.

## Consequences

The Rust schema may be cleaner than the Python schema, but compatibility is
implemented as an explicit import/upgrade boundary. Test fixtures must cover
every supported source schema and corrupted/partial states.

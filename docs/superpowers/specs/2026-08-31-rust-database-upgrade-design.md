# Rust Database Upgrade Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Open fresh Rust databases and upgrade supported Python databases without silent
data loss, split roots, partial conversion, or an unusable rollback.

## Schema Identification

Read-only preflight classifies the selected data root as:

- empty;
- supported Python schema, with exact feature/column fingerprint;
- supported Rust schema version;
- newer Rust schema;
- unknown, corrupt, or partially upgraded.

If a legacy database exists, Rust must not create a fresh sibling database.
Unknown, corrupt, and newer schemas fail closed with a report and make no
changes.

## Preflight Report

The JSON/human report includes source path, detected schema, database size,
SQLite integrity result, foreign-key violations, table counts, pending legacy
conversion counts, unsupported rows with reasons, required free space, target
schema, backup path, and whether conversion is safe. API keys and memory text
are not copied into diagnostics.

`hiero migrate --dry-run` performs the full read and transformation validation
without writing the source database, backup, config, or indexes.

## Upgrade Protocol

1. Resolve and lock the explicit data root; refuse if the daemon is active.
2. Run preflight and require a safe result.
3. Create and fsync a timestamped sibling backup using SQLite's backup API.
4. Record backup metadata and source application version outside the database.
5. Begin an exclusive upgrade and create the schema-version metadata.
6. Run ordered SQL steps and typed Rust converters.
7. Rebuild external-content FTS tables from authoritative rows.
8. Run `foreign_key_check`, integrity checks, domain invariants, and row
   accounting.
9. Commit and atomically write the successful upgrade receipt.
10. Enqueue semantic rebuild; semantic artifacts are not copied as authority.

An interrupted transaction rolls back. An interruption after commit but before
receipt is recovered by inspecting the committed schema version and verification
markers; it does not rerun non-idempotent conversion.

## Terminology Conversion

Legacy strict terms convert into structured rule records, concept/facet links,
and searchable rule-crystal projections. The converter preserves:

- source and target languages;
- source form and canonical rendering;
- approved and forbidden aliases;
- alias language, kind, and case-sensitivity;
- tags, notes, status, provenance, and stable legacy identity.

Every source row has a ledger outcome: converted with target ids, deliberately
skipped with a bounded reason code, or blocking error. Row-count equality alone
is insufficient; deterministic validation fixtures must produce equivalent
findings before and after conversion.

## Other Conversion Rules

Existing series, sessions, memories, crystals, concepts, facets, proposals,
events, dream audit, RAG sources/chunks, language tags, story scopes, semantic
tags, and provenance are preserved according to the compatibility manifest.
The existing memory-graph ledger is retained and linked to the Rust upgrade
receipt.

Timestamp parsing accepts documented legacy formats and writes one RFC 3339 UTC
format going forward. Boolean and JSON text fields are validated before insert;
invalid values block or receive an explicit supported repair, never silent
coercion.

## Rollback And Retention

The receipt records backup checksum, prior application version, target version,
and migration report checksum. Rollback stops Rust, restores the verified backup,
and reinstalls the recorded Python release. The installer never deletes the
last verified pre-upgrade backup automatically.

## Acceptance Criteria

- Fresh, each supported legacy, current Python, corrupt, and interrupted-upgrade
  fixtures behave as specified.
- Dry-run performs no writes.
- Failure injection at every protocol step leaves either the original database
  or a verified committed target, never an ambiguous half-state.
- Foreign keys, FTS content, ledgers, and deterministic validation pass after
  conversion.
- Restoring the backup and running the recorded Python version is rehearsed.

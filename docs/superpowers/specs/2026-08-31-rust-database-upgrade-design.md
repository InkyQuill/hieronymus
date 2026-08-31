# Rust Database Upgrade Design

**Status:** Proposed for review on 2026-08-31.

## Goal

Open fresh Rust databases and upgrade supported Python databases without silent
data loss, split roots, partial conversion, or a false promise of Python
rollback after the one-way cutover.

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

The shared bounded `StateClassifier` only identifies schema/config/journal
state and is safe on every daemon start. `hiero migrate --dry-run` begins with
that result, then performs the full typed transformation and verification
against a disposable target database and staged in-memory config model. It
writes neither the source database nor authoritative backup, config, job, or
index state; temporary dry-run artifacts are outside the data root and removed
after reporting.

## Upgrade Protocol

1. Resolve and lock the explicit data root; refuse if the daemon is active.
2. Run joint database/config preflight and require a safe result.
3. Render, parse, cross-validate, fsync, and checksum staged current-format
   config files without promoting them.
4. Create and fsync a timestamped sibling database/config backup set.
5. Write a root-level cutover journal in `prepared` state with source/target and
   staged/backup checksums.
6. Begin an exclusive upgrade and create the schema-version metadata.
7. Run ordered SQL steps and typed Rust converters through the same transaction
   handle; converters cannot commit, open a second write connection, or mutate
   filesystem config.
8. Rebuild external-content FTS tables from authoritative rows.
9. Run `foreign_key_check`, integrity checks, domain invariants, and row
   accounting.
10. In the same transaction, create a durable semantic-rebuild job in
    `pending` state for the new authoritative generation; no live handoff is
    attempted while the daemon is stopped.
11. Commit once and set the cutover journal to `database_committed`.
12. Atomically promote staged config files and set the journal to `complete`.
13. Write the successful receipt. The daemon claims the pending job only after
    the cutover journal is `complete`; semantic artifacts are not copied as
    authority.

SQL steps, typed converters, FTS rebuild, authoritative row accounting, foreign
key checks, target schema-version write, and durable semantic-job creation
share one exclusive transaction.
The transaction commits once, after verification. An interrupted transaction
rolls back. An interruption after commit but before receipt is recovered by
inspecting the committed schema version and verification markers; it does not
rerun non-idempotent conversion.

The daemon starts only when the cutover journal is absent or `complete`. A crash
after database commit but before config promotion is an explicit
`config_promotion_required` state, not rollback: rerunning `hiero migrate`
verifies the staged checksums, resumes atomic file promotion, completes the
journal, and never reruns the committed database converters.

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

The database-upgrade track owns the target `term_rules` and `term_rule_forms`
schema plus conversion into it. `term_rules` stores lifecycle, scope, concept,
canonical rendering, matching policy, provenance, revision linkage, and the
optional advisory projection id. `term_rule_forms` stores source, approved, and
forbidden forms with language and case sensitivity. The typed converter records
legacy strict-term, alias, tag, concept, and rule-crystal ids alongside all
target ids. Runtime terminology code never migrates on read.

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

## Backup Retention And Rust Recovery

The receipt records backup checksum, prior application version, target version,
and migration report checksum. Python downgrade/reinstallation is unsupported
after a successful cutover. The installer never deletes the last verified
pre-upgrade backup automatically. Rust recovery copies that backup, imports it
with the current converter into a new Rust database, verifies the result, and
atomically promotes it; it never launches Python or opens the backup in place.

## Acceptance Criteria

- Fresh, each supported legacy, current Python, corrupt, and interrupted-upgrade
  fixtures behave as specified.
- Dry-run performs no writes.
- Failure injection at every protocol step leaves the original database, a
  verified committed target with explicit `config_promotion_required`, or a
  complete cutover; the daemon never starts in the middle state.
- Foreign keys, FTS content, ledgers, and deterministic validation pass after
  conversion.
- Importing the immutable backup through current Rust recovery tooling is
  rehearsed; no acceptance path depends on a Python runtime.

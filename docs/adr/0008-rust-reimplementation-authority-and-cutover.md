# Reimplement Hieronymus In Rust Through Contract-Gated Cutover

## Status

Accepted on 2026-08-31. This ADR supersedes the Python-authority paragraph in
[ADR 0005 §Decision](0005-product-vision.md#decision), specifically the decision
that Python remains authoritative for backend behavior. ADR 0005's product
model and non-language-specific boundaries remain current.

## Context

Hieronymus is currently a Python application with a Svelte web console, a local
SQLite database, CLI and MCP integrations, background dreaming, and managed
installation. The documents under `docs/rust-migration-proposal/` describe a
Rust replacement, but they mix durable product decisions, illustrative Rust
APIs, migration mechanics, and release sequencing. Several of those details
conflict with the current database and frontend contracts.

A language rewrite is not evidence of behavioral compatibility. Existing user
data and agent integrations must remain usable, and the Python implementation
must remain the behavioral reference until an explicit cutover gate passes.

## Decision

Reimplement Hieronymus as a Rust workspace and distribute it as one `hiero`
binary. Use an ADR-first, contract-gated program rather than treating the six
existing proposal documents as implementation-ready specifications.

This narrower, later ADR controls any conflict with ADR 0005's
Python-authority paragraph. The authority order during the migration is:

1. project instructions and ADR 0008's Rust replacement/cutover decision;
2. other accepted ADRs, with narrower later ADRs controlling explicit conflicts;
3. explicit compatibility specifications and versioned contract fixtures;
4. tests that implement those contracts;
5. current Python behavior where no higher-level decision changes it;
6. `docs/rust-migration-proposal/` as design input only.

The migration uses staged replacement:

1. snapshot current CLI, MCP, HTTP, configuration, and database contracts;
2. build independently testable Rust vertical slices;
3. run parity tests against fixed fixtures and copied databases;
4. rehearse one-way upgrade and failure recovery on production-shaped copies;
5. switch managed installation to Rust only after every release gate passes;
6. retain the immutable pre-upgrade backup for data recovery and forensic
   comparison, not as a supported Python runtime rollback path.

No mixed-language runtime is required after cutover. Temporary test harnesses
may invoke both implementations, but Rust and Python must not concurrently
mutate the same database.

## Cutover Gates

Cutover requires all of the following:

- every compatibility-manifest entry is implemented, intentionally changed by
  an accepted ADR, or explicitly removed;
- upgrade dry-run and real upgrade succeed on clean, minimal legacy, and
  production-shaped database fixtures;
- deterministic terminology tests pass independently of fuzzy recall tests;
- daemon authentication and lifecycle tests pass on every supported platform;
- Rust unit, integration, contract, and frontend tests pass;
- release artifacts install without Python, Node, or Bun on the target machine;
- one-way upgrade, pre-commit failure recovery, and backup data recovery are
  exercised against a release candidate.

## Consequences

The rewrite proceeds more slowly at the beginning because contracts and data
conversion are made explicit. It avoids a big-bang release whose failures would
be discovered only against a user's database or agent configuration.

The existing proposal documents must be reconciled with the accepted ADR/spec
set before they can be marked historical or removed. Implementation plans must
reference the normative specs, not proposal pseudocode.

Cutover is one-way at the product level. After a database is successfully
upgraded and the Rust release is activated, Python is not a supported runtime or
rollback target. Recovery tooling belongs to Rust and operates on the Rust
schema or imports data from the immutable pre-upgrade backup.

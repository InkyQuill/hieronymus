# 0008 — Rust runtime and one-way cutover

Status: accepted 2026-08-31; migration is complete. Consolidated 2026-10-05.
Supersedes ADR 0005's Python-backend authority. The old migration sequence and
qualification reports are history, not an ongoing Python parity release gate.

## Context

Replacing the backend language must preserve user data and public behavior.
A working compilation is not evidence that upgrades, terminology or agent
integrations still work. The earlier Python application supplied frozen reference
fixtures while the Rust runtime was built.

## Decision

Rust owns domain behavior, CLI, daemon, MCP and distribution. The Svelte console
remains the interactive frontend. Bun is a build/release-helper dependency;
Python is not part of the application, current build or release workflow.
Native GUI helpers and the local decision protocol retain explicit workspace
boundaries under [ADR 0009](0009-runtime-topology-and-daemon-lifecycle.md).

Keep frozen historical fixtures as reusable test inputs and versioned Rust
expectations for intentional changes. Public behavior is governed by accepted
product decisions and relevant tests, not by preserving a retired Python
implementation indefinitely. Do not silently edit historical expected outputs
to make a new implementation pass.

Managed schema cutover is one-way. Preserve immutable pre-upgrade backups;
unknown, corrupt, newer or unsafe sources fail before mutation. Rust recovery
uses current schemas or verified import of those backups. Never launch an older
binary against a newer schema or promise Python runtime rollback. Rust and
Python must not concurrently mutate one database.

## Consequences and verification

The old application is available on `stale/python-v0.7.0`; later migration tooling
has separate historical snapshots. [Archive policy](../archive/python-v0.7.0.md)
explains their provenance. Neither is a current deployment dependency.

Use focused behavior checks and the contributor checklist in AGENTS.md. Package,
real-model and native-host checks have distinct evidence boundaries; see
[runtime checks](../rust-cutover-rehearsal.md). Release failures follow the current
P0/advisory policy rather than obsolete cutover gates.

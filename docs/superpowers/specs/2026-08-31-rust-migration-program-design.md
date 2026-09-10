# Rust Migration Program Design

**Status:** Accepted on 2026-08-31.

> **Amendment (2026-09-03, owner):** certification-light process. Program
> Sequence step 2 is satisfied (qualification records for MCP transport,
> semantic native, frontend embedding, and legacy database import are merged;
> aggregate decision `qualified`, release mode `semantic-enabled`) and is no
> longer a gate for writing dependent implementation plans. Acceptance of
> implementation work is: ported tests green plus owner review of the diff.
> No recorded attestations, per-surface owner registry, or evidence gates are
> required. The `tools/qualification` records remain one-time archived
> evidence; nothing going forward builds on them.

## Purpose

Replace the Python backend with a Rust application without losing user data,
weakening deterministic terminology, or breaking CLI, MCP, HTTP, frontend, and
agent integration contracts.

This document is the map for the migration program. Detailed behavior is owned
by the linked specifications and ADRs. The documents under
`docs/rust-migration-proposal/` remain useful analysis but are not normative.

ADR 0008 supersedes ADR 0005's Python-authority paragraph and
controls that conflict. ADR 0011 controls deterministic rule authority, and ADR
0014 controls the interactive frontend. ADR 0005 continues to own the durable
product model only where those narrower later decisions do not supersede it.

## Governing Decisions

- [ADR 0008](../../adr/0008-rust-reimplementation-authority-and-cutover.md):
  contract-gated reimplementation and cutover.
- [ADR 0009](../../adr/0009-runtime-topology-and-daemon-lifecycle.md): one
  binary, explicit daemon/CLI/stdio processes, daemon-owned mutations.
- [ADR 0010](../../adr/0010-data-locations-schema-ownership-and-upgrade.md):
  existing data-root compatibility and typed database upgrade.
- [ADR 0011](../../adr/0011-deterministic-terminology-and-graded-memory.md):
  deterministic terminology separated from graded recall.
- [ADR 0012](../../adr/0012-mcp-transport-authentication-and-discovery.md):
  authenticated local transports and versioned discovery.
- [ADR 0013](../../adr/0013-semantic-index-and-platform-support.md): derived
  semantic index and empirically gated platforms.
- [ADR 0014](../../adr/0014-web-console-replaces-terminal-ui.md): Svelte web
  console replaces the retired terminal UI.
- [ADR 0015](../../adr/0015-mcp-protocol-and-transport.md): exact MCP revision,
  standard stdio, and Streamable HTTP transports.

## Specification Set

1. [Compatibility contracts](2026-08-31-rust-compatibility-contracts-design.md)
   defines the inventory and parity evidence.
2. [Data-root and config migration](2026-08-31-rust-data-root-config-migration-design.md)
   defines file locations, config conversion, credentials, and atomic promotion.
3. [Database upgrade](2026-08-31-rust-database-upgrade-design.md) defines
   preflight, conversion, verification, and one-way cutover recovery.
4. [Terminology and memory](2026-08-31-rust-terminology-memory-design.md)
   defines authoritative rules, recall, feedback, and reconsolidation.
5. [Dreaming](2026-08-31-rust-dreaming-design.md) defines phase boundaries,
   locking, bounded mutation, and audit.
6. [Semantic RAG](2026-08-31-rust-semantic-rag-design.md) defines authoritative
   RAG storage, jobs, generations, and fallback.
7. [Daemon, MCP, and security](2026-08-31-rust-daemon-mcp-security-design.md)
   defines lifecycle, discovery, authentication, and network contracts.
8. [Distribution and cutover](2026-08-31-rust-distribution-cutover-design.md)
   defines build artifacts, installation, one-way release rehearsal, and
   post-cutover Rust recovery.

## Architecture

The Rust workspace contains a domain library and an application binary. The
domain library owns typed models, validation, scoring, stores, migration
conversion, and provider-independent algorithms. The binary owns CLI parsing,
daemon lifecycle, transports, service installation, and presentation.

The daemon is the sole normal/live writer for SQLite and the semantic index.
Short-lived CLI and stdio processes communicate with it. Offline rebuild and
other exclusive maintenance commands may write only while the daemon is
stopped and they hold the data-root ownership lock; they use the same index
implementation, generation manifest, and durable job protocol as daemon
workers.

SQLite remains authoritative. FTS tables and the semantic index are rebuilt
from ordinary SQLite rows. Configuration and generated agent integrations are
local files governed by explicit compatibility contracts.

All credentials and secret-bearing configuration values cross domain boundaries
as `Secret<T>`. Redacted DTO projections are the only way they enter logging,
diagnostics, audit, CLI JSON, MCP, HTTP, or frontend serialization.

## Contract Ownership

Pavel Obruchnikov `<me@inkyquill.net>` is the acceptance owner for every public
compatibility surface until a manifest entry explicitly delegates another
named owner. Technical ownership is split by normative spec: data/config and
database upgrade; terminology/memory; dreaming; semantic RAG; daemon/MCP/
security; and distribution/cutover. Every manifest entry records both the named
acceptance owner and one of these technical owners.

## Program Sequence

1. ~~Freeze behavior in a machine-readable compatibility manifest and
   fixtures.~~ Done (2026-09-03): `compatibility/manifest.json`,
   `compatibility/snapshots/state.json`, frozen fixtures under
   `compatibility/`.
2. ~~Produce qualification records for MCP transport, semantic native
   dependencies, frontend embedding, and legacy database import before writing
   the dependent implementation plan.~~ Done (2026-09-03):
   `qualification/records/*` accepted, gate `qualified`. No longer a gate.
3. Build the Rust workspace and contract harness.
4. Implement upgrade tooling before any destructive cutover path.
5. Implement vertical product slices: configuration/series, terminology,
   memory/recall, RAG/semantic, dreaming, daemon/transports, frontend.
6. Produce native release artifacts and install them in clean environments.
7. Rehearse upgrade, normal use, pre-commit failure recovery, and Rust-only
   backup recovery (a manual checklist is sufficient; no recorded matrix).
8. Cut over managed installation when the owner is satisfied the port is
   correct.

## Global Invariants

- Approved active terminology cannot be weakened by fuzzy recall or passive
  scoring.
- Existing supported databases are never opened for mutation without a
  successful preflight and recoverable backup.
- A failed upgrade does not leave a database marked as upgraded.
- Normal mutations use one daemon-owned domain path across CLI, MCP, and web.
- Secret values do not appear in URLs, logs, discovery records, diagnostics, or
  frontend JSON; the `Secret<T>` type and redacted projections enforce this.
- Semantic retrieval failure degrades to FTS5 rather than failing recall.
- Every bounded background operation is resumable, auditable, or safely
  repeatable.
- Rust does not write source code into translation workspaces.

## Acceptance

Accepted 2026-08-31 (ADRs and child specifications reviewed together;
contradictions resolved). Amendment 2026-09-03: the "recorded owner and test
strategy per surface" requirement is waived; each child spec's Acceptance
Criteria section is read as the checklist of behaviors the ported Rust tests
must demonstrate, not as a recorded attestation.

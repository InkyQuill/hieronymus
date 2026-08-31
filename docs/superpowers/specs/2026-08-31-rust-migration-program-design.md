# Rust Migration Program Design

**Status:** Proposed for review on 2026-08-31.

## Purpose

Replace the Python backend with a Rust application without losing user data,
weakening deterministic terminology, or breaking CLI, MCP, HTTP, frontend, and
agent integration contracts.

This document is the map for the migration program. Detailed behavior is owned
by the linked specifications and ADRs. The documents under
`docs/rust-migration-proposal/` remain useful analysis but are not normative.

On acceptance, ADR 0008 supersedes ADR 0005's Python-authority paragraph and
controls that conflict. ADR 0005 continues to own the durable product model.

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

The daemon is the normal writer for SQLite and the sole writer for the semantic
index. Short-lived CLI and stdio processes communicate with it. Exclusive
maintenance commands acquire the data-root ownership lock before opening the
database for mutation.

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

1. Freeze behavior in a machine-readable compatibility manifest and fixtures.
2. Produce qualification records for MCP transport, semantic native
   dependencies, frontend embedding, and legacy database import before writing
   the dependent implementation plan.
3. Build the Rust workspace and contract harness.
4. Implement upgrade tooling before any destructive cutover path.
5. Implement vertical product slices: configuration/series, terminology,
   memory/recall, RAG/semantic, dreaming, daemon/transports, frontend.
6. Produce native release artifacts and install them in clean environments.
7. Rehearse upgrade, normal use, pre-commit failure recovery, and Rust-only
   backup recovery.
8. Cut over managed installation only after all gates in ADR 0008 pass.

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

The program design is accepted when the ADRs and child specifications have
been reviewed together, contradictions with current accepted ADRs are resolved,
and each compatibility surface has its named acceptance owner, technical owner,
and test strategy recorded. Detailed implementation plans are written only
after that review.

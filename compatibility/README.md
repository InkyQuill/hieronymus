# Compatibility inputs

This directory mixes production MCP definitions, current behavior-test fixtures
and frozen Python-port evidence. It is not disposable as a whole and is not a
standing Python parity release gate.

## What is still used

- `snapshots/mcp.json` is compiled into the daemon as the base tool registry.
- `rust/authority-context-v1.json` and `rust/authority-ingress-v1.json` are also
  compiled into the daemon and extend its advertised tool schemas.
- Current Rust tests load MCP/HTTP fixtures, the MCP snapshot and selected Rust
  expectations for recall, terminology, RAG, CWS and tool coverage.
- The remaining CLI/HTTP/state snapshots, manifests and legacy database fixtures
  preserve migration history; legacy database fixtures are consumed by the isolated
  qualification harness, not the ordinary workspace test suite.

The misleading directory name is a cleanup opportunity: production schemas should
eventually live beside the daemon and active test data beside its tests. Move them
only with their consumers updated and verified; preserve frozen historical bytes.
Current behavior is defined by accepted ADRs, public Rust schemas and behavior tests.

Do not regenerate frozen snapshots to match new output. For an intentional current
contract change, update its Rust expectation and owning ADR/guide with focused
behavior coverage. Fields saying `outstanding` or `first_rust_release:null` in the
historical manifest describe that freeze; they do not mean the runtime is still
unimplemented. Historical command/source references resolve in Git, not the active
Python-free checkout. See [archive policy](../docs/archive/python-v0.7.0.md).

## Current Rust expectations

`rust/` holds additive/versioned contracts for response deltas. Inspect the file
and its current owning test before relying on a shape; later additions need not
be captured by a historical prose inventory.

- Recall separates deterministic terminology from ranked memory and exposes
  semantic warnings. Ranked limits do not remove the deterministic contract.
- Strict RAG search requires a ready semantic lane; lexical-only results are
  not silently returned as a complete semantic search.
- RAG import reports indexing state (`queued`, `owed` or `not-required`) and
  the actual durable job ID when queued; enqueue failure stays visible.
- Browser authentication is optional; Host/Origin validation remains active.
  Launch grants use a URL fragment, are single-use and are removed before exchange.
- MCP `/mcp` keeps its exact revision; stdio has the explicit initialization
  negotiation bridge recorded in [ADR 0015](../docs/adr/0015-mcp-protocol-and-transport.md).
- Autonomous learned decisions and explicit-user corrections use evidence,
  scope and verified origin, not historical approval-wrapper labels.

[Authority ingress](../docs/authority-ingress.md),
[hook context](../docs/agent-hook-context.md) and
[business logic](../docs/business-logic.md) explain current integration behavior.
[CWS fixture provenance](rust/cws-project-v1.md) records the external structural
examples. Native/installed evidence belongs to [runtime checks](../docs/rust-cutover-rehearsal.md),
not to fixture counts.

Frozen source data and receipts remain unchanged during documentation cleanup.
The archived Python application and later migration orchestration are distinct
snapshots; do not install Python merely to replay commands copied from old reports.

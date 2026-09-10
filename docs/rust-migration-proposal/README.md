# Rust Migration Proposal (historical)

**Status: historical design input (2026-09-03).** Per
[ADR 0008](../../adr/0008-rust-reimplementation-authority-and-cutover.md) these
documents are not normative and mix durable decisions with illustrative APIs
and superseded details. The accepted ADR set (0008–0015) and the
`docs/superpowers/specs/2026-08-31-rust-*.md` specifications supersede them;
the reconciliation ADR 0008 required before marking them historical is
satisfied by that spec set. Do not implement from these documents; consult the
specs. Notable divergences already decided elsewhere: data-root paths and the
database filename are preserved (ADR 0010), SQLx migrations alone are not the
import protocol (ADR 0010), MCP transport is the pinned 2026-07-28 protocol
(ADR 0015), the terminal UI is replaced by the Svelte web console (ADR 0014).

# Rust Port Handoff (2026-09-03)

> Historical record: Python source and commands below belong to the
> [archived Python line](archive/python-v0.7.0.md). Current release acquisition uses
> `bun scripts/stage-release-assets.ts`; these records are not active release gates.

Handoff for continuing the Hieronymus Python→Rust rewrite on another
machine. Everything needed is in this repository; nothing lives outside it
except the git remote.

## How to work here (owner's process, binding)

- **Subagent-driven development** (superpowers SDD): controller dispatches an
  implementer subagent per task with a written brief, then an independent
  reviewer subagent over the diff; fix loop until Approved. Briefs/reports go
  under /tmp (ephemeral) — the brief for the NEXT task is embedded in this
  handoff.
- **Plain TDD port**: port tests from the Python suite first (RED — compile
  failure counts), implement until GREEN. Error messages match the Python
  reference verbatim where tests assert them.
- **No over-engineering**: no certification machinery, no records,
  fingerprints, gates, ledgers. Tests + owner review are acceptance.
- **No worktrees**: work directly on `feat/rust-rewrite-proposals`.
- Spec authorities: ADRs 0005–0015 and `docs/superpowers/specs/2026-08-31-rust-*.md`
  (certification trimmed 2026-09-03, auth lightened per ADR 0012 amendment —
  one static bearer token, loopback, Host/Origin, no rotation/CSRF ceremony).
- `docs/roadmap.md` — active slice order and deferred gaps.
- When the owner stops you, stop; when they say go — dispatch the next task
  from the queue below without asking again.

## Environment quirks (this machine's setup may differ)

- Invoke cargo ONLY as `env cargo ...` (the desktop sandbox clobbers argv[0]
  for direct rustup-proxy calls; `env cargo` fixes argv[0]). Example:
  `env cargo test --workspace`.
- Toolchain: Rust 1.96.0 pinned via root `rust-toolchain.toml`; target
  x86_64-unknown-linux-gnu only (ADR 0013).
- `uv` lives at `/home/inky/.hermes/bin/uv` if bare `uv` is missing (Python
  side only; the Rust work never needs it).
- Network is permitted for one-time dependency fetches; everything else
  offline. Frozen fixtures are never refetched or reformatted.
- Old broken path: `~/.bun` etc. are irrelevant; bun 1.4.0 is the amended
  prerequisite (frontend builds come much later).

## State (commit `HEAD` on `feat/rust-rewrite-proposals`)

All slices below are merged into this branch, tests green (currently 180
plus ~34 WIP, see Next Task):

- Slices 1–2 (storage complete): workspace skeleton, config files +
  `Secret<T>`, data root, series registry, sessions + short-term memories,
  concepts/facets, crystals, SQLite classify, `hiero version|classify`
  skeleton.
- Slice 3 (terminology core): `term_rules`/`term_rule_forms`, propose/approve
  lifecycle, deterministic contract + validate findings, context
  disambiguation (semantic tags / story scopes / languages).
- Slice 4 (recall): FTS-lane recall with boosts and active-rule protection,
  activations; RAG store (markdown/text/glossary import, checksum-aware
  re-import, FTS search) and the RAG recall lane with protected merge;
  DOCX/PDF ingestion hardened (panic containment, decompression cap).
- Slice 5 (dreaming core): run registry + phase records, OS flock cycle lock
  (no wait/retry), selection/persistence phases, bounded caps (tested),
  provider trait seam + deterministic fake, redacted audit, fail-closed
  rules; workflow fail-closed gate is a recorded TODO for the
  provider-client slice.
- In progress: daemon skeleton (see Next Task; a WIP commit exists on
  `wip/daemon-skeleton`).

Migration of Python `strict_terms` into `term_rules` belongs to the
`hiero migrate` slice (ADR 0010), not the runtime port.

## Deferred / recorded gaps

- DOCX/PDF ingestion: closed (zip+quick-xml, pdf-extract; hardened against
  malformed documents; inline DOCX formatting unwrapped to plain text).
- Dreaming: provider-client slice must add the workflow fail-closed gate
  (disabled/invalid workflows, provider resolution) — TODO in
  `DreamService::open`; reconsolidation, feedback consumption, scheduler
  timers are separate later slices.
- Concept recall boosts (`recall_boosts_for_crystals`) port with dreaming
  follow-ups.
- Legacy `vague`/`solid` statuses and Python `strict_terms` migration are
  upgrade-slice concerns.

## Next task (dispatch this first): daemon skeleton completion

A prior implementer was cancelled mid-task; its partial work is preserved on
branch `wip/daemon-skeleton` (WIP commit; 34/35 tests green — the failing
test is `route_case_failures_reproduce_the_frozen_http_contract`, incomplete
header-mismatch implementation). To continue: check out the WIP tree (e.g.
`git checkout wip/daemon-skeleton -- crates/ Cargo.toml Cargo.lock`), finish
it against the brief below, and land ONE commit on
`feat/rust-rewrite-proposals`.

The task brief (recreate /tmp briefs per session; content follows):

---
### Brief: daemon skeleton with MCP transports (slice 6, part 1)

Authorities: ADR 0009 (one binary, roles), ADR 0012 AS AMENDED (light auth:
loopback, one static bearer token 0600 CSPRNG, Host/Origin, no rotation/CSRF),
ADR 0015 (protocol exactly 2026-07-28), spec
2026-08-31-rust-daemon-mcp-security-design.md AS AMENDED.

Reuse: `qualification/harnesses/mcp-transport/` is the QUALIFIED protocol
reference (rmcp 3.1.4 pinned; header rules, -32020 HeaderMismatch,
-32022 UnsupportedProtocolVersion with exact requested/supported data,
stateless — no initialize/session, JSON or request-scoped SSE). Frozen
fixtures: `compatibility/fixtures/mcp/protocol.json`,
`compatibility/fixtures/http/route-cases.json`. Python registry snapshot:
`src/hieronymus/mcp_server.py`, `mcp_operations.py`.

Scope:
1. Discovery + credentials: verify DB opens (classify, fail closed),
   bind 127.0.0.1:9768 configurable (occupied port = error, no scan), CSPRNG
   bearer token 0600 in data root, atomic discovery record (protocol
   version, endpoint, pid, instance id, start time).
2. `hiero daemon`: foreground, graceful shutdown, loopback only.
3. HTTP: `/health` minimal unauthenticated; `POST /mcp` bearer-authenticated,
   JSON or request-scoped SSE, header rules per fixtures.
4. `hiero mcp` stdio adapter: NDJSON JSON-RPC proxy to the daemon, bounded
   stderr diagnostics, optional daemon start (config flag, default off
   documented).
5. Registry skeleton: `tools/list` full snapshot with `cacheScope: "private"`,
   `ttlMs: 0`, `resultType: "complete"`; `tools/call` table with
   `hieronymus_status` implemented; unported tools return a clean JSON-RPC
   error.
6. Tests from frozen fixtures: header mismatch, unsupported version, auth
   required, tools/list shape, tools/call round trip, stdio framing.
   In-process server tests (port 0) fine; no network egress.

Out of scope: REST admin/provider/settings, WebSocket, Svelte console, real
domain tool implementations beyond status, rotation, TLS.

Gates: full workspace tests green (216 baseline + additions), fmt, clippy 0,
one commit `feat: add daemon skeleton with mcp transports and discovery`.
---

## Task queue after the daemon

1. Daemon completion (above) → then REST routes (providers/settings/admin),
   WebSocket, embedded Svelte console (slice 6 part 2).
2. Dreaming provider-client slice (workflow fail-closed gate, real provider
   clients), reconsolidation, feedback consumption.
3. Recall boosts + RAG semantic lane (LanceDB per qualified pins).
4. `hiero migrate` (strict_terms → term_rules conversion, upgrade protocol).
5. Distribution, rehearsal, cutover.

## Rulings archive (owner decisions made during the port)

- Certification trimmed from specs (3fa0a2d); auth lightened (46771b1).
- bun 1.3.14 → 1.4.0 everywhere; `uv.lock` synced to 0.7.0 (646529c).
- Basic-metrics qualification (warm cargo target, no hour-long clean builds).
- Task 9 acquisition hardening independently re-reviewed and ACCEPTED.
- Owner accepted all four qualification records (881702f); gate is
  `qualified (semantic-enabled)`.

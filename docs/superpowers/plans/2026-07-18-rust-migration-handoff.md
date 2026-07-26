# Rust Migration Continuation Handoff

**Updated:** 2026-07-27

**Branch:** `feat/rust-rewrite`

**Worktree:** `/home/inky/Development/hieronymus/.worktrees/rust-rewrite`

**Reviewed implementation head:** `8cefbe2` (`fix: make dream liveness authoritative`)

## Stop Point

Phase 005, Service and MCP, is complete. Its task reviews, whole-phase review,
Rust-practices audit, and targeted lifecycle/status corrections are closed with
no remaining Critical, Important, or High findings. Continue in this same branch
and worktree; do not create another migration branch or worktree.

Only Phase 006 remains. Its next implementation unit is Task 1, Build and
Enforce the Parity Manifest.

## Sources of Truth

- `AGENTS.md` was updated at migration baseline commit `4bc815d` before Rust
  implementation began. It describes the rewrite, Rust verification chain, and
  Python's temporary role as the parity oracle.
- Proposals: `docs/rust-migration-proposal/001-initial-setup.md` through
  `docs/rust-migration-proposal/006-testing-and-deployment.md`.
- Plan index: `docs/superpowers/plans/2026-07-18-rust-migration-index.md`.
- Completed Phase 005 plan:
  `docs/superpowers/plans/2026-07-18-rust-migration-005-service-mcp.md`.
- Final plan:
  `docs/superpowers/plans/2026-07-18-rust-migration-006-parity-release.md`.

The CLI decision remains settled: use intuitive nested subcommands while
preserving the proposal's external command names. Do not flatten the command
enum.

Where proposal 005's REST table differs from the shipping
`frontend/src/web/lib/api.ts`, the current frontend is the compatibility source
of truth. The proposal's mention of a semantic-index worker is also ahead of the
available API: proposal 003 §5.3 explicitly says there is no concrete
job-to-index worker loop yet. Do not invent one during release work.

## Implemented and Reviewed

### Phase 001: Foundation — complete at `f4c1fb5`

- Rust 2024 workspace, XDG-aware configuration, canonical paths, nested CLI,
  series registry, agent assets, doctor diagnostics, and binary composition.

### Phase 002: Database and FTS — complete at `068173e`

- Safe SQLite/migration protocol, authoritative schema and trigger-owned FTS,
  typed rows, strict-term conversion, and cross-process migration locking.

### Phase 003: Memory and Recall — complete at `11ee6b8`

- Crystal/workspace stores, concepts, deterministic termbase, feedback, recall,
  RAG, rebuildable semantic retrieval, ingestion, and Phase 003 acceptance.

### Phase 004: Dreaming — complete at `6f7d9a4`

- Hardened provider/catalog/cache transports; complete typed dreaming phases;
  cross-process locking and audit lifecycle; bounded maintenance,
  reconsolidation, concept proposals, scheduler, and two-process exclusion.

### Phase 005: Service and MCP — complete at `8cefbe2`

- One Axum/Tokio daemon owns HTTP, WebSocket, Streamable HTTP MCP, and recurring
  dreaming. Binding is exact IPv4 loopback with Host/Origin/token enforcement,
  stable JSON errors, request IDs, body limits, and authorized shutdown.
- The current Svelte console's provider/settings/admin routes and response
  shapes are preserved. Provider catalog mutation is serialized without holding
  the lock across external network calls.
- Embedded frontend assets and the filesystem override use descriptor-anchored,
  no-follow traversal. Override files are capped at 16 MiB and growing files are
  bounded with `take(limit + 1)`.
- `/ws/admin` publishes production refresh events after successful provider,
  settings, admin, direct MCP, stdio-compat MCP, and dream transitions. Failed
  mutations are silent; lag requests a resync.
- rmcp 2.2 provides stateful Streamable HTTP MCP at `/mcp`. The catalog contains
  the exact 39 Python tool names plus `hieronymus_recall_feedback`; schemas and
  dispatch are covered for all 40 unique tools.
- `hiero mcp` owns one stdio MCP session and forwards decoded operations through
  authenticated `/api/mcp/{operation}`. The loopback client disables proxies
  and redirects and has connect/request deadlines.
- `hiero` with no subcommand starts the daemon in the foreground when absent or
  prints authenticated status when it is already running. Nested commands are
  unchanged.
- Ctrl+C/SIGTERM broadcast cancellation before Axum drains requests. Worker
  supervision owns recurring and one-shot jobs, surfaces fatal recurring
  failures, bounds graceful and abort/reap phases, and releases dream locks
  before terminal refresh events.
- Dashboard liveness is authoritative: a run is live only while the anchored OS
  dream lock is contended. Stale JSON or `dream_runs.status = 'running'` rows do
  not produce a false `WORKING` state.
- `hieronymus_series_init` is an exact MCP compatibility alias for series create;
  workspace file initialization remains a CLI-only boundary.

The final Phase 005 hardening sequence is:

1. `fe8f0be` — start the production dream scheduler and harden worker lifecycle.
2. `7eb30b5` — complete dashboard, events, provider serialization, alias,
   cleanup-deadline, and asset-bound contracts.
3. `52ab720` — close stale lock-state and stdio notifier gaps.
4. `8cefbe2` — make dashboard liveness and terminal refresh ordering
   authoritative.

Independent whole-phase review at `8cefbe2` reported `Ready: Yes`. The
independent Rust-practices audit reported `CLEAR — no Rust blocker`.

## Verification at the Stop Point

Fresh verification on the reviewed Phase 005 tree passed:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- `cargo test --workspace --all-targets --all-features --locked --no-fail-fast`;
- `cargo doc --workspace --no-deps --all-features --locked`;
- focused suites: service 51, MCP 23, process lifecycle 8, dreaming 35 passed
  with one intentional process helper ignored;
- `cd frontend && bun run typecheck && bun run test && bun run build && bun run format`
  (4 files, 16 tests);
- `git diff --check 48200c1..HEAD`;
- `rg -n 'ServiceManager|Popen|server.json|PID file|port allocation' crates`
  returned no legacy lifecycle matches.

The Phase 005 execution ledger and reports are retained under:

- `.superpowers/sdd/2026-07-18-rust-migration-005-service-mcp/progress.md`;
- `.superpowers/sdd/2026-07-18-rust-migration-005-service-mcp/task-1-report.md`
  through `task-5-report.md`;
- `.superpowers/sdd/2026-07-18-rust-migration-005-service-mcp/final-fix-report.md`.

## Deferred Non-Blocking Work

- Classify an unexpected early `Ok(())` from a recurring worker as fatal when a
  second concrete recurring worker makes the generalized policy useful.
- Replace timing-sensitive polling in the pre-existing dream cancellation test
  with an injected persistence latch.
- Retain test `TempDir`s instead of leaking kept paths; broaden seeded negative
  coverage for remaining admin/MCP branches; add less common font MIME types.
- Move series initialization's blocking filesystem work behind an async-safe
  boundary where it remains used by the direct CLI.
- Improve `hiero-core` public API documentation, introduce missing-docs
  enforcement gradually, and decide which public error enums should become
  `#[non_exhaustive]`.
- Earlier Phase 004 extreme-scale consolidation notes remain in its retained
  progress ledger and do not block Phase 006.

## Exact Next Step

Resume from the worktree and start Phase 006 Task 1:

```bash
cd /home/inky/Development/hieronymus/.worktrees/rust-rewrite
task-brief docs/superpowers/plans/2026-07-18-rust-migration-006-parity-release.md 1
```

Use `superpowers:subagent-driven-development` and follow TDD. Build the manifest
from every tracked `tests/test_*.py`; every entry must map to concrete Rust or
mounted frontend tests, or carry an exact allowed legacy-exclusion rationale.

Continue in dependency order:

1. Phase 006 Task 2: cross-cutting Rust parity fixtures and suites.
2. Task 3: replace frontend source-grep tests with mounted behavior tests.
3. Task 4: native CI and cargo-dist for four targets.
4. Task 5: binary installers and artifact smoke tests.
5. Task 6: only after every gate and rollback reference pass, remove Python.
6. Final whole-branch review and artifact-level acceptance verification.

## Environment Notes

- The Windows GNU target cannot compile project code here because
  `x86_64-w64-mingw32-gcc` is not installed. Phase 006 requires native Windows
  MSVC CI rather than treating this local limitation as release evidence.
- Git commits in this linked worktree may need approval because the index and
  worktree metadata live under the main repository's `.git/worktrees`.
- Local TCP-bind tests can receive sandbox `EPERM`; rerun them with the approved
  `cargo test` permission rather than weakening tests.
- The `.superpowers/sdd` workspace is intentionally retained for continuation.

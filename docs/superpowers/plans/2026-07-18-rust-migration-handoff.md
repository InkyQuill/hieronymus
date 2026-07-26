# Rust Migration Continuation Handoff

**Updated:** 2026-07-26

**Branch:** `feat/rust-rewrite`

**Worktree:** `/home/inky/Development/hieronymus/.worktrees/rust-rewrite`

**Reviewed implementation head:** `6f7d9a4` (`fix: finish bounded dreaming maintenance`)

## Stop Point

Phase 004, Dreaming, is complete. Its final whole-phase review and both narrowly
authorized hardening waves are closed with no remaining Critical, Important, or
High findings. Continue in this same branch and worktree; do not create another
migration branch or worktree.

The next implementation unit is Phase 005 Task 1, Build App State, Router, and
Security Layers.

## Sources of Truth

- `AGENTS.md` was updated at migration baseline commit `4bc815d` before Rust
  implementation began. It describes the rewrite, Rust verification chain, and
  Python's temporary role as the parity oracle.
- Proposals: `docs/rust-migration-proposal/001-initial-setup.md` through
  `docs/rust-migration-proposal/006-testing-and-deployment.md`.
- Plan index: `docs/superpowers/plans/2026-07-18-rust-migration-index.md`.
- Completed Phase 004 plan:
  `docs/superpowers/plans/2026-07-18-rust-migration-004-dreaming.md`.
- Next plan:
  `docs/superpowers/plans/2026-07-18-rust-migration-005-service-mcp.md`.
- Final plan:
  `docs/superpowers/plans/2026-07-18-rust-migration-006-parity-release.md`.

The CLI decision is settled: use intuitive nested subcommands while preserving
the proposal's external command names. Do not flatten the command enum.

## Implemented and Reviewed

### Phase 001: Foundation — complete at `f4c1fb5`

- Rust 2024 workspace and locked quality gates.
- XDG-aware typed configuration and canonical paths.
- Complete nested CLI grammar and snapshots.
- Series registry, agent/skill assets, doctor diagnostics, and binary
  composition skeleton.

### Phase 002: Database and FTS — complete at `068173e`

- Safe SQLite connection and migration protocol.
- Authoritative schema, trigger-owned FTS, and typed row models.
- Transactional strict-term conversion and legacy-table removal.
- Cross-process migration locking with hardened file identity checks.

### Phase 003: Memory and Recall — complete at `11ee6b8`

- Crystal and workspace-memory stores.
- Concepts, deterministic termbase, feedback scoring, and recall activation.
- RAG parsing/import/FTS search and bounded conversion.
- Rebuildable semantic retrieval, queue leases/heartbeats, and hybrid recall.
- Ingestion configuration and the complete Phase 003 gate.

### Phase 004: Dreaming — complete at `6f7d9a4`

- Provider catalog, discovery/cache behavior, hardened transports, proxy policy,
  secret redaction, deterministic local provider, and typed phase prompts.
- Strict dream configuration, workflow resolution, bounded safe persistence,
  and provider/model default resolution.
- Cross-process cycle locking, transactional and redacted audit lifecycle,
  cancellation-safe cleanup, and resumable cycle stages.
- Typed parsing and sequential execution for all provider phases.
- Atomic reconsolidation, passive and provider reinforcement, pair combination,
  supersession, bounded indexed decay, and retry-safe persistence.
- Dream service, due-cycle scheduler, graceful in-flight shutdown, aggregate
  unique-crystal budgets, and true two-process exclusion coverage.
- Durable reviewable concept-merge proposals and bounded, resumable, live
  duplicate consolidation with concurrency-safe durable budget admission.
- New migrations `0009` through `0013` add the maintenance index, unique cycle
  maintenance events, merge proposals, consolidation scan state, dirty queues,
  and durable affected-id accounting without rewriting prior migrations.

The final hardening sequence is:

1. `f40483b` — complete normal-cycle integration and deterministic provider.
2. `502fc21` — make aggregate affected-id accounting and consolidation atomic.
3. `6f7d9a4` — finish bounded/live consolidation and concurrent admission.

Independent final review at `6f7d9a4` reported the phase ready. The independent
Rust/SQLite audit reported no Critical or High findings.

## Verification at the Stop Point

Fresh controller verification at `6f7d9a4` passed:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- the focused Phase 004 suite: 168 passed, 3 intentional helper tests ignored;
- `cargo test --workspace --all-targets --all-features --locked --no-fail-fast`;
- `cargo doc --workspace --no-deps --all-features --locked`;
- `git diff --check 11ee6b8..HEAD`.

The focused suite includes true two-process service exclusion, cancellation and
audit-cleanup paths, scheduler completion of unfinished consolidation,
concurrent durable budget admission, Unicode casefold parity, and
`EXPLAIN QUERY PLAN` assertions for the named maintenance indexes. Provider
tests use deterministic or injected local transports; they perform no real
external model calls.

The Phase 004 execution ledger and final Task 6 report are retained at:

- `.superpowers/sdd/2026-07-18-rust-migration-004-dreaming/progress.md`
- `.superpowers/sdd/2026-07-18-rust-migration-004-dreaming/task-6-report.md`

## Deferred Non-Blocking Hardening

- If a partially emitted consolidation group's target is deleted, source-cursor
  repair can wait until the key cursor wraps. A later hardening pass can persist
  the active target id or invalidate that group immediately.
- Dirty-group exclusion has no collation-aware raw-key index. At unusually
  large dirty-backlog scale, add an index or persist an indexed normalized dirty
  key.
- Earlier Task 5/6 minor notes remain in the retained progress ledger. They are
  cleanup, observability, or extreme-scale improvements and do not block Phase
  005.

## Exact Next Step

Resume from the worktree and start Phase 005 Task 1:

```bash
cd /home/inky/Development/hieronymus/.worktrees/rust-rewrite
task-brief docs/superpowers/plans/2026-07-18-rust-migration-005-service-mcp.md 1
```

Use `superpowers:subagent-driven-development` and follow the task's TDD order.
Keep HTTP, WebSocket, frontend, and MCP contracts grounded in the migration
proposal and existing Python parity tests. After every task, review the complete
task range and rerun the relevant Rust gates before moving forward.

Then continue in dependency order:

1. Phase 005 Tasks 2–5: admin/settings contracts, WebSocket and embedded
   frontend, MCP tools, stdio shim, and graceful lifecycle.
2. Phase 006: parity manifest, cross-cutting suites, native release, and only
   then removal of the Python runtime.
3. Final whole-branch review and full acceptance verification.

## Environment Notes

- The Windows GNU target reaches native dependencies but cannot compile project
  code here because `x86_64-w64-mingw32-gcc` is not installed. Keep structural
  Windows coverage and run a real Windows/native-target gate when that toolchain
  is available.
- Git commits in this linked worktree may need approval because the index and
  worktree metadata live under the main repository's `.git/worktrees` directory.
- Some local TCP-bind tests can receive sandbox `EPERM`; rerun those focused
  tests with the required approval rather than weakening them.
- `.superpowers/sdd` reports, review packages, and progress are intentionally
  retained local execution artifacts. This tracked file is the durable
  continuation handoff.

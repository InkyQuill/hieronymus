# Rust Migration Continuation Handoff

**Recorded:** 2026-07-18

**Branch:** `feat/rust-rewrite`

**Worktree:** `/home/inky/Development/hieronymus/.worktrees/rust-rewrite`

**Reviewed implementation head:** `b14cb45` (`fix: close dream audit allocation gaps`)

## Stop Point

Stop after Phase 004 Task 3, Cross-Process Lock and Audit Lifecycle. Its complete
diff from `1d0a531` through `b14cb45` received the independent result:

> No findings; spec compliant; quality approved.

Phase 004 Task 4 has not started. Continue in the same branch and worktree; do
not create another migration branch or worktree.

## Sources of Truth

- `AGENTS.md` was updated at migration baseline commit `4bc815d` before Rust
  implementation began. It describes the rewrite, Rust verification chain, and
  Python's temporary role as the parity oracle.
- Proposals: `docs/rust-migration-proposal/001-initial-setup.md` through
  `docs/rust-migration-proposal/006-testing-and-deployment.md`.
- Plan index: `docs/superpowers/plans/2026-07-18-rust-migration-index.md`.
- Active plan: `docs/superpowers/plans/2026-07-18-rust-migration-004-dreaming.md`.
- Later plans: `2026-07-18-rust-migration-005-service-mcp.md` and
  `2026-07-18-rust-migration-006-parity-release.md` in the same directory.

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

### Phase 004: Dreaming — Tasks 1–3 complete

- **Task 1, `38b5e2f`:** provider catalog and adapters, current flat-config
  compatibility, transport seams, discovery/cache behavior, proxy policy,
  secret redaction, and pass-name validation.
- **Task 2, `1d0a531`:** strict dream configuration/workflow resolution and
  symmetric bounded save/load behavior.
- **Task 3, `b14cb45`:** shared Unix/Windows file identity, securely anchored
  cross-process dream locking, identity-safe cleanup, bounded/redacted audit
  lifecycle, nonnegative lifecycle validation, and schema/tests.

Task 3 spans these commits:

1. `c22fdd8` — `feat: add safe dream-cycle locking and audit`
2. `b184445` — `fix: harden dream lock identity and audit bounds`
3. `b14cb45` — `fix: close dream audit allocation gaps`

## Verification at the Stop Point

The final Task 3 implementation passed:

- focused dream-lock tests: 25 passed, 2 intentional child-process helpers
  ignored;
- JSON encoded-length budget unit test;
- adjacent database tests: 17 passed;
- legacy migration-lock tests: 19 passed;
- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- `cargo test --workspace --all-features --locked`;
- `cargo doc --workspace --no-deps --all-features --locked`;
- `uv run --frozen pytest`: 1,193 passed;
- `uv run --frozen ruff check .`;
- `uv run --frozen ruff format --check .`.

The complete final review package is an ignored local artifact at
`.superpowers/sdd/review-1d0a531..b14cb45.diff`. The Task 3 implementation report
is `.superpowers/sdd/phase004-task-3-report.md`, and the local progress ledger is
`.superpowers/sdd/progress.md`.

## Exact Next Step

Start Phase 004 Task 4, Phase Parsing and Execution, from the reviewed Task 3
head plus this handoff commit:

```bash
task-brief docs/superpowers/plans/2026-07-18-rust-migration-004-dreaming.md 4
```

Use `superpowers:subagent-driven-development` with a fresh implementer and an
independent reviewer. Follow the task's TDD order. Keep phase tests deterministic
and do not make real network/model calls. After implementation, generate a
complete base-to-head review package, fix every Critical/Important finding and
useful Minor finding, regenerate the complete package, and repeat review until
it is clean.

Then continue in dependency order:

1. Phase 004 Task 5: Reconsolidation, Reinforcement, and Decay.
2. Phase 004 Task 6: Dream Service and Background Scheduler.
3. Phase 005: service, HTTP/WebSocket/frontend contracts, MCP, and lifecycle.
4. Phase 006: parity manifest, cross-cutting suites, native release, and only
   then removal of the Python runtime.
5. A final whole-branch review and full acceptance verification.

## Environment Notes

- The Windows GNU target reaches native dependencies but cannot compile project
  code here because `x86_64-w64-mingw32-gcc` is not installed. Keep structural
  Windows coverage and run a real Windows/native-target gate when that toolchain
  is available.
- Git commits in this linked worktree may need approval because the index and
  worktree metadata live under the main repository's `.git/worktrees` directory.
- `uv run --frozen` currently rewrites only the editable Hieronymus entry in
  `uv.lock` from `0.4.0` to workspace version `0.6.0`; restore that incidental
  line after Python verification until the planned version transition owns it.
- Some local TCP-bind tests can receive sandbox `EPERM`; rerun those focused
  tests with the required approval rather than weakening them.
- The full Python parity suite currently takes roughly two minutes.
- `.superpowers/sdd` reports, review packages, and progress are intentionally
  ignored local execution artifacts. The present file is the tracked handoff.

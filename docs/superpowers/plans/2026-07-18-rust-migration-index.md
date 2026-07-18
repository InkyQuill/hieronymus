# Rust Migration Plan Suite

This index maps the six standalone proposal documents to dependency-ordered implementation plans. Execute them in numeric order; each phase must pass its acceptance gate before the next phase starts. Keep the Python implementation and its passing 1,193-test baseline until Phase 006's final removal gate.

| Proposal coverage | Implementation plan | Independent deliverable |
|---|---|---|
| 001 §§1-2, 8-9 | `2026-07-18-rust-migration-001-foundation.md`, Task 1 and Task 6 | Compiling, linted two-crate workspace and composition root |
| 001 §§3, 7 | Phase 001, Task 2 and Task 6 | XDG-aware configuration and typed diagnostics |
| 001 §§4-6 | Phase 001, Tasks 3-5 | Frozen CLI, series registry, agent/skill integration |
| 002 §§1, 4 | `2026-07-18-rust-migration-002-database-fts.md`, Tasks 1 and 5 | Safe pool and lossless legacy conversion |
| 002 §§2-3 | Phase 002, Tasks 2-3 | Authoritative schema and trigger-owned FTS |
| 002 §§5-6 | Phase 001 Task 2 plus Phase 002 Task 4 | Canonical helpers and typed row models |
| 003 §§1-2 | `2026-07-18-rust-migration-003-memory-recall.md`, Tasks 1-4 | Domain stores, termbase, and scoring authority |
| 003 §3 | Phase 003, Task 5 | Deterministic multi-lane recall and activation feedback |
| 003 §§4, 6 | Phase 003, Tasks 6 and 8 | RAG and ingestion services |
| 003 §5 | Phase 003, Task 7 | Rebuildable semantic indexing and hybrid rank fusion |
| 004 §§1-3, 8-9 | `2026-07-18-rust-migration-004-dreaming.md`, Tasks 1-4 | Providers, configuration, lock/audit, and phases |
| 004 §§4-5 | Phase 004, Task 5 | Reconsolidation, reinforcement, and bounded decay |
| 004 §§6-7 | Phase 004, Task 6 | Dream service and non-overlapping scheduler |
| 005 §§1-3 | `2026-07-18-rust-migration-005-service-mcp.md`, Tasks 1, 4, and 5 | Daemon and both MCP transports with 40 tools |
| 005 §§4-5 | Phase 005, Tasks 2-3 | Frontend-compatible REST/WebSocket/assets |
| 006 §§1-2 | `2026-07-18-rust-migration-006-parity-release.md`, Tasks 1-3 | Auditable backend/frontend parity |
| 006 §§3-4 | Phase 006, Tasks 4-5 | Native release artifacts and simple installers |
| 006 §5 | Phase 006 acceptance plus Task 6 | Explicit future-work boundary and final Python removal |

## Resolved Mechanical Corrections

The plans preserve proposal behavior while correcting these execution mechanics:

1. `sqlx::migrate!` resolves its path where the macro is compiled, so the single migrator is defined in `hiero-core`; it does not change paths based on `hiero-bin` calling it.
2. A plain SQL migration cannot invoke `strict_term_to_crystal`. Phase 002 runs the structured conversion and parity checks in a Rust-owned immediate transaction before the guarded legacy-drop migration.
3. SQLx 0.9 requires Rust 1.94, so the workspace and `AGENTS.md` use that exact floor rather than an unspecified stable compiler.
4. Current transport/library pins used for planning are `rmcp 2.2`, Axum 0.8, clap 4.6, LanceDB 0.30, and `ort 2.0.0-rc.12`. Phase 001 commits the lockfile; upgrades require rerunning the affected contract tests rather than silently floating APIs.
5. cargo-dist may cross-compile by default, but ORT and LanceDB have native dependencies. Phase 006 explicitly assigns native target runners and validates the generated plan.

## Execution Rule

Use one branch/worktree for the program, but preserve each task's commit boundary. At every phase gate, run the focused tests first, then the full Cargo verification chain and any named frontend/Python parity commands. Do not begin Phase 006 removal until the parity manifest reports complete coverage and release artifacts have passed smoke tests.

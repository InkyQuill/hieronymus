# Rust Migration Phase 006 Parity and Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove behavioral parity, replace source-grep frontend tests, ship native binaries for four targets, simplify installation, and remove the superseded Python runtime only after all gates pass.

**Architecture:** A fixture manifest maps every Python test module to Rust unit/integration or mounted frontend coverage. CI separates fast deterministic checks from native semantic-index smoke and per-platform release jobs. Removal is the final reversible commit after artifact-level smoke tests succeed.

**Tech Stack:** Cargo test/clippy/fmt/doc, Vitest, Testing Library Svelte, Bun 1.3.14, cargo-dist, GitHub Actions, shell/PowerShell installers.

## Global Constraints

- Reproduce coverage of every `tests/test_*.py`; document intentional legacy exclusions with exact rationale.
- CI performs no provider network calls or model downloads in deterministic tests.
- Release builds run natively on Linux, macOS x86/arm64, and Windows x86_64.
- All `reqwest` usage uses rustls; artifacts require checksums.
- `cargo build --release` is the single release build recipe and embeds a prebuilt frontend.
- Do not delete Python sources/tests until fixture mapping, Rust/frontend suites, artifact smoke, and rollback tag all pass.

---

## File Map

- `tests/parity/manifest.toml` and fixtures: auditable Python-to-Rust coverage map.
- Rust integration tests named in proposal 006 §1.2.
- `frontend/src/**/*.test.ts`: mounted behavior tests.
- `.github/workflows/{pr,release}.yml`, `dist-workspace.toml`: verification and native releases.
- `install.sh`, `install.ps1`, `uninstall.sh`: binary lifecycle.
- Python package/build files: removed only in the last task.

**Focused commands:** Task 1 `cargo test -p hiero-bin --test parity_manifest`; Task 2 `cargo test --workspace --all-features --locked`; Task 3 `(cd frontend && bun run test && bun run typecheck && bun run format && bun run build)`; Task 4 `cargo dist plan`; Task 5 `cargo test -p hiero-bin --test installer && scripts/smoke-release.sh`; Task 6 runs every preceding GREEN command plus the workspace fmt/clippy/doc chain. RED means the targeted new assertion fails before implementation; GREEN means every command exits 0.

### Task 1: Build and Enforce the Parity Manifest

**Files:** Create `tests/parity/manifest.toml`, `crates/hiero-bin/tests/parity_manifest.rs`, `docs/rust-parity.md`.

**Interfaces:** Each entry has `python_test`, non-empty `rust_tests`, `status = "ported"|"legacy-excluded"`, and `rationale` required only for exclusions.

- [ ] Generate the initial manifest from tracked `tests/test_*.py`; write tests that fail for missing/duplicate/stale entries, nonexistent Rust targets, empty assertions, or exclusions without rationale.
- [ ] Run `cargo test -p hiero-bin --test parity_manifest`; expect RED until every current Python file is classified.
- [ ] Map each file to concrete Rust/frontend test names. Allowed exclusions are only dropped subprocess/service-state/TUI protocol/release-Python mechanics and source-grep frontend tests, each pointing to its replacement behavior test.
- [ ] Run manifest test; expect GREEN. Commit `test: inventory Python parity coverage`.

### Task 2: Complete Cross-Cutting Rust Parity Suites

**Files:** Complete all Rust integration files listed in proposal 006 §1.2; create `tests/fixtures/reference/` datasets and expected JSON.

**Interfaces:** Produces deterministic reference outputs for rule extraction, termbase findings, concept graph, recall lane membership/order, and RRF tolerance.

- [ ] Export fixed expected results from the passing Python baseline into reviewed JSON fixtures without importing Python at Rust test runtime.
- [ ] Add missing negative/error/concurrency assertions until every manifest entry is ported; use fake providers/embeddings and temp LanceDB.
- [ ] Run each integration file individually while developing, then `cargo test --workspace --all-features --locked`; expect GREEN and no network.
- [ ] Commit in reviewer-sized domain groups, ending with `test: complete Rust behavioral parity`.

### Task 3: Replace Frontend Source-Grep Tests

**Files:** Modify `frontend/src/web/app.test.ts` and affected component tests; modify `frontend/vitest.config.ts` if needed.

**Interfaces:** Frontend tests interact through rendered roles/labels and mocked API/WebSocket boundaries.

- [ ] Replace every raw `.svelte` file read and `toContain` assertion with Testing Library render, accessible query, user event, and visible state/API assertion. Cover loading, success, validation, server error, reconnect/resync, and destructive confirmation.
- [ ] Run `cd frontend && bun run test`; expect RED before each behavior is wired and GREEN after.
- [ ] Run `bun run typecheck`, `bun run format`, and `bun run build`; expect GREEN. Commit `test: verify console through mounted behavior`.

### Task 4: Configure Native CI and cargo-dist

**Files:** Modify `.github/workflows/pr.yml`, `.github/workflows/release.yml`; create `dist-workspace.toml`, `.bun-version`; modify workspace/build metadata.

**Interfaces:** Produces PR jobs `rust`, `python-parity`, `frontend`; release artifacts for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc` plus SHA-256 checksums.

- [ ] Add workflow validation tests or `actionlint`; assert release jobs use native target-appropriate runners, cache by lockfiles, pin Bun 1.3.14 once, build frontend once per job, then run `cargo build --release --locked`.
- [ ] Run `cargo dist plan` and inspect that all four targets/artifacts/installers/checksums are present; expect failure until configuration is complete.
- [ ] Configure target runners explicitly rather than relying on cargo-dist cross defaults, because ORT/LanceDB carry native dependencies. Pin third-party actions to immutable commits in release workflow.
- [ ] Run PR-equivalent commands locally and `cargo dist plan`; expect GREEN. Commit `ci: add native Rust release pipeline`.

### Task 5: Replace Installers and Smoke-Test Artifacts

**Files:** Rewrite `install.sh`; create/update `install.ps1`; modify `uninstall.sh`; create `scripts/smoke-release.sh`, `crates/hiero-bin/tests/installer.rs`.

**Interfaces:** Installer detects supported target, downloads versioned archive/checksum from GitHub Releases, verifies checksum, atomically installs `hiero`, then runs `hiero doctor`.

- [ ] Test target mapping, unsupported target, HTTP/checksum/archive failure, existing binary replacement/rollback, non-interactive execution, install path override, uninstall, and doctor failure using a local fixture server/archive.
- [ ] Run installer tests; expect RED.
- [ ] Implement POSIX and PowerShell installers with `set -eu`/terminating errors, temporary directories, checksum verification before extraction, atomic replacement, and cleanup traps. No Python/Node/Bun prerequisite or TTY prompt.
- [ ] Build release locally, install from fixture archive into a temp prefix, run `--version`, `doctor --json`, start/health/stop, MCP initialize/tool list, and one SQLite write/read. Expect GREEN. Commit `build: simplify binary installation`.

### Task 6: Remove the Python Runtime at the Final Gate

**Files:** Delete `src/hieronymus/`, Python `tests/`, `pyproject.toml`, `uv.lock`, `hatch_build.py`, obsolete release/service scripts; update `README.md`, docs, `AGENTS.md`, CI, changelog.

**Interfaces:** Leaves `hiero` as the only supported runtime and Cargo as the release authority; retains parity fixtures/manifest and historical migration documentation.

- [ ] Create a signed or annotated rollback tag/commit reference before removal and record it in `docs/rust-parity.md`.
- [ ] Run the complete Rust/frontend/artifact gate once with Python still present and archive its results in the PR description; do not proceed on any failure.
- [ ] Remove only files proven obsolete by the manifest and `rg` references. Update commands, installation, architecture, paths, MCP setup, and contributor verification in all live docs.
- [ ] Run `rg -n 'uv run|pytest|src/hieronymus|hatchling|python-semantic-release|hieronymus-mcp' README.md AGENTS.md docs .github install.sh uninstall.sh`; expected matches are only clearly labeled historical migration notes or compatibility command descriptions.
- [ ] Run final Cargo verification, frontend test/typecheck/format/build, installer/artifact smoke, `cargo dist plan`, and `git status --short`; expect GREEN with only intentional tracked changes.
- [ ] Commit `refactor!: complete Rust rewrite`.

## Phase Acceptance

- [ ] Parity manifest covers 100% of the pre-removal Python test files.
- [ ] Rust, frontend, native semantic smoke, and all four release jobs pass.
- [ ] Installed artifacts work without Python, Node, or Bun on the target machine.
- [ ] The final documentation and `AGENTS.md` contain only Rust-era commands and architecture.

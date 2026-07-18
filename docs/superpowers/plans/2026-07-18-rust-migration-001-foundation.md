# Rust Migration Phase 001 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the two-crate Rust workspace, configuration, complete CLI grammar, series registry, agent integration, diagnostics, and binary composition required by later migration phases.

**Architecture:** `hiero-core` owns configuration and domain-facing services without CLI or HTTP dependencies. `hiero-bin` owns `clap`, process composition, presentation, and later transports. The Python package remains unchanged as the parity oracle until Phase 006 removes it.

**Tech Stack:** Rust 2024, Cargo, Tokio, clap 4.6, serde, sqlx 0.9, anyhow, thiserror, tracing, rust-embed.

## Global Constraints

- Full parity, no legacy: do not create compatibility tables or migration-on-read behavior.
- SQLite remains authoritative; derived indexes must be rebuildable.
- Deterministic terminology must outrank fuzzy and semantic evidence.
- Ship one `hiero` binary; `hiero mcp` is a temporary compatibility shim.
- Set workspace `rust-version = "1.94"`, the SQLx 0.9 MSRV, and commit `Cargo.lock`.
- Library APIs use typed `thiserror` errors; the binary adds `anyhow::Context`.
- Pure parsing and normalization stay synchronous; async is reserved for I/O.
- Verification for every task: `cargo fmt --all -- --check`, focused tests, and `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.

---

## File Map

- `Cargo.toml`: workspace members, shared package metadata, dependencies, and lint policy.
- `crates/hiero-core/src/config.rs`: XDG/data-root/port resolution and derived paths.
- `crates/hiero-core/src/values.rs`: canonical UTC, clamping, normalization, and JSON helpers.
- `crates/hiero-core/src/registry.rs`: series CRUD contract over `SqlitePool`.
- `crates/hiero-core/src/agent/`: plugin discovery, safe config patching, and embedded skills.
- `crates/hiero-core/src/doctor.rs`: typed diagnostic checks without terminal formatting.
- `crates/hiero-bin/src/cli/`: clap-only command grammar and dispatch boundary.
- `crates/hiero-bin/src/main.rs`: explicit composition root.

**Focused commands:** Task 1 `cargo test -p hiero-bin --test smoke`; Task 2 `cargo test -p hiero-core config && cargo test -p hiero-core values`; Task 3 `cargo test -p hiero-bin --test cli_schema`; Task 4 `cargo test -p hiero-core --test registry`; Task 5 `cargo test -p hiero-core --test agent`; Task 6 `cargo test -p hiero-core --test doctor`. RED means the named test fails for the missing interface under construction; GREEN means exit 0 with all named tests passed.

### Task 1: Bootstrap the Workspace and Quality Gates

**Files:** Create `Cargo.toml`, `Cargo.lock`, `build.rs`, `crates/hiero-core/Cargo.toml`, `crates/hiero-core/src/lib.rs`, `crates/hiero-bin/Cargo.toml`, `crates/hiero-bin/src/main.rs`; modify `.gitignore`, `.github/workflows/pr.yml`.

**Interfaces:** Produces crates `hiero_core` and binary `hiero`; no domain interfaces yet.

- [ ] Write `crates/hiero-bin/tests/smoke.rs` using `assert_cmd::Command::cargo_bin("hiero")` and assert `--version` succeeds with stdout containing `hiero 0.6.0`.
- [ ] Run `cargo test -p hiero-bin --test smoke`; expect RED because the workspace and binary do not exist.
- [ ] Create a resolver-2 Rust-2024 workspace with shared `version = "0.6.0"`, `authors = ["Pavel Obruchnikov <me@inkyquill.net>"]`, `rust-version = "1.94"`, `[workspace.lints.rust] unsafe_code = "forbid"`, and the exact Phase 001 dependency families. Do not add a Cargo `license` field until the repository has an explicit license decision. Keep `sqlx` features to `runtime-tokio`, `sqlite`, `migrate`, `macros`, `chrono`, `json`; use rustls for `reqwest`.
- [ ] Implement `main` as `Cli::parse()` followed by `run(cli).await.context("hiero command failed")`; initialize `tracing_subscriber`; return `ExitCode::FAILURE` only at the outer boundary.
- [ ] Add CI commands `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --all-features --locked`, and `cargo doc --workspace --no-deps --all-features --locked` without removing Python checks yet.
- [ ] Run the four Cargo commands; expect GREEN. Commit `chore: bootstrap Rust workspace`.

### Task 2: Implement Configuration and Canonical Values

**Files:** Create `crates/hiero-core/src/config.rs`, `crates/hiero-core/src/values.rs`; modify `crates/hiero-core/src/lib.rs`; test in colocated `#[cfg(test)]` modules.

**Interfaces:** Produces `HieronymusConfig::load(Option<PathBuf>) -> Result<Self, ConfigError>`, all path methods from proposal 001 §3, `resolve_port(Option<u16>) -> u16`, `utc_now`, `clamp_score`, `normalize_tuple`, and `json_object`.

- [ ] Write table-driven tests for explicit root > `HIERONYMUS_DATA_ROOT` > `XDG_DATA_HOME/hieronymus` > `$HOME/.local/share/hieronymus`, separate config root, every derived filename, CLI port > valid env > 9768, invalid/non-Unicode port values falling back to 9768, and `0.0..=1.0` clamping.
- [ ] Run `cargo test -p hiero-core config && cargo test -p hiero-core values`; expect RED with unresolved modules.
- [ ] Implement environment access behind a private `Environment` trait so tests never mutate process-global environment concurrently. Make `load` create no directories; add a separate `ensure_directories(&self) -> Result<(), ConfigError>` for startup.
- [ ] Ensure `provider.conf`, `dream.conf`, `ingest.conf`, `semantic.conf`, `llm-cache.json`, and `auth-token` resolve under config root while DB, LanceDB, models, log, lock/state, and agent plugins resolve under data root.
- [ ] Run focused tests and full clippy; expect GREEN. Commit `feat: add Rust configuration and value helpers`.

### Task 3: Freeze the Complete CLI Grammar

**Files:** Create `crates/hiero-bin/src/cli/mod.rs`, `crates/hiero-bin/src/cli/commands.rs`, `crates/hiero-bin/tests/cli_schema.rs`; modify `crates/hiero-bin/src/main.rs`.

**Interfaces:** Produces `Cli`, `Commands`, `RagCommand`, `AgentHookCommand`, and `async fn dispatch(cli: Cli, config: HieronymusConfig) -> anyhow::Result<()>`.

- [ ] Add `Cli::try_parse_from` tests for every command in proposal 001 §4, including no-subcommand behavior, nested `rag`, `agent-hook session-start|session-end`, repeated tag values, JSON flags, and rejected missing/invalid arguments.
- [ ] Run `cargo test -p hiero-bin --test cli_schema`; expect RED.
- [ ] Implement the exact clap schema, using nested subcommand enums rather than 30 flat variants where command spelling is grouped (`series`, `session`, `concept`, `skills`, `rag`, `agent-hook`). Preserve the proposal's external command names with `#[command(name = "...")]`.
- [ ] Encode boundary classification as `CommandExecution::{DirectStore, DaemonHttp, StartDaemon, StdioMcp}` and assert `status|stop|config` are the only daemon HTTP commands.
- [ ] Run focused tests; snapshot `hiero --help` and each group help with `trycmd`. Commit `feat: define Rust CLI contract`.

### Task 4: Implement Series Registry

**Files:** Create `crates/hiero-core/src/registry.rs`, `crates/hiero-core/tests/registry.rs`; modify `crates/hiero-core/src/lib.rs`.

**Interfaces:** Consumes `&SqlitePool` from Phase 002 when available; initially compile behind test helper. Produces `SeriesRegistry<'a> { pool: &'a SqlitePool }`, `create`, `list`, `get`, `init`, and the series language-tag mutation used by MCP with `SeriesRecord`.

- [ ] Port fixtures and assertions from `tests/test_registry.py` and `tests/test_series_language_tags.py`: uniqueness, missing series, stable ordering, language-tag replacement transaction, and `.hieronymus.json` contents.
- [ ] Run `cargo test -p hiero-core --test registry`; expect RED.
- [ ] Implement parameterized SQL queries and wrap tag replacement in one write transaction. Write workspace config atomically via a same-directory temporary file, `sync_all`, rename, and Unix `0600` permissions.
- [ ] Run focused tests; expect GREEN. Commit `feat: port series registry`.

### Task 5: Implement Agent Plugins and Skill Assets

**Files:** Create `crates/hiero-core/src/agent/{mod.rs,context.rs,config_patch.rs,skills.rs,claude.rs,codex.rs,gemini.rs,opencode.rs,openclaw.rs}`, `crates/hiero-core/tests/agent.rs`; add embedded skill assets under `assets/skills/`.

**Interfaces:** Produces the `AgentPlugin`, `AgentAvailability`, `InstallStep`, `InstallPlan`, `ProjectAgentContext`, and `SkillPlan` APIs from proposal 001 §5.

- [ ] Port behavioral cases from `tests/test_agent_context.py`, `test_agent_plugin_installers.py`, `test_cli_project_skills.py`, and `test_project_skills.py`, including malformed config rejection, unrelated-key preservation, dry-run immutability, idempotence, and uninstall ownership boundaries.
- [ ] Run `cargo test -p hiero-core --test agent`; expect RED.
- [ ] Implement `patch_json_config`/`patch_toml_config` as read-validate-clone-patch-serialize-atomic-replace operations; never truncate a valid existing config before replacement succeeds.
- [ ] Use `Box<dyn AgentPlugin>` only for heterogeneous plugin discovery. Claude/Codex write Streamable HTTP URL; Gemini/OpenCode/OpenClaw write `hiero mcp` command config.
- [ ] Run focused tests; expect GREEN. Commit `feat: port agent integrations`.

### Task 6: Implement Doctor and Composition Skeleton

**Files:** Create `crates/hiero-core/src/doctor.rs`, `crates/hiero-core/tests/doctor.rs`, `crates/hiero-bin/src/output.rs`; modify `crates/hiero-bin/src/main.rs` and CLI dispatch.

**Interfaces:** Produces `CheckStatus`, `DoctorCheck`, `DoctorReport`, `async fn run_doctor(&HieronymusConfig) -> DoctorReport`, and JSON/human renderers in the binary.

- [ ] Port all non-obsolete checks from `tests/test_doctor.py` and `test_doctor_agent_plugins.py`; replace Python/Bun/OpenTUI runtime checks with database path/writeability, config parse, agent plugin, bind-port, and derived-index rebuildability checks.
- [ ] Run `cargo test -p hiero-core --test doctor`; expect RED.
- [ ] Implement checks as independent functions returning data, never printing or exposing secret values. Compose only the completed commands; phase-dependent dispatch paths remain absent from the production enum until their phase adds them, while CLI schema tests cover their spelling separately.
- [ ] Run all workspace verification and the existing `uv run pytest`; expect both GREEN. Commit `feat: add diagnostics and Rust composition root`.

## Phase Acceptance

- [ ] `cargo run -p hiero-bin -- --help`, `doctor --json`, configuration tests, CLI snapshots, registry tests, and agent tests pass.
- [ ] A placeholder scan across `crates`, `Cargo.toml`, and `build.rs` returns no incomplete implementation markers.
- [ ] Public types named here match proposal 001 and the Phase 002 consumer plan.

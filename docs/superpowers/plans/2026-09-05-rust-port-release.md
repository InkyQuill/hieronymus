# Rust distribution and cutover verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver an installable Linux artifact whose memory, semantic RAG, console and agent workflows survive upgrade and rollback.

**Architecture:** Keep existing versioned application directories and explicit migration boundaries. Bundle verified semantic assets with each release and resolve remote releases into the same local staging flow used by the updater; verify the actual installed artifact in an isolated environment.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** ADRs 0006, 0008–0015; Astra release/lifecycle/documentation follow-ups; mandatory memory plus semantic RAG owner requirement. Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

## Global Constraints

- “All domain mutations used by normal CLI, MCP, and frontend workflows go through the daemon.” (ADR 0009)
- “The first Rust cutover supports `x86_64-unknown-linux-gnu`.” (ADR 0013)
- Owner requirement (2026-09-05): working memory and semantic RAG are mandatory. ADR 0013’s FTS-only allowance is not an acceptable completion or release alternative for this program.
- “Only an authenticated user acting through the explicit rule-approval operation may transition `candidate` to `active`” (ADR 0011).
- “the separate CSRF token layer is waived.” (ADR 0012, 2026-09-03 amendment)
- MCP revision remains `2026-07-28`. Preserve frozen Python inputs; new ADR-backed Rust expectations are separate versioned fixtures.
- Preserve unrelated work, immutable backups, local plaintext credentials, and the single data-root layout. No Python runtime rollback, dependency-pin refresh, publication, or user-service changes during unit tests.
- Steps below specify planned code, not code already implemented. Existing types are referenced by source module; new cross-task interfaces are declared explicitly.

---

## File structure and execution boundary

`release_source.rs` only resolves/downloads a release; update.rs retains activation and rollback. Build/install scripts package assets and select the versioned runtime. Documentation records real evidence, without adding an attestation subsystem.

- **F1:** Package required semantic assets and support remote update sources — `crates/hiero/src/release_source.rs`, `crates/hiero/src/lib.rs`, `crates/hiero/src/main.rs`, `crates/hiero/src/update.rs`, `crates/hiero/src/app.rs`, `crates/hiero/src/doctor.rs`, `scripts/release-build.sh`, `scripts/install.sh`, `crates/hiero/tests/release_source.rs`, `crates/hiero/tests/installer_flow.rs`, `docs/distribution.md`
- **F2:** Rehearse the complete product and reconcile documentation — `docs/rust-cutover-rehearsal.md`, `docs/distribution.md`, `docs/usage.md`, `docs/agent-workflows.md`, `compatibility/README.md`, `AGENTS.md`, `crates/hiero/src/app.rs`, `crates/hiero/src/daemon/assets.rs`, `crates/hiero/tests/product_rehearsal.rs`

## Tasks

### Task F1: Package required semantic assets and support remote update sources

**Coverage:** Astra release-source follow-up; S2 runtime persistence; no end-user Python/Node/Bun.

**Dependencies:** R4, S1–S3.

**Files:**

- Create: `crates/hiero/src/release_source.rs`
- Modify: `crates/hiero/src/lib.rs`
- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/update.rs`
- Modify: `crates/hiero/src/app.rs`
- Modify: `crates/hiero/src/doctor.rs`
- Modify: `scripts/release-build.sh`
- Modify: `scripts/install.sh`
- Test: `crates/hiero/tests/release_source.rs`
- Test: `crates/hiero/tests/installer_flow.rs`
- Modify: `docs/distribution.md`

**Interfaces:** Produce `release_source::validate_base_url(&str) -> Result<(),String>` and `stage_remote(base_url: &str, channel: &str, destination: &Path) -> Result<PathBuf,String>`; result is the existing verified local release directory input for update. Build inputs HIERO_RELEASE_ONNX_RUNTIME, HIERO_RELEASE_ONNX_SHA256, HIERO_RELEASE_MODEL_DIR point to the qualified runtime and pinned model/tokenizer. Artifact adds lib/libonnxruntime.so, models/minilm/model.onnx, models/minilm/tokenizer.json and checksummed metadata; daemon resolves these from its versioned executable directory.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::release_source::validate_base_url;
#[test]
fn updater_requires_https_for_remote_release_sources() {
    assert!(validate_base_url("http://example.org/releases").is_err());
    assert!(validate_base_url("https://user:secret@example.org/releases").is_err());
    assert!(validate_base_url("https://example.org/releases").is_ok());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test release_source --test installer_flow`
Expected before the change: updater has only a local directory input and installed semantic runtime/model setup is not established.

- [ ] **Step 3: Implement the bounded change**

Use the existing TLS URL parser and transport to validate HTTPS/no-userinfo. Add --release-url with the installer's existing HIERONYMUS_RELEASE_URL source, preserve --release-dir, and reject specifying both. Resolve stable/dev to the configured channel metadata; report the chosen version/target without silently following a different channel. Do not invent a public hosting URL: require the configured or explicit base URL and return an actionable error when absent. Bound downloads, preserve TLS checks, reject HTTPS→HTTP redirects, verify existing metadata/archive checksums and signature-policy rules before staging. Feed the verified directory to R4 activation. Build requires a checksum-matching qualified ONNX runtime and S1 model/tokenizer hashes, copies assets into the versioned directory, preserves licenses, and validates load/inference during packaging. Install uses bundled assets with no network/model download needed after unpacking. Rollback changes binary and bundled runtime together; shared data-root generations remain identity-checked. Configured explicit asset overrides must be verified and shown by doctor. The release builder must fail if required assets are absent.

```rust
pub fn validate_base_url(value: &str) -> Result<(), String> {
    let authority = value.strip_prefix("https://")
        .ok_or("remote release source must use HTTPS")?
        .split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || value.contains(['\r', '\n']) {
        return Err("remote release source has an invalid authority".into());
    }
    Ok(())
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test release_source --test installer_flow`
Expected after the change: remote sources stage a verified release, and the installed package loads real semantic assets.

The snippet is an early guard; use the existing strict TLS URL parser for complete host/port parsing. Test local loopback TLS transport injection, stable/dev selection, interrupted download, bad checksum, wrong target, signature-policy refusal, traversal/symlink archives, and no activation after any failure. Run the packaged binary with PATH lacking Python/Node/Bun and without development LD_LIBRARY_PATH; load ONNX and run S3's real retrieval workflow. Treat a missing/mismatched runtime as failure, not accepted degraded doctor status. Record actual runtime digest/version from qualified input, never fabricate a checksum or reuse a synthetic-vector receipt.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/release_source.rs crates/hiero/src/lib.rs crates/hiero/src/main.rs crates/hiero/src/update.rs crates/hiero/src/app.rs crates/hiero/src/doctor.rs scripts/release-build.sh scripts/install.sh crates/hiero/tests/release_source.rs crates/hiero/tests/installer_flow.rs docs/distribution.md
git commit -m "feat: ship verified semantic assets and remote updates"
```

### Task F2: Rehearse the complete product and reconcile documentation

**Coverage:** All reviewed gaps; Sonnet fixture marker; rustdoc/AGENTS drift; owner question-list.

**Dependencies:** All R/M/D/W/S tasks and F1.

**Files:**

- Create: `docs/rust-cutover-rehearsal.md`
- Modify: `docs/distribution.md`
- Modify: `docs/usage.md`
- Modify: `docs/agent-workflows.md`
- Modify: `compatibility/README.md`
- Modify: `AGENTS.md`
- Modify: `crates/hiero/src/app.rs`
- Modify: `crates/hiero/src/daemon/assets.rs`
- Test: `crates/hiero/tests/product_rehearsal.rs`

**Interfaces:** No new production API. Tests consume installed CLI aliases, public MCP HTTP/stdio, browser REST/WS and the S3 corpus. Evidence records commit, artifact checksum, test configuration and observed outcomes; no fixed-count or machine-attestation gate is added.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
#[test]
fn shipped_aliases_resolve_in_one_version_directory() {
    let app = std::env::var_os("HIERO_TEST_INSTALLED_APP")
        .expect("set HIERO_TEST_INSTALLED_APP to a disposable installed application");
    let bin = std::path::PathBuf::from(app).join("bin");
    let executable = std::fs::canonicalize(bin.join("hiero")).unwrap();
    for name in ["hieronymus", "hieronymus-agent-hook", "hieronymus-mcp"] {
        assert_eq!(std::fs::canonicalize(bin.join(name)).unwrap(), executable);
    }
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test product_rehearsal -- --ignored --nocapture`
Expected before the change: initial product rehearsal reproduces missing paths from the review; explicit live suite requires a disposable installed candidate.

- [ ] **Step 3: Implement the bounded change**

Mark the provided installation-dependent test #[ignore = "requires disposable installed release"] so default tests do not need a release fixture, but the explicit command must fail on missing inputs. Build and install via actual scripts under a temporary app/data/unit root; run a real browser and real MCP client. Execute the ordered acceptance checklist below and record failures without reclassifying mandatory features as optional. Correct rustdoc argv[0] escaping/private embedded link; update AGENTS.md to actual Rust/SQLite/Svelte ownership and required cargo/frontend verification, retaining Python commands for changes to the retained Python reference/tools. Annotate CSRF and recall/rule lifecycle Rust compatibility deltas and link the accepted ADR amendments, preserving frozen inputs. Update distribution/usage only to behavior demonstrated by the installed artifact.

```rust
// Use a normal Rust test with this attribute for installed-artifact checks:
#[ignore = "requires disposable installed release"]
#[test]
fn installed_candidate_is_executable() {
    let app = std::env::var_os("HIERO_TEST_INSTALLED_APP")
        .expect("HIERO_TEST_INSTALLED_APP is required");
    let output = std::process::Command::new(
        std::path::PathBuf::from(app).join("bin/hiero")
    ).arg("--version").output().unwrap();
    assert!(output.status.success());
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test product_rehearsal -- --ignored --nocapture`
Expected after the change: every checklist item below passes on the installed artifact, with no FTS-only substitution.

Acceptance checklist (record each result in docs/rust-cutover-rehearsal.md):

- Fresh install: no Python/Node/Bun, valid schema, one owner, stable token, all aliases, authenticated readiness including required semantic state.
- Import representative Python data, preserve terminology/concepts/provenance, then upgrade Rust v1→v2 after a completed cutover journal; exercise failure/recover and reject malformed/newer/partial schemas.
- Actual MCP HTTP and stdio: series/session→remember/correction→complete→configured multi-provider Dream→recall with deterministic contract; all 39 registry cases execute.
- Import RAG, complete semantic rebuild, retrieve semantic paraphrases and learned memory; verify provenance/series isolation, restart reconciliation, and stale-generation rejection with S3's real model.
- Browser launched through both admin/config commands: session exchange, ten views, thirteen actions, provider checks/settings, correct Origin handling, reconnect/refresh and logout-by-restart behavior.
- SIGTERM and explicit stop join workers before ownership release; second daemon/migrate/recover cannot enter concurrently.
- Remote/local update: candidate load failure, doctor exit42, semantically disarmed candidate, failed spawn/reload, and failed rollback all report truthfully and preserve recovery artifacts; successful update and rollback reach the expected authenticated instance.
- Read/review the existing owner cutover questions. Keep actual evidence; do not resurrect removed fixed-count, attestation or extra approval tooling.

Final checks: `cargo fmt --all -- --check`; `cargo clippy --all-targets --all-features --locked -- -D warnings`; `cargo test --all-features --locked`; `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`; `cd frontend && bun run typecheck && bun run test && bun run build`. Run `uv run pytest`, `uv run ruff check .`, `uv run ruff format --check .` when retained Python implementation/tools were changed. Separately run the explicit S3 and installed-artifact tests; default green unit tests do not satisfy them.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add docs/rust-cutover-rehearsal.md docs/distribution.md docs/usage.md docs/agent-workflows.md compatibility/README.md AGENTS.md crates/hiero/src/app.rs crates/hiero/src/daemon/assets.rs crates/hiero/tests/product_rehearsal.rs
git commit -m "docs: record verified rust product cutover"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.


# AGENTS.md

## Project

Hieronymus is a local desktop server providing several types of agent memory for writing and literary translation projects. Source code lives here, while book projects and runtime databases live elsewhere.

The server serves the main web interface. Authors use it to inspect what their agent remembers, add pointers or context, and flag wrong or stale memories; it is not a comprehensive human-maintained project knowledge base. The main flow is to start the server, connect an agent by adding both its MCP connection and Hieronymus skills, verify both, and continue writing in Codex, Cowork or pi. Keep this flow approachable for nontechnical writers.

`hiero` is a convenience CLI for quick tasks. The running server owns its tray presence, directly or through a supervised sidecar, whenever the desktop supports it. Browser authentication is off by default and optional in configuration; this does not disable authenticated MCP access.

## Development Defaults

- Use `Pavel Obruchnikov <me@inkyquill.net>` for formal author metadata unless local git config overrides it.
- Prefer small, testable Rust modules with explicit domain boundaries.
- Keep strict terminology logic deterministic. Fuzzy memory and semantic recall must never silently override approved termbase entries.
- Do not write tool source code into `/home/inky/Yandex.Disk/Translation`.

## Current Stack

- Rust 1.96 workspace: `crates/hieronymus` owns SQLite/domain operations; `crates/hiero` owns CLI, daemon, MCP and distribution.
- SQLite with FTS5; LanceDB/ONNX and the pinned multilingual tokenizer provide mandatory semantic retrieval.
- Svelte 5 console in `frontend`; Bun 1.4.0 builds and tests it. The release binary embeds its production assets.
- Authenticated local MCP HTTP (revision 2026-07-28) and a stdio adapter.
- Bun TypeScript in `scripts` owns release acquisition and metadata validation. No Python tooling is required. The previous Python implementation is archived on `stale/python-v0.7.0`.

## Verification

Run relevant focused tests while editing, then these before claiming Rust implementation work is complete:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
bun test scripts/*.test.ts
cd frontend
bun run typecheck
bun run test
bun run build
```

Real model and installed-artifact tests are explicitly ignored by default. Supply their documented disposable fixture inputs and run them explicitly when qualifying those paths; missing inputs must fail. See `docs/rust-cutover-rehearsal.md`. Passing synthetic provider or transport tests does not establish native agent-host acceptance.

Current Rust plans and accepted ADR amendments govern product behavior. Preserve frozen Python fixtures as historical evidence; do not introduce a Python parity release gate. ADR 0016's autonomous authority design is not evidence that its runtime has been implemented.

## Release checks and blockers

- Only P0 issues block a release. P1, P2, P3, missing native-host evidence, smoke-test failures, lint/documentation findings, and incomplete platform qualification are warnings; record actionable failures in GitHub issues with reproduction details and the relevant run/log link. Reuse an existing issue for the same problem. Never silently label a failed or unrun check as passed.
- Run focused, inexpensive checks first. Keep the verification commands above as the verification checklist, but report non-P0 failures as warnings rather than starting an indefinite fix/rebuild cycle or withholding the release.
- Use `CARGO_BUILD_JOBS=2` for local Rust checks: many concurrent debug linkers for the retrieval stack can exhaust memory and spend minutes swapping. An interrupted or unrun full check must be reported honestly as a verification warning with an issue, not restarted indefinitely.
- Build native binaries once and retain the artifacts. Installer, packaging, documentation, and CI-only fixes must reuse those binaries when their build inputs are unchanged. Rerun the affected check or packaging job, not the entire platform matrix. Rebuild only when binary inputs change or the required artifact is missing.
- Native installation checks and the separate evidence workflow are advisory. A missing optional installer or evidence record must not prevent publication of the available working artifacts; disclose the limitation in release notes and an issue.
- Keep release automation small. Do not introduce extra qualification layers, duplicate gates, or new mandatory evidence scaffolding without an explicit product need. These rules supersede stricter release-blocker language in older plans and ADRs.
- Artifact integrity checks and safeguards against corrupting user data remain runtime requirements. Do not make a checksum mismatch executable or bypass data ownership merely to turn a release check green.

## Default paths

- Use the operating system's standard configuration directory by default: `$XDG_CONFIG_HOME/hieronymus` (fallback `~/.config/hieronymus`) on Linux, `~/Library/Application Support/Hieronymus` on macOS, and `%APPDATA%/Hieronymus` on Windows.
- `--data-root` overrides `HIERONYMUS_DATA_ROOT`, which overrides the platform default. Keep the CLI, installers, desktop launchers, and service registrations consistent. Never persist development or temporary fixture paths into a user's real service registration.
- Tests that install services or desktop registrations must use disposable configuration, data, and registration directories and must not contact the real user service manager.

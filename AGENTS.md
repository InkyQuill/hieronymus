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

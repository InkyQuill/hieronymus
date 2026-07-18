# AGENTS.md

## Project

Hieronymus is a local-first translation memory MCP for literary translation workflows. It is separate from the translation workspace; source code lives here, while book projects and runtime databases live elsewhere.

## Development Defaults

- Use `Pavel Obruchnikov <me@inkyquill.net>` for formal author metadata unless local git config overrides it.
- This branch implements the Rust rewrite specified by `docs/rust-migration-proposal/001-initial-setup.md` through `006-testing-and-deployment.md`. Treat those documents as the migration source of truth.
- Prefer small, testable Rust modules with explicit boundaries. Keep domain logic in `hiero-core`; keep CLI, HTTP, WebSocket, MCP, and process composition in `hiero-bin`.
- Preserve the Python implementation only as a behavioral and test-parity reference while the Rust rewrite is incomplete. Do not add new production behavior to the Python implementation unless a migration plan explicitly requires it.
- Keep strict terminology logic deterministic. Fuzzy memory and semantic recall must never silently override approved termbase entries.
- Keep SQLite authoritative. LanceDB indexes and ONNX models are rebuildable derived artifacts and must not become the sole store for domain data.
- Keep the runtime architecture to one shipped binary. The daemon owns HTTP, WebSocket, MCP, and background workers in one Tokio runtime; stdio MCP is only a compatibility shim.
- Do not write tool source code into `/home/inky/Yandex.Disk/Translation`.

## Migration Stack

- Rust 1.94+ with the 2024 edition
- Cargo workspace with `hiero-core` and `hiero-bin`
- SQLite with FTS5
- `sqlx` with embedded migrations
- Tokio, Axum, and `rmcp`
- LanceDB plus ONNX Runtime for rebuildable semantic indexes
- Rust unit and integration tests, with the Python suite retained as the parity reference during migration
- CLI for local debugging, imports, exports, and validation

## Verification

Run these before claiming implementation work is complete:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --no-deps --all-features --locked
```

When a migration task changes a frontend contract or embedded assets, also run the relevant checks from `frontend/package.json` and build `frontend/dist/` before the release build.

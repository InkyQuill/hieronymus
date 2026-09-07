# AGENTS.md

## Project

Hieronymus is a local-first translation memory MCP for literary translation workflows. It is separate from the translation workspace; source code lives here, while book projects and runtime databases live elsewhere.

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
- Python/uv remain for the historical reference and acquisition/qualification tools; they are not installed-runtime dependencies.

## Verification

Run relevant focused tests while editing, then these before claiming Rust implementation work is complete:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cd frontend
bun run typecheck
bun run test
bun run build
```

Real model and installed-artifact tests are explicitly ignored by default. Supply their documented disposable fixture inputs and run them explicitly when qualifying those paths; missing inputs must fail. See `docs/rust-cutover-rehearsal.md`. Passing synthetic provider or transport tests does not establish native agent-host acceptance.

For intentional changes to the retained Python reference/tools, also run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Current Rust plans and accepted ADR amendments govern product behavior. Preserve frozen Python fixtures as historical evidence; do not introduce a Python parity release gate. ADR 0016's autonomous authority design is not evidence that its runtime has been implemented.

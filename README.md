# Hieronymus

Hieronymus gives your writing agent persistent memory for writing and literary
translation projects: translation decisions, plot facts, voice notes and other
working context. Approved terminology stays deterministic alongside searchable
memory and semantic recall.

Start the local server, open its web interface, and connect your agent by adding
both the Hieronymus MCP connection and skills. Once your agent verifies both,
continue writing in Codex, Cowork or pi. Return to the web interface to see what
it remembers, add pointers, or flag memories that are wrong or stale.

The web interface is the main author interface. Browser authentication is off
by default, with optional configuration; MCP access remains authenticated.
The server maintains a tray icon when the desktop supports one. The `hiero`
CLI provides shortcuts for quick tasks.

## Install

- **Windows:** [Download Setup](https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/Hieronymus-0.9.1-Setup.exe), open it, and follow the steps.
- **macOS:** [Download the installer](https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/Hieronymus-0.9.1.pkg), open it, and follow the steps. It chooses Apple Silicon or Intel automatically.
- **Linux x86_64:** paste this into a terminal:

```bash
curl -fsSL https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/install-hieronymus.sh | bash
```

The installers download the app and its memory model, verify the files, and install
for your user account. Keep your internet connection on during setup. No Git clone,
compiler, Bun, Node, Python, or separate PowerShell installation is needed.

The web interface opens after installation. Use it to inspect your agent's memory,
add pointers, and flag stale or wrong entries. Browser authentication is optional
and off by default; MCP access remains authenticated.

Windows and macOS installers are unsigned. Intel Mac desktop support is unqualified;
see the [release notes](https://github.com/InkyQuill/hieronymus/releases/latest) for testing gaps.

For configuration, updates, removal, and advanced offline installation, see the
[usage guide](docs/usage.md) and [distribution guide](docs/distribution.md).

## Develop and verify

The application is a Rust 1.96 workspace with SQLite/FTS5, LanceDB and mandatory
ONNX semantic inference. A Svelte 5 console is built with Bun 1.4.0 and embedded
in the release binary. Python is not required to build, test, release or run it.

Install the pinned Rust toolchain and Bun 1.4.0 (`rust-toolchain.toml`, `mise.toml`).
Build dependencies are Rust/Cargo, Bun, Git, the standard Linux build tools and
`protoc` (Protocol Buffers compiler, required by LanceDB). On Debian/Ubuntu,
install it and the standard proto definitions with
`sudo apt-get install protobuf-compiler libprotobuf-dev`.
Run `./scripts/check-protobuf.sh` to verify compilation of standard imports.
These dependencies are only needed when
building from source, not when running the installed application.

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend build
bun test scripts/*.test.ts
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
bun run --cwd frontend typecheck
bun run --cwd frontend test
```

The release helpers use Bun's built-in TypeScript support. Follow
[build ownership](docs/distribution.md#build-ownership) to acquire pinned assets
and build an archive. Ordinary tests use fixtures; real model and installed-artifact
tests require explicit disposable inputs and are ignored by default.

The previous Python application, package, tests and qualification orchestration
are archived on [`stale/python-v0.7.0`](https://github.com/InkyQuill/hieronymus/tree/stale/python-v0.7.0).
Historical fixtures remain available to Rust migration tests. See
[the archive policy](docs/archive/python-v0.7.0.md) for older plans and records.

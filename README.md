# Hieronymus

Hieronymus is a local-first translation memory MCP for literary translation.
It keeps approved terminology deterministic while providing searchable working
memory and semantic recall for translation decisions, plot facts and voice notes.

The application is a Rust 1.96 workspace with SQLite/FTS5, LanceDB and mandatory
ONNX semantic inference. A Svelte 5 console is built with Bun 1.4.0 and embedded
in the release binary. Python is not required to build, test, release or run it.

## Install a native candidate

The supported target is Linux x86_64. Given a verified release directory containing
`release.json`, its archive and checksum file, run from this checkout:

```bash
./install.sh --release-dir /path/to/release-dist
```

For a disposable installation without service activation:

```bash
./install.sh --release-dir /path/to/release-dist \
  --app-dir /tmp/hiero-rehearsal/app --data-root /tmp/hiero-rehearsal/data \
  --unit-dir /tmp/hiero-rehearsal/units --no-activate
```

The installer delegates to `scripts/install.sh`; it is intended for checkout
usage. The installed executable includes its model/runtime assets and needs no
Bun or compiler. No public release feed is assumed. Native host acceptance and
installed workflow qualification remain separate gates; see the
[rehearsal](docs/rust-cutover-rehearsal.md) and
[host acceptance record](docs/agent-host-acceptance.md).

`hiero config` opens the local web console. `hiero status --json` reports service
and semantic readiness. `hiero uninstall --yes` removes owned application files
and integrations while preserving data by default. See the [usage guide](docs/usage.md)
and [distribution guide](docs/distribution.md) for configuration, updates and recovery.

## Develop and verify

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

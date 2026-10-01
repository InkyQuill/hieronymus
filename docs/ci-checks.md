# CI checks

Pull requests run Linux formatting, all-target/all-feature Clippy, library unit tests,
a focused integration set covering data export/upgrades, authority, prompt delivery,
authenticated discovery, service lifecycle and stdio, plus Rust documentation,
script tests and frontend format/typecheck/tests/build. Production safety assertions
remain enabled. CI provider tests use mocks and disposable loopback fixtures only.

`.github/workflows/nightly.yml` runs the full Rust integration suite and the Linux,
Windows, Apple Silicon macOS and Intel macOS helper/native contract matrix daily
at 00:00 UTC (03:00 Moscow). It also supports manual workflow dispatch for a selected
branch. Nightly checks supplement PR checks; a green PR does not establish native
platform acceptance. Inspect nightly failures and file/reuse an issue with the run
link. Installer/release workflows retain their existing checks.

Both workflows cache Rust dependencies with a pinned rust-cache action. Workspace
outputs are excluded from the cache, avoiding retention of the many large test
executables. Linux headless jobs share a cache key; each desktop target has its own
key. Cache reuse remains keyed by toolchain, dependency and Rust environment inputs.
Cold builds still compile the retrieval stack; measure actual workflow durations
rather than assuming a fixed speedup.

The full local verification checklist in AGENTS.md remains unchanged. Run focused
integration tests for affected behavior while editing, then the full local checklist
before claiming Rust implementation complete. Real provider and installed-artifact
tests remain explicitly opt-in, local-only, and require their disposable inputs.

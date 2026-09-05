# Distribution: build, archive, and release (Rust era)

Status: first slice of the distribution work (build + ship machinery;
installer, update flow, and rehearsal execution follow). Authority:
`docs/superpowers/specs/2026-08-31-rust-distribution-cutover-design.md` AS
AMENDED (2026-09-03) and ADR 0006.

## Support matrix

| Property | Value |
| --- | --- |
| Initial target | `x86_64-unknown-linux-gnu` |
| Semantic retrieval | **enabled** — qualified record `qualification/records/semantic-native.json` (decision `semantic-enabled`) |
| Fallback mode | FTS5-only (the required mode when no qualified semantic record exists for the target) |
| Rust pin | 1.96.0 (`rust-toolchain.toml`) |
| Bun pin (console build only) | 1.4.0 (`frontend/bun.lock`, CI `setup-bun`) |

macOS and Windows receive no installer (spec §Support Matrix); a later ADR is
required per additional target after native release and service-lifecycle
rehearsal.

## Build ownership

The Cargo workspace is virtual (root `Cargo.toml` has no package); build
scripts live in the `hiero` package (`crates/hiero`). The release order is
fixed: the production console bundle is built by an explicit orchestration
step BEFORE the binary is compiled, and the binary package embeds the
resulting `frontend/dist` directory.

1. `scripts/release-build.sh` checks the Bun pin and builds the bundle
   (`bun install --frozen-lockfile` + `bun run --cwd frontend build`).
2. The `console-embed` cargo feature (default OFF) switches the daemon's
   asset abstraction (`crates/hiero/src/daemon/assets.rs`) to the embedded
   backend: `rust-embed` lookup/iteration over `frontend/dist`, served in the
   same fixed-set shape as the test/dev backends.
3. `crates/hiero/build.rs` enforces the bundle at compile time: a
   **release-profile** build with `console-embed` fails when
   `frontend/dist/index.html` is missing or empty. Frontend-free developer
   targets (debug profile) compile without the embedded backend and the
   daemon reports `web_console_not_built` — ordinary tests never invoke Bun.
4. The embedded set drops `.map` entries; embedded-asset tests additionally
   assert that served javascript/css carries no `sourceMappingURL` reference
   (`crates/hiero/tests/console_embed.rs`). Those tests run wherever the
   bundle exists (CI, release builds) and skip cleanly when it does not.

The clippy gate (`cargo clippy --all-targets --all-features --locked -- -D
warnings`) stays dist-free by design: without the bundle present the embedded
backend compiles to a no-op, so all-features builds never require Bun.

The shipping binary serves the embedded console through `Assets::release()`
(the `hiero daemon` path); `Assets::default()` remains the empty set in every
configuration, which is what the frozen static-route suites pin.

## Artifacts

`scripts/release-build.sh` (full mode) produces under `target/release-dist/`:

- `hieronymus-<version>-x86_64-unknown-linux-gnu.tar.gz` containing the
  `hiero` binary plus relative command links `hieronymus -> hiero`,
  `hieronymus-agent-hook -> hiero`, `hieronymus-mcp -> hiero` (the binary
  routes those names by `argv[0]`; the links carry them into `PATH`);
- `hieronymus-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256`
  (`sha256sum` format), verified together with an extraction round-trip
  (link targets, `hiero version`, `hieronymus version` probes).

Modes: `--dry-run` (plan only), `--assets-only` (Bun pin + bundle +
console-embed compile check), default full pipeline. Running the full
pipeline locally is optional; CI runs it on every release tag.

## Release workflow

`.github/workflows/release-rust.yml` runs the orchestration script on
`ubuntu-latest` for `x86_64-unknown-linux-gnu` and publishes the archive and
checksums to a GitHub release on `v*` tags. It pins Rust 1.96.0 and Bun 1.4.0
and builds from the repository lockfiles (`Cargo.lock`, `bun.lock`).

Protection: the job declares the GitHub Environment `release`; the required
reviewer (Pavel Obruchnikov `<me@inkyquill.net>`, ADR 0006) is configured in
repository settings (Settings → Environments → `release` → Required
reviewers). A workflow cannot create that rule, so a tag alone is
insufficient to publish.

Transition: the Python-era `.github/workflows/release.yml` (uv +
semantic-release + Hatch) remains the release authority on `main` until the
distribution cutover completes; it is retired by the cutover task.

Signing/SBOM/provenance are WAIVED for the first Rust release line (spec
amendment 2026-09-03): the release ships SHA-256 checksums and the owner
approval gate only. Introducing them later requires a small ADR before any
public distribution.

## Release rehearsal checklist (owner-run skeleton)

The rehearsal matrix is a manual checklist run by the owner per release
candidate (spec §Release Rehearsal); it is not a recorded CI matrix.

- [ ] Clean install and first launch.
- [ ] Install over the last Python managed release.
- [ ] Migration dry-run and confirmed upgrade on representative copied
      databases.
- [ ] CLI, MCP HTTP, MCP stdio, web, dreaming fake-provider, FTS, and
      semantic smoke tests.
- [ ] Forced daemon crash and restart.
- [ ] Failed update before schema change.
- [ ] Rust-only import and promotion from the immutable pre-upgrade backup.
- [ ] Uninstall with user-data preservation.

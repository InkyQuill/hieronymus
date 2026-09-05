# Distribution: build, install, update, and release (Rust era)

Status: build, ship, and install/update machinery complete. Authority:
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

## Install and update machinery

Two interlocking flows share one on-disk layout
(`crates/hiero/src/app.rs`):

```text
<app>/versions/<version>/{hiero,hieronymus,hieronymus-agent-hook,hieronymus-mcp}
<app>/bin/<name>            stable links switched by rename, never in place
~/.local/bin/<name>         PATH links into <app>/bin (owned names only)
~/.config/systemd/user/hieronymus.service   per-user daemon unit
```

- `scripts/install.sh` — the bootstrap installer (spec §Installer, 8 steps:
  platform resolve without executing content; fetch metadata + archive from
  `--release-dir` or `--release-url`; SHA-256 verify and refuse any signature
  the waived first line cannot verify; atomic versioned-dir install; stable
  link switch; `hiero service install`; non-mutating `hiero doctor`; daemon
  start only when the schema is already compatible). A required database
  upgrade completes the install but leaves the daemon stopped with the
  `hiero migrate` instruction printed.
- `hiero service install|uninstall|status|start|stop` — the systemd **user**
  unit (absolute binary, explicit `--data-root`, `Restart=on-failure`,
  `RestartSec=5s`, journald via `SyslogIdentifier`). Install/uninstall are
  idempotent and never touch user data. The manager is only contacted for the
  default unit location; a `--unit-dir` override renders/removes the file only
  (the documented test seam and the non-systemd degrade path).
  `hiero doctor` reports service-definition health (present, current binary,
  correct data root) for managed installs.
- `hiero update --release-dir <dir>` — the §Update flow: resolve (a
  `release.json` or exactly one release-build archive), verify the checksum,
  stage alongside, probe the candidate via `version --json` (protocol
  revision + supported schema), gate (protocol change ⇒ refuse; newer-schema
  target ⇒ refuse; running unmanaged daemon ⇒ refuse), stop the service,
  switch links, re-render the unit, start + health-check via the candidate's
  `doctor`. Health failure before a schema upgrade restores the prior links
  and unit automatically. A Python-schema database completes as
  `migration-pending`: links switched, daemon stopped, `hiero migrate`
  reported. Downgrades are refused. The updater never launches an older
  binary against a newer schema and never installs Python.
- `hiero uninstall [--yes] [--delete-data]` — removes the service unit, the
  application directory, owned PATH links that point into it, and the
  generated agent-plugin entries; preserves databases, configuration, models,
  backups, and audit data. Data deletion happens only through the explicit
  `--delete-data`, which names the exact data root in its report.
- Agent integrations keep referencing the stable `hiero`/`hieronymus*` link
  names, so after a healthy update the new binary serves existing entries
  without host-configuration edits; the Rust side has no host-config
  generation surface, so there is nothing else to refresh or back up.

The installer and update flows are covered end-to-end by
`crates/hiero/tests/installer_flow.rs` and `crates/hiero/tests/update_flow.rs`
against local fixture releases (no network, no real systemd user manager).

## Release rehearsal checklist (owner-run)

The rehearsal matrix is a manual checklist run by the owner per release
candidate (spec §Release Rehearsal); it is not a recorded CI matrix. Set up a
scratch environment first — every command below is safe to rerun:

```bash
VERSION=<candidate version>
REL=$HOME/rehearsal/releases          # holds the release artifacts
APP=$HOME/rehearsal/app
DATA=$HOME/rehearsal/data
UNITS=$HOME/rehearsal/units           # unit-dir override keeps systemd out
mkdir -p "$REL" "$DATA"
cp target/release-dist/hieronymus-$VERSION-x86_64-unknown-linux-gnu.tar.gz* "$REL/"
```

- [ ] **Clean install and first launch** — `scripts/install.sh --release-dir
      "$REL" --app-dir "$APP" --data-root "$DATA" --unit-dir "$UNITS"`; expect
      exit 0, "sha256 verified", "doctor: healthy", a started user service
      (rerun without `--unit-dir` on the real machine to exercise systemd),
      and `"$APP"/bin/hiero doctor` healthy. `curl` the daemon per the
      discovery record in `$DATA/daemon.json` (bearer from
      `$DATA/daemon.token`) for an MCP `tools/list`.
- [ ] **Install over the last Python managed release** — with the Python-era
      `hiero`/`hieronymus` on PATH (uv tool install) and its data root, run
      the installer; expect the owned PATH link names to be taken over with a
      notice, doctor to report `database-upgrade-required`, and the daemon to
      stay stopped (step 8).
- [ ] **Migration dry-run and confirmed upgrade on copied databases** — copy
      a representative Python-era data root; `"$APP"/bin/hiero migrate
      --data-root <copy> --dry-run` (expect the plan, no changes), then
      `"$APP"/bin/hiero migrate --data-root <copy>`; expect the pre-upgrade
      backup under `<copy>/backups/`, a `complete` cutover journal,
      `hiero classify` reporting `rust-schema`, and doctor degraded-free on
      the copy.
- [ ] **CLI, MCP HTTP, MCP stdio, web, dreaming fake-provider, FTS, and
      semantic smoke** — `hiero agent-hook session-start --cwd <project>`;
      MCP HTTP `tools/list` + one `tools/call` against the daemon; `hiero
      mcp` framed initialize/tools-list over stdio; `http://<host:port>/`
      serving the embedded console; a dreaming run with the fake provider on
      a copied root; a recall over strict terms (FTS lane); `hiero semantic
      status` (FTS-only is the required baseline; the semantic lane per the
      qualification record).
- [ ] **Forced daemon crash and restart** — `kill -9 $(jq .pid
      "$DATA/daemon.json")`; expect doctor `daemon-unreachable` (degraded),
      then `hiero service start` (or `systemctl --user restart
      hieronymus.service`) recovering: fresh discovery record, doctor
      healthy.
- [ ] **Failed update before schema change** — build a fixture feed whose
      binary's `doctor` exits 2 (see `tests/update_flow.rs`), run `hiero
      update --release-dir <fixture> --app-dir "$APP" --data-root "$DATA"`;
      expect exit 1, "health check failed", the stable links restored to the
      prior version, and the daemon still serving.
- [ ] **Rust-only import and promotion from the immutable pre-upgrade
      backup** — on a copy that has undergone the confirmed upgrade, simulate
      loss (move the live DB aside) and run `hiero recover --data-root
      <copy>`; expect the rebuilt database from the immutable backup, the
      replaced file moved aside, and doctor healthy on the copy.
- [ ] **Uninstall with user-data preservation** — `hiero uninstall --yes
      --app-dir "$APP" --data-root "$DATA" --unit-dir "$UNITS"`; expect the
      application directory, unit, and generated agent plugins removed while
      `$DATA/hieronymus.sqlite`, `backups/`, configs, and semantic state
      survive; `--delete-data` (separate run) then removes exactly `$DATA`.
      A real-machine rehearsal also reruns this with the default unit dir so
      the manager paths are exercised once.

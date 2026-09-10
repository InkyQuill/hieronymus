# Distribution: build, install, update, and release (Rust era)

Status: F1 packaging and verified release staging are implemented. Installed
release qualification is recorded in [the installed rehearsal](rust-cutover-rehearsal.md),
and native host gates in [host acceptance](agent-host-acceptance.md); this document does not declare those gates
passed. Current Rust plans and ADR 0016 govern the product. Historical Python
qualification records are not release-asset inputs.

The daemon now serves the wired memory and retrieval tools. Both working
memory and real semantic RAG are required. P1 autonomous correction/viewpoint
runtime has passed scoped implementation gates; full installed 0.8.0 native/semantic
workflow acceptance remains open. Actual modern protocol and hook diagnostic probes
are distinct from that candidate qualification.

## Support matrix

| Property | Value |
| --- | --- |
| Initial target | `x86_64-unknown-linux-gnu` |
| Semantic retrieval | Required — qualified multilingual MiniLM replacement; see `docs/semantic-validation.md` |
| Release readiness | Real semantic lane must report `ready`; acquiring, rebuilding, missing/mismatched assets and FTS-only operation do not pass |
| Rust pin | 1.96.0 (`rust-toolchain.toml`) |
| Bun pin (console and release helpers) | 1.4.0 (`frontend/bun.lock`, CI `setup-bun`) |

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

## Pinned release acquisition

From the checkout, run `bun scripts/stage-release-assets.ts`. It prints a JSON
object with `HIERO_RELEASE_ONNX_RUNTIME`, `HIERO_RELEASE_ONNX_SHA256` and
`HIERO_RELEASE_MODEL_DIR`; set those exact values in the environment before
`./scripts/release-build.sh`. CI uses `--github-env "$GITHUB_ENV"` to export them.
The helper verifies fixed model/tokenizer/license/card and runtime archive/library
hashes, bounds transfers and extraction, and restricts HTTPS redirect hosts.
Runtime notices come from a fresh extraction of the verified archive; internal
library links are materialized as regular files. Successful extractions are kept
under ignored `qualification/.artifacts/models/.release-runtime-*` for the build.
Run in an owned checkout; do not modify staged files while a release build runs.

`bun scripts/check-rust-release.ts --allow-untagged` checks a local candidate.
Publishing requires `--ref refs/tags/v<workspace-version>` resolving to the exact
checked-out commit. Add `--release-dir target/release-dist --channel stable`
to bind metadata to the actual archive bytes. Offline behavioral tests run with
`bun test scripts/*.test.ts`. This replaces the archived Python acquisition and
release helpers; no Python or uv step is part of the release pipeline.

## Artifacts

`scripts/release-build.sh` (full mode) produces under `target/release-dist/`:

- `hieronymus-<version>-x86_64-unknown-linux-gnu.tar.gz` containing the
  `hiero` binary plus relative command links `hieronymus -> hiero`,
  `hieronymus-agent-hook -> hiero`, `hieronymus-mcp -> hiero` (the binary
  routes those names by `argv[0]`; the links carry them into `PATH`);
- bundled ONNX runtime/model/tokenizer, licenses/runtime notices and verified assets.json;
- release.json naming the exact archive, version, target, channel and SHA256;
- `hieronymus-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256`
  (`sha256sum` format), verified together with an extraction round-trip
  (link targets, `hiero version`, `hieronymus version` probes).

Modes: `--dry-run` (plan only), `--assets-only` (Bun pin + bundle +
console-embed compile check), default full pipeline. Running the full
pipeline locally is optional; CI runs it on every release tag.

## Release workflow

The 0.8.0 Rust alpha line is the release source authority; historical Python 1.x
tags are not Rust version metadata. The competing Python publisher is retired.
`.github/workflows/release-rust.yml` guards exact Cargo workspace/lock/inheritance
and tag-to-HEAD equality before downloads, stages pinned native assets, builds
the embedded console/native archive, validates release.json version/target/channel/
archive/hash/null signature, and attaches archive, checksum and release.json.
Rust 1.96.0, Bun 1.4.0 and build-only Python 3.12 remain pinned.

`scripts/stage-release-assets.py` reuses verified acquisition for model/runtime,
verifies committed tokenizer/license bytes and bounded HTTPS model-card transfer,
checks runtime notices/version, then exports the three builder inputs. Cache hits
are verified; fresh CI needs no machine-specific cache. `check-rust-release.py
--allow-untagged` is an explicit local-candidate check, never used by publishing CI.
`release-build.sh` honors CARGO_TARGET_DIR for the built binary; downloadable files
still go to worktree target/release-dist.

The job declares the release environment. Actual remote required-reviewer settings
are external and have not been verified by this source change. No tag, publication
or feed deployment accompanies local candidate preparation. Attaching release.json
does not create the updater's required `<base>/<channel>/release.json` HTTPS feed;
local --release-dir installation is the documented available path until a real
feed origin is selected and qualified.

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
- [ ] **CLI, MCP HTTP, MCP stdio, web, dreaming fake-provider, working-memory,
      and semantic smoke** — `hiero agent-hook session-start --cwd <project>`;
      MCP HTTP `tools/list` + one `tools/call` against the daemon; `hiero
      mcp` framed initialize/tools-list over stdio; `http://<host:port>/`
      serving the embedded console; a dreaming run with the fake provider on
      a copied root; a `hieronymus_recall` that returns saved working memory;
      a successful `hieronymus_rag_search` that returns the expected semantic-only
      source; and `hiero semantic status --json` showing daemon state `ready`, an
      intact active generation, and the selected provider/model identity.
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

## Required semantic package inputs

The full builder requires these explicit inputs; there is no model download
or language-runtime setup on the installed machine:

```sh
export HIERO_RELEASE_ONNX_RUNTIME="$PWD/qualification/.artifacts/models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"
export HIERO_RELEASE_ONNX_SHA256=1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab
export HIERO_RELEASE_MODEL_DIR="$PWD/qualification/.artifacts/models/paraphrase-multilingual-MiniLM-L12-v2"
scripts/release-build.sh
```

Bun 1.4.0 is a build-time dependency only. The builder copies the real runtime,
model, tokenizer, model card, Apache license and runtime notices, verifies
pinned hashes and runs native document/query inference. `assets.json` records
the actual identity and file hashes. Archive round-trip repeats verification
and native inference. Missing or mismatched inputs fail the build.

The version directory contains `lib/libonnxruntime.so`,
`models/minilm/{model.onnx,tokenizer.json,LICENSE,README.md}`, runtime notices
under `licenses/runtime/`, and `assets.json`. `minilm` is only a filesystem
label: metadata retains `paraphrase-multilingual-MiniLM-L12-v2` at immutable
revision `e8f8c211226b894fcb81acc59f3b34ba3efd5f42`. Runtime is ONNX Runtime
1.28.0. No qualification-record schema or attestation platform is required.

The daemon resolves bundled paths from its canonical versioned executable.
Switching back to an earlier binary therefore switches the bundled assets
with it; shared index generations still require exact identity. An explicit
`semantic.conf` runtime override retains priority and must match the qualified
runtime digest before loading. `HIERO_SEMANTIC_MODEL_DIR` explicitly overrides
the model/tokenizer directory; those files retain the current pinned checksums.
Doctor reports the selected paths and checksum failures. Checksums alone do
not claim native readiness: the supervised daemon publishes that separately.

## Local and remote release sources

No public hosting endpoint is assumed. Configure `HIERONYMUS_RELEASE_URL` or
pass `--release-url`; `--release-dir` remains available. Supplying both is an
error. `--channel stable|dev` (or `HIERONYMUS_RELEASE_CHANNEL`, default stable)
selects `<base>/<channel>/release.json`. Remote metadata must declare that
same channel. The archive is fetched beside that metadata. A feed directory
can therefore be published under either configured channel without changing
the updater. The builder writes the chosen channel to `release.json`.

```json
{"version":"0.7.0","target":"x86_64-unknown-linux-gnu","channel":"stable","archive":"hieronymus-0.7.0-x86_64-unknown-linux-gnu.tar.gz","sha256":"<actual 64-digit archive digest>","signature":null}
```

Remote URLs require HTTPS, valid authority/port, and no userinfo, control
characters, query or fragment. Rust staging uses the shared parsed URL and
rustls trust roots. Redirects are refused, so a downgrade cannot occur.
Metadata is limited to 64 KiB and archives to 1 GiB; expanded archives are
limited to 3 GiB. Transport certificate verification stays enabled. A non-null
signature is refused because signature verification remains unconfigured for
this release line; SHA-256 is integrity evidence, not an independent signature.

Local and remote staging share checksum, target, archive-path and link checks
before activation. Archive paths are a fixed allowlist; duplicate entries,
traversal, hardlinks, devices and symlinks other than the three relative command
aliases are refused. Bootstrap verifies checksums, extracts a single bounded
regular bootstrap binary into a temporary directory, then invokes its typed
release verifier and the existing ownership-checked updater. Required assets
and real native inference are verified before stable links change.

Post-start update readiness waits up to 90 seconds and accepts only actual
`ready`, never an acquiring or rebuilding response. Timeouts fail and route
through the existing rollback. Cold installed timings are measured by the
explicit installed test, not inferred from the historical debug run:

```sh
HIERO_TEST_RELEASE_DIR="$PWD/target/release-dist" CARGO_BUILD_JOBS=4 \
  cargo test -p hiero --test installer_flow --locked -- \
  --ignored --nocapture
```

This test runs the installed daemon and MCP CLI with an empty PATH, no
`LD_LIBRARY_PATH`, no explicit runtime setting, and no model copy in the data
root. It checks real bundled startup and the fixed multilingual retrieval
corpus. It is application integration evidence; it does not substitute for
native host-handshake or autonomous-correction acceptance.

IPv4 loopback TLS staging is exercised with a real trusted test certificate,
including certificate rejection and redirect refusal. IPv6 URL parsing and
bracketed HTTP authority formatting are covered deterministically. The native
IPv6 loopback test is explicit and opt-in: this qualification host permits the
listener but times out the TCP connection before TLS (60-second bounded
connect). That probe is recorded as unverified, not a passing IPv6 TLS claim.


## Installed rehearsal and remaining cutover gates

[The F2 rehearsal](rust-cutover-rehearsal.md) attributes each check to its actual
archive and separates native ONNX/CLI/MCP/browser results from controlled
provider or service-manager fixtures. Both mandatory memory lanes remain
required. The accepted autonomous authority design and actual Claude/Codex/Pi
native workflow compatibility remain open product gates; zCode is paused/unqualified. Do not
infer readiness for live migration or publication from the packaging checks.

The installed runtime has been exercised with an empty PATH and no external
runtime/model override. Build and browser-test tools are not runtime
prerequisites. Local rehearsal uses explicit disposable `--app-dir`,
`--data-root`, `--unit-dir` and `--no-activate`; it does not operate the user's
service manager. A separately compiled test-only version qualifies the
version transition without changing or publishing the repository version.

The owner amendments dated 2026-09-03 in the Rust migration program and
compatibility contracts specifications preserve qualification records as archived
evidence. PR CI validates retained fixtures with their explicit historical test
inventory and runs the tool tests, but does not require archived measurement
fingerprints or per-current-test manifest dispositions to remain fresh. Current
Rust tests and installed artifact/native host acceptance govern this release.

### Alternative semantic backend: Ollama

ONNX remains the bundled default. `hiero semantic configure --provider ollama
--base-url http://127.0.0.1:11434 --model <installed-model>` selects externally
managed Ollama embeddings while retaining the bundle's pinned tokenizer for
existing chunk segmentation. It does not require loading the ONNX model/runtime
and never pulls an Ollama model. The existing authenticated daemon, revision
acknowledgement, durable rebuild and strict semantic readiness checks apply.
Provider/model digest/dimension changes require a coherent rebuilt generation.

Development roots without bundled assets must stage the pinned tokenizer or
complete explicit tokenizer acquisition; the current downloader rejects upstream
redirects. See the Ollama setup section in `usage.md`. Native agent-host matrix
qualification remains deferred separately from this embedding functionality.

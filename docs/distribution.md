# Build, install and publish

This is the maintainer guide. Authors should start with [Usage](usage.md).
Rust binaries embed the Svelte console and run without Python, Node or Bun on
an installed machine. Bun builds the console and runs release helpers.

## Build ownership

Use the pinned toolchains and lockfiles in the repository. The workspace is
virtual; `crates/hiero/build.rs` owns binary build integration.

1. `scripts/release-build.sh` delegates to `release-build.ts` and builds the
   production frontend before compiling binaries with `console-embed`.
2. The release-profile embedded build refuses a missing/empty `frontend/dist/index.html`.
   Frontend-free developer targets remain supported; no bundle means
   `web_console_not_built`, not an invented console.
3. Native builds produce CLI, matching desktop helper and Windows launcher as needed.
   Headless builds do not link native tray libraries.
4. Packaging consumes final executable bytes after stripping/signing where applicable,
   preserves upstream runtime bytes and retains diagnostic symbols separately.

Ordinary tests do not invoke Bun. All-features lint can compile without a console
bundle. Served embedded assets exclude source maps. For focused development and
full verification, follow [AGENTS.md](../AGENTS.md#verification).

## Acquire and package

`bun scripts/stage-release-assets.ts --target <triple>` verifies pinned runtime/model
inputs and returns staging paths. Exact hashes and provenance live in
`scripts/onnxruntime-targets.json` and the model acquisition code. Do not duplicate
those inventories in prose. Transfers/extraction are bounded and unsafe paths,
wrong hashes or incompatible native inputs fail before activation.

Produce the common model archive once with `scripts/shared-model.ts`, retain its
`common-model.json`, and distribute identical bytes to every native target. Do not
recompress per platform. The build uses `HIERO_RELEASE_RUNTIME_DIR` and
`HIERO_COMMON_MODEL_DIR`:

```sh
HIERO_RELEASE_RUNTIME_DIR=/absolute/verified/runtime \
HIERO_COMMON_MODEL_DIR=/absolute/canonical/common-model \
CARGO_BUILD_JOBS=2 scripts/release-build.sh --target x86_64-unknown-linux-gnu
```

Use `--dry-run` to inspect the build plan. `--assets-only` packages existing native
inputs; `--out` selects distribution output. `CARGO_TARGET_DIR` locates compiled
files, not the distribution directory. Keep binaries when their inputs are unchanged;
installer/documentation-only fixes do not justify another native matrix build.

Each target emits `release-<triple>.json`, a platform archive, the shared model archive
and available installers/diagnostics. Format 2 binds both archives and assembled
assets. Platform archives contain no model payload. Installers assemble a complete
immutable version tree before selecting it. Legacy monolithic `release.json` remains
input-only; old updaters bootstrap split support through the current standalone installer.
See [platform artifacts](desktop-platforms.md) for details.

## Release workflow

[Automatic releases](automatic-releases.md) owns the conventional-commit/version-PR
sequence. `desktop-candidate.yml` builds retained native artifacts;
`release-rust.yml` promotes verified compatible bytes without rebuilding.
It checks exact tag/workspace identity and open P0 issues. Merging the version PR
is the publication decision; routine code/doc merges do not publish.

Native evidence is optional. `desktop-evidence.yml` ingests captures from a separate
immutable data commit; missing evidence or optional installers remains a disclosed
warning. Available working artifacts can be published without a complete interactive
matrix. Failed/unrun checks must retain their real status and actionable non-P0
failures go to an existing or new issue with reproduction/run links. Corrupt archives,
wrong hashes, unsafe ownership or inconsistent retained identities remain refusals.
The release environment declaration does not prove its remote reviewer configuration.

Validate a local candidate with `bun scripts/check-rust-release.ts --allow-untagged`.
Publication binds an immutable `refs/tags/v<workspace-version>`; never move a published
tag. Recovery reuses retained candidate artifacts and reruns the affected packaging
or check job. See [desktop checks](desktop-qualification.md).

## Install, update and recovery

The installation owns `<app>/versions/<version>` and stable command entry points
under `<app>/bin`. Linux user PATH links point there. Windows launchers select the
verified version record; macOS app layout is bound by external manifests.
Every installed version carries its own model/runtime files, so rollback does not
rely on a mutable acquisition cache.

Bootstrap selects target metadata from official GitHub releases or an explicit
`--release-dir`, verifies bytes and safe extraction, installs atomically and runs
owned service/desktop registration and diagnosis. Schema upgrade is explicit:
a legacy database leaves the server stopped with migration instructions.
[ADR 0010](adr/0010-data-locations-schema-ownership-and-upgrade.md) owns backups,
conversion and recovery.

Update checks protocol/schema compatibility and ownership, stages alongside,
stops/retire-coordinates the owned processes, switches the version and registrations,
and checks the authenticated candidate. Pre-schema health failure restores the prior
version/registration. After an incompatible schema upgrade, downgrade is unsupported.
An acquiring/rebuilding index can leave the verified candidate installed with an
`index-rebuilding` warning; it is not reported as semantic ready.

Uninstall removes owned software/registration/integration entries and preserves
memory, configuration, models, backups and audits by default. Explicit `--delete-data`
identifies the selected root and clears contents while retaining recognized held
coordination inodes. Never delete unrelated book folders or foreign registrations.
Custom registration directories are test seams; they must not contact the real
manager or be persisted into normal user registration.

## Release sources and integrity

`hiero update --check` reads published releases from `InkyQuill/hieronymus` without
installing. `--channel stable|dev` overrides `release.conf`; stable selects a published
stable release and dev a published prerelease with target assets. Drafts and missing
target metadata are excluded. There are no arbitrary remote feeds or release-source
environment overrides. `--release-dir` supplies local artifacts and cannot combine
with `--check`.

HTTPS redirects are bounded and TLS verification stays enabled. Metadata/archive/
expanded-size limits, duplicate/path/link validation and exact member hashes protect
installation before activation. Archive authentication uses retained snapshots so
source-file changes after hashing cannot change accepted bytes. SHA-256 is integrity
evidence, not an independent signature. This unsigned release line refuses non-null
signatures it cannot verify rather than pretending they were checked.

Pins bind model/tokenizer/runtime identity; native inference establishes readiness
separately. macOS/Windows signing state and Intel desktop qualification are disclosed
in release notes. Historical receipts qualify their exact inputs only; see
[runtime checks](rust-cutover-rehearsal.md) and [host checks](agent-host-acceptance.md).

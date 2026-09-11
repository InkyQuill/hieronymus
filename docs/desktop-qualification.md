# Desktop qualification and release promotion

Implementation does not establish native delivery. The initial matrix is Linux
x86_64 (KDE and GNOME AppIndicator, each on Wayland and X11), Windows x86_64,
Apple Silicon, and Intel macOS. Seven session records are required. Unavailable
or failed checks block publication. Current Intel ONNX 1.28.0 remains
`source-build-required`; no complete four-target candidate can succeed yet.
Windows/macOS interactive acceptance and Linux real-panel/login/native-manager
acceptance are also outstanding. See the final Linux receipt and limits below.

## Candidate, evidence, promotion

1. Once this workflow has reached the repository default branch, select a branch
   pointing at the reviewed source commit. Dispatch `desktop-candidate.yml`
   using that branch as `--ref`. `workflow_dispatch` takes a branch/tag ref;
   do not assume a raw SHA works. Record the run's `head_sha` and do not move
   the branch during dispatch. No workflow is dispatched by this implementation.
2. The Linux/Bun 1.4.0 common-model job produces the canonical archive once.
   Every native job downloads those exact bytes, verifies its descriptor, stages
   its own pinned runtime, builds the console before embedding, strips only
   Hieronymus binaries, and packages v2 metadata. Native fixture inference/MCP
   tests use the final archive assembly. Candidate inventories bind all files,
   including installers and actual diagnostic symbols. No model recompression
   occurs in target jobs. Candidate artifacts remain available for 90 days.
3. Download each `candidate-<exact-target>` artifact for manual acceptance.
   `gh run download RUN --name candidate-TARGET --dir candidate` is read-only.
   A failed Intel job does not erase successful other-target artifacts; these can
   be inspected/qualified locally, but the failed overall run cannot be promoted.
   Keep archive bytes unchanged. Native build jobs are execution on their target,
   but do not demonstrate a working desktop menu or next login.
4. Save manual records and captures under `qualification/desktop-evidence/` on
   a **separate evidence data branch/commit**. Keep logs and screenshots small;
   this data commit is not the commit that built the candidate. Record its full
   immutable SHA. Never execute scripts supplied by the evidence branch.
5. Dispatch `desktop-evidence.yml` from the same candidate source branch/ref
   (still pointing to the same commit), with `candidate_run=RUN` and
   `evidence_commit=FULL_DATA_SHA`. It accepts only a successful same-repository,
   same-source `desktop-candidate.yml` workflow-dispatch run. It checks out only
   evidence data separately, validates all records and files, then uploads one
   `desktop-evidence` artifact with candidate/data provenance. Evidence-side
   code is never executed. Unknown attachments, aliases and paths escaping the
   evidence directory are rejected; each capture is 1 byte–50 MiB, total 250 MiB,
   and each JSON document is at most 64 KiB.
6. After all four targets, Intel pin promotion and all native observations pass,
   set repository variables `DESKTOP_CANDIDATE_RUN` and `DESKTOP_EVIDENCE_RUN`
   to those numeric successful run IDs. A reviewed exact `v<workspace-version>`
   tag pointing to the candidate commit triggers `release-rust.yml`. The release
   environment still needs independently configured required reviewers. Its
   declaration is not evidence of that remote setting.
7. Promotion verifies source/tag/version, expected workflow names, repository,
   successful workflow-dispatch events, exact source SHA, inventory digests,
   evidence provenance and all seven sessions again. It publishes an explicit
   verified file allowlist. It never rebuilds, recompresses, strips or signs.
   Merely merging changes cannot publish or create a tag. No partial target
   release is allowed. Missing IDs, expired artifacts or failed evidence stop
   publication; retain/renew qualification before creating the tag. If a tagged
   run fails only because IDs were missing, set the reviewed variables and rerun
   that same workflow. Do not move or recreate a published tag.

The cost is deliberate: publication waits for the complete matrix, reviewed
Intel pins and retained artifacts. Signing remains explicitly `unsigned-waiver`;
this is not signing or notarization acceptance. Any future signing/stapling
happens before final manifests and qualification, requiring a reviewed producer
change. Never mutate pinned upstream runtime bytes. On macOS the final assets
manifest remains outside the sealed app; inner code precedes outer bundle seals.

The full historical default workspace integration suite runs on Linux in the
headless job. Native desktop jobs run helper clippy/tests/rustdoc, compiling the
shared native graph, plus `private_file::tests` on Windows/macOS and the named
Windows regular-launcher selected-helper contract. Candidate jobs additionally
run the explicit final-payload inference/MCP fixture. Linux/systemd-only legacy
fixtures are not mislabeled as a portable full-suite gate. Native Scheduler and
LaunchAgent state acceptance remains separate.

## Evidence format

`records.json` is an array of exactly seven relative JSON filenames, one per
matrix session. Each record contains:

```json
{
  "candidate_commit": "<40 lowercase hex characters from candidate run>",
  "target": "x86_64-unknown-linux-gnu",
  "os_version": "<actual OS/build>",
  "desktop": "kde",
  "session": "wayland",
  "scale": [1, 2],
  "artifact_sha256": "<platform archive digest>",
  "model_sha256": "<common model digest>",
  "metadata_sha256": "<exact release-TARGET.json digest>",
  "assets_sha256": "<assets-TARGET.json digest>",
  "signing": "unsigned-waiver",
  "checks": [{
    "name": "interactive-menu",
    "result": "unavailable",
    "evidence_path": "captures/kde-wayland-menu.txt",
    "evidence_sha256": "<actual capture digest when available>"
  }]
}
```

Create a complete unavailable template on each native host from the reviewed
source, for example (use the matching native desktop/session):

```sh
bun scripts/new-desktop-evidence.ts candidate FULL_CANDIDATE_SHA kde wayland \
  qualification/desktop-evidence/kde-wayland.json
```

The generator uses the host target and exact metadata identity, never infers a
pass, and leaves capture hashes null. Record observed results, actual scale values
and capture SHA256s. The template cannot pass the gate unchanged. Save no bearer
credentials, launch grants or book text in captured logs. Add all seven relative
record filenames to `records.json` after combining the native observations.

Include every name returned by `requiredChecks(target)` in
`scripts/check-desktop-evidence.ts`; the example above is intentionally incomplete
and must fail validation. Use desktop/session `gnome-appindicator/wayland`,
`gnome-appindicator/x11`, `kde/x11`, `windows/native`, or `macos/native` for the
remaining records. Both Mac targets need separate records. Scale must include
1 and an actually observed higher scale. Record `fail` or `unavailable` honestly;
only observed `pass` plus an existing, nonempty, SHA256-matching capture qualifies.
Captures should describe the trigger, expected/observed behavior, exact fixture
and time; include screenshots when they help show a menu/theme/DPI result. A
reused capture can cover multiple observations if its content substantiates each.
The validator checks binding/completeness, not the truthfulness of human reports;
reviewers must inspect them. Synthetic test fixtures are never native evidence.

Run locally from the reviewed candidate source:

```sh
bun scripts/check-desktop-evidence.ts --release-dir candidate \
  --evidence-dir qualification/desktop-evidence --commit FULL_CANDIDATE_SHA
```

## Weekend native procedure

Use a disposable OS user account or VM snapshot for **native registration,
login/session and panel/Explorer restart**. The normal user account may already
own registrations with the product's names. A custom `--unit-dir` is
**definition-only**; it cannot qualify active manager behavior. Never use a real
book or existing Hieronymus data root. Keep the verified platform/common archives,
metadata, receipt, assets manifest and standalone installer together. No deployed
channel URL is configured; use `--release-dir`.

On Windows x64, use PowerShell 7.4 or later in that disposable account:

```powershell
$release = (Resolve-Path .\candidate).Path
$fixture = Join-Path $env:TEMP ("hiero-native-" + [guid]::NewGuid())
New-Item -ItemType Directory $fixture | Out-Null
$app = Join-Path $fixture "app"
$data = Join-Path $fixture "data"
& "$release/install-desktop-x86_64-pc-windows-msvc.ps1" -ReleaseDir $release -AppDir $app -DataRoot $data -NoActivate
$cli = Join-Path $app "bin/hiero.exe"
& $cli version --json
& $cli doctor --data-root $data --json
# Controlled native activation, in the disposable account only:
& $cli tray --data-root $data --binary $cli
```

Use `Get-FileHash -Algorithm SHA256` for archive/record/capture digests. Record
`Get-ComputerInfo` OS/build and `$PSVersionTable.PSVersion`, actual Task Scheduler
state, private root/token ACLs, enabled-but-stopped behavior, timeout continuation,
held native-operation refusal, and replacement while executables are in use.
Do not turn an indeterminate native timeout into permission to remove a record.
For uninstall, execute the external installer `-Uninstall` so an in-use selected
binary does not remove itself; retain disposable settings to verify preservation.

On Apple Silicon, use Terminal in the disposable account:

```sh
release="$PWD/candidate"
fixture="$(mktemp -d)"
app="$fixture/app"
data="$fixture/data"
bash "$release/install-desktop-aarch64-apple-darwin.sh" \
  --release-dir "$release" --app-dir "$app" --data-root "$data" --no-activate
"$app/bin/hiero" version --json
"$app/bin/hiero" doctor --data-root "$data" --json
"$app/bin/hiero" tray --data-root "$data" --binary "$app/bin/hiero"
```

Record `sw_vers`, `uname -m`, launchctl loaded/unloaded and RunAtLoad state,
unsigned bundle launch behavior, and `otool -L` dependency diagnostics. Runtime
and model live outside the app in the verified version tree. Never edit a sealed
bundle during install. Intel is unavailable until the native source-build receipt
is reviewed and a compiled `source-built` authority is implemented with actual
archive/member/provenance hashes. `macos-15-intel` candidate CI is prepared;
no native Intel execution occurred in this work. A candidate receipt never grants
runtime authority by itself and requested deployment target is not measured OS
compatibility. Record actual minimum OS/dependencies before promotion.

Linux uses the corresponding `.sh` installer and checks `ldd` on CLI and helper.
GTK is required only for the helper. Test KDE and GNOME AppIndicator separately
in Wayland and X11 sessions. A private watcher fixture is not the real panel;
record an actual host/panel restart and missing-host behavior. Keep these visible
host operations in the disposable account.

For every native session observe: startup/duplicate start; actual menu actions;
model acquisition and actual inference; missing/corrupt semantic payload; provider
failure, recovery and never-tested state without background paid calls; three
probe failures; restart; failed stop; Quit waiting for confirmed daemon stop and
preserving login; toggle login then next login; missing tray host and host restart;
light/dark theme and standard/high DPI; one-use console grant; native active
upgrade, rollback and uninstall/settings preservation; pending native operations,
private storage failures and secret-free diagnostics. Use distinct honest compiled
versions for upgrade/rollback, not edited metadata or replacing immutable 0.9.0.
Never delete ambiguous ownership records based only on PID disappearance.

`qualify-desktop-native.ts`/the ignored `desktop_native` test verifies real final
payload inference, authenticated MCP and shutdown in a disposable root, separately
from these interactive checks. Explicit missing fixture inputs fail. For a manual
assembled payload set `HIERO_DESKTOP_INSTALLED_CLI` to its final CLI and run:

```sh
cargo test -p hiero --locked --test desktop_native -- --ignored --nocapture
```

PowerShell uses `$env:HIERO_DESKTOP_INSTALLED_CLI = ".../hiero.exe"`. Keep logs
bound to the candidate. Record cleanup of only the disposable owned registration
through the installer/uninstaller after confirming stopped state; preserve the
retained captures/archives and do not touch the regular account's registrations.

## Durable local evidence

Final outputs/logs are under `qualification/.artifacts/desktop-task14/` (git
ignored). The final measured receipt is `docs/desktop-task14-linux-receipt.json`.
Historical Task 9–13 reports, rulings, logs and fixture sources are retained under
`qualification/.artifacts/desktop-history/`, with their former SDD-relative paths
preserved. Old absolute `.superpowers` references in archived reports identify
historical locations; resolve their suffix within that durable directory. The
canonical common archive is also copied to `qualification/.artifacts/common-model/`.
None of these binaries/models is committed. Historical Task 12 receipts do not
qualify the final embedded Task 13 frontend. The final receipt states exactly what
was rerun and which native gates remain unavailable.

### Producing the currently missing weekend candidates

No Windows or Mac binary has been built on this Linux host. If the native CI
candidate artifacts do not yet exist, build on the actual target from the reviewed
source. Native development prerequisites are Rust 1.96.0, Bun 1.4.0, protobuf
compiler **and its standard includes**, and the platform C/C++ toolchain (MSVC on
Windows, Xcode command-line tools on macOS; GTK development headers on Linux).
The packaged application itself does not require Rust, Bun or Python. Use
`scripts/check-protobuf.sh` to detect missing standard imports. Check Windows DLL
load failures/dependencies with the MSVC `dumpbin /DEPENDENTS` tool when available;
record unavailable diagnostics honestly.

Copy the already-produced canonical model archive plus `common-model.json` into
`target/common-model` without recompressing. On Windows PowerShell:

```powershell
$stage = bun scripts/stage-release-assets.ts --target x86_64-pc-windows-msvc | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Asset staging failed' }
foreach ($property in $stage.PSObject.Properties) {
  [Environment]::SetEnvironmentVariable($property.Name, [string]$property.Value, 'Process')
}
$env:HIERO_COMMON_MODEL_DIR = (Resolve-Path target/common-model).Path
$env:CARGO_BUILD_JOBS = '2'
bun scripts/release-build.ts --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Native package failed' }
$env:DESKTOP_TARGET = 'x86_64-pc-windows-msvc'
bun scripts/qualify-desktop-native.ts
```

On Apple Silicon:

```sh
bun scripts/stage-release-assets.ts --target aarch64-apple-darwin > target/native-stage.json
export HIERO_RELEASE_RUNTIME_DIR="$(bun -e 'console.log((await Bun.file("target/native-stage.json").json()).HIERO_RELEASE_RUNTIME_DIR)')"
export HIERO_COMMON_MODEL_DIR="$PWD/target/common-model"
export CARGO_BUILD_JOBS=2
bun scripts/release-build.ts --target aarch64-apple-darwin
DESKTOP_TARGET=aarch64-apple-darwin bun scripts/qualify-desktop-native.ts
```

These local outputs support observation/review only. They are not a substitute for
the successful candidate-run provenance required by tag promotion. If later CI
rebuilds different bytes, repeat the observations on those final bytes. Keep the
same candidate alive through qualification; do not overwrite an immutable output
directory or assign a different binary the same installed version identity.

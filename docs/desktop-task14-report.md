# Task 14: candidate CI, native evidence and final qualification

Task 14 is implemented for independent review, with external native release gates
still open. No workflow dispatch, push, tag, publication, real user daemon/login
or book mutation occurred. Baseline was
`077732f7b7a1b58239de9c1237f8ee64cf630afd`; worktree is
`/home/inky/Development/hieronymus/.worktrees/desktop-tray-design`.

## Implemented interfaces

- PRs preserve the default headless graph and protobuf preflight/resource limits;
  explicit helper jobs compile/test/doc on Ubuntu x64, Windows x64, macOS arm64 and
  `macos-15-intel`. GUI dependencies are installed only in desktop jobs. The full
  historical default integration suite stays Linux-only; named private-file tests
  run on Windows/macOS and the selected-helper contract runs on Windows.
- `desktop-candidate.yml` produces one canonical common-model archive and feeds
  the exact bytes to native jobs. The portable `release-build.ts` builds the
  console before embedding, native CLI/helper and v2 packages. The old shell
  entry point delegates; no monolithic output/publication path remains.
- `desktop_native` is an explicit ignored fixture test for final payload
  inference, authenticated MCP status and shutdown. Missing fixture input fails.
  The test creates a disposable root and does not register a native service.
- `check-desktop-evidence.ts` validates exact candidate/platform/model/metadata/
  assets identity, OS/desktop/session/DPI, unsigned waiver and named observations.
  Every required check must pass and point at nonempty SHA256-matching regular
  evidence. Missing, failed, unavailable, duplicate/unknown and escaping records
  fail. All seven sessions/four targets are required, with identical model hashes.
- `desktop-ci.ts` verifies workflow/repository/source/event/success identity,
  numeric run IDs and explicit candidate attachment inventories; changed installer
  bytes or unknown attachments fail. Evidence ingestion never executes data-branch
  code, checks a separate full data commit, bounds captures, and records provenance.
- The sole publisher remains an exact matching `v*` tag workflow. It downloads
  retained successful same-source candidate/evidence runs from fixed workflow
  identities and publishes only verified files after revalidation. It never
  rebuilds/recompresses/signs. Missing Intel pins/native checks or expired artifacts
  block publication; no partial-target release is allowed.

The controller approved this policy and separate data-commit ingestion before
publication wiring. It avoids committing evidence into the source that built the
same bytes. Local/native operator commands and future dispatch/tag order are in
[desktop qualification](desktop-qualification.md). Remote environment reviewer
settings remain independently unverified. CI source checks do not establish that
those native jobs executed.

## Regression evidence and review observation

The validator's deliberately disabled enforcement produced **1 pass / 8 fail**;
restoring it produced **9 pass / 0 fail**. Subsequent tests also cover unknown
checks, absent files, candidate-run provenance, all-four-target file inventories
and unexpected evidence attachments. RED/GREEN logs are in durable ignored
`qualification/.artifacts/desktop-task14/logs/`.

The first full Rust run stopped after **772 pass / 1 fail / 16 ignored** at
`uninstall_cli::delete_data_removes_the_exact_named_root_only`. Its stale expected
set omitted `.desktop-launch.lock`; the existing production unit test already
expects that persistent gate. Controller approved updating the CLI test to exactly
`.desktop-launch.lock`, `.lifecycle.lock`, `.owner.lock`, followed by focused and
full reruns. Focused CLI tests passed **4/4**. No production uninstall change was
made.

A separate source observation remains reserved by the controller for whole-branch
review: `uninstall.rs` retains every basename ending `.lock`, so arbitrary user
`foo.lock` survives explicit `--delete-data`. Native manager/browser and per-session
tray gates also require persistent inodes. This report does not claim only three
known files can survive or silently narrow that contract. The durable observation
is `qualification/.artifacts/desktop-task14/whole-branch-review-observation.md`.

## Global update-flow regressions and approved ownership correction

The next full run reached `update_flow` and found six failures. The shared fake
candidate fixture now emits the required scoped doctor JSON; an exit-1 fixture
carries a semantic warning and still rolls back. The active custom-manager test
retains exit 2 and state assertions with the current truthful diagnostic. The
service-unit fixture now verifies the stable launcher and its new selected
version, matching the desktop installer contract. Empty doctor output is never
accepted and full/service/semantic rejection tests remain in place.

One failure was a real phase-classification issue: missing discovery made a held
root appear offline, helper retirement started, ownership acquisition failed, and
unconditional rollback attempted to stop the still-owned old daemon. The controller
approved an early offline ownership guard after the existing lifecycle and
registration claims, before native snapshot/helper retirement. Failure now refuses
with exit 2; success retains the **same** guard through preparation/activation.
The live path still captures state, performs authenticated stop, proves release
and acquires ownership, including rollback after potentially partial native
suppression. No unlock/reacquire gap was added.

The new regression starts a disposable actual daemon, removes only its discovery
record, and injects a manager spy. RED observed the unwanted `["stop"]` call. GREEN
requires no manager calls or retirement gate, unchanged selection, absent candidate
and staging, and continued old-root ownership. This is source/fixture evidence,
not native Windows/macOS manager acceptance. The controller approved the narrow
production correction and preserved lock order explicitly.

## Verification and final Linux artifact

All Cargo commands were serialized after sourcing the supplied `environment.sh`,
with Rust 1.96.0, jobs 2 and test threads 2. The preserved reproduction scripts
now source `qualification/desktop-environment.sh`. Final checks passed:

| Command | Actual result |
|---|---|
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo test --all-features --locked` | 1,491 passed, 0 failed, 17 ignored |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | passed |
| `cargo clippy -p hiero-desktop --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo test -p hiero-desktop --all-features --locked` | 18 passed, 0 failed |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p hiero-desktop --no-deps --all-features --locked` | passed |
| `bun test scripts/*.test.ts` | 100 passed, 0 failed, 179 expectations |
| frontend `bun run typecheck` | passed |
| frontend `bun run test` | 85 passed across 9 files |
| frontend `bun run build` | passed, 124 modules |
| actionlint 1.7.9 on all four workflows | passed |

Focused ownership/update/uninstall verification passed 25 unit tests, 15 update
integration tests and 4 uninstall integration tests before the final full rerun.
No retained Python source or tools changed, so the Python-reference gates do not
apply. Logs are retained under `qualification/.artifacts/desktop-task14/logs/`.

The final Linux receipt is
[desktop-task14-linux-receipt.json](desktop-task14-linux-receipt.json). Historical
Task 12 hashes are not reused as final Task 14 evidence.

`bash qualification/.artifacts/desktop-task14/build-final.sh` rebuilt the frontend,
passed four production embed checks, built the optimized native CLI/helper and
packaged private stripped copies. The final platform archive is 97,766,207 bytes,
SHA256 `a889b5a31f01a0757e64f2eda96a8cb0d31989d610e681c5047413056817bdd4`.
The one canonical common archive stays 435,109,879 bytes, SHA256
`4a23a216615c6224b1dba9fcd067e2da0d0be2b460b39dbb468b185f152911c0`.
Final CLI/helper shipped sizes are 223,518,824 / 13,012,840 bytes. The receipt
binds installed members, separate symbols/Build IDs, runtime, partial source
inventory and durable log hashes; [binary sizes](binary-size.md) distinguish
published 0.8.0, historical Task 12 and current bytes.

`bash qualification/.artifacts/desktop-task14/qualify-final.sh` assembled that
pair, observed the mandatory missing-input failure, then explicitly ran:

- `HIERO_DESKTOP_INSTALLED_CLI=<final-payload>/hiero cargo test -p hiero --locked --test desktop_native -- --ignored --nocapture`: **1 passed**, 6.56 seconds; real final-payload inference, authenticated MCP status and shutdown.
- `HIERO_DESKTOP_RELEASE_DIR=<final-release> HIERO_DESKTOP_BASELINE_DIR=<complete-published-v0.8.0> cargo test -p hiero --locked --test desktop_update real_linux_bootstrap_upgrade_rollback_and_uninstall -- --ignored --nocapture`: **1 passed**, 54.28 seconds; fresh install, genuine stopped baseline upgrade, foreign-registration rollback, uninstall and settings preservation.

The retained disposable fixture `/tmp/hiero-real-desktop-TRkjOm` was copied in
full to `qualification/.artifacts/desktop-task14/disposable-offline/`; the receipt
records that relocation so absolute historical paths remain interpretable. There
is no live service in the evidence copy. Fresh headless dependency inspection,
CLI/helper `ldd`, and `check-rust-release.ts --allow-untagged --target
x86_64-unknown-linux-gnu --release-dir <final-release> --channel dev` passed.
The evidence CLI rejected an empty local record index with `complete seven-session
native matrix required`; this expected rejection establishes no native pass.
No real user service manager, login configuration or book data was touched.

## Native release gaps and preserved evidence

Windows, both macOS architectures, final KDE/GNOME real-panel/session/login and
native-manager active update/rollback/live helper acceptance remain unexecuted.
Intel ONNX 1.28.0 source-build candidate tooling is wired on an Intel runner, but
there is no measured/promoted runtime archive or compiled `source-built` authority.
Its receipt cannot authorize itself. Signing/notarization credentials are absent;
unsigned waiver is explicit. Native automated fixtures do not qualify menu,
credentials/ACL behavior, next login, OS compatibility or native agent hosts.

Historical reports, rulings, logs, native runtime inputs and fixture sources are
preserved under `qualification/.artifacts/desktop-history/` with SDD-relative paths
retained; root will refresh final reviews before cleanup. Canonical model bytes
are also retained at `qualification/.artifacts/common-model/`, pinned local Bun at
`qualification/.artifacts/desktop-tools/`. Final artifacts/logs/reproduction scripts
live under `qualification/.artifacts/desktop-task14/`. The committed
`qualification/desktop-environment.sh` makes the local tool/resource setup usable
without SDD. Large binaries/models/debug artifacts are not committed.

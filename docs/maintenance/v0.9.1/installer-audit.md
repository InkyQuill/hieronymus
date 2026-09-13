# v0.9.1 installer maintenance

The owner requested a Windows installer, a macOS package, and a Linux one-liner.
No repository clone or developer runtime is required. Windows and macOS signing
are explicitly waived. Intel macOS remains unqualified for physical desktop use.

## Failures and fixes

| Evidence | Finding | Disposition |
| --- | --- | --- |
| Actions runs 34745284872 and 34745520245 | Actual macOS package installation failed because `launchctl print` reports a `probabilistic guard malloc policy` dictionary. | Accept this exact diagnostic field while preserving strict ownership and launch-policy validation. The captured native output is a regression fixture. |
| Actions run 34745520245 | NSIS launched the redirected 32-bit PowerShell environment; `Get-FileHash` was unavailable. | Run the native 64-bit PowerShell environment and use .NET SHA-256 streaming directly. |
| Local installer tests after version bump | Candidate inventory fixture hardcoded 0.9.0 while validation correctly required the source version. | Derive the fixture version from the checked workspace. All 129 script tests pass. |
| Removal review | Windows cannot delete the currently executing installed CLI. | Retain the verified real CLI outside the managed application directory for NSIS removal. |
| Actions run 34746393407, native Windows setup | Setup and installed version output succeeded, but the CI check read a stale exit code from the GUI subsystem launcher. | Explicitly wait for the process and check its returned exit code. Apply the same rule to opening the author interface. |
| Actions run 34746393407, direct PowerShell setup | Native installation reported access denied during link switching and rolled back. | Retained for repeat investigation; not yet qualified as resolved. |

## CodeRabbit review

The complete CLI receipt is retained in `coderabbit-installer-review.jsonl`.

- **Major: Cargo.lock version mismatch.** Not reproduced. Cargo.lock already has
  hiero, hiero-desktop, and hieronymus 0.9.1. Locked Cargo checks and the release
  source checker passed.
- **Minor: manual workflow still defaults to v0.9.0.** Fixed to v0.9.1. Baseline
  investigation runs continue to pass v0.9.0 explicitly.
- **Critical: duplicate try/finally block in PowerShell.** Not present in the
  checked file. It contains one outer cleanup finally and keeps the removal
  helper copy inside the try. Native PowerShell parsed it and NSIS completed
  installation in run 34746393407; its later exit-code assertion was separate.

The companion Rust practices verifier reviewed the exact parser amendment and
regression test without actionable findings. Its review was static.

## Local validation

- Cargo fmt, all-target/all-feature locked Clippy, all-feature locked tests, and
  warning-free rustdoc passed.
- All 129 release-script tests passed, including corrupt archive rejection,
  bounded cleanup, spaced paths, offline cache verification, Rosetta selection,
  and the public download whitelist.
- Frontend typecheck, all 86 tests, and production build passed.
- A real standalone online Linux installation of published v0.9.0 binaries
  succeeded from `/tmp`, outside a source checkout, into disposable directories.
  This checks the installation entry point; it is not qualification of v0.9.1.

## Release evidence policy

Public downloads contain three setup entry points and nine app/model/metadata
payloads used by automatic installation. Native qualification captures, candidate
inventories, debug symbols, and build receipts remain maintainer/CI evidence.
The old v0.9.0 ONNX source-build URLs remain intact because source acquisition
pins them; they are not repeated in the new release.

Final candidate identities, native installer outcomes, Linux qualification, and
publication verification will be recorded after those checks complete.

## Subsequent installer checks

Runs 34746509038 and 34746652888 retained the intermittent Windows link-switch
access error and exposed NSIS's asynchronous uninstaller copy. The removal check
now waits for the actual uninstaller, per the official NSIS `_?=` contract, and
retains its native receipt. Run 34746837453 then revealed a data-root mismatch:
shortcuts and removal now explicitly use the Windows installer data directory.
Runs 34746965265 and 34747008497 caught an NSIS finish-page macro quoting error;
an explicit finish callback corrected that invocation.

Run [34747086141](https://github.com/InkyQuill/hieronymus/actions/runs/34747086141)
passed Linux installation, built-in PowerShell 5.1 installation, and the Windows
NSIS normal install/start/version/remove path using published v0.9.0 payloads.
The macOS jobs still reproduce the old binary's diagnostic-field rejection;
new v0.9.1 payloads remain required. No Defender threat events were observed in
the captured diagnostic run. The intermittent Windows access failure is retained
as an observed limitation; successful repeats alone do not identify its cause.

The committed CodeRabbit re-review is preserved in
`coderabbit-committed-review.jsonl`. It contains four minor/trivial comments,
two pairs addressing the same subjects: unsigned installation guidance (added
to the usage guide) and redundant candidate hashing in the publisher (removed;
`verifyEvidence` still performs candidate verification). No major or critical
finding was reported in that re-review.

All local Rust checks passed after the diagnostic-only promotion error amendment.
Superseded candidate runs 34746871118, 34747006839, and 34747193074 were cancelled
when installer fixes or the owner-requested Node 24 migration changed the source.
Cancelled runs do not qualify the final release.

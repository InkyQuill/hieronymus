# v0.9.1 installer maintenance

The owner requested a Windows installer, a macOS package, and a Linux one-liner.
No repository clone or developer runtime is required. Windows and macOS signing
are explicitly waived. Intel macOS remains unqualified for physical desktop use.

## Failures and fixes

### Windows runtime prerequisite

Inspection of the actual Windows payload imports found `VCRUNTIME140.dll` in
the CLI and `MSVCP140.dll`, `MSVCP140_1.dll`, `VCRUNTIME140_1.dll`, and
`VCRUNTIME140.dll` in ONNX Runtime. Hosted runners already contain these files;
their previous passing smoke tests did not establish installation on a clean PC.
Setup now checks the x64 Visual C++ runtime registry explicitly in the 64-bit
view, downloads Microsoft's pinned redistributable when missing or older,
checks SHA-256 before execution, and invokes its normal elevated installer.
The app remains installed for the original Windows account. Reboot-required
results stop before app installation with instructions to restart and rerun.

The official [Microsoft runtime download](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
resolved on 2026-09-13 to version **14.51.36247.0**, size **18731856**, SHA-256
`843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c`.
The immutable Microsoft URL and hash are embedded in setup. No new public
Hieronymus release asset is needed. A PowerShell 5.1 policy test covers existing,
newer, absent and old runtimes, cancellation, failed verification, false success,
concurrent upgrade, and reboot-required results; native CI also exercises the
real download and installer without removing shared runner prerequisites.
Its first harness run (34748241049) caught an unexpanded template placeholder
in the test parser; the harness now removes that data-only marker before
extracting function definitions. This was a test harness failure, not a native
installer result.

Run 34748305162 passed all nine prerequisite policy cases and the real Microsoft
download/install, reading installed version 14.51.36247.0 afterward. Windows
single-file installation and Linux installation passed. NSIS installed and
started v0.9.0 successfully, then reproduced its previously recorded transient
removal error 5; the separate diagnostic repeat succeeded four seconds later.
Both macOS paths reproduced the known v0.9.0 parser failure. These old published
payloads do not include the prepared Rust fixes, so final validation must use
new v0.9.1 candidates. Candidate run 34747754991 was superseded because setup now
includes the missing Windows prerequisite.

CodeRabbit's prerequisite review reported two findings, both fixed: the template
parser issue above and prioritizing reboot-required guidance before reading the
runtime registry. The latter has an additional policy case for a registry that
has not updated before restart. The review is retained in
`coderabbit-runtime-review.jsonl`.

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

## Windows file lifetime follow-up

In run [34747514646](https://github.com/InkyQuill/hieronymus/actions/runs/34747514646),
job 103698071164 first failed removal with Win32 error 5 at 08:24:16 UTC.
No owned process was visible in the subsequent snapshot. A separate diagnostic
repeat removed the app successfully at 08:24:20, preserving the same data root.
The primary workflow failure remains recorded. No Defender threat was reported.

Read-only Rust review found that daemon ownership/session locks may be released
before the corresponding Windows process fully exits. Executable mappings may
therefore remain briefly live. This supports a file-lifetime race as an explanation
for removal; the earlier combined install error does not establish the exact
installation operation or prove antivirus involvement.

Only staging-directory promotion and owned application-tree removal now retry
Windows errors 5/32/33 for up to three seconds, under the existing lifecycle and
retirement guards. Permissions and authority checks are unchanged. Other errors
return immediately; exhausted failures include the operation and path. Unix
operations remain single-attempt. The full transaction is never replayed.

Focused tests cover transient success, persistent/deadline failure, immediate
non-retryable failure, and path context. The native Windows candidate job also
exercises promotion/removal while a real non-delete-sharing file handle is held.
The companion Rust review found no actionable regression in the bounded retry.

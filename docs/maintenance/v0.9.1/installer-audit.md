# v0.9.1 installer maintenance

The owner requested a Windows installer, a macOS package, and a Linux one-liner.
No repository clone or developer runtime is required. Windows and macOS signing
are explicitly waived. Intel macOS remains unqualified for physical desktop use.

## Feedback contention found by full PR CI

PR run 34749181126, backend job 103702491615, failed
`rest_route_applies_replays_and_rejects_mismatches` on source 40e2ad1: the first
authenticated feedback request returned HTTP 400 with `database is locked`
instead of HTTP 200. An ordinary local rerun passed, so that rerun alone was
not accepted as a resolution.

A controlled competing-writer regression reproduced `DatabaseBusy` before the
fix. `FeedbackStore::record_recall_outcome` used a deferred transaction, reading
its ledger and scoring snapshot before obtaining writer authority. The store
now begins an immediate transaction, allowing SQLite's existing busy timeout to
wait for the writer before reading. The regression then passes and verifies the
feedback delta is applied to the other writer's committed score exactly once,
including replay and ledger count. All five feedback-surface tests pass.
SQLite documents this read-to-write upgrade behavior in its
[isolation guide](https://www.sqlite.org/isolation.html).

The required Rust companion review found no actionable issues in the change
or its adjacent correction caller, which already owns a write transaction.
CodeRabbit also reported zero findings; its receipt is retained in
`coderabbit-feedback-review.jsonl`. Formatting, Clippy, the full Rust test suite,
and rustdoc with warnings denied all passed after this fix. This
product fix must be included in a new candidate; the earlier 5e928e3 and
40e2ad1 candidates do not qualify the release containing it.

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

The follow-up review (`coderabbit-runtime-recheck.jsonl`) found no remaining
installer defect and requested only replacing the local working-directory path
in review metadata with a relative path; that cleanup is applied. All 129 Bun
script tests pass after the restart-message correction.

### Pending final candidate dispatch

On 2026-09-13 at approximately 08:44–08:48 UTC, GitHub rejected three attempts
to dispatch `desktop-candidate.yml` on `codex/easy-install` (then source
0bcbf72d32db8216ff33b9be5fcf3373f8e78563), returning HTTP 500,
`Failed to run workflow dispatch`. Both `gh workflow run` and the direct REST
endpoint failed; run listings confirmed that none created a new candidate run.
GitHub's status API reported Actions operational, so no public incident is
inferred. The workflow itself remained active. A new successful candidate run
is still required before installer qualification, promotion, and v0.9.1
publication. No release tag or release is created for these unqualified changes.

Dispatch recovered at 08:55:52 UTC by explicitly sending the REST body
`{"ref":"codex/easy-install","inputs":{}}`. GitHub returned 204 and created
run 34748689507 at source 5e928e3c0ce2d76a147814218ecdd9d6b678a9c9.
The request-body difference is observed; no internal GitHub cause is claimed.

PR #30's initial CodeRabbit review (5190280607) is retained in
`coderabbit-pr30-initial.json`. Three findings were addressed: installer workflow
checkouts disable credential persistence; PowerShell temporary-directory and
transcript cleanup cannot replace the original error; older review metadata
uses repository-relative working-directory values. Its remaining finding about
`@@EVIDENCE_URL@@` is inapplicable to published notes: `desktop-ci.ts` replaces
the placeholder with the verified immutable evidence-data commit URL before
calling `gh release create`. The Markdown file is now explicitly marked as a
publication template. New native candidates must include these review fixes.

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

## Final a99a305 candidate and installer follow-up

PR CI [34750077942](https://github.com/InkyQuill/hieronymus/actions/runs/34750077942)
and the four-platform candidate run
[34750088390](https://github.com/InkyQuill/hieronymus/actions/runs/34750088390)
passed on source `a99a305a67543696869fdfdb63fdb34838e9f07b`. All four final
packages passed native model inference and authenticated MCP. The Linux archive
`447d2c170aea768d9db724f8798c3f2c6c6bfc1eece73362edb6c299937b89fd`
also passed local KDE Wayland startup/duplicate/stop/restart and tray lifecycle
checks, unsafe-token refusal, standalone piped installer and data-preserving
uninstall in isolated directories. These receipts identify this source only.

Installer run [34755165337](https://github.com/InkyQuill/hieronymus/actions/runs/34755165337)
passed Linux and standalone macOS installation but exposed two remaining defects:

- The macOS PKG installation itself completed successfully and started the server.
  Its following `hiero stop` failed with `Foreign daemon login definition`.
  Unix stable endpoint resolution treated an invoked `bin/hiero` symlink differently
  from the versioned payload path. Canonicalizing before recognizing the packaged
  layout now preserves the registered stable endpoint in both cases. A regression
  failed before this change and passed after it. CodeRabbit's local review reported
  zero findings (receipt `coderabbit-launcher-review.jsonl`); companion Rust review
  found no actionable issue. Final native verification is still required.
- Both Windows paths failed promotion of `.staging-0.9.1` to `versions/0.9.1`
  with Win32 error 5 despite the three-second retry. The installer rolled back;
  this run does not qualify Windows installation. Static review found no retained
  staging handle in Hieronymus. A separate diagnostic run
  [34755736458](https://github.com/InkyQuill/hieronymus/actions/runs/34755736458)
  reproduced the failure and used read-only Restart Manager queries to observe
  `provjobd.exe` PID 1352 still using staged resources after the verification CLI
  exited. Its precise resource lifetime is under investigation; no process was
  terminated and no permissions or runner security settings were changed.

The first diagnostic run, 34755635383, failed to compile its PowerShell 5.1 C#
interop helper because `FILETIME` was ambiguous; qualifying the framework type
fixed that diagnostic error. These diagnostic workflows are isolated on
`codex/installer-diagnostics`, not release qualification gates.

Diagnostic run [34755853743](https://github.com/InkyQuill/hieronymus/actions/runs/34755853743)
installed the same Windows binary successfully under different observation timing;
its separate native payload rename probe also passed immediately. The diagnostic
workflow itself was red because its exploratory harness ended with an unconditional
failure marker; this is not a second product failure. Together with the earlier
recorded failure and external resource user, this establishes intermittent file
lifetime interference rather than a deterministic updater-owned staging handle.

The Windows mutation deadline is now 30 seconds, still restricted to errors
5/32/33 and the same guarded individual operations. The native Windows regression
holds a real non-delete-sharing reader for four seconds, beyond the former limit.
No transaction retries, reader termination, ACL changes, or CI security exclusions
were introduced. Companion Rust review found no actionable issue; a fresh actual
installer run must verify this amendment before release.

The combined launcher/lifetime amendment received zero CodeRabbit local findings
(`coderabbit-native-followup-review.jsonl`). Local formatting and Clippy checks
passed; source-matched native installer acceptance remains a release prerequisite.
The complete local Rust verification chain also passed for the combined amendment:
formatting, all-target/all-feature Clippy, all-feature tests and warnings-denied
rustdoc. Native Windows and macOS installer results must still come from the next
source-matched candidate run; a99a305 observations are not reused for new bytes.

## Discovery-port assertion false positive

Backend job 103721180100 in PR run
[34756320568](https://github.com/InkyQuill/hieronymus/actions/runs/34756320568)
failed `discovery_consumers_read_the_published_port_and_never_assume_9768` on
source `92df1cfd01f8aa39b73b9bec8fcf75444192d9b3`. The actual hook JSON correctly
advertised `http://127.0.0.1:39777`; its unrelated process ID was `19768`.
The test's whole-output substring check for `9768` matched the PID. The assertion
now parses the JSON and compares only `service.base_url` with the actual bound
nondefault port. This retains the port-discovery contract without depending on
unrelated numeric fields. All 24 focused runtime/shutdown tests passed locally;
companion Rust review found no issue. Production behavior is unchanged.

Candidate run 34756328507 was cancelled after this test-source amendment so the
release candidate can carry the same source revision as the corrected test suite.
The cancelled run supplies no final-byte qualification for the next revision.

The complete local Rust verification chain passed for the assertion amendment:
formatting, all-target/all-feature Clippy, all-feature tests, and warnings-denied
rustdoc. CodeRabbit local review reported zero findings
(`coderabbit-discovery-assertion-review.jsonl`).

## Publisher cleanup raced with a lifecycle operation

The first attempt of publisher run
[34765897982](https://github.com/InkyQuill/hieronymus/actions/runs/34765897982)
installed the macOS PKG successfully and printed version 0.9.1. Its immediate
`hiero stop` cleanup then exited 2 with the exact diagnostic
`another lifecycle operation is in progress for this data root; retry after it finishes`.
The captured launchctl state showed the installed daemon running. This was a
cleanup conflict, not the earlier foreign-launcher failure. The log does not
identify which concurrent client held the lifecycle lock.

`LifecycleOperation::acquire` returns this refusal before service-manager actions.
The CI cleanup helper now waits at most 30 seconds for that exact exit-code/message
pair. Foreign registrations, additional diagnostics, other exit codes, and a
persistent busy condition still fail. Tests cover one busy refusal followed by
success and immediate failure for other messages or exit codes, including paths
containing spaces. This changes CI cleanup only; released binary
behavior and the immutable v0.9.1 tag remain unchanged.

The same publisher's single failed-job retry passed the macOS PKG check on the
unchanged candidate. The earlier complete installer run
[34765431342](https://github.com/InkyQuill/hieronymus/actions/runs/34765431342)
also passed all six jobs. Both the initial failure and retry are retained rather
than describing the first attempt as green.

All 133 release-script tests passed, including four focused cleanup tests.
CodeRabbit reviewed the workflow and cleanup implementation; its single minor
finding asked for more precise coverage wording, which is corrected above.
The persistent-busy deadline is not claimed as a tested fixture.

# Installed Rust product rehearsal — 2026-09-07

The product is **not release-ready**. ADR 0016's autonomous authority/correction
runtime remains unimplemented, Claude and Codex native initialize handshakes
remain incompatible with mandatory MCP 2026-07-28, and zCode remains unverified.
No legacy adapter, FTS-only alternative, live cutover, publication, user-host
configuration change or additional approval/attestation system was introduced.

This rehearsal uses Linux `x86_64-unknown-linux-gnu`, Rust 1.96.0, genuine Bun
1.4.0 for builds, and Chromium 152 through Playwright. App, data, HOME and unit
roots are disposable. The installed CLI/daemon children have an empty PATH and
no model/runtime environment overrides. Python/Playwright are test drivers;
Python, Node and Bun are not installed application dependencies.

## Evidence and artifact lineage

Durable local rehearsal evidence is retained in the main repository at
`/home/inky/Development/hieronymus/target/release-readiness-2026-09-07/evidence/`.
The original `/tmp` paths below identify execution receipts; retained copies
of the rehearsal logs, sanitized browser drivers/reports and screenshots live
in that directory so they survive task scratch/worktree cleanup. These local
files are not a published release or an external-host acceptance claim.

F1's baseline archive was built at `c4f0e44`; final installer fixes are in
`d31a0d9`, and `bffa161` only moves a Rust function block for Clippy. Its SHA-256
is `f81f70308a3130e08618dca010b3fc97375db454a15bd0e0126acc7c2d90fb54`.
The preserved archive is under
`target/release-readiness-2026-09-07/0.7.0/` in the main repository.

F1's `/tmp/hieronymus-f1-installed-final.log` is the existing actual installed
11-case multilingual corpus receipt: both public retrieval endpoints, Japanese
and Russian expected-source rank 1, learned memory, deterministic contract,
version-relative native runtime, and no-activation reinstall passed. Cold
true-ready took 3.921 seconds with pre-staged assets/warm OS cache; the daemon's
VmHWM was 2,804,652 KiB and VmRSS 2,039,992 KiB at corpus end. These are retained
measurements, not fresh F2 measurements. [Semantic validation](semantic-validation.md)
records the model identity, immutable revision and corpus, including prior model
failures. Both working memory and real semantic RAG remain mandatory.

F2 ran all 39 registry cases over HTTP and all 39 over stdio against that
installed baseline in 532.15 seconds:
`/tmp/hieronymus-f2-installed-tests.log`. The same existing case runner and
independent SQLite persistence assertions were reused. Actual ONNX inference
uses the installed bundle. Dream provider replies are controlled loopback
fixtures; neither those replies nor a direct MCP client establishes native
agent-host or commercial-provider acceptance.

F2 found and corrected three browser defects: form-encoded multiword view names
were not decoded, Merge Selected had no way to select two records, and an action
could leave the loading indicator stuck when superseding a pending refresh.
The first rebuilt archive, SHA-256
`ab474c785a4ea5a2be527b2d737c5b9f668c7779edda3be2508430464fd10010`, passed ten
actual view responses and thirteen actual action responses in Chromium. It was
then superseded by the empty-corpus upgrade correction below; its browser
receipt remains attributed to that archive, not relabeled as a later run.

The actual two-version rehearsal also exposed an upgrade defect: migration
creates a queued/building semantic generation even when there are no RAG chunks.
The worker skipped it forever because no activation sample existed, leaving
`rebuilding`. The correction cancels that unneeded candidate through the existing
store and reconciles the job before normal readiness re-evaluation. It does not
activate a fictitious empty index or bypass arming, corpus revision or ownership.

The semantic-fixed `0.7.0` archive is SHA-256
`e7a7e08e010fc3bc8b281d90b240cd7c62432c09b553e5c52e8f10c0353a9c22`
(533,131,348 bytes), preserved in the main repository under
`target/release-readiness-2026-09-07/final-semantic-0.7.0/`.
Its fresh installed aliases, public MCP status, authenticated native readiness,
CLI lifecycle/SIGTERM, migration/recovery and controlled updater checks passed.
The independently compiled test-only `0.7.1` companion is SHA-256
`8bce78a1635393dfe2c1904b0de2fceb8aa139d32aba80694e94062b1ffc4985`.
The actual baseline-to-companion upgrade after completed Python cutover passed
both update activation and authenticated semantic readiness. It preserves the
migrated series and reports the different compiled version. This is not a
same-version reinstall or a metadata-only relabel. A separate actual compiled-candidate rollback passed: a child-only missing-model
override makes the real `0.7.1` doctor fail after switching links; all aliases
return to the semantic-fixed `0.7.0`, which then starts without the injection and
reports authenticated semantic readiness and the previous version. The receipt
is `/tmp/hieronymus-f2-real-rollback.log`. The fifteen other updater
refusal/rollback cases use controlled candidates and service seams, even though
their updater executable is installed.
The final four installed rehearsals and fifteen updater cases are recorded in
`/tmp/hieronymus-f2-installed-complete.log`. The complete browser flow remains
attributed to the earlier UI-fixed artifact; the semantic-worker change retains
identical browser sources/assets. The later delete-confirmation correction
below changes the final UI and carries its own focused installed receipt.

## Acceptance outcomes

| Area | Observed result and limit |
| --- | --- |
| Fresh install | Actual script, disposable HOME/app/data/unit roots; aliases resolve together; native assets and readiness verified. Installed version command is `hiero version --json`, not an invented `--version` surface. |
| Registry / CLI | All 39 cases × two installed transports passed on the F1 baseline; series, sessions, remember, feedback, completion, Dream, recall and deterministic contract use existing assertions. P1's new immediate authority correction is not implemented or credited. |
| Semantic retrieval | Existing F1 real-model corpus receipt retained above. Empty-corpus migration regression is separately tested; a pending rebuild is never treated as ready. |
| Python migration / recovery | Installed migrate dry-run and conversion preserve an approved Japanese term, an existing concept and short-memory source reference; corrupt live DB refuses startup, recover rebuilds from backup and preserves them; newer and partial schemas refuse startup. Synthetic fixture contains representative domain rows, not private book data. |
| Real two-version upgrade | Baseline `0.7.0` → separately compiled test-only `0.7.1`, after completed migration: activation, persisted series, compiled version and authenticated semantic readiness passed. |
| Controlled update recovery | Fifteen installed-updater cases passed, including checksum/protocol/older-version refusal, health/doctor/degraded/install-link rollback and unit refresh. Candidate and service-manager fault seams remain fixtures; native user-manager acceptance is unverified. Separate real compiled-candidate rollback with injected missing-model health failure restored all aliases/version and authenticated semantic readiness. |
| Ownership and shutdown | Installed second daemon, migrate and recover refuse while an owner runs; explicit stop and SIGTERM exit successfully before discovery disappears; a subsequent owner starts; token survives restart. Worker joining has covering integration tests. |
| Browser bootstrap | Both actual admin/config commands launch through a private capture opener and single-use grant exchange. Fragment is removed; no grant/bearer is printed or written into evidence. |
| Browser views | Ten successful HTTP snapshot responses with the requested view verified. Initial presence-only checks were rejected because they could observe stale rows after a failed view request. |
| Browser actions | All thirteen rendered actions return HTTP 200 on the fixed UI, including two selected records for merge, destructive confirmation, proposal approval/rejection, inspections and manual Dream/review. These existing administrative actions do not implement ADR 0016's autonomous policy. |
| Browser settings / provider | Two controlled loopback provider profiles saved; actual connection checks and model discovery succeeded. Release settings saved. An Ingest save interrupted by Chromium `net::ERR_NETWORK_CHANGED` passed on an affected-step retry. Styled Dream switches were exercised through their rendered labels. A subsequent rendered manual Dream used both saved provider lanes, completed knowledge-crystal/coverage/persistence phases and stored learned memory. Earlier failed phases from closed fixture endpoints remain separately recorded; no commercial provider is credited. |
| Browser lifecycle / Origin | Refresh, reconnect after network loss, explicit foreign-Origin refusal, old-cookie rejection after restart, and a fresh grant restoring access passed. Environmental network failures are retained separately from application responses. |
| Actual Claude / Codex / Pi | **Open.** See [native host evidence](agent-host-acceptance.md). Generated adapter tests and installed stdio tool calls are not substitutes for native sessions. zCode is retained as paused/unqualified historical evidence. |
| ADR 0016 authority | **Open.** Accepted [authority design](superpowers/specs/2026-09-06-autonomous-authority-and-corrections.md) is a design, not implemented correction/viewpoint runtime. |
| Real service manager / publication | **Unverified.** Tests use disposable unit directories and controlled service seams; no user's systemd manager/configuration or live book data was changed. Remote transport tests are loopback TLS fixtures, not a deployed public update feed. |

## Reproduction

Build/install with the explicit qualified asset inputs in
[Distribution](distribution.md). Use an independently compiled second version
for the upgrade inputs; metadata-only relabeling or a same-version reinstall
does not count. F2's second version is a disposable test-only `0.7.1` build:
only the workspace package version and the two workspace lock entries are
changed in a disposable source copy. The repository version stays `0.7.0`.

```bash
HIERO_TEST_INSTALLED_APP=/path/to/disposable/app \
HIERO_TEST_BASE_RELEASE_DIR=/path/to/verified/first-release \
HIERO_TEST_ROLLBACK_BASE_RELEASE_DIR=/path/to/semantic-fixed/first-release \
HIERO_TEST_UPGRADE_RELEASE_DIR=/path/to/distinct/compiled/release \
CARGO_BUILD_JOBS=4 cargo test --locked -p hiero --test product_rehearsal \
  -- --ignored --nocapture --test-threads=1

HIERO_TEST_INSTALLED_APP=/path/to/disposable/app \
CARGO_BUILD_JOBS=4 cargo test --locked -p hiero --test tool_completeness \
  installed_registry_executes_both_transports -- --ignored --nocapture

HIERO_TEST_INSTALLED_APP=/path/to/disposable/app \
CARGO_BUILD_JOBS=4 cargo test --locked -p hiero --test update_flow
```

Installation-dependent tests are ignored by default and fail when explicitly
invoked without their required inputs. The updater fault tests deliberately
use controlled candidates; they do not prove external service-manager support
or a real two-version upgrade. Ordinary tests never silently substitute a
missing explicit installed executable with the development binary.

Browser drivers and sanitized reports are retained with the task's local
implementation report. They use rendered roles/labels and response assertions,
not source-code inspection as browser acceptance. No fixed-count machine
attestation requirement was added.

The existing [ADR 0008 owner questions](adr/0008-rust-reimplementation-authority-and-cutover.md)
were read: public contracts/deltas, clean and representative migration,
independent deterministic terminology, authentication/lifecycle, Rust/frontend
verification, runtime without Python/Node/Bun, one-way upgrade, precommit
failure and backup recovery. Outcomes and remaining gaps are reported above;
no additional approval tool or human-curation requirement was resurrected.

## Final verification

Formatting, all-targets/all-features Clippy with warnings denied, strict
Rustdoc, frontend typecheck, all 66 frontend tests and production build passed.
Normal Rust target coverage totals 1,045 passing tests and nine default-ignored
checks across the recorded run and its isolated continuation. The original
all-features command failed when fifteen parallel update fixtures exhausted
`/tmp` copying debug binaries; it is not recorded as passing. The fifteen tests
passed serially, and all skipped core/worker/doc targets passed separately.
Five explicit installed live rehearsals passed separately, including real
compiled-candidate health-failure rollback. The normal development-binary
registry matrix is distinct from the installed baseline matrix receipt.

## Review correction — destructive target confirmation

Review found that the new checked selections could differ from the detail row
named in the existing delete confirmation. The dialog now renders the same
count and stable IDs used by the delete request. A matching detail label is
shown alongside its ID; the detail-only fallback applies only with no checked
IDs. Regressions first failed on the misleading display, then all 13 dialog
and all 69 frontend tests passed, as did typecheck and the release builder's
production frontend build, embedding checks, native inference and round-trip.

The final review-fixed archive is SHA-256
`eae0185ee5efd56fa4d9c02de021fed438b2899e3f0f374054d6ae16395b966e`
(533,132,292 bytes), preserved in the main repository at
`target/release-readiness-2026-09-07/final-delete-confirmation-0.7.0/`.
A fresh disposable install reached authenticated semantic readiness. Actual
Chromium checks retained detail A while checking B, checked two other records,
and used detail A with no checks: each displayed count/target list exactly
matched the real HTTP 200 deletion body. Receipt:
`/tmp/hieronymus-f2-fix1-browser.json`. The earlier full browser and matrix
receipts remain attributed to their original artifacts; they were not repeated
or relabeled. The earlier `e7a7e08e…` artifact remains the semantic-worker and
upgrade/rollback receipt. Authority/native-host/external-service gates remain
open. No broad Rust suite was repeated for this UI/documentation correction.

## Desktop split-artifact qualification

The current release path is documented in [desktop qualification](desktop-qualification.md).
`scripts/release-build.sh` delegates to the exact-native v2 producer; it no longer
emits monolithic `release.json`. One canonical common-model archive is produced
once and consumed unchanged by every target job. `desktop-candidate.yml` builds
retained bytes, `desktop-evidence.yml` ingests separate data-commit captures, and
only an exact matching version tag invokes `release-rust.yml` promotion. No
rebuild happens after evidence. All four targets and seven native session records
are mandatory; Intel runtime promotion and unavailable native checks block release.

The explicit final-payload fixture is:

```sh
HIERO_DESKTOP_INSTALLED_CLI=/path/to/verified/disposable/payload/hiero \
  cargo test -p hiero --locked --test desktop_native -- --ignored --nocapture
```

It runs final native inference, authenticated MCP and shutdown. It does not claim
native manager, live tray, login, panel restart or agent-host acceptance. The
Linux-only `desktop_update` ignored test additionally needs the documented real
0.8.0 baseline and final split release for offline upgrade/rollback/uninstall.
Custom unit directories remain definition-only. See the final Task 14 receipt;
Task 12 hashes describe an earlier embedded console and are historical.

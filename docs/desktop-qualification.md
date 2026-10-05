# Desktop checks and retained candidates

Native builds, packaged inference and visible desktop behavior are separate checks.
Record actual outcomes and qualification gaps. Optional evidence is advisory under
[AGENTS.md](../AGENTS.md#release-checks-and-blockers); it must not become a second
publication gate or a substitute for runtime integrity checks.

## Retain and reuse candidates

`desktop-candidate.yml` builds native packages with the production console,
verified runtime and identical common-model archive. Retain these artifacts and
their provenance. Installer/packaging-only fixes reuse unchanged binaries; rebuild
only when binary inputs change or an artifact is missing.

[Automatic releases](automatic-releases.md) owns tag/version dispatch. Promotion
verifies retained identity and publishes verified bytes without recompilation.
Missing optional evidence or installers is disclosed rather than blocking available
working artifacts. Failed/unrun non-P0 checks go to issues with reproduction and
run links; P0 and corruption/data-safety refusals retain their force.

## Native observation matrix

| Platform | Useful visible sessions |
| --- | --- |
| Linux x86_64 | KDE and GNOME AppIndicator, Wayland and X11 |
| Windows x86_64 | Interactive native session |
| Apple Silicon / Intel macOS | Separate native sessions for each target |

Intel uses reviewed source-built runtime pins. Upstream source tests and acquisition
verification do not qualify Intel desktop use. Portable tests and a private Linux
watcher do not establish real panel, login or native-manager behavior.
Historical reports describe their recorded versions only; check current run artifacts
before claiming a present platform pass.

Use a disposable OS user or VM for registration, login, panel/Explorer restart
and active update. A custom `--unit-dir` is definition-only; it cannot test a real
manager. Keep app/data roots disposable and book files out of the test. Install
with the matching native standalone installer and `--no-activate` when staging
without manager contact. Use [Linux](desktop-linux.md), [Windows](desktop-windows.md)
or [macOS](desktop-macos.md) for platform behavior and fixture commands.

Observe startup/duplicate launch, menu responsiveness, restart, failed stop and
Quit acknowledgement. Quit must preserve login preference and intentional stop;
passive polling must not restart the server. Check toggle persistence and next
login, missing tray host and recovery, panel/Explorer restart, both themes and
actual scales. Observe authenticated console access where enabled, failure/recovery
without paid background probes, installed inference and missing/corrupt payloads.

For active update/rollback/uninstall, use distinct real compiled versions and
verify registration/mode restoration, data preservation, continuing operations,
private-file failures and secret-free diagnostics. Ambiguous ownership is not
permission to delete a record. Confirm stopped state and remove only disposable
owned registration before deleting its directories.

Final-payload inference/MCP and Linux offline two-version rehearsal commands are in
[runtime checks](rust-cutover-rehearsal.md). Native tests marked ignored need their
explicit fixture inputs; authored but unexecuted tests remain unverified.

## Optional evidence workflow

If publishing structured native evidence, use the existing format rather than a
new handwritten inventory. `scripts/check-desktop-evidence.ts` owns schema/check
names and digest validation. Its seven-session inventory is completeness for that
optional evidence format, not a requirement to fabricate missing observations.

`records.json` lists relative record filenames. Bind each record to candidate
commit, target, archive/model/metadata/assets digests, actual OS/desktop/session/scale
and signing state. Each observed check needs a nonempty hash-bound capture.
`partial`/`unqualified` with concrete reasons preserve unavailable observations;
changing qualification status never converts a failed check into a pass.

Create a template from reviewed source, using the actual target/session:

```sh
bun scripts/new-desktop-evidence.ts candidate FULL_CANDIDATE_SHA kde wayland \
  qualification/desktop-evidence/kde-wayland.json
```

A template is not passing evidence. Fill actual observations and digests, then:

```sh
bun scripts/check-desktop-evidence.ts --release-dir candidate \
  --evidence-dir qualification/desktop-evidence --commit FULL_CANDIDATE_SHA
```

Use a separate immutable evidence data commit with `desktop-evidence.yml` and its
`candidate_run`/`evidence_commit` inputs. Evidence-side code must never execute;
reject escaping paths, aliases and undeclared files. Captures remain bounded and
must contain no tokens, grants or manuscript content. The validator verifies
identity/completeness, not the truth of a human observation; review captures.

The release may consume that optional artifact when available. Its absence is
an explicit limitation, not a claimed pass. Signing/notarization inputs are external;
all transformations precede final manifests. Do not mutate verified archives or
move a published tag to attach different candidate bytes.

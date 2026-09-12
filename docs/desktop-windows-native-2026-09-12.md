# Windows native qualification — 2026-09-12

This is a local source-qualification receipt, not a desktop-candidate CI receipt
or release-promotion approval. Source started at main commit
`10dac0d820003b4b0859552c7ce63e975a153a2d`; the accompanying PR contains the fixes.

## Host and fixtures

- Windows x64 build 26200, NTFS, normal unelevated desktop account.
- Rust 1.96.0/MSVC, Bun 1.4.0, PowerShell 7.6.5, protoc 36.1 with its matching
  standard include directory in `PROTOC_INCLUDE`.
- Disposable roots, acquired tools, complete logs, and command/exit metadata are
  retained locally under `qualification/.artifacts/windows-native-2026-09-12/`.
  No book project or pre-existing Hieronymus installation was used.
- The user authorized native Task Scheduler and Explorer checks in this account.
  No logoff/login was performed.
- Throne is required for connectivity in this restricted network. Even an
  independent, standard-library-only TCP loopback fixture receives connection
  resets. The user explicitly requested leaving the network configuration alone
  and treating these resets as environmental. Network-dependent failures remain
  inconclusive, not passing. Experimental daemon networking changes were reverted.
  The console response comparison also captured injected `local.adguard.org`
  scripts and modified HTTP framing. Those assertions remain unchanged.

The four model members were extracted from the user-provided Linux v0.8.0 archive.
Every member matches main's pinned SHA-256. Only model data was reused; all Windows
executables were built from main plus the PR fixes. The local common-model archive
was repackaged on Windows, with SHA-256
`77f70f3ef86bbd007b9e62bcd6c8edc48eaea8bfc47011c115f3b0361aad2607`.
This does **not** establish provenance for the canonical Linux-produced candidate
archive. The native runtime is pinned ONNX Runtime 1.28.0, DLL SHA-256
`18370c375f07357fa5874344a9d9ac17e6b6fe1eb18b1dd209d79483b4470257`.

## Reproduced defects

- Credential replacement failed with Windows error 5 while a validated reader
  retained the original file. Publication now retains the protected creation
  handle through extended rename. Readers denying delete sharing still prevent
  replacement, and a regression proves the old bytes and temporary cleanup.
- Native broker entrypoint compilation called `as_slice()` on an existing slice.
- A successful broker response could arrive before its process exited, leaving
  the executable mapped during uninstall. The caller now verifies successful
  process exit within the existing deadline before returning success. Timeout
  does not kill a committed operation. The delayed-exit regression failed before
  this fix and passed afterward; all four native uninstall CLI contracts pass.
- Saved host prompt records inherited folder ACLs on Windows. They now use
  owner-only atomic publication, including when an acknowledgement replaces a
  pending record. The focused privacy regression failed before and passed after.
- Task Scheduler persisted a rooted registration URI and resolved a logon SID to
  an account name. Native normalization now compares the full XML after resolving
  that account back to its SID. It also reads omitted default enabled state through
  the native definition. Ownership checks remain strict.
- Export requested unsuitable directory rights, omitted explicit synchronous and
  metadata rights, and used the Win32 rename wrapper with an NT-relative leaf.
  It now uses least required access and `NtSetInformationFile` relative to retained
  directory handles. Junction and parent-swap protections remain covered.
- The ignored real modal-menu fixture waited for `WM_ENTERMENULOOP`, which its own
  `TPM_NONOTIFY` flag suppresses. A bounded timer now injects events during the real
  modal loop; production menu behavior is unchanged.
  Both fixtures serialize access to the desktop foreground so concurrent test
  threads cannot dismiss each other's menus.
- Portable tests assumed Unix permissions, signals, executable names, path
  separators, systemd behavior, readable byte-range locks, or English OS messages.
  Native credential validation and actual Windows launchers replace those
  assumptions. Linux shell/systemd fixtures remain explicitly platform-scoped.
- Loopback test servers inherited nonblocking accepted sockets on Windows. Their
  blocking handlers now explicitly set blocking mode. TLS fixtures also had
  incorrect declared body lengths.
- The tray's small center made status colors difficult to distinguish. At the
  user's request, the outer SVG path now carries the status accent and the inner
  path uses `currentColor`. Geometry and the original color/monochrome source
  assets are unchanged. All seven rendering/asset tests pass.

## Verified results

| Check | Observed result |
| --- | --- |
| Release-script tests | 100 passed using PowerShell-expanded test paths |
| Frontend | Typecheck, 85 tests, production build passed |
| Desktop helper | 18 passed; both additional ignored native modal tests passed |
| Native Task Scheduler adapter | 4 unit contracts and 5 CLI desktop contracts passed |
| Native broker | Timeout/continuation-gate and delayed-process-exit contracts passed |
| Final CLI library | All 120 unit contracts passed |
| Final private files | All 5 native contracts passed |
| Native uninstall CLI | All 4 contracts passed, including exact-root deletion and data preservation |
| Daemon lifecycle and release source | 15 and 14 tests passed respectively; 2 real-release cases remain explicitly ignored |
| Rust static checks | Required fmt, Clippy with warnings denied, and rustdoc with warnings denied passed |
| Filesystem integration | 8 platform filesystem checks and 9 dream-lock checks passed |
| Export native implementation | Both retained-parent tests passed |
| Upgrade integration | 22 passed after the best-effort owner-message assertion was corrected |
| Packaged installation | Native package validation and PowerShell `-NoActivate` installation passed in paths containing spaces |
| Final installed payload | Explicit ignored ONNX inference, semantic readiness, authenticated MCP and graceful shutdown test passed |
| Tray display | User confirmed the icon and menu were visible; the icon transitioned from red to green, and an authenticated status probe confirmed semantic readiness |
| Duplicate helper | Second invocation exited 0; the original helper remained the only process |
| Explorer recovery | Explorer PID changed; helper PID and start time stayed unchanged; user confirmed icon and menu returned |
| Distinct compiled update | Stopped-instance update from 0.9.0 to 0.9.1-qualification passed; helper retired and restarted, then the daemon started through Task Scheduler |
| Health rollback | A missing-model override made the candidate doctor fail; updater restored 0.9.0, stable launcher identity matched, and restored-payload inference/MCP/shutdown passed |
| Packaged uninstall | Separate verified application removed successfully; its data root remained and its native tasks were removed |

The original-icon checksum and private-file PR steps are required again rather
than advisory. Windows CI also runs the broker, native registration, private prompt
publication, retained-directory export, filesystem and uninstall regressions.

The final required `cargo test --all-features --locked` run exited 101 at
`agent_prompt_delivery`: seven passed, three failed with connection reset 10054,
and one browser fixture was ignored. The actual CLI private-file test passed.
The earlier full `--no-fail-fast` run covered all targets; non-network failures
were corrected and rerun in focused suites. Remaining HTTP-dependent failures,
including modified console bodies and provider retry counts, remain inconclusive.
No network assertions were relaxed to manufacture a green full-suite result.

The first final-payload rerun completed inference and authenticated MCP but reset
on shutdown. After the rollback test, the complete same-payload test passed.
Three live-update attempts refused before mutation because the running endpoint
could not be verified reliably. The normal stop command then confirmed shutdown
and ownership release; the stopped-instance update passed. This is **not** a
passing active-daemon upgrade record.

`0.9.1-qualification` is a separately compiled local fixture from the same source
plus fixes, with only workspace/lockfile version identities changed in an ignored
source copy. The project version remains 0.9.0. No immutable installed version was
overwritten. The updated disposable installation remains running for visual
inspection, with `autostart: true` and foreground `auto`; other fixture tasks and
orphan test processes were removed. The user's always-visible tray preference
was not deliberately changed, but the user observed that Windows moved the rebuilt
helper back into overflow. The user confirmed that the outer status color is
easier to see. Persistence of the always-visible shell setting across helper
replacement is not established; its identity/checksum mechanism was not diagnosed.

## Local evidence snapshot

`qualification/.artifacts/windows-native-2026-09-12/log-manifest.json` records
SHA-256 and byte length for every retained log at the end of this run. Its SHA-256
is `4d3a6550b622d574c0c01fa3249bd6036a9f460ff93519a153999497bd7cf3b1`.
The final 0.9.0 platform archive SHA-256 is
`a9ad46317ef5db992ac317e5dbd5a0fde46f2ab93184f909ceabf7cdf7f6c0e6`;
the distinct 0.9.1-qualification archive SHA-256 is
`022e01feb7d7cb3fda478fdd50d4634f064bb52db8c6ff8a9d9ab32ff46b4e84`.
These files remain local, ignored artifacts; this receipt does not imply they
were uploaded to or produced by CI.

## Remaining evidence

Power-loss durability, a fresh logon, the full appearance/DPI matrix, native agent
host acceptance, and canonical candidate CI provenance are separate evidence
requirements. A successful inference/MCP run does not erase environmental resets
in other runs or prove all seven desktop session records.

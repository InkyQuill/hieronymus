# Windows desktop lifecycle and native qualification

The Windows adapter is implemented for `x86_64-pc-windows-msvc`. It has been source/API checked on Linux, **not executed on Windows**. Native Task Scheduler registration, real logon, Explorer recovery, visual appearance, terminal suppression and authenticated daemon lifecycle require the Windows qualification below. Cross-compilation is not native acceptance.

## Registration and lifecycle

`service/mod.rs` preserves the public `ServiceOptions`, lifecycle, status, doctor and updater manager interfaces. Linux's systemd implementation is extracted into `service/linux.rs`; Windows selects `service/windows.rs` at compile time. The compatibility name `SystemdManager` remains at the updater seam, but selects the native scheduler backend on Windows.

Windows uses two distinct Task Scheduler registrations. Names hash the canonical data root and actual current-process user SID, with `daemon`/`tray` suffixes. Local records also bind the exact CLI path and canonical registration directory. A foreign/missing/modified registration is refused rather than silently overwritten. Initial creation uses `TASK_CREATE`; updates use `TASK_UPDATE`. Both use the actual SID with `TASK_LOGON_INTERACTIVE_TOKEN`, `LeastPrivilege`, separate executable/argument fields, no time limit, no idle/battery/network requirement, no hard termination, and no delayed catch-up launch. The daemon has no trigger; the tray has one logon trigger for that SID. Daemon failure recovery is bounded to three retries at one-minute intervals. An explicit stop disables the daemon task and removes recovery before sending authenticated `/shutdown`; a failed/indeterminate suppression aborts stop. Shared acknowledgement, discovery removal, root-release and replacement-instance checks remain authoritative. Explicit Start re-enables recovery even if an earlier failed stop left the daemon live. Quit preserves tray login preference.

`hiero desktop install`, `uninstall`, and `autostart on|off|status` use the native adapter. Install does not start a daemon. Toggle reads the scheduler's actual enabled state, including changes made in Task Scheduler, before saving preferences. A new daemon registration is rolled back if tray registration fails normally; an indeterminate call prevents a competing rollback. Package activation, live helper/launcher replacement and complete repair/removal coordination remain Task12 work.

Installed actions use immutable `<app>/bin/hiero.exe`; its Windows-subsystem launcher starts selected console children with `CREATE_NO_WINDOW`. The helper retains that stable endpoint, while broker execution and doctor comparison resolve the selected actual CLI. Direct executable paths support disposable development fixtures. Paths must be absolute UTF-8, without controls, at most 512 bytes; the entire broker request is capped at 2 KiB. Managed launcher verification remains mandatory. Windows `hiero tray` and `hiero-desktop` accept explicit `--data-root`, `--unit-dir` and `--binary` fixture paths; login arguments retain the registration directory. GUI libraries remain outside the headless dependency graph.

## Bounded native calls and transaction ownership

Task Scheduler COM and the shell opener run in a hidden child CLI broker because those synchronous native calls do not offer a demonstrated universal cancellation bound. The caller has a 30-second scheduler deadline / 10-second browser deadline; there is no Task Scheduler End/Stop or daemon-kill fallback.

1. The parent holds the common lifecycle and registration guards, including public status/read/doctor inspection. Lifecycle and updater internals pass the already-held operation into guarded validation instead of recursively acquiring it.
2. Before READY, the broker acquires persistent `.windows-native.lock` files in canonical root and registration directory, in that order (one lock if directories coincide). These continuation locks are separate from the parent guards, so there is no recursive acquisition.
3. The bounded request and exact COMMIT token travel over inherited stdin. Readiness alone never authorizes native work. EOF/malformed input or timeout before COMMIT aborts and may terminate/reap only the uncommitted broker.
4. After COMMIT, a timeout returns an explicit **indeterminate error**, never success. The broker stays alive, retains both gates through native mutation/readback/rollback, and prevents later operations. `LifecycleOperation::acquire` checks the root gate after obtaining its lock; `register_unit` checks the registration gate before later lifecycle/registration/update actions. New broker calls check gates before spawning, preventing hung queries from accumulating retries.
5. Successful responses are emitted only after the native operation and any rollback finish and the gates are released. Persistent coordination files are never unlinked.

Every registration transaction persists its before/after records before touching Task Scheduler. Readback retains the connected `ITaskService` for the entire operation and creates every parser definition from it. COM initialization/normalization and malformed readback failures propagate as errors, separately from a foreign-definition mismatch. Readback normalizes both actual and expected XML through Task Scheduler's `ITaskDefinition.SetXmlText`/`XmlText`, then compares the complete sorted element/attribute trees, including duplicate counts. The small portable allowlist accepts only inert default insertions (`IdleSettings` duration/wait, false unified scheduling and remote-app restriction). Extra principals/actions/triggers/security settings and changed execution fields are rejected. A failed mutation restores the exact previous registration; an interrupted transaction can be restored on the next **mutating** call, only if the scheduler still matches its authenticated before/after record. Passive inspection reports actual pending state without mutating it. Journal cleanup failures stay errors and remain recoverable. External conflicts are never overwritten.

The browser uses its own persistent root `.windows-browser.lock`. The one-time grant URL travels only over inherited stdin; it is never an argv value, log, or disk record. Windows does not honor the Linux command-opener override. A committed timeout can produce one late browser opening; retries are refused while that opener holds its gate. Errors remain fixed and secret-free.

## Native window and resource lifetime

The Win32 implementation directly owns `Shell_NotifyIcon`, one hidden **top-level** window and one icon ID. This avoids adding a second broadcast/window recovery path around a library's private hidden window. `TaskbarCreated` is registered once and received by the top-level window; only this window/ID is deleted/re-added. Repeated broadcasts cannot create another owned icon. Failed re-registration retains the prior HICON and a diagnostic, then retries on subsequent controller wakeups (normally every three seconds); initial publication failure exits before Start.

RGBA rendering, accent, state, menu projection, one worker, two-second probes, failure confidence, deadlines and session singleton are shared. The native callback retains coalesced menu/appearance/Explorer/teardown flags in constant-size thread-local state, then posts a wake. Nested `TrackPopupMenu` dispatch can consume wakes without losing those flags. The owner drains flags before blocking again and performs icon/controller work outside callbacks. Confirmed `WM_ENDSESSION` ends any popup and exits without submitting Quit; `WM_QUERYENDSESSION` and a canceled `WM_ENDSESSION` do not request teardown. A confirmed teardown received during a popup also suppresses its selected action. Menus are destroyed after dismissal. Straight RGBA is converted to premultiplied BGRA and a top-down 1bpp AND mask with alpha-zero transparent bits and DWORD-aligned rows. Fractional-alpha/channel-order/transparent-mask tests cover this boundary. Temporary GDI bitmaps are destroyed after creating an owned HICON; the old HICON remains alive until successful replacement. Final teardown removes the owned icon, destroys its HICON and window, and joins the controller. Logoff/session teardown exits the helper without submitting user Quit against the root shared with another session.

System taskbar theme, high contrast and taskbar DPI are observed on native settings/theme/DPI/display notifications. High contrast uses system foreground while retaining the semantic accent. An unknown theme uses light ink plus an explicit menu diagnostic. Set `foreground` to `light` or `dark` in the disposable root's `desktop-settings.json` and relaunch the helper to override ambiguous appearance (`auto` is default). Accent colors remain green `#2EAD68`, amber `#E5A72B`, red `#D94A48`; the source SVG silhouette is unchanged.

## Reproducible Windows tests

Run from a native interactive, non-elevated Windows user account with the required Rust 1.96/MSVC build prerequisites. Default fixtures create fresh temporary roots and task names; no installed book root or existing registration is an input.

```powershell
cargo test -p hiero --locked --test windows_desktop
cargo test -p hiero --locked --lib service::windows::tests
cargo test -p hiero --locked --lib platform::windows_broker::tests
cargo test -p hiero-desktop --locked --lib platform::windows::tests::native_modal_menu_retains_explorer_theme_and_confirmed_session_end -- --ignored --exact
```

The integration test copies Cargo's test CLI into a TempDir, installs only its unique daemon/tray tasks, checks native readback, toggles real login state, suppresses recovery without running a task, and removes both. The scheduler unit test creates a disposable task, applies a journaled interrupted change, checks passive inspection, runs production recovery, and verifies a foreign replacement is refused. Cleanup owns only the unique test names. If a test fails during cleanup, inspect the printed failing assertion and remove only its disposable `Hieronymus-<root/SID hash>-daemon|tray` names; never bulk-delete Hieronymus tasks.

The Windows service unit tests additionally distinguish invalid normalization from foreign-definition mismatch and hold each ordinary guard while public read/status/doctor inspection is attempted, asserting rejection before the instrumented broker spawn boundary. The explicitly selected helper regression opens only a disposable window/popup, injects Explorer/theme/canceled-session/confirmed-session events through the real native modal dispatcher, consumes wakes, and checks retained flags. A two-second menu timer bounds a broken regression; no daemon, scheduler task, browser or actual session ending is requested by this GUI fixture.

The transport test compiles `tests/fixtures/windows-native-broker.rs` with local `rustc`. That child never contacts Task Scheduler, browsers or daemons. It exercises the actual parent timeout/private-pipe implementation, pre-commit cancellation, post-commit continuation locks, later lifecycle refusal and eventual gate release. It is transport evidence, not native scheduler/lifecycle evidence.

## Interactive acceptance (not yet run)

Use a disposable assembled Windows desktop package with the Task9 verified model/runtime assets. Do not point these commands at a book project or an installed real root. Set paths explicitly and retain the fixture across the one logoff/login test:

```powershell
$fixture = Join-Path $env:TEMP ('hiero-desktop-qualification-' + [guid]::NewGuid())
New-Item -ItemType Directory $fixture | Out-Null
$root = Join-Path $fixture 'root'
$units = Join-Path $fixture 'tasks'
# $cli must name the stable bin/hiero.exe in a separately assembled disposable package.
& $cli desktop install --data-root $root --unit-dir $units --binary $cli
& $cli tray --data-root $root --unit-dir $units --binary $cli
```

Record OS build, account/session, exact commit, package/model/runtime digests and executable signing state. Confirm no terminal flashes for login/tray/daemon/broker/browser operations; clean duplicate launch with only one tray and no second Start; real logoff/login launch; checkbox toggle and persistence; one-time-grant browser console; Restart; Quit acknowledgement and root release without reappearance; next explicit Start re-arms recovery; Explorer restart leaves one usable icon; light/dark/high-contrast and multiple scales retain shape/accent/contrast. With a second interactive session, verify session teardown does not send user Quit to its shared daemon. Keep all acceptance roots disposable and report unsupported/missing inputs as failures, not skips or success.

After Quit, remove fixture registrations with the same exact paths before deleting its directories:

```powershell
& $cli desktop uninstall --data-root $root --unit-dir $units --binary $cli
```

Native inference, whole-package minimum OS, final signing, terminal behavior, actual scheduler normalization and native host acceptance remain pending. No public release or real registration modification was performed on the Linux implementation host.

Primary API references: [task registration and interactive principals](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskfolder-registertaskdefinition), [notification icon ownership/version/events](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw), [scheduler recovery interval](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-restartinterval), [alpha-blended native icons](https://learn.microsoft.com/en-us/windows/win32/menurc/using-cursors#creating-an-alpha-blended-cursor).

Package capture checks both daemon and tray registration journal paths while the
native continuation gates remain held. A pending journal blocks the package
snapshot even if scheduler readback matches its before/after state or absence.
Normal authenticated registration recovery remains separate from passive inspection.
The native unit fixture `native_package_capture_refuses_pending_before_after_and_absence_then_accepts_recovery`
checks these states and successful capture after recovery; it has not run on the Linux host.

Controller wakes are also latched in the existing UI-thread pending flags. Nested
menu dispatch consumes the posted message but leaves delivery pending, so the owner
drains before blocking again; callbacks neither repost that wake nor do controller work.
Portable helper tests cover this latch. On a disposable interactive Windows desktop, run
`cargo test -p hiero-desktop --locked native_modal_menu_drains_retirement_after_only_wake_is_consumed -- --ignored`
to verify retirement delivery after native modal dismissal without another event.
That authored native fixture remains unexecuted locally.

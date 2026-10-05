# Desktop installation and login startup

On Linux, install a matching CLI and `hiero-desktop` beside one another, then run:

```sh
hiero desktop install --data-root /absolute/data/root
hiero tray --data-root /absolute/data/root
```

Registration creates a Hieronymus application launcher, its Folio H icon, and an
XDG graphical login entry. A new desktop installation enables future-login
startup. Reinstall preserves an existing opt-out. The registration command does
not launch the helper or daemon; open Hieronymus from the application menu to
start using it. The helper starts the daemon once after initializing its UI.

For a release containing the matching helper, `scripts/install-desktop.sh` wraps
the verified bootstrap installer with `--desktop`; it accepts the same release,
application, data-root, unit-directory and `--no-activate` arguments. The ordinary
`scripts/install.sh` preserves its headless installation defaults. Packaging and actual native installed-model qualification are separate checks.

```sh
hiero desktop autostart off --data-root /absolute/data/root
hiero desktop autostart on --data-root /absolute/data/root
hiero desktop autostart status --data-root /absolute/data/root --json
hiero desktop uninstall --data-root /absolute/data/root
```

The tray's **Start at login** checkbox uses this same registration backend.
Changing it affects future logins, leaves the current daemon running, and keeps
its systemd user unit available for explicit starts. The checkbox reads actual
registration after every attempt, including failures. A failure saving
`desktop-settings.json` is reported separately from the actual registration.
Unknown, foreign, or modified registration is an error, not a successful toggle.

Desktop installation records the previous owned daemon unit and login symlink in
`.hieronymus-desktop.json` beside the service unit. It disables the old standalone
daemon login entry without stopping the daemon. Root or binary mismatches and
foreign files are refused. The previous registration is an audit/recovery record;
unregistering desktop mode does not silently restore standalone daemon logins.
`hiero service install` enables headless login startup again after desktop
unregistration. Ordinary unit repair while desktop mode is registered keeps the
service on demand. Selected-version registration updates use the separate package transaction;
registration commands alone do not qualify that flow.

`hiero desktop uninstall` removes only unchanged, owned desktop registration
files and preserves the daemon service, software and data. Full `hiero uninstall
--yes` also removes the recorded desktop files before deleting software, while
preserving user data and preferences by default. Quit preserves login startup.

For disposable tests or custom paths, desktop commands accept `--binary`,
`--unit-dir`, `--autostart-dir`, `--applications-dir`, and `--icons-dir`.
Defaults use XDG_CONFIG_HOME/XDG_DATA_HOME (or the usual HOME defaults) for desktop
files; the existing daemon unit default remains `~/.config/systemd/user`.
`--no-activate` prevents manager contact. Custom unit directories also prevent
manager contact. Registration can still remove an owned local
`default.target.wants/hieronymus.service` symlink; it never stops a daemon.
Set HOME and both XDG variables as well as disposable paths when testing.

Desktop Exec entries pass quoted absolute arguments without a shell. Quotes,
spaces, backslashes, Unicode and percent characters are encoded. Newlines,
control characters, invalid UTF-8 and `=` in executable paths are rejected.
GLib checks the executable before expanding percent escapes, so an executable
path containing `%` uses fixed `/usr/bin/env --` followed by the literal CLI
argv. This case requires `/usr/bin/env` from coreutils. Ordinary executable paths
are launched directly. The daemon unit separately escapes systemd specifier and
environment expansion. Its special executable paths also use the same fixed
dispatcher when systemd cannot represent them directly (quotes, backslashes
and dollar signs). Unit ownership parsing accepts only the exact fixed prefix.
The escaping follows the [Desktop Entry Exec specification](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html).

Linux needs GTK 3, an Ayatana/AppIndicator runtime, and a working graphical user
D-Bus session. On Debian/Ubuntu, install `libgtk-3-0` and
`libayatana-appindicator3-1`. GNOME additionally needs an enabled
[AppIndicator Support extension](https://extensions.gnome.org/extension/615/appindicator-support/).
The helper reports a missing host and waits for it to return. See
[Linux helper behavior and qualification](desktop-linux.md) for theme behavior
and the limits of existing native evidence. This guide makes no fresh native acceptance claim.

## Packaging and replacement

Native packaging consumes matching prebuilt binaries, verified runtime inputs and
the once-produced common-model archive. Format-2 metadata binds final archives
and assembled assets. See [Distribution](distribution.md) and
[platform artifacts](desktop-platforms.md); this guide keeps no second inventory.
Intel uses reviewed source-built runtime pins.

Install/update owns one lifecycle transaction through preflight, authenticated
helper retirement, stop/root release, registration snapshot, immutable selection
and activation/rollback. Restore actual manager enabled/mode state as well as files.
Pending operations and ambiguous ownership refuse competing replacement.
[ADR 0009](adr/0009-runtime-topology-and-daemon-lifecycle.md) owns the boundaries;
platform guides own native detail.

Helper control uses a private root/session record and persistent OS lock. Replacement
needs authenticated acknowledgement, session-lock acquisition and completed record
removal. User Quit is persisted before shutdown and preserves intentional stop
through replacement. Late cleanup can fail after UI exit: it is indeterminate
cleanup, not proof the helper remains alive.

For a known stale record in the current session, repair the reported private-file
error and launch the currently selected helper directly with `--resume`, the actual
`--data-root`, `--unit-dir` and stable `--binary`, then use Quit. Use the platform's
helper path/extension. Resume suppresses initial Start; it bypasses no lock.
Unknown/other-session records require investigation. Never delete an ambiguous
record based only on an absent PID.

Windows executing CLI removal uses an external verified installer path; direct
self-uninstall refuses before mutation. macOS preserves the sealed bundle hierarchy
and external manifest. Settings/data remain by default and coordination inodes
remain intact. [Desktop checks](desktop-qualification.md) owns native acceptance;
portable tests do not qualify visible menus, login or active update.

## Status and diagnostics

The tray uses green for ready, blue for work in progress (including semantic
index rebuilding, memory indexing and memory consolidation), amber for warnings, and red for
failures. The first menu row shows the current reason; **Open console** shows
readiness details on the dashboard. `hiero status --json` provides the same server
diagnostics from the command line. Memory indexing reports indexed/total counts
in the tray reason and Overview, where failures and remaining work are visible.
Background activity does not disable desktop
actions; only a pending desktop command does.

The helper writes stdout, startup and native desktop errors to
`<data-root>/desktop-helper.log` (owner-only, append-only). On Linux, server and supervisor errors are also available with
`journalctl --user -u hieronymus`. The supervisor checks session ownership before
launching a companion; all launch paths still acquire the same OS singleton.

Set `HIERONYMUS_HEADLESS=1` to suppress the automatic companion for headless
servers. Cargo runs default to this setting so disposable contract-test servers
do not create icons in the real desktop session. Explicit native qualification
uses `HIERONYMUS_HEADLESS=0` with an isolated display and D-Bus session.

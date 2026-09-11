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
`scripts/install.sh` preserves its headless installation defaults. Desktop
packaging and native installed-model qualification remain separate release work.

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
service on demand. Updating selected-version desktop registrations is separate
upgrade integration; do not assume these commands qualify that flow.

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
and the limits of existing native evidence. No GNOME login/install, Windows or
macOS acceptance is claimed here.

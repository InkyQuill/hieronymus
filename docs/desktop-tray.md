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

## Desktop distribution (unpublished 0.9.0 candidate)

Build a native, exact-target CLI with its embedded console and `hiero-desktop` separately. The headless `hiero` graph does not link GTK, AppKit or Windows tray libraries. `bun scripts/desktop-package.ts --target <triple> --out <dir>` consumes prebuilt files from `$CARGO_TARGET_DIR/<triple>/release`; `--input`, `--runtime-dir`, and `--common-model-dir` are explicit offline input overrides. Runtime input is the descriptor-verified upstream extraction directory. Common-model input holds the canonical archive plus `common-model.json`, produced once by `packageCommonModel`. Missing binaries, stale native executable versions, wrong architecture, missing runtime companions/notices, and corrupt model/cache bytes fail before success metadata is published. Native packaging tools must be available on the exact target.

Each release has one `release-<triple>.json` format-2 manifest, one platform archive and the common model archive. All target manifests bind the same common archive bytes. Platform archives contain no model payload. Acquisition caches it at `<app>/cache/models/<sha256>.tar.gz` and verifies every reuse; both archives assemble into a complete immutable `<app>/versions/<version>` tree. Model files remain inside each installed version for complete rollback. Missing offline model input can use that verified cache; a supplied corrupt model or a corrupt cache is refused. Cache contents are never runtime authority.

Old monolithic `release.json` releases remain readable as legacy input. Split output intentionally has no `release.json` alias: old updaters cannot understand the pair. Existing 0.8.0 users bootstrap with the current standalone installer and a local release directory. There is no invented download URL, tag, publication or deployed feed. The ordinary new Rust updater supports a configured HTTPS feed, exact-target metadata and content-addressed model reuse.

Linux/macOS: `scripts/install-desktop.sh --release-dir <dir> [--app-dir <dir>] [--data-root <dir>] [--unit-dir <dir>] [--no-activate]`. Keep `desktop-metadata.awk` beside the installer. Windows: PowerShell 7.4+ (a bootstrap prerequisite for built-in .NET `JsonDocument` duplicate-preserving property enumeration and bounded ZIP streaming; inbox 5.1 fails early via `#requires`, and PowerShell is not a Hieronymus runtime dependency), `scripts/install-desktop.ps1 -ReleaseDir <dir> [-AppDir <dir>] [-DataRoot <dir>] [-UnitDir <dir>] [-NoActivate]`. Both verify bounded bootstrap archives before executing their CLI, then use its strict complete pair verifier and `desktop-bootstrap` activation operation. `--no-activate` refuses takeover of running components; it installs registration without starting them. Missing native runtime prerequisites or denied native login registration return an error rather than enabling a checkbox optimistically. Use the installed application menu/CLI to start the tray after an offline installation. On Linux, a custom `--unit-dir` is definition-only: fresh activation and replacement of a running daemon refuse before retirement or selection because managed restart is unavailable. Use `--no-activate` for offline installation there. Doctor accepts `--unit-dir` and candidate checks use the transaction’s exact directory.

`hiero update` retires only an authenticated helper in the caller's native session, refuses pending actions or other active sessions, confirms OS session-lock release, and switches the complete version. An existing registered but stopped daemon remains stopped. Replacement helpers use `--resume` and do not perform their normal initial Start. A failed Quit keeps its helper open and refuses retirement until resolved. User Quit persists intent before shutdown; replacement and rollback never clear it. A failed replacement is itself retired before rollback restores the complete prior version, matching assets and prior registration; only components active before the attempt are restored. Indeterminate native manager/browser operations retain continuation gates and refuse competing replacement/removal. Native registration rollback restores actual manager state as well as files.

Uninstall authenticates daemon shutdown, retires helpers, confirms ownership release, removes owned registrations, launchers, application versions and acquisition cache, and preserves settings/databases by default. Windows cannot delete an executing installed CLI: use `install-desktop.ps1 -ReleaseDir <verified-dir> -AppDir <app> -DataRoot <root> -Uninstall`; it runs an external verified bootstrap CLI. A direct installed Windows `hiero uninstall` refuses before mutation with that instruction. Persistent lifecycle/session/native/launch coordination lock files are retained even by explicit data deletion.

macOS installs `versions/<version>/Hieronymus.app/Contents/MacOS/hiero-desktop`, its Info.plist and icon inside the bundle. CLI, runtime, model and `assets.json` are outside the bundle in the version root. The outer manifest binds final bundle bytes, avoiding a signing-manifest cycle. Final hierarchy must be preserved at installation. CLI/helper sibling resolution understands this layout; the helper retains the stable app command endpoint. Intel runtime packaging remains refused until the pinned source build is independently promoted.

These candidates are **unsigned**. No Authenticode identity, Apple Developer signing identity, notarization API credentials or stapling receipt was provided. Linux strips only Hieronymus executables and retains diagnostic symbol files keyed by the exact original executable SHA-256. macOS uses native `strip -S` before any future signing; Windows retains native MSVC output and records that PDBs are separate when generated. Pinned upstream runtime bytes are never stripped or re-signed. The measured published Linux saving and dependency explanation are in [binary-size.md](binary-size.md). Candidate size evidence must come from the final package receipt, not the published 0.8.0 measurement.

Windows/macOS native install, update, in-use executable, login-manager and rollback acceptance remain external qualification gates. Cross-source checks and portable tests do not establish native acceptance. Task14 owns the final platform/CI matrix and exact final-artifact evidence. Linux package qualification is recorded separately; it does not imply native tray-host/session acceptance on every Linux desktop.

Helper control uses a private `.tray-<root/session-hash>.json` instance record and the matching persistent OS lock. Native callbacks and controller submission only set flags/enqueue actions. The owned lifecycle worker persists Quit before requesting daemon shutdown. The owned control thread flushes queued Quit before retirement acknowledgement and again after the native exit drain and lifecycle-worker join, then removes its instance record. Replacement requires authenticated acknowledgement, actual session-lock acquisition, and completed record removal. A failed user Quit retains the UI. A late intent-flush or record-cleanup failure can occur after UI exit: it is an indeterminate cleanup outcome, not evidence that the old tray remains alive, and blocks replacement through the remaining stale record.

For a known stale record in the current login session, resolve the reported storage/private-file permission error, launch the **currently selected** packaged helper directly with `--resume --data-root <root> --unit-dir <unit-dir> --binary <app>/bin/hiero` (use `.exe` on Windows and the bundle executable on macOS), then use Quit. This acquires the actual session lock, writes a new authenticated instance record, and completes normal cleanup; `--resume` only suppresses initial daemon Start and grants no lock bypass. Records from unknown/other sessions remain a refusal requiring investigation. Never delete ambiguous records or infer ownership from a missing PID.

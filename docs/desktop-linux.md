# Linux desktop helper

`hiero tray --data-root /absolute/path` runs the installed sibling
`hiero-desktop`. Install both matching binaries in the same directory. The CLI,
MCP adapter and daemon remain usable without GTK or a graphical session.
A missing helper reports that the matching desktop package is required.

The Linux helper uses GTK 3 and `tray-icon` 0.24.2 through AppIndicator. It needs
GTK 3 runtime libraries and either `libayatana-appindicator3.so.1` or
`libappindicator3.so.1`. The build uses GTK bindings 0.18.2. The unused libxdo
feature is disabled. A working graphical session and user D-Bus session are
required. KDE supplies a StatusNotifier host; GNOME requires a compatible,
enabled AppIndicator extension. The helper reports a missing host and waits
for it to return. AppIndicator performs registration again when the watcher
owner changes; the helper retains its existing icon and menu.

A fresh helper explicitly starts the daemon once after owning its session and
creating the native UI. Duplicate launches for the same root/session exit
before startup. Polling never starts a stopped daemon. Quit stops the daemon
and exits only after the shared lifecycle controller confirms success. Failed
or timed-out stops keep the tray available with a textual reason.

The menu contains status, Open console, Start, Restart, Start at login, and
Quit, in that order. Status is always text and remains meaningful without
color. Actions are disabled while another action is pending. Open console uses
the existing one-time browser grant flow.

The concrete Linux login-registration adapter is a subsequent integration
step. At this stage Start at login is marked unavailable and disabled; the
helper does not claim a successful registration. Preference reconciliation
and readback run on the controller worker, including after a failed toggle.
A registration state that was actually read remains distinct from a settings
persistence failure.

## Foreground ink

The optional `desktop-settings.json` in the data root accepts:

```json
{
  "autostart": false,
  "foreground": "auto"
}
```

`foreground` accepts `auto`, `light`, or `dark`. Light and dark select the
**icon ink**, not the panel background. Preserve the existing autostart value
when editing this file. Restart the helper to apply an edited override.

Auto observes KDE's `Colors:Window` / `ForegroundNormal` in `kdeglobals`,
including atomic replacements of that file. Custom Plasma panels may differ
from the global palette; use an explicit override in that case. For GNOME,
the helper checks an available default Shell stylesheet for a literal `#panel`
foreground and observes the user-theme name and stylesheet changes. It does
not infer Shell colors from GTK or application color-scheme settings. Resource
stylesheets, inherited colors and custom Shell themes can leave the foreground
unknown; the helper diagnoses this and uses light ink until explicitly
overridden. Green, amber and red status accents remain in every mode.

## Qualification recorded on 2026-09-11

An isolated fixture ran on the available KDE Wayland session with GTK 3.24.52
and Ayatana AppIndicator 0.6.0. Native D-Bus registration and dbusmenu inspection
verified one item, menu order, disabled status/login entries, duplicate launch,
console launch invocation, start/restart, external-stop persistence, and
successful quit/unregistration. The exported native icon changed when the
fixture KDE palette was atomically replaced, then changed back on restoration.
An owned private D-Bus watcher fixture verified recovery from an absent host and
exactly one re-registration of the same item after watcher-owner replacement;
this was not a restart of the user panel. Its empty daemon correctly reported unavailable
semantic retrieval. HOME/XDG paths, service units, manager commands and browser
opening were isolated; no user service or panel was reconfigured. Browser
opening used a fixture sink that discarded the grant URL.

This does not establish visual panel acceptance, a real browser-console session,
login/install acceptance, packaged inference, GNOME acceptance, or minimum
supported distribution versions. Those remain release qualification work.

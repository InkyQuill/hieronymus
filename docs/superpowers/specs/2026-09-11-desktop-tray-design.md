# Desktop tray and cross-platform distribution

Date: 2026-09-11
Status: Design agreed in conversation; written specification awaiting review.
Baseline: `origin/main` at `af55690` (Rust release and standalone installer).

## Purpose and agreed scope

Hieronymus presents a persistent desktop indicator of server/MCP readiness,
opens its existing browser console, and controls the server without requiring
a terminal. Deliver complete desktop installations for Windows, macOS, and
Linux with KDE or GNOME AppIndicator support. Preserve headless operation.

The user selected the Folio H artwork and supplied `icon-color.svg` and
`icon-mono.svg` in the original checkout. Preserve these originals; copy them
into versioned asset sources during implementation. No redesign is required.

## Architecture

Add `hiero tray` as an independent process role. It owns presentation and
desktop actions, never databases, inference, or MCP sessions. The daemon
continues owning all runtime work. A tray crash does not terminate the daemon;
a daemon crash leaves the tray available for diagnosis and restart.

Separate these responsibilities into small Rust modules:

- A presentation-independent status reducer consumes authenticated snapshots,
  operation results, and elapsed time; it produces status, reason, and actions.
- A controller invokes existing lifecycle and console entry points. All blocking
  work runs outside the native UI event loop, with bounded work and cancellation.
- Platform adapters implement tray/menu integration, theme changes, browser
  opening, login registration, and per-user daemon service management.
- An asset pipeline derives platform images from the supplied SVGs.

Keep GUI dependencies outside the headless dependency graph. `hiero tray` can
dispatch a separately packaged native helper, while remaining the stable public
entry point. This allows the CLI/server binary to work without GTK or a display
server and avoids flashing a console window on Windows. Helper failure must
produce an actionable diagnostic. Native adapter/library selection and exact
crate placement are implementation-plan decisions, subject to these boundaries.

Use the existing authenticated discovery and process-instance checks. Tray
singleton ownership is scoped to canonical data root and desktop session;
daemon ownership remains per data root. Duplicate tray launches exit cleanly.
Concurrent CLI/tray lifecycle operations must serialize through a shared
per-root cross-process operation lock and recheck instance identity.

## Menu and user actions

Menu order:

1. Non-actionable current state and short reason.
2. Open console.
3. Start (enabled when safely known to be stopped).
4. Restart.
5. Start at login (checked preference).
6. Quit.

Use existing application localization conventions for labels. No separate
embedded webview, hide-only action, or notification stream is in this scope.
All actions are accessible through the menu; no essential behavior depends on
platform-specific left-click or tooltip support.

Open console reuses the one-time launch-grant flow. Tokens and grant-bearing
URLs never enter logs. Browser launch failure is shown in the menu without
changing otherwise healthy server status. Opening a stopped server's console
may start it through the same serialized lifecycle operation.

Start connects to an existing valid instance or starts the registered service.
Restart keeps the tray present, performs graceful stop, waits for resource
release, then starts and waits for readiness. Duplicate lifecycle actions are
disabled while a transition is pending; status and menu remain responsive.

Quit stops the daemon through the shared lifecycle controller, confirms it is
stopped, then exits the tray. Failure or timeout keeps the tray present with a
reason; never start a second daemon or force-kill as an implicit fallback.
Quit preserves the login preference. Explicit stop/quit must not be undone by
service-manager recovery or a concurrent tray operation. A later explicit
start, existing opt-in CLI autostart, or next enabled login can start it again.
Passive status polling never starts a server. An external CLI stop leaves the
tray showing stopped until an explicit start.

## Startup and platform integration

Desktop installation enables login startup by default and offers an opt-out.
Headless installation remains available without desktop registration. Changing
the preference affects future login startup, not the current running instance.
Read back registration failures; never display a successful toggle on failure.

Planned per-user mechanisms (no administrator/root requirement):

| Platform | Tray login entry | Daemon manager |
| --- | --- | --- |
| Linux KDE/GNOME | XDG desktop autostart | Existing systemd user service |
| macOS | User LaunchAgent in graphical login session | Separate user LaunchAgent |
| Windows | Task Scheduler logon trigger with interactive user token | Separate per-user scheduled task, started on demand |

Login launches the tray, which ensures the daemon is running. Desktop install
must reconcile existing daemon login registration so the checkbox governs the
pair and no old entry bypasses it. Retain the existing explicit headless service
mode. Service-manager recovery can recover crashes, but not intentional stops;
normal tray exit has no unconditional restart policy. No daemon autostart
registration may silently overwrite a conflicting data-root installation.

Linux uses AppIndicator/StatusNotifier. GNOME requires a compatible enabled
extension; document this installation requirement. Detect an unavailable tray
host without claiming that an invisible icon was successfully displayed. Keep
the CLI usable, report the cause, and support re-registration after panel or
tray-host restart. Windows Explorer restart and macOS session lifecycle must
likewise restore or cleanly recreate presentation without duplicating servers.

Port all necessary lifecycle, browser-opening, file locking, permissions,
process identity, installation, and native inference paths; changing the tray
library alone does not establish Windows/macOS support. Preserve secret
protection with native user-only permissions/ACLs and equivalent atomicity and
exclusive ownership guarantees. Amend ADRs 0009 and 0013 when implementing
the new platform/service support; this document is not evidence of acceptance.

## Status and health semantics

The daemon owns an additive authenticated readiness summary shared by tray,
CLI, and console. Do not make each consumer infer readiness from unrelated
historical fields. Existing `/status` already exposes semantic worker state,
but provider configuration is not evidence of provider reachability.

Poll every 3 seconds, with at most one request in flight and a 2-second probe
deadline. The first transient timeout produces a warning/checking state; three
consecutive failed probes produce red. A verified successful response recovers
according to its readiness summary. Explicit authentication/instance mismatch
is immediately an error, never a reason to adopt or signal a foreign process.

| Condition | Accent | Menu meaning |
| --- | --- | --- |
| Authenticated server/MCP ready, required local features ready, no current relevant failures | Green | Ready |
| Startup, model acquisition/rebuild, restart, checking after transient timeout | Amber | Transition with reason |
| Server responds but an enabled feature is degraded | Amber | Limited functionality with reason |
| Confirmed server absence, unexpected exit, failed start, identity/authentication failure | Red | Stopped or error with reason |

Normal dreaming or ordinary ongoing work does not itself cause a warning.
Semantic failure with a responsive server is degraded/amber. A provider that
is not used by an enabled feature cannot degrade the aggregate status.

Track real provider outcomes per effective provider/model/configuration
revision, including timestamp and affected capability. A relevant current
failure gives amber; a later successful real request on that configuration
clears it. Reconfiguration resets observations to untested. Success for another
provider or a stale request must not clear a failure. Cancellation caused by
normal shutdown and invalid user input are not provider health failures.

No background paid generation calls. Untested providers are labeled "not yet
checked" in details and are not falsely reported as verified. Green means local
readiness and no known current restrictions, not a guarantee of external
availability. Retain last observed outcome during a daemon lifetime; after
restart display untested rather than trusting historical successes or errors.
Expose sanitized reasons only; never include prompts, credentials, or raw
provider responses in status/menu text.

## Artwork and theme behavior

Keep the two structural regions of the supplied color SVG. Replace its navy
`#102134` region with `currentColor`; replace copper `#a45734` in tray renderings
with a semantic accent. Original copper remains available for ordinary app
branding. Proposed accent palette: green `#2EAD68`, amber `#E5A72B`, red `#D94A48`.
Validate these on supported panels and adjust for legibility without changing
their meanings.

Resolve foreground against the panel/menu-bar appearance, not merely the web
console or app theme. `currentColor` is an asset convention, not a promise that
every native tray renderer applies CSS. Native adapters resolve it and update
the rendered icon on appearance changes. Preserve the colored accent on macOS
using a color image rather than treating the whole icon as a monochrome template.
Where a custom panel cannot expose its appearance, provide a documented
auto/light/dark tray foreground setting; retain the status accent in all modes.

Rasterize/cache required size and scale variants deterministically. Keep the
original silhouette, transparency, generous clear padding, and crisp 16–24 px
appearance; verify high-DPI variants. Mono artwork remains a distributable asset
and accessibility fallback, not the default replacement for colored status.
Text in the menu always communicates the same status independently of color.

## Distribution and acceptance

Initial desktop artifact targets: Linux x86_64 (KDE and GNOME AppIndicator),
Windows x86_64, and macOS arm64 plus x86_64. Other CPU architectures are outside
this delivery. Exact minimum OS/dependency versions must be pinned by native
dependency compatibility checks before the implementation plan claims support.

Ship installable artifacts with the CLI/server, tray helper where applicable,
icons, embedded console, and matching ONNX/native runtime. Preserve mandatory
multilingual retrieval. Installers must configure per-user startup, support
upgrade/uninstall, clean owned registration/assets, and preserve user data and
preferences. Report required desktop libraries and install prerequisites.
macOS packaging must respect bundle identity; Windows desktop launch must not
open a terminal. Record signing/notarization status honestly; release credentials
are external inputs, never assumed available or embedded in the repository.

Validate with focused status-reducer and controller tests, then native tests:

- Every state transition, untested provider, configuration revision change,
  failure recovery, and inactive-provider exclusion.
- Responsive menus during slow probes/start/stop; bounded shutdown and no
  duplicate operations, trays, or daemon owners under concurrent CLI use.
- Start, restart, quit, failed stop, crash recovery, login, disabled login,
  installation upgrade, and uninstall preserving data.
- Correct launch-grant console entry and absence of secrets in diagnostics.
- Light/dark panels, live theme changes, all accents, 16/22/24 px and high DPI,
  missing tray host, panel/Explorer restart, and session logout/login.
- Real packaged native inference and MCP roundtrip on each artifact target.

Run repository-required Rust, documentation, scripts, and frontend checks.
Record actual OS, desktop, scale, artifact hash, and result for native manual
checks. Mocked adapters and cross-compilation do not establish desktop
acceptance. Missing native hosts or credentials remain explicit release
qualification gaps, not passing results. Preserve existing unrelated deferred
provider/agent qualification work; this feature does not close it by assertion.

## Sources checked during design

- [Rust tray-icon platform requirements](https://docs.rs/tray-icon/latest/tray_icon/)
- [StatusNotifierItem protocol](https://specifications.freedesktop.org/status-notifier-item/latest/status-notifier-item.html)
- [Apple template image semantics](https://developer.apple.com/documentation/appkit/nsimage/istemplate)
- [Apple Service Management](https://developer.apple.com/documentation/servicemanagement)
- [Microsoft logon-triggered executables](https://learn.microsoft.com/en-us/windows/win32/taskschd/starting-an-executable-when-a-user-logs-on)
- [GNOME AppIndicator support](https://extensions.gnome.org/extension/615/Appindicator-support/)

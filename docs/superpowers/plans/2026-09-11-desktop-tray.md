# Desktop Tray Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a theme-aware, two-color Hieronymus tray with truthful readiness, lifecycle controls, and installable Linux, Windows, and macOS builds.

**Architecture:** The existing headless `hiero` binary launches a sibling `hiero-desktop` helper for `hiero tray`. The helper consumes a shared pure presentation model and calls the authenticated lifecycle/console client. Native desktop adapters remain outside the server dependency graph; service management and portability primitives are shared by CLI and tray.

**Tech Stack:** Rust 1.96, existing SQLite/LanceDB/ONNX runtime, Bun 1.4.0 build tooling, Svelte 5 console, `tray-icon` 0.24.2 and its menu implementation; GTK3/AppIndicator on Linux, native Windows and AppKit event loops. Use `resvg` for deterministic asset rasterization in the desktop asset build tool. Pin added dependencies in Cargo.lock after verifying Rust/platform compatibility; do not upgrade the existing semantic stack incidentally.

**Spec:** [Desktop tray design](../specs/2026-09-11-desktop-tray-design.md), approved by the user on 2026-09-11.

## Global Constraints

- Initial desktop artifact targets: Linux x86_64 (KDE and GNOME AppIndicator), Windows x86_64, and macOS arm64 plus x86_64.
- No background paid generation calls.
- Quit stops the daemon through the shared lifecycle controller, confirms it is stopped, then exits the tray.
- Quit preserves the login preference.
- Passive status polling never starts a server.
- Tray singleton ownership is scoped to canonical data root and desktop session; daemon ownership remains per data root.
- Poll every 3 seconds, with at most one request in flight and a 2-second probe deadline.
- Three consecutive failed probes produce red; identity/authentication mismatch is immediately an error.
- Keep GUI dependencies outside the headless dependency graph.
- Preserve mandatory multilingual retrieval and user-only credential protection.
- Preserve the original user SVGs; derive platform images without changing the silhouette.
- Proposed accent palette: green `#2EAD68`, amber `#E5A72B`, red `#D94A48`.
- No implementation or testing may touch a real book workspace or the user's running daemon. Use disposable data roots, service names, and registrations.
- No release tag, publication, or deployed update feed is part of executing this plan. Produce reviewable artifacts and qualification evidence.

## Delivery sequence and baseline

Work from `codex/desktop-tray-design`, based on `af55690`, with approved spec commit `1149e0a`. Before implementation use the worktree skill and verify the branch, local instructions, and a clean working tree. The original checkout has user changes and untracked artwork; do not switch or clean it.

This is one dependent feature with three reviewable milestones:

1. Tasks 1–7: shared health/lifecycle plus working Linux desktop integration.
2. Tasks 8–11: portable runtime, Windows and macOS native integrations.
3. Tasks 12–14: installation/update and native release qualification.

Do not label milestone 1 as completion of cross-platform support. Tasks 8–11 can be reviewed separately from the Linux UI, but depend on the same contracts. Default execution is sequential unless the user chooses delegation.

Each test-bearing task follows RED → minimal implementation → GREEN → focused review → commit. Every checkbox is a concrete work action; split lengthy implementation actions into local checkboxes as needed without changing contracts. Code blocks below specify boundary types, regression examples, and implementation algorithms; complete production error handling according to the explicit cases in each task.

## File map

| Files | Responsibility |
| --- | --- |
| `crates/hiero/src/readiness.rs` | Serializable aggregate readiness contract |
| `crates/hieronymus/src/provider_observation.rs` | Sanitized provider outcome sink and configuration identity |
| `crates/hiero/src/daemon/readiness.rs` | In-memory provider observations and readiness snapshot |
| `crates/hiero/src/desktop/{mod,state,controller,settings,singleton}.rs` | Non-GUI tray model and orchestration |
| `crates/hiero/src/lifecycle/operation.rs` | Cross-process lifecycle operation ownership |
| `crates/hiero/src/service/{mod,linux,macos,windows}.rs` | Platform service backends; move existing service.rs into this module |
| `crates/hiero/src/platform/{mod,browser,credentials,install,export}.rs` | Platform operations currently coupled to Unix |
| `crates/hiero-desktop/{Cargo.toml,src/main.rs,src/lib.rs}` | GUI helper; never depended upon by hiero |
| `crates/hiero-desktop/src/{menu,icons,events}.rs` | Native view projection and event delivery |
| `crates/hiero-desktop/src/platform/{mod,linux,macos,windows}.rs` | Native event loop, theme and tray-host integration |
| `crates/hiero-desktop/src/bin/build-icons.rs` | Offline deterministic SVG → PNG/ICO/iconset asset build |
| `assets/icons/{icon-color,icon-mono,icon-tray}.svg` | Supplied originals and semantic two-region tray source |
| `scripts/{desktop-targets,desktop-package}.ts` | Target manifest and packaging |
| `scripts/install-desktop.{sh,ps1}` | Per-user desktop installation entry points |
| `docs/desktop-tray.md` | User-facing desktop installation and behavior |
| `docs/desktop-platforms.md` | Exact tested dependency/OS matrix and limitations |
| `docs/desktop-qualification.md` | Native acceptance procedure and recorded evidence |

Existing integration points confirmed on main: `lifecycle.rs`, `console.rs`, `daemon/rest/status.rs`, `daemon/dream_worker.rs`, `application/dream.rs`, `dream_workflows.rs`, `dream_providers.rs`, `export.rs`, `app.rs`, `update.rs`, `uninstall.rs`, `semantic_arming.rs`, `scripts/release-assets.ts`, `scripts/release-build.sh`, and `.github/workflows/release-rust.yml`.

## Task 1: Define aggregate readiness and pure tray state

**Files:** Create `crates/hiero/src/readiness.rs`, `crates/hiero/src/desktop/mod.rs`, `crates/hiero/src/desktop/state.rs`, `crates/hiero/tests/desktop_state.rs`; modify `crates/hiero/src/lib.rs`.

**Interfaces:** Export these public types with `Debug, Clone, PartialEq, Eq`; readiness DTOs additionally derive Serialize/Deserialize using snake_case. `DesktopState` owns the consecutive failure count and current `View`.

```rust
pub enum ReadinessLevel { Ready, Degraded, Starting }
pub enum ProviderCondition { Untested, Healthy, Failed }
pub struct ProviderReadiness {
    pub provider: String,
    pub model: String,
    pub revision: u64,
    pub condition: ProviderCondition,
    pub observed_at: Option<String>,
    pub reason: Option<String>,
}
pub struct ReadinessSummary {
    pub level: ReadinessLevel,
    pub reasons: Vec<String>,
    pub providers: Vec<ProviderReadiness>,
}
pub enum Accent { Green, Amber, Red }
pub enum Action { OpenConsole, Start, Restart, SetAutostart(bool), Quit }
pub struct View {
    pub accent: Accent,
    pub reason: String,
    pub busy: bool,
    pub can_start: bool,
    pub exit_requested: bool,
}
pub enum Event {
    Snapshot(ReadinessSummary),
    ProbeTimeout,
    InvalidIdentity,
    Stopped,
    Begin(Action),
    Finished { action: Action, error: Option<String> },
}
// DesktopState::new() -> Self
// DesktopState::apply(&mut self, event: Event) -> &View
```

- [ ] Add reducer tests, including this regression, authenticated recovery, busy-state precedence, stopped/start-enabled, quit failure, and successful quit.

```rust
#[test]
fn three_timeouts_confirm_failure() {
    let mut state = DesktopState::new();
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Amber);
    assert_eq!(state.apply(Event::ProbeTimeout).accent, Accent::Red);
}
```

- [ ] Run `cargo test -p hiero --locked --test desktop_state`; expect unresolved new imports before implementation.
- [ ] Implement event reduction: failures saturate at three, valid snapshots reset the counter, identity failure is immediate red, pending stop/restart remains amber, failed quit never requests UI exit. Snapshot events cannot clear a pending operation or its failure accidentally. A fresh missing record after explicit start is amber until the existing startup deadline; otherwise it is stopped/red.
- [ ] Run the focused test; add exhaustive table cases for each spec status row and ensure all pass.
- [ ] Commit `feat: define desktop readiness and state transitions`.

## Task 2: Observe actual provider outcomes and expose readiness

**Files:** Create `crates/hieronymus/src/provider_observation.rs`, `crates/hiero/src/daemon/readiness.rs`, `crates/hiero/tests/readiness_contract.rs`; modify `crates/hieronymus/src/lib.rs`, `dream_providers.rs`, `dream_workflows.rs`, `crates/hiero/src/daemon/mod.rs`, `daemon/dream_worker.rs`, `application/dream.rs`, `daemon/rest/status.rs`, `daemon/rest/providers.rs`, `daemon/rest/settings.rs`.

**Interfaces:** The core owns a narrow observer, without depending on hiero. The daemon implements it and stores observations in memory. Retain existing constructors; add opt-in observation to the production resolver.

```rust
pub struct ProviderKey {
    pub profile: String,
    pub model: String,
    pub revision: u64,
}
pub enum ProviderOutcome { Success, Unavailable, Authentication, RateLimited, InvalidResponse }
pub trait ProviderObserver: Send + Sync {
    fn completed(&self, key: &ProviderKey, sequence: u64, outcome: ProviderOutcome);
}
// WorkflowResolver::with_observer(self, observer: Arc<dyn ProviderObserver>,
//                                 revision: u64) -> Self
// RuntimeReadiness::snapshot(&self) -> ReadinessSummary
// RuntimeReadiness::activate(&self, keys: Vec<ProviderKey>)
```

- [ ] Add contract tests that configure one enabled workflow, feed failure then success observations, and assert amber then green; an unused profile must not degrade the result. Add a stale-revision regression:

```rust
#[test]
fn stale_provider_success_cannot_clear_current_failure() {
    let runtime = RuntimeReadiness::default();
    let old = ProviderKey { profile: "primary".into(), model: "model".into(), revision: 1 };
    let current = ProviderKey { revision: 2, ..old.clone() };
    runtime.activate(vec![current.clone()]);
    runtime.completed(&current, 2, ProviderOutcome::Unavailable);
    runtime.completed(&old, 3, ProviderOutcome::Success);
    assert_eq!(runtime.snapshot().providers[0].condition, ProviderCondition::Failed);
}
```

- [ ] Run `cargo test -p hiero --locked --test readiness_contract`; expect the new contract to fail.
- [ ] Implement monotonically increasing request sequences and in-memory configuration generations. Reconcile effective enabled workflows and defaults at request-start and snapshot time, including edits made outside REST; compare configuration privately and never expose keys or key hashes. Ignore older sequences/revisions. Unused and deterministic profiles contribute no external warning. Untested contributes no claim of confirmed reachability.
- [ ] Attach observations at real provider completion across scheduled and manual dream paths. Classify transport/auth/rate/invalid-response errors using structured results, not arbitrary error-string substring matching. Ordinary user input errors and shutdown cancellation do not publish failures. Model-list checks must not clear failed generation readiness.
- [ ] Add `readiness` to authenticated `/status` without removing existing fields. Compute semantic state from the worker snapshot; never run inference/network requests in the status handler. Verify identity/protocol checks still precede use of readiness. Add a sentinel-secret test ensuring no prompt, key, URL credentials, or raw provider response is returned.
- [ ] Run `cargo test -p hiero --locked --test readiness_contract --test daemon_rest_routes --test dream_execution --test provider_checks` and focused core observer tests; expect PASS. Commit `feat: expose observed runtime readiness`.

## Task 3: Serialize lifecycle and make explicit quit authoritative

**Files:** Create `crates/hiero/src/lifecycle/operation.rs`, `crates/hiero/tests/lifecycle_operations.rs`; modify `crates/hiero/src/lifecycle.rs`, `service.rs`, `main.rs`, `update.rs`, `uninstall.rs`.

**Interfaces:** `LifecycleOperation::acquire(config: &HieronymusConfig) -> io::Result<Self>` holds a distinct `.lifecycle.lock`, not the daemon ownership lock. Preserve public start/stop/restart signatures. Internal unlocked operations accept `&LifecycleOperation` so restart does not recursively acquire it.

- [ ] Add a child-process test: process A holds the lifecycle lock; process B receives `WouldBlock`; after A exits, B acquires it. Add restart-versus-stop, install/root mismatch, and stop-timeout tests using a fake manager and disposable daemon.

```rust
#[test]
fn lifecycle_lock_does_not_replace_daemon_ownership() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let owner = RootOwnership::acquire(&config, "daemon").unwrap();
    let operation = LifecycleOperation::acquire(&config).unwrap();
    assert!(RootOwnership::acquire(&config, "second-daemon").is_err());
    drop(operation);
    drop(owner);
}
```

- [ ] Run `cargo test -p hiero --locked --test lifecycle_operations`; verify RED.
- [ ] Wrap all managed lifecycle entry points in nonblocking operation acquisition. Under the guard, re-probe instance and unit ownership before changing anything. Suppress service-manager recovery for an explicit stop while retaining future login registration. Use authenticated graceful shutdown and existing bounded wait; never force termination after a timeout. For foreign/manual daemon instances, act only after authenticated identity verification and never operate a mismatched service unit.
- [ ] Exercise each manager call through injected command execution and assert restart performs stop/wait/start once, stops on failure, and status polling never starts anything. Keep the Linux `Restart=on-failure` behavior for crashes while deliberate successful shutdown remains stopped.
- [ ] Run `cargo test -p hiero --locked --test lifecycle_operations --test daemon_lifecycle --test service_cli --test runtime_shutdown`; commit `fix: serialize managed daemon lifecycle operations`.

## Task 4: Implement non-GUI desktop controller and settings

**Files:** Create `crates/hiero/src/desktop/controller.rs`, `settings.rs`, `singleton.rs`, `crates/hiero/tests/desktop_controller.rs`; modify `desktop/mod.rs`, `lifecycle.rs`, `console.rs`.

**Interfaces:** All errors in these boundaries are sanitized human-readable strings. Worker shutdown joins owned threads; only native platform signal delivery belongs to the GUI helper.

```rust
pub trait DesktopBackend: Send + 'static {
    fn probe(&mut self) -> Event;
    fn perform(&mut self, action: &Action) -> Result<(), String>;
}
pub enum ForegroundMode { Auto, Light, Dark }
pub struct DesktopSettings {
    pub autostart: bool,
    pub foreground: ForegroundMode,
}
// Controller::spawn(backend: impl DesktopBackend) -> Controller
// Controller::submit(&self, action: Action) -> Result<(), String>
// Controller::try_event(&self) -> Option<Event>
// Controller::shutdown(self) -> Result<(), String>
// TraySingleton::acquire(config: &HieronymusConfig, session: &str) -> io::Result<Self>
```

- [ ] Write tests with an inline fake backend recording calls through `Arc<Mutex<Vec<Action>>>`. Its `probe()` increments an atomic counter and returns `Event::Stopped`; `perform()` pushes the action and returns a configurable result. Assert passive probes produce zero start calls, failed Quit emits an error and no exit, and duplicate actions are rejected while busy.
- [ ] Run `cargo test -p hiero --locked --test desktop_controller`; verify RED.
- [ ] Implement one bounded worker command queue, one in-flight operation, and coalesced snapshot delivery. Use 3-second polling with bounded 2-second authenticated probe transport; wake immediately for user actions. Preserve existing longer lifecycle deadlines. Introduce a deadline-aware probe entry point rather than changing all consumers globally.
- [ ] Persist settings atomically under the configuration root. Derive singleton identity from canonical root and trusted OS session identity; hold an OS lock for helper lifetime without unlinking it. Duplicate helper startup returns a distinct clean-success outcome. Register/unregister autostart first, read back actual result, then persist; repair interrupted preference changes from registration state.
- [ ] Run controller and settings tests with a fake clock or injected poll schedule; no multi-second sleeps. Commit `feat: add desktop controller and persistent preferences`.

## Task 5: Preserve SVG artwork and build semantic assets

**Files:** Create `assets/icons/icon-color.svg`, `icon-mono.svg`, `icon-tray.svg`, `crates/hiero-desktop/Cargo.toml`, `src/lib.rs`, `src/icons.rs`, `src/bin/build-icons.rs`, `tests/icons.rs`; modify workspace `Cargo.toml` and lockfile.

**Interfaces:** `render_icon(foreground: [u8; 3], accent: [u8; 3], size: u32) -> Result<Vec<u8>, String>` returns straight-alpha RGBA. `build-icons --out <directory>` writes named light/dark status PNGs, app ICO, and macOS iconset. Only these two-color source regions may be substituted.

- [ ] Copy the originals from `/home/inky/Development/hieronymus/icon-color.svg` and `icon-mono.svg` byte-for-byte. Record SHA-256 in `assets/icons/README.md`; inspect their paths/fills before deriving `icon-tray.svg` with explicit region IDs and `currentColor` foreground.
- [ ] Create the desktop crate as a workspace member with default-members remaining the two headless crates. Put rasterization and GUI dependencies only in the desktop crate. Add pixel-content tests:

```rust
#[test]
fn icon_has_transparent_padding_and_both_regions() {
    let rgba = render_icon([240, 240, 240], [46, 173, 104], 24).unwrap();
    assert_eq!(rgba.len(), 24 * 24 * 4);
    assert_eq!(rgba[3], 0);
    assert!(rgba.chunks_exact(4).any(|p| p == [46, 173, 104, 255]));
    assert!(rgba.chunks_exact(4).any(|p| p == [240, 240, 240, 255]));
}
```

- [ ] Run `cargo test -p hiero-desktop --locked --test icons`; verify RED before adding rendering.
- [ ] Implement source-region replacement before SVG parsing and deterministic resvg rasterization. Reject absent/duplicate region IDs and unsupported sizes. Produce sizes 16, 20, 22, 24, 32, 40, 44, 48, 64 for status icons and 16 through 1024 powers of two for app branding. Preserve original copper in app icons; set explicit foreground for all generated bitmaps. Cache by foreground/accent/size.
- [ ] Render a review sheet on light and dark backgrounds at native size and 4× nearest-neighbor magnification; inspect curves, padding, contrast, and matching silhouettes. Run the pixel tests and verify source checksums unchanged. Commit `feat: add semantic tray icon assets`.

## Task 6: Native Linux tray helper and CLI dispatch

**Files:** Create `crates/hiero-desktop/src/main.rs`, `menu.rs`, `events.rs`, `platform/mod.rs`, `platform/linux.rs`, `tests/menu.rs`, `crates/hiero/src/desktop/launch.rs`; modify `crates/hiero/src/main.rs`, `desktop/mod.rs`, `crates/hiero-desktop/Cargo.toml`.

**Interfaces:** `MenuProjection::from_view(view: &View, settings: &DesktopSettings) -> Self` is pure. `platform::run(config: HieronymusConfig) -> Result<(), String>` owns the native UI event loop. `desktop::launch::launch(config: &HieronymusConfig) -> Result<(), String>` starts the installed sibling helper using an absolute path.

- [ ] Add menu projection tests asserting spec order, disabled status line, Start eligibility, checked autostart, and no duplicate action while busy. Add CLI tests for `hiero tray --data-root`, missing helper diagnostic, and argument forwarding without shell interpolation.
- [ ] Run `cargo test -p hiero-desktop --locked --test menu` and `cargo test -p hiero --locked --test argv_routing`; verify new tests fail.
- [ ] Add GTK event loop plus tray-icon/AppIndicator menu construction. Deliver controller messages through a wakeable event-loop channel, never synchronous HTTP from callbacks. Retain native menu/icon objects for helper lifetime. Dispatch only enum actions by stable IDs, not localized text.

```rust
let tray = tray_icon::TrayIconBuilder::new()
    .with_menu(Box::new(native_menu))
    .with_icon(tray_icon::Icon::from_rgba(rgba, width, height)
        .map_err(|error| error.to_string())?)
    .build()
    .map_err(|error| error.to_string())?;
// Keep `tray` alive for the complete native event loop.
```

- [ ] Resolve Linux panel foreground from available KDE palette or GNOME shell appearance rather than the console theme. Observe relevant theme changes; use the spec's explicit foreground override for custom panels. Detect StatusNotifierWatcher owner changes and re-register once; avoid duplicate registrations. Missing host must yield a diagnostic and a recoverable waiting state if the helper is running.
- [ ] Verify `cargo tree -p hiero` has no GTK, tray-icon, or desktop-helper dependency. Run the menu/controller tests and manually open/quit a disposable daemon in KDE and GNOME with AppIndicator, including panel restart and Wayland session. Commit `feat: add native Linux tray launcher`.

## Task 7: Linux desktop installation and autostart

**Files:** Create `scripts/install-desktop.sh`, `crates/hiero/tests/desktop_install_linux.rs`, `docs/desktop-tray.md`; modify `crates/hiero/src/service.rs`, `desktop/settings.rs`, `uninstall.rs`, `scripts/install.sh`.

**Interfaces:** `hiero desktop install`, `hiero desktop uninstall`, and `hiero desktop autostart on|off|status` provide non-GUI registration actions. Reuse the controller backend for the checkbox. Commands accept disposable directory overrides and `--no-activate` consistent with existing installer tests.

- [ ] Add an installer test invoking registration in a temp home and fake systemctl recorder. Assert `.desktop` entry uses absolute quoted executable/root, autostart preference is read back, and old standalone daemon login enablement is reconciled only when owned by this install.
- [ ] Run `cargo test -p hiero --locked --test desktop_install_linux`; verify RED.
- [ ] Render the graphical autostart entry and application launcher. Systemd daemon service stays startable on demand, with no independent desktop-mode login enablement. Preserve headless install defaults. Switching mode records prior owned registration and rejects mismatched roots/binaries rather than overwriting them. Disable login without stopping the current daemon; unregister removes owned files only.

```ini
[Desktop Entry]
Type=Application
Name=Hieronymus
Terminal=false
Icon=hieronymus
```

The renderer supplies `Exec` with escaped absolute arguments, never user text concatenated into a shell command. GNOME/AppIndicator prerequisite errors must include a usable recovery instruction.

- [ ] Test paths containing spaces, quotes, Unicode, `%`, and newlines (encode valid desktop arguments and reject unrepresentable values). Exercise enable/disable failure, reinstall, uninstall, and `--no-activate`. Commit `feat: install Linux desktop startup and launcher`.

## Task 8: Port filesystem and credential guarantees

**Files:** Create `crates/hiero/src/platform/mod.rs`, `credentials.rs`, `install.rs`, `export.rs`, `crates/hiero/tests/platform_filesystem.rs`; modify `export.rs`, `app.rs`, `update.rs`, `uninstall.rs`, `daemon/discovery.rs`, `doctor.rs`, `crates/hiero/Cargo.toml`, `crates/hieronymus/src/atomic.rs`, `upgrade.rs`, `dream_locks.rs`.

**Interfaces:** Keep existing public export/install operations. Place handle-based implementations behind cfg modules; no non-Unix success stubs. Use std `File::try_lock` for ownership on all targets. Windows credential operations require an owner-only protected DACL at file creation, not a permissive create followed by repair.

- [ ] Add OS-native tests for user-only secret creation/readback, rejecting permissive credentials, atomic replace/no-clobber, export symlink/hardlink/reparse-point guards, and installed-version switching.

```rust
#[test]
fn credential_creation_never_overwrites_an_existing_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("token");
    create_private_new(&path, b"first").unwrap();
    assert!(create_private_new(&path, b"second").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
}
// platform::credentials::create_private_new(path: &Path, bytes: &[u8]) -> io::Result<()>
```

- [ ] Run the new test on Linux plus native `cargo check -p hiero --all-targets --locked` on Windows/macOS. Capture the actual unsupported imports; begin with known `app.rs` symlink, `export.rs` Unix metadata/rustix, and permission checks.
- [ ] Preserve Linux descriptor-relative export behavior. Implement Windows file identity/volume ID and reparse-point checks using opened handles, exclusive creation, and native atomic publication; test parent path replacement attacks. Do not replace these protections with path-only prechecks. For version installation, use immutable version directories and a small atomically updated selected-version record/native launcher on Windows instead of privileged symlinks.
- [ ] Remove `/proc` as the only way to determine stale dream ownership: validate lock availability and recorded instance/run identity conservatively across platforms; PID existence alone never grants ownership or authorizes deletion. Add crash/PID-reuse cases.
- [ ] Run native filesystem/ownership/export tests on all three OS families; preserve Unix-specific regressions under cfg and add Windows equivalents. Commit `feat: preserve runtime filesystem guarantees across desktop platforms`.

## Task 9: Platform runtime assets and target manifest

**Files:** Create `scripts/desktop-targets.ts`, `scripts/desktop-targets.test.ts`, `docs/desktop-platforms.md`; modify `scripts/release-assets.ts`, `stage-release-assets.ts`, `check-rust-release.ts`, `crates/hieronymus/src/semantic_arming.rs`, `crates/hiero/src/release_source.rs`, `crates/hiero/tests/release_source.rs`.

**Interfaces:** Target descriptors explicitly bind architecture, archive extension, runtime member path, release metadata filename, and verified runtime hashes. Existing Linux pins stay unchanged. Target names are exact Rust triples, not host guesses.

```typescript
export type DesktopTarget =
  | "x86_64-unknown-linux-gnu"
  | "x86_64-pc-windows-msvc"
  | "aarch64-apple-darwin"
  | "x86_64-apple-darwin";
export function runtimeName(target: DesktopTarget): string {
  if (target.endsWith("windows-msvc")) return "onnxruntime.dll";
  if (target.endsWith("apple-darwin")) return "libonnxruntime.dylib";
  return "libonnxruntime.so";
}
```

- [ ] Write `expect(runtimeName("x86_64-pc-windows-msvc")).toBe("onnxruntime.dll")` and tests rejecting unknown/mismatched target/archive/runtime metadata, duplicate extraction members, traversal, and decompression overflows. Run `bun test scripts/desktop-targets.test.ts`; verify RED.
- [ ] Read the pinned ONNX Runtime 1.28.0 upstream release metadata; acquire each official target asset, independently compute archive/member hashes and record them in the target manifest. Record minimum OS requirements from the actual binaries and upstream build configuration. Fail acquisition if a required target asset is absent; never invent a hash or silently substitute model/runtime versions.
- [ ] Extend bounded extraction for Windows ZIP and Darwin tar assets, preserving origin allowlists and size/path protections. Select DLL/dylib/so per build target and carry any required companion native libraries and notices. Bind all members in assets.json.
- [ ] Run installed semantic verification on each native architecture with the pinned model and a disposable data root. Record toolchain, OS floor tested, deployment target, libc/CRT requirements, and evidence in `docs/desktop-platforms.md`. No generic compatibility claim can replace this task's measured matrix.
- [ ] Run script tests plus `cargo test -p hiero --locked --test release_source`; commit `feat: bind desktop releases to native runtime targets`.

## Task 10: Windows service, login, browser and tray adapter

**Files:** Move `crates/hiero/src/service.rs` to `service/mod.rs` and extract `service/linux.rs`; create `service/windows.rs`, `platform/browser.rs`, `crates/hiero-desktop/src/platform/windows.rs`, `crates/hiero/tests/windows_desktop.rs`; modify `console.rs`, `service/mod.rs`, desktop helper dependencies/main.

**Interfaces:** Preserve `ServiceOptions` and public service APIs. Backend selection is compile-time. Separate `install/start/stop/status` daemon operations from desktop `set_autostart/read_autostart`; both bind user SID and root. Use Task Scheduler COM registration with interactive token and no elevation.

- [ ] Add native Windows tests that render/register only disposable task names and inspect action executable, separate arguments, principal, login trigger, no execution-time limit, no idle/battery restrictions, and no unconditional restart after explicit stop. Test registration rollback and rejection of another installation's task.
- [ ] Run `cargo test -p hiero --locked --test windows_desktop`; verify new native assertions fail.
- [ ] Implement an on-demand daemon task and a separate logon-triggered tray task. Start/restart use the shared operation lock. Quit obtains an authenticated shutdown acknowledgement and confirms exit; disable recovery while intentional stop is in progress. Recovery re-enables on explicit next start. Never use Task Scheduler hard End as a graceful-stop fallback. Handle logoff as session teardown, not a user Quit command to a different session.
- [ ] Implement native browser opening with the existing one-time grant URL, no shell interpolation or logging. Set the helper Windows subsystem attribute and start console-binary child actions without a visible terminal.

```rust
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
```

- [ ] Implement the Win32 event loop and tray/menu adapter using the shared projection. On Explorer TaskbarCreated recreate the icon once. Observe system taskbar-theme/high-contrast/DPI changes; preserve accent and use explicit foreground override when appearance is ambiguous. Reuse core controller deadlines and singleton semantics.
- [ ] Test on a native interactive Windows user account: no terminal flash, duplicate launch, real login autostart, toggle, browser console, restart/quit, Explorer restart, and theme/scale changes. Record results; commit `feat: support native Windows desktop lifecycle`.

## Task 11: macOS LaunchAgents, browser and tray adapter

**Files:** Create `crates/hiero/src/service/macos.rs`, `crates/hiero-desktop/src/platform/macos.rs`, `crates/hiero/tests/macos_desktop.rs`, `assets/macos/Info.plist`; modify `service/mod.rs`, `platform/browser.rs`, desktop helper dependencies/main.

**Interfaces:** Same service and desktop registration APIs as Task 10. Stable bundle identity `net.inkyquill.hieronymus`; per-root agent labels append a non-secret canonical-root digest. The helper is an agent-style app without a Dock window.

- [ ] Add tests for escaped plist arguments, current-user graphical domain, distinct daemon/tray labels, no unconditional KeepAlive after Quit, and recovery after registration failure. Run `cargo test -p hiero --locked --test macos_desktop`; verify RED.
- [ ] Implement per-user LaunchAgent install/bootstrap/bootout and state readback. Tray login entry starts the helper; daemon is loaded/startable on demand without independent RunAtLoad in desktop mode. During explicit stop, suppress recovery while preserving future login preference. Use authenticated graceful shutdown before any bootout that might terminate a process; report timeouts without force-killing.
- [ ] Implement browser open through the platform adapter without secret-bearing diagnostics. Start native tray creation only after the AppKit main-thread event loop has begun. Use color images (not a whole-icon template) and resolve foreground from status-button appearance, including highlight; refresh for appearance and scale changes.

```xml
<key>CFBundleIdentifier</key><string>net.inkyquill.hieronymus</string>
<key>LSUIElement</key><true/>
```

- [ ] On arm64 and x86_64 native hosts verify menu, browser auth, autostart toggle, next login, no duplicate icon, launch without a terminal, full-screen app transitions, display-scale changes, and light/dark menu bars. Compile-only results remain separate from this acceptance list. Commit `feat: support native macOS desktop lifecycle`.

## Task 12: Cross-platform desktop installation and safe updates

**Files:** Create `scripts/desktop-package.ts`, `scripts/desktop-package.test.ts`, `scripts/install-desktop.ps1`, `crates/hiero/tests/desktop_update.rs`; modify `scripts/install-desktop.sh`, `app.rs`, `update.rs`, `uninstall.rs`, `release_source.rs`, `docs/desktop-tray.md`, ADRs `0009` and `0013`.

**Interfaces:** `bun scripts/desktop-package.ts --target <triple> --out <dir>` packages prebuilt exact-target binaries/native assets. Installers consume a local release directory, verify its complete manifest, and support `--no-activate`/equivalent. No download from an invented channel URL.

- [ ] Add packaging tests for missing helper/runtime/model, wrong architecture, stale version, mismatched checksums, and platform metadata filename collisions. Example: package a Windows descriptor with only `libonnxruntime.so`; assert a missing-DLL failure before writing a success manifest. Run `bun test scripts/desktop-package.test.ts`; verify RED.
- [ ] Build Linux desktop archive plus installer, Windows ZIP plus PowerShell installer, and macOS `.app` inside a distributable archive plus shell installer. Keep headless artifacts usable. Generate unique `release-<triple>.json` metadata so publication cannot overwrite another platform's manifest; preserve existing Linux `release.json` compatibility for old installers.
- [ ] Add staged desktop update tests: running helper/daemon, replacement helper startup failure, pending operation, and Windows executable in-use. Implement coordinated helper exit using authenticated per-user/root-bound local control; wait for it before binary replacement. Use versioned directories, rollback to the prior complete pair and matching assets, and restart only components previously active. Avoid generic unauthenticated local kill/control endpoints. Preserve explicit Quit intent if it races with update.
- [ ] Make uninstall stop/confirm owned processes, remove owned registrations, launchers and old assets, and preserve databases/settings. Missing dependencies or OS-denied login permissions produce actionable errors without a falsely enabled checkbox.
- [ ] Test isolated install/upgrade/rollback/uninstall on each OS. Document signing/notarization status and any external credentials required; do not mark signed when unsigned. Update ADRs to describe actual service modes and the measured support matrix. Commit `feat: package and update desktop installations`.

## Task 13: Share readiness with CLI and console

**Files:** Modify `crates/hiero/src/lifecycle.rs`, `frontend/src/web/lib/types.ts`, `frontend/src/web/components/AdminDashboard.svelte`; create `frontend/src/web/lib/readiness.ts`, `frontend/src/web/lib/readiness.test.ts`; extend `crates/hiero/tests/readiness_contract.rs`.

**Interfaces:** Read the additive readiness DTO from Task 1. The console can display server states Ready/Degraded/Starting and provider Untested/Healthy/Failed; it must not independently derive competing aggregate health from historical dream errors.

- [ ] Add frontend tests for unknown/missing readiness (older daemon), untested providers, degraded semantic state, and no accidental secret display. Use the existing server-status fixture with `readiness: {level: "degraded", reasons: ["Semantic index rebuilding"], providers: []}` and assert that exact explanation is displayed, not "Ready".
- [ ] Run `bun run test` in frontend for the added tests; verify RED.
- [ ] Add a small readiness formatter/DTO parser and render the summary in the existing dashboard status area. Preserve existing console design and authentication. CLI human status prints the same readiness/reasons; JSON retains the full additive DTO. Missing or unknown DTO version means readiness unknown, never automatic green.
- [ ] Run CLI contract tests and frontend typecheck/test/build; commit `feat: share readiness across tray CLI and console`.

## Task 14: CI matrix and installed native acceptance

**Files:** Modify `.github/workflows/pr.yml`, `.github/workflows/release-rust.yml`, `scripts/release-build.sh`, `docs/rust-cutover-rehearsal.md`; create `docs/desktop-qualification.md`, `scripts/check-desktop-evidence.ts`, `scripts/check-desktop-evidence.test.ts`.

**Interfaces:** Qualification JSON records `target`, `os_version`, `desktop`, `scale`, `artifact_sha256`, `checks`, and `signing`. Each check has `name`, `result` (`pass|fail|unavailable`), and `evidence_path`; evidence validation rejects missing files, mismatched hashes, failures, and unavailable required checks.

- [ ] Add evidence validator tests with temporary evidence files: a fully populated record passes, absent interactive-menu evidence fails, changed artifact hash fails, and `unavailable` cannot count as pass. Run `bun test scripts/check-desktop-evidence.test.ts`; verify RED.
- [ ] Implement validation with an explicit required-check list covering the spec's native matrix. Wire PR jobs to headless checks plus platform desktop compilation/tests; install GUI build dependencies only in desktop jobs. Preserve existing resource limits and protobuf preflight.
- [ ] Split release build jobs by exact target and aggregate only successfully verified artifacts. Keep explicit matching `v*` tag publication semantics; merely merging code must not publish or tag. Run fixture-based installed semantic/MCP tests on each native target and interactive acceptance separately; headless CI cannot prove a menu works.
- [ ] Execute all repository-required checks:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cargo clippy -p hiero-desktop --all-targets --all-features --locked -- -D warnings
cargo test -p hiero-desktop --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc -p hiero-desktop --no-deps --all-features --locked
bun test scripts/*.test.ts
```

In frontend run `bun run typecheck`, `bun run test`, and `bun run build`. On PowerShell set `RUSTDOCFLAGS` using native environment syntax; do not skip rustdoc checks.

- [ ] Perform installed manual acceptance on KDE and GNOME AppIndicator (Wayland and an X11 session), Windows, and both macOS architectures: startup, duplicate start, model acquisition, semantic failure, provider failure/recovery/untested, three probe failures, restart, failed stop, Quit, login toggle, next login, missing tray host, host restart, theme, DPI, console grant, upgrade/rollback/uninstall, and secret-free diagnostics. Use documented disposable fixtures for real model tests; missing fixtures must fail. Save evidence bound to exact artifacts, without claiming unavailable hosts passed.
- [ ] Review dependency graph for GUI leakage into headless release, run evidence validator, and commit `ci: qualify cross-platform desktop artifacts`. Report native gaps separately if an external host or signing credential is unavailable; do not mark full-platform delivery complete in that case.

## Self-review and handoff record

- Scope mapping: status → 1–2, lifecycle/singleton → 3–4, icons → 5, Linux → 6–7, portable server → 8–9, Windows/macOS → 10–11, install/update → 12, common consumers → 13, acceptance → 14.
- Untested external services are explicit; no periodic paid calls or historical-success guarantees.
- GUI dependencies live only in the helper; workspace default checks are supplemented with explicit helper checks.
- Source artwork is copied before any derived edits; original checkout changes remain untouched.
- Native target hashes/OS compatibility are acquired and measured in Task 9, not invented by this planning document. This plan is an execution sequence, not a claim that those platforms already work.
- Tests require real observable behavior; no implementation-mirroring menu snapshots alone establish acceptance.
- Source references: approved spec, [tray-icon 0.24.2 platform requirements](https://docs.rs/tray-icon/0.24.2/tray_icon/), and [ONNX Runtime compatibility](https://onnxruntime.ai/docs/reference/compatibility.html).

After user choice of execution method, follow the corresponding execution skill and begin Task 1. Do not publish a release or overwrite user startup configuration as part of ordinary development verification.

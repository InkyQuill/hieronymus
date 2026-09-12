# Use One Binary With Explicit Multi-Process Runtime Boundaries

## Status

Accepted on 2026-08-31.

## Context

The Rust proposal describes “one binary, one process,” while also allowing a
long-running daemon, direct CLI commands, and a stdio MCP adapter. Those are
separate operating-system processes and can contend for SQLite, the semantic
index, and dream-cycle ownership.

The current product also needs a daemon that survives terminal closure and is
available to configured HTTP MCP clients after login or reboot.

## Decision

Ship one distributable `hiero` binary with three explicit execution roles:

- `hiero daemon`: a foreground server process owning HTTP, WebSocket, MCP,
  background dreaming, and semantic-index workers;
- short-lived CLI processes for read-only inspection and narrowly approved
  maintenance commands;
- `hiero mcp`: a stdio adapter that connects to the daemon and never opens the
  application database directly.

All domain mutations used by normal CLI, MCP, and frontend workflows go through
the daemon. Offline database maintenance is limited to commands documented as
exclusive operations, including database upgrade, verification, backup, and
repair. Such commands must refuse to run while the daemon owns the data root.

`hiero start` installs or starts the per-user service appropriate to the
platform. `hiero daemon` remains foreground-only for supervisors and debugging.
`hiero stop` requests authenticated graceful shutdown through the discovered
daemon endpoint. Supported service managers are:

- Linux: systemd user service;
- macOS: LaunchAgent;
- Windows is outside the initial supported cutover. Adding it requires a later
  ADR that selects and verifies a per-user service mechanism.

The daemon writes bounded logs and a non-secret discovery record atomically.
The record contains protocol version, endpoint, process identity, start time,
and instance id. The bearer token is stored separately with user-only
permissions. Stale discovery state is detected by authenticated health probing
and process-instance comparison, never by PID existence alone.

Dreaming remains protected by an OS-level cross-process lock because exclusive
offline maintenance and manual debug execution can still be separate
processes. Every caller performs one nonblocking OS `try_lock_exclusive` on a
dedicated blocking thread. The scheduler skips the tick and records `locked`;
manual daemon requests return conflict; exclusive CLI maintenance exits with a
diagnostic naming the current owner. No caller waits or retries while holding a
Tokio worker thread. The guard owns the open file handle for the entire critical
section and releases it on drop.

Before binding a port or publishing discovery, daemon startup runs the shared
bounded `StateClassifier`. Classification reads schema/config version markers,
required file presence, and the cutover-journal state; it does not run typed
converters, integrity scans, backups, or index work. `hiero migrate --dry-run`
begins with this classifier but additionally performs the full converter and
verification rehearsal defined by ADR 0010. Only the current supported Rust
schema, current config versions, and a complete or absent cutover journal may
start. A legacy Python schema exits with
`migration_required`; legacy config exits with `config_migration_required`; a
post-database/pre-config cutover exits with `config_promotion_required`. Each
diagnostic includes the exact command. Newer, unknown, corrupt, or partially
upgraded state fails closed. Startup never auto-migrates and never publishes
readiness for rejected state.

## Consequences

The binary is operationally simple to distribute but the runtime topology is
honest about multiple processes. Centralizing normal mutations in the daemon
removes concurrent LanceDB writers and gives CLI, MCP, and web actions one
authorization, audit, and scoring path.

Direct-store CLI behavior from the initial proposal is rejected except for
explicit exclusive maintenance commands.

### 2026-09-11 desktop packaging amendment

The unpublished 0.9.0 candidate ships a separate native helper. Linux Desktop mode uses an on-demand daemon plus XDG tray login registration; Windows uses owned per-user native tasks; macOS preserves Headless/ Desktop `RunAtLoad` mode with separate owned LaunchAgents and no implicit crash restart. Actual native execution evidence is recorded in the desktop platform support matrix; source inclusion checks do not establish OS acceptance.

Desktop install/update holds one lifecycle operation through verified pair assembly, authenticated helper retirement, stop/root release, native registration snapshot, immutable version selection and activation/rollback. A root launch gate closes enumeration races, session OS locks prove retirement, and secret-protected root/session/instance records authenticate a distinct retirement action. Pending desktop actions, other active sessions and continuing native manager/browser gates refuse mutation. Explicit Quit intent survives retirement and failed activation; helper resume does not start a previously stopped daemon. Registrations and actual native enabled/mode state are restored along with the prior complete version. An indeterminate rollback retains artifacts and reports the pending state.

Standalone installers use local verified releases and the same Rust activation transaction. No installed Python or Bun is required. Windows direct self-uninstall refuses before mutation because the executing image cannot be removed; the PowerShell installer provides an external verified CLI uninstall path. Data/settings are preserved, and persistent coordination lock files are not unlinked.

A Linux custom unit directory remains definition-only. A package operation that needs managed startup/restoration refuses before retirement, stop or selection when that capability is unavailable. Offline install/upgrade evidence plus a separately launched disposable daemon does not establish native manager active-update acceptance.

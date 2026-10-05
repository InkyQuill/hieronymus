# 0009 — Daemon ownership and process boundaries

Status: accepted 2026-08-31; desktop packaging and server ownership amended
2026-09-11/12. Consolidated 2026-10-05.

## Context

A single distribution entry point does not mean one process. CLI, daemon, stdio
and native UI have different lifetimes; letting each write live memory creates
competing authority and recovery paths.

## Decision

`hiero daemon` owns HTTP/WebSocket/MCP, normal live mutations, Dreaming and semantic
workers. Short-lived CLI and `hiero mcp` processes communicate with it. The stdio
adapter never opens SQLite. Export is read-only; exclusive upgrade/recovery refuses
while the daemon owns the data root.

The domain crate owns storage and algorithms. `hiero` owns CLI/daemon/transports/
distribution. `hiero-desktop` isolates native UI dependencies from headless operation;
`hiero-decision` owns only the named model protocol. These are explicit boundaries,
not duplicated memory backends.

The server owns the running application and tray presence directly or through its
supervised helper. Desktop startup can ensure the server is running; the helper is
not the owner of database or inference. Supported managers are systemd user service,
macOS LaunchAgents and owned per-user Windows tasks. Desktop login preferences
and headless service mode remain distinct; see [tray registration](../desktop-tray.md).

Publish nonsecret discovery and separate private credentials atomically. Detect
stale state using authenticated health and process-instance identity, not PID alone.
Keep bounded readable diagnostics with credential/grant redaction. Startup classifies
schema/config/journal state before publication and never silently migrates legacy,
newer, corrupt or partial state. Report the exact remediation command.

Live mutations share domain validation/audit regardless of their frontend. Dream
cycles serialize with an OS lock per data root; scheduled collisions skip and manual
requests report conflict unless explicitly waiting. Never break a live lock or wait
while holding a database write transaction.

### Desktop lifecycle transactions

Install/update serializes verified pair assembly, helper retirement, stop/root release,
registration snapshot, immutable selection and activation/rollback. Session locks,
authenticated instance records and the root launch gate prevent enumeration races.
Continuing native operations, another active session or ambiguous ownership refuse
mutation. Explicit Quit intent survives retirement and failed activation; passive
polling or helper resume must not restart an intentionally stopped server.

Rollback restores registrations, native enabled/mode state and the prior complete
version. Indeterminate recovery retains artifacts and reports pending state.
Persistent coordination locks are not unlinked. Windows self-uninstall needs an
external verified helper path; data/settings remain unless explicitly deleted.

A custom Linux unit directory is definition-only and cannot establish native-manager
startup/update acceptance. Native GUI, manager, inference and host evidence remain
separate; source inclusion and portable tests do not qualify the desktop.

## Consequences

There is one live mutation authority and one shared validation path, while platform
UI can crash or recover independently. Headless operation needs no GUI library.
This topology keeps distribution small without hiding multi-process coordination.

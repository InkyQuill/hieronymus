# Use One Binary With Explicit Multi-Process Runtime Boundaries

## Status

Proposed.

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
- Windows: per-user startup integration selected during the Windows support
  spike; Windows is not declared supported until this path passes CI.

The daemon writes bounded logs and a non-secret discovery record atomically.
The record contains protocol version, endpoint, process identity, start time,
and instance id. The bearer token is stored separately with user-only
permissions. Stale discovery state is detected by authenticated health probing
and process-instance comparison, never by PID existence alone.

Dreaming remains protected by an OS-level cross-process lock because exclusive
offline maintenance and manual debug execution can still be separate
processes. Synchronous file-lock acquisition must run outside Tokio worker
threads or use bounded asynchronous retry.

## Consequences

The binary is operationally simple to distribute but the runtime topology is
honest about multiple processes. Centralizing normal mutations in the daemon
removes concurrent LanceDB writers and gives CLI, MCP, and web actions one
authorization, audit, and scoring path.

Direct-store CLI behavior from the initial proposal is rejected except for
explicit exclusive maintenance commands.

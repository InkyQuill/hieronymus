# Hieronymus Service Toolkit

Hieronymus installs two equivalent console commands:

- `hieronymus`
- `hiero`

Every subcommand works through either command. For example, `hieronymus status` and `hiero status`
call the same CLI entry point.

Running `hiero` with no subcommand starts the local daemon if it is not already running, then prints a
short status surface. The identity portion looks like this:

```text
🪶 Hieronymus v0.1.0
Remembers things for you.
```

The daemon is the normal service surface for the global SQLite store. Some legacy and debug CLI
commands still access storage directly in this pass; future agent adapters should discover the daemon
through runtime files under the configured root, defaulting to `~/.config/hieronymus`. `--data-root`
and `HIERONYMUS_DATA_ROOT` move these files:

- `server.json`
- `server.pid`
- `server.lock`

The daemon exposes a local HTTP JSON API on `127.0.0.1`. Human CLI output may use the Hieronymus
identity line, while automation should use `--json`.

## Direct Store Boundaries

These commands still use Python domain stores directly in this pass:

- `hiero init-series`: bootstrap command that creates registry rows before a service mutation API exists
- `hiero propose-term`: legacy termbase helper retained for local debugging of deterministic terminology storage
- `hiero validate`: legacy termbase validator that reads files locally and checks deterministic terminology rules
- `hiero remember`: legacy long-memory helper retained until old memory primitives are fully retired
- `hiero session-start`: agent workflow primitive that starts local workspace sessions through the domain store
- `hiero session-complete`: agent workflow primitive that completes local workspace sessions through the domain store
- `hiero remember-short`: agent workflow primitive that writes short-term observations through the domain store
- `hiero recall`: agent workflow primitive that combines recall service output without parsing human CLI text
- `hiero feedback`: agent workflow primitive that records correction events through the feedback store
- `hiero dream`: maintenance command that invokes DreamService directly so local dreaming works without a daemon

`hieronymus-mcp` is also a direct-store boundary: stdio MCP adapter uses Python domain stores directly because the local daemon currently exposes lifecycle and status HTTP only; it does not parse person-facing CLI output.

## Commands

- `hiero status --json` shows daemon status.
- `hiero doctor --json` checks settings parseability, active provider enablement,
  provider key env configuration, store health, and service health.
- `hiero stop` requests graceful shutdown.
- `hiero restart` restarts the daemon.
- `hiero config` opens the configuration TUI for providers, dreaming automation,
  service status, paths, and diagnostics.
- `hiero config --json` reports settings, provider status, and dreaming
  automation state.
- `hiero admin` opens the management TUI.
- `hiero install <app> --dry-run` shows the safe installer plan for an agent.
- `hiero install <app>` writes plugin assets and patches supported agent host config.

## Agent Install Boundary

This pass provides real Claude, Codex, OpenClaw, opencode, and Gemini CLI installer wiring. Pi and
Hermes are reserved detectable targets until their host config formats are known.

See [Agent workflows](agent-workflows.md) for the host integration contract.

`hiero doctor` also reports detected agent hosts where Hieronymus has not been installed yet.

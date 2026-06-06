# Hieronymus Service Toolkit

Hieronymus installs two equivalent console commands:

- `hieronymus`
- `hiero`

Every subcommand works through either command. For example, `hieronymus status` and `hiero status`
call the same CLI entry point.

Running `hiero` with no subcommand starts the local daemon if it is not already running, then prints a
short status surface:

```text
🪶 Hieronymus v0.1.0
Remembers things for you.
```

The daemon is the only normal owner of the global SQLite store. Thin CLI commands and future agent
adapters discover it through runtime files under `~/.config/hieronymus`:

- `server.json`
- `server.pid`
- `server.lock`

The daemon exposes a local HTTP JSON API on `127.0.0.1`. Human CLI output may use the Hieronymus
identity line, while automation should use `--json`.

## Commands

- `hiero status --json` shows daemon status.
- `hiero doctor --json` checks config, store, and service health.
- `hiero stop` requests graceful shutdown.
- `hiero restart` restarts the daemon.
- `hiero config` shows config paths for now; the TUI is separate work.
- `hiero admin` reports that the management TUI is separate work.
- `hiero install <app> --dry-run` shows the safe installer plan for an agent.

## Agent Install Boundary

This pass provides the installer framework and stubs. Real Claude, Codex, OpenClaw, opencode, Gemini
CLI, Pi, and Hermes integrations belong to follow-up specs based on:

`docs/superpowers/specs/2026-06-06-hieronymus-agent-workflows.md`

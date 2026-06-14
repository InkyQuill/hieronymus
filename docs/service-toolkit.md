# Hieronymus Service Toolkit

Hieronymus is alpha software: local-first, usable at your own risk. It is built
for a single-user local data root and should not be treated as a stable
networked service or multi-user database server.

Hieronymus installs two equivalent console commands:

- `hieronymus`
- `hiero`

Every subcommand works through either command. For example, `hieronymus status`
and `hiero status` call the same CLI entry point.

Running `hiero` or `hieronymus` with no subcommand starts the local daemon if it
is not already running, then prints the human status surface. The identity
portion looks like this:

```text
🪶 Hieronymus v0.2.0α
Remembers things for you.
```

## Data Root And Runtime Files

The data root is selected in this order:

- the global `--data-root <path>` CLI option;
- `HIERONYMUS_DATA_ROOT`;
- the default root, `~/.config/hieronymus`.

`--data-root` must point to a directory when it already exists. The selected
root contains the local SQLite store, configuration files, and daemon runtime
files:

- `hieronymus.sqlite`: the local SQLite database.
- `dream.conf`: dreaming provider, workflow, prompt, threshold, cap, and local
  API key configuration.
- `ingest.conf`: ingestion policy configuration.
- `release.conf`: managed update configuration.
- `server.json`: daemon discovery state, including host, port, token, PID,
  version, data root, database path, and start time.
- `server.pid`: daemon PID written with `server.json`.
- `server.lock`: advisory lock used to serialize daemon startup.

Service discovery should read runtime files and call JSON APIs. Components
should not parse person-facing CLI output.

## Service API

The daemon is the normal lifecycle and status surface for the global SQLite
store. It binds a local HTTP JSON API on `127.0.0.1` and requires the
`X-Hieronymus-Token` from `server.json`.

Current daemon endpoints are:

- `GET /health`: health check.
- `GET /status`: daemon, provider, dreaming, MCP adapter, and housekeeping
  status.
- `POST /shutdown`: graceful shutdown request.

The daemon does not expose mutation endpoints yet. The direct-store boundaries
below document the commands and adapters that still use Python domain stores
directly.

## Human And JSON Output

Human CLI output is allowed to include the Hieronymus identity line, alpha risk
language, icons, headings, and readable prose. Automation should request
machine-readable output with `--json`.

Examples:

```bash
hiero status --json
hiero doctor --json
hiero config --json
hiero admin --json
hiero session-start oso --task-type translation --json
hiero recall 1 --series oso --query "style" --source-language ja --target-language en --task-type translation --json
hiero dream --json
```

## Direct Store Boundaries

These commands intentionally access SQLite through Python domain stores. The
reason strings are part of the current boundary catalog.

- `hiero init-series`: bootstrap command that creates registry rows before a
  service mutation API exists
- `hiero propose-term`: legacy termbase helper retained for local debugging of
  deterministic terminology storage
- `hiero validate`: legacy termbase validator that reads files locally and
  checks deterministic terminology rules
- `hiero remember`: legacy long-memory helper retained until old memory
  primitives are fully retired
- `hiero session-start`: agent workflow primitive that starts local workspace
  sessions through the domain store
- `hiero session-complete`: agent workflow primitive that completes local
  workspace sessions through the domain store
- `hiero remember-short`: agent workflow primitive that writes short-term
  observations through the domain store
- `hiero recall`: agent workflow primitive that combines recall service output
  without parsing human CLI text
- `hiero feedback`: agent workflow primitive that records correction events
  through the feedback store
- `hiero dream`: maintenance command that invokes DreamService directly so
  local dreaming works without a daemon

`hieronymus-mcp` is also a direct-store boundary: stdio MCP adapter uses Python
domain stores directly because the local daemon currently exposes lifecycle and
status HTTP only; it does not parse person-facing CLI output.

MCP/storage primitives still use domain stores directly where the boundary
catalog says they do. Agent hooks and the `hieronymus_status` MCP tool report
local service discovery from configured runtime files instead of parsing human
CLI output.

## Commands

### Service

- `hiero`: starts or connects to the local daemon and prints human status.
- `hiero status`: prints human daemon and provider status.
- `hiero status --json`: emits daemon and provider status for scripts.
- `hiero stop`: requests graceful daemon shutdown.
- `hiero stop --json`: emits shutdown status for scripts.
- `hiero restart`: restarts the local daemon.
- `hiero restart --json`: emits restarted daemon status for scripts.

### Management

- `hiero config`: opens the configuration TUI for providers, dreaming
  automation, service status, paths, and diagnostics.
- `hiero config --json`: reports settings, provider status, ingest state,
  release state, and dreaming automation state.
- `hiero admin`: opens the management TUI.
- `hiero admin --json`: emits management counts and available views.
- `hiero doctor`: checks settings parseability, active provider enablement,
  provider profile API key configuration, store health, and service health.
- `hiero doctor --json`: emits configuration and service diagnostics.

### Agent and automation

- `hiero session-start <series> --task-type <type> --json`: starts a workspace
  session and emits a machine-readable payload.
- `hiero session-complete <session-id> --json`: completes a workspace session
  and emits a machine-readable payload.
- `hiero remember-short <session-id> --role user --kind correction --text <text> --json`:
  records a short-term observation and emits a machine-readable payload.
- `hiero recall <session-id> --series <series> --query <query> --source-language <src> --target-language <dst> --task-type <type> --json`:
  emits recall output for automation without parsing human text.
- `hiero feedback <crystal-id> --event helpful --role user --json`: records
  feedback and emits a machine-readable payload.

### Maintenance

- `hiero init-series <slug> --title <title> --json`: bootstraps registry rows
  for a series and emits a machine-readable payload.
- `hiero propose-term ... --json`: records a deterministic terminology proposal
  and emits a machine-readable payload.
- `hiero validate ... --json`: validates deterministic terminology rules and
  emits a machine-readable payload.
- `hiero remember ... --json`: writes a legacy long-memory entry and emits a
  machine-readable payload.
- `hiero install <app> --dry-run`: shows the safe installer plan for an agent.
- `hiero install <app>` writes plugin assets and patches supported agent host
  config.
- `hiero dream --json`: runs local dreaming and emits machine-readable status.
- `hiero update --check --json`: checks managed update state for scripts.
- `hiero update --json`: applies an available managed update and emits
  machine-readable status.

## Agent Install Boundary

This pass provides real Claude, Codex, OpenClaw, opencode, and Gemini CLI
installer wiring. Pi and Hermes are reserved detectable targets until their
host config formats are known.

See [Agent workflows](agent-workflows.md) for the host integration contract.

`hiero doctor` also reports detected agent hosts where Hieronymus has not been
installed yet.

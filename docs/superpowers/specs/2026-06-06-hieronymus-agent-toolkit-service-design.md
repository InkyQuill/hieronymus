# Hieronymus Agent Toolkit and Service

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

The agent toolkit and service layer make Hieronymus easy to install, start, inspect, and connect to
coding agents. Hieronymus should feel like a reliable local tool: one service owns memory state, while
CLI commands, TUIs, MCP adapters, and agent hooks act as thin clients.

## Core Decision

Keep the core service in Python. The project already uses Python, SQLite, uv, pytest, and MCP tooling,
and Python can support the daemon, local HTTP API, CLI, TUIs, installer planners, and agent adapters.

Node should not replace the core. A small Node or npm launcher can be added later if distribution needs
it, but it should call the Python-installed service instead of owning runtime behavior.

## Console Commands

The package exposes two equivalent console entry points:

- `hieronymus`
- `hiero`

`hiero` is a true alias for `hieronymus`. Every subcommand must work through either command:

```text
hieronymus config
hiero config
hieronymus admin
hiero admin
hieronymus install codex
hiero install codex
```

Running either command without a subcommand ensures the service is running, then shows a short
human-facing status.

## Service Shape

Hieronymus runs as one local daemon process. For service-managed operations, it owns:

- the global SQLite store
- memory providers
- strict terminology and crystal operations
- dreaming and housekeeping jobs
- MCP-facing memory operations
- runtime health state

New lifecycle commands, TUIs, agent hooks, and future adapters are thin clients. They discover the
active daemon, start it when appropriate, call the local API, and render output. Existing legacy and
debug CLI commands may still access storage directly in this pass.

The daemon exposes a local HTTP JSON API bound to `127.0.0.1`. The port can be configured or selected
automatically. The active server state is stored under `~/.config/hieronymus`, preserving the global
configuration location.

Suggested runtime files:

- `server.lock`
- `server.pid`
- `server.json`

`server.json` should include at least the daemon port, version, startup time, config path, data path,
and any local access token needed by thin clients.

## Single-Instance Behavior

Startup is idempotent:

1. Read server state from `~/.config/hieronymus`.
2. If a healthy daemon is already running, connect to it.
3. If stale state exists, clean it and retry once.
4. If another live process owns the lock, report the conflict clearly.
5. If no daemon is running, start one and write fresh state.

Only the daemon should write directly to the global store for service-managed operations. Existing
legacy and debug CLI commands may still write directly until a later pass routes them through the
daemon; TUIs and agent hooks should call the daemon.

## Presentation Contract

Hieronymus should present itself with a small, memorable identity line for human-facing commands:

```text
🪶 Hieronymus v0.1.10
Remembers things for you.
```

This is part of the CLI personality, not the internal protocol. Human output can use a limited set of
symbols such as `🪶`, `📜`, and `📖` for greetings, headings, and major status blocks. Logs, JSON
responses, tests, and automation output must stay plain and structured.

Commands that support automation should provide `--json`.

## CLI Surface

### `hieronymus` / `hiero`

Ensures the daemon is running. If it starts the daemon, show the identity line and short status. If the
daemon is already running, show identity plus current daemon status.

### `hiero status`

Shows daemon health, PID, version, data path, config path, active port, enabled providers, MCP adapter
state, and pending or last housekeeping cycle. Supports `--json`.

### `hiero stop`

Requests graceful shutdown from the daemon. If no daemon is running, report that cleanly with a
predictable result.

### `hiero restart`

Stops any running daemon, starts a fresh daemon, then runs a lightweight doctor check. Human output may
show warnings in a boxed format.

### `hiero doctor`

Checks:

- config readability and schema
- global store readability
- daemon state
- local port reachability
- provider configuration
- MCP adapter availability
- agent integration installs

Results should be separated into autofixed items, warnings, and errors requiring user action.

### `hiero config`

Opens the configuration TUI. Initial scope includes provider setup, daemon options, path display, and
provider test actions. Configuration remains global under `~/.config/hieronymus`.

### `hiero admin`

Opens the management TUI for memories, crystals, concepts, strict terms, lessons, review queues, dream
cycles, and feedback.

### `hiero help`

Shows a concise practical guide, not only generated argument-parser output.

### `hiero install <app>`

Installs Hieronymus integration assets for a supported coding agent. The installer must be
non-destructive by default: detect current state, show planned changes, patch narrowly, and verify the
result.

Initial install targets:

- `claude`
- `codex`
- `openclaw`
- `opencode`
- `gemini`

The model should leave room for later targets such as `pi` and `hermes`.

## Integration Model

Each install target has its own detector, planner, installer, verifier, and future uninstaller. The
planner produces a human-readable and machine-readable change plan before mutation.

This service/toolkit pass should implement the installer framework and safe planning surface, not the
full real integrations for every agent. Concrete per-agent integration behavior, prompt contracts,
hooks, and workflow semantics belong to the agent workflow spec and later per-agent implementation
specs.

Possible integration assets:

- MCP configuration pointing the agent at a Hieronymus adapter
- agent instructions or skills for recall, remember, read, learn, feedback, and dreaming
- startup hooks that ensure Hieronymus is running before memory-sensitive calls
- doctor checks that verify the agent can reach the daemon
- install metadata for future audit and uninstall

Installed integrations must stay thin. They must not reimplement storage, scoring, dreaming,
validation, strict terminology logic, or provider behavior.

## Installer Framework

The first tooling pass should provide the shared install framework:

- a target registry that resolves names such as `codex`, `openclaw`, `opencode`, and `gemini`
- target metadata with display name, detected paths, protocol note, and docs link
- `detect`, `plan`, `install`, `verify`, and future `uninstall` boundaries
- result kinds such as installed, already installed, planned, skipped, conflict, and failed
- `--dry-run`, `--force`, and `--json` support
- atomic writes for JSON/TOML-style config files
- backups under `~/.config/hieronymus/backups`
- confirmation before risky or ambiguous patches
- tests that use temporary fake home directories, never the real user config

Reference implementations worth learning from:

- Socraticode uses separate MCP manifests, hook manifests, skills, and agent instruction files.
- Agentmemory uses a registry of per-agent connect adapters with detection, protocol notes, dry runs,
  backups, atomic config writes, and verification.

Hieronymus should copy the shape of those patterns, not their product-specific behavior.

## Integration Scope Boundary

For this pass, `hiero install <app>` may expose targets and produce honest plans or stubs for targets
whose real integration has not been implemented yet. A stub is acceptable when it explains what will be
needed later and points to the agent workflow spec.

Real integrations are out of scope for this pass when they require:

- writing final Claude/Codex/OpenClaw/opencode/Gemini hook behavior
- installing complete skills or prompt packs
- defining per-agent lifecycle event mappings
- adding per-agent capture/recall policy
- implementing uninstall for every target
- making host-specific workarounds such as global hook fallbacks

Those belong to later specs based on `2026-06-06-hieronymus-agent-workflows.md`.

## Data Flow

1. A user runs a `hieronymus` or `hiero` command, or an installed agent hook calls Hieronymus.
2. The thin client discovers daemon state in `~/.config/hieronymus`.
3. If the daemon is needed and not healthy, the client starts it.
4. The client calls the daemon over local HTTP.
5. The daemon performs store, provider, MCP, validation, or housekeeping work.
6. The client renders human output, TUI screens, or stable JSON.

Agent integrations follow the same rule: they reach the daemon through the local API or an MCP adapter.
There is one memory authority.

## Error Handling

Common failures should be repairable and explicit:

- stale PID or lock: detect, clean, and retry once
- port occupied: choose a new port when auto-configured, or report conflict when fixed
- daemon unhealthy: report health details and suggest `hiero doctor`
- unreadable or corrupt database: fail loudly, never silently reset
- bad provider config: mark the provider unhealthy while keeping the daemon alive
- agent install conflict: show the planned patch and require confirmation unless the change is safe
- automation failure: return structured errors through `--json`

## Testing

Tests should cover:

- `hieronymus` and `hiero` expose identical subcommands
- startup is idempotent
- only one daemon instance can own the service
- stale lock recovery
- `status`, `doctor`, `stop`, and `restart` flows
- HTTP client/server contract
- config and runtime state remain under `~/.config/hieronymus`
- install planners for supported agents do not mutate real home directories in tests
- JSON output remains stable enough for automation

## Initial Non-Goals

- Do not add systemd, launchd, or Windows service integration in the first implementation.
- Do not replace the Python core with Node.
- Do not let agent integrations bypass the daemon for memory writes.
- Do not make emoji or formatted boxes part of the machine-readable API.
- Do not implement every future install target before the installer framework exists.

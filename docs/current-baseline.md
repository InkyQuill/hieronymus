# Current Baseline

This document is the structural baseline for the current Hieronymus codebase.
Execution plans and one-off implementation specs are not durable project state;
completed behavior belongs here, in the usage guides, in ADRs, and in tests.

## Product Shape

Hieronymus is an alpha local-first translation memory application. Runtime data,
configuration, daemon files, and SQLite databases live under the selected data
root, not in this source checkout and not in translation workspaces.

The current implemented surfaces are:

- Python CLI commands through `hiero` and `hieronymus`.
- A stdio MCP server through `hieronymus-mcp`.
- Generated agent integration assets and host installers.
- A local HTTP daemon for lifecycle, status, web administration, dreaming, and MCP operations.
- A React/OpenTUI management app launched by Python and running on Bun.
- Local SQLite storage with FTS5-backed recall and explicit migrations.

The alpha roadmap is closed. Future work should start from a new plan, issue, or
ADR-backed decision instead of reviving removed execution plans.

## Repository Structure

- `src/hieronymus/cli.py` owns the Click command surface and delegates to Python
  domain modules.
- `src/hieronymus/config.py` defines the data-root contract and canonical paths
  for `hieronymus.sqlite`, `provider.conf`, `dream.conf`, `ingest.conf`,
  `release.conf`, `llmcache.tmp`, backups, and generated agent assets.
- `src/hieronymus/db.py`, `migrations/`, `registry.py`, `concepts.py`,
  `crystals.py`, `rule_crystals.py`, `memory.py`, `memory_models.py`, and
  `memory_migration.py` own the SQLite-backed memory graph.
- `src/hieronymus/short_memory.py`, `workspace.py`, `agent_ingestion.py`, and
  `ingest_config.py` own session-scoped short-term memory and ingestion policy.
- `src/hieronymus/recall.py` and `scoring.py` own recall and ranking behavior.
- `src/hieronymus/dream_config.py`, `provider_config.py`,
  `dream_workflows.py`, `dream_providers.py`, `dreaming.py`,
  `dream_audit.py`, `dream_locks.py`, and `dream_autostart.py` own provider
  configuration, workflow resolution, dream cycles, audit records, and
  scheduling.
- `src/hieronymus/admin.py` and `admin_models.py` expose backend admin data and
  mutation operations.
- `src/hieronymus/tui_bridge/` is the JSON-RPC boundary used by the React/OpenTUI
  app. `ConfigBridge` and `AdminBridge` are the mutation boundaries for the
  frontend.
- `frontend/src/` contains the TypeScript React/OpenTUI application, runtime RPC
  schemas, shared test harness, config UI, and admin UI.
- `src/hieronymus/service_*.py` and `service_discovery.py` own daemon lifecycle,
  HTTP status, client calls, and runtime-file discovery.
- `src/hieronymus/mcp_server.py`, `agent_assets.py`, `agent_context.py`,
  `agent_hooks.py`, and `agent_plugins/` own MCP tools, agent workflow assets,
  hooks, and host installers.
- `src/hieronymus/release.py`, `release_config.py`, `release_guard.py`,
  `presentation.py`, `install.py`, `install.sh`, and `uninstall.sh` own managed
  install, update, release, and user-facing version presentation.
- `tests/` contains Python behavior and contract coverage. `frontend/src/**/*.test.tsx`
  and `frontend/src/**/*.test.ts` contain frontend, bridge-schema, and OpenTUI
  rendering coverage.

## Configuration Baseline

The canonical local configuration files are:

- `provider.conf`: provider endpoint profiles, defaults, API keys, timeouts, and
  provider catalog metadata.
- `dream.conf`: dreaming workflow assignments, prompts, trigger thresholds, and
  bounded-memory caps.
- `ingest.conf`: short-term memory warning and rejection thresholds plus Learn
  block splitting policy.
- `release.conf`: managed update channel state.

`settings.toml` is no longer a source configuration model. Legacy provider
payloads under `dream.conf [providers]` are migrated into `provider.conf` when
possible. Provider keys are stored as local plaintext and redacted from bridge
responses, doctor/config output, logs, provider checks, and dream audits.

Enabled dream workflows resolve their effective provider and model through
`dream.conf` plus `provider.conf` defaults. Missing enabled provider profiles
fail closed instead of silently falling back to deterministic dreaming.

## Memory And Dreaming Baseline

Series are language-neutral. Language tags, story scopes, semantic tags,
concepts, facets, short-term memories, crystals, and active rule crystals are
the current memory model.

User corrections and agent observations enter as short-term memory. Dreaming
turns completed-session memories into long-term crystals, rule crystals,
concepts, facets, semantic tags, links, supersessions, and thought memories.
Fuzzy recall and advisory crystals must not silently override active rule
crystals.

Dream cycles are bounded and auditable. They select affected short-term memories
and nearby graph context, run configured workflow phases, validate provider JSON,
apply accepted mutations, and write append-only audit records with provider,
prompt, input, output, parse-warning, affected-memory-set, and maintenance
decision context.

## CLI, Service, And MCP Baseline

Human CLI output is allowed to be readable and alpha-branded. Automation uses
`--json` and should not parse human text.

The local daemon exposes authenticated loopback HTTP endpoints using the token
from `server.json`. The stdio MCP adapter starts or reuses that daemon and
invokes its bounded internal MCP operations; it does not access domain stores
directly.

MCP exposes primitive storage, recall, dreaming, concept, facet, feedback, and
rule-crystal operations. Read, Learn, and Remember are generated agent skill
workflows because they require agent judgment.

## Agent Integration Baseline

Writable installers exist for Claude Code, Codex, OpenCode, OpenClaw, and Gemini
CLI. They render common skill/MCP/hook assets, add a host-specific manifest, and
patch host config with backups.

Xiaomi MiMo, Pi, and Hermes are detectable reserved targets. They appear in
status and doctor output, but Hieronymus does not write host configuration for
them until a safe noninteractive protocol exists.

## Local Web Console Baseline

The local web console is a Svelte application under `frontend/`, built by Vite
into `frontend/dist`. Python serves those assets from the loopback service.

The config UI provides provider CRUD and edits `dream.conf`, `ingest.conf`, and
`release.conf` through narrowly scoped HTTP APIs backed by `ConfigBridge`.

The admin UI displays local statistics and backend-owned memory views through
`AdminBridge` HTTP endpoints.

Both routes use the same local-session model and avoid terminal-renderer state.

## Release And Quality Baseline

The project is on the alpha `0.x` version line. Package metadata and
`src/hieronymus/__init__.py` currently report `0.2.0`; human-facing version text
adds the alpha marker.

Release and managed-install paths install frontend dependencies and build the
Svelte assets before packaging or reinstalling. The wheel includes
`frontend/dist`.

Before claiming implementation work complete, run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

For frontend changes, also run the relevant Bun checks from `frontend/`:

```bash
bun run --cwd frontend typecheck
bun run --cwd frontend test
bun run --cwd frontend build
```

# Hieronymus Config TUI and Dreaming Hardening

## Status

Draft follow-up spec for work remaining after the config TUI, provider integration,
and cycle-based dreaming implementation.

## Context

The config TUI and LLM dreaming branch now supports persisted settings,
provider status checks, OpenAI/Gemini/Anthropic dream providers, configured
CLI/MCP/admin dream runs, cycle-based daemon autostart, service/doctor status,
and public documentation.

The final integration review did not find release-blocking defects, but it left
two concerns that should be addressed before treating the admin configuration
surface as complete:

- The config TUI can activate, save, reload, check, and display providers, but it
  does not yet provide full in-app editing for provider fields or dreaming
  automation values.
- The daemon can run automatic dream cycles in the background while an admin or
  MCP client may also start a manual dream run. The current implementation
  should be hardened against overlapping dream-cycle work.

## Goals

- Make the configuration TUI a complete admin control surface for provider and
  dreaming settings.
- Prevent concurrent dream cycles from processing the same pending sessions or
  producing confusing run state.
- Keep secret handling explicit: API key values must remain outside settings,
  logs, JSON output, TUI detail panes, and error records.

## Non-Goals

- Do not add new provider families beyond deterministic, OpenAI, Gemini, and
  Anthropic.
- Do not store raw API key values in `settings.toml`.
- Do not replace the existing deterministic provider; it remains the offline
  fallback and test provider.

## Requirements

### Config TUI Editing

- The config TUI must allow editing these provider fields:
  - `enabled`
  - `model`
  - `api_key_env`
  - `base_url`
  - `timeout_seconds`
- The config TUI must allow editing these dreaming automation fields:
  - `active_provider`
  - `autostart_enabled`
  - `min_interval_minutes`
  - `new_short_term_memory_threshold`
  - `max_cycles_per_autostart`
- Edits must stay in memory until the admin saves them.
- Reload must discard unsaved edits and re-read `settings.toml`.
- Provider checks must use the currently edited in-memory settings where
  practical, without saving temporary secrets or raw key values.
- The TUI must clearly distinguish saved values, unsaved changes, check results,
  and validation errors.
- The TUI must validate invalid numeric settings before save and present errors
  without crashing.
- The TUI must not show raw environment variable values. It may show only the
  configured environment variable name and whether the environment value exists.

### Dream Cycle Concurrency

- Manual and automatic dream runs must be serialized per data root.
- If a dream cycle is already running, a second manual request must either:
  - return a clear "dream cycle already running" error, or
  - wait for the active cycle when the caller explicitly requests waiting.
- Automatic dreaming must skip a scheduled run when another cycle is active and
  record enough status for admins to understand why it skipped.
- Locking must be process-safe so CLI, MCP, admin TUI, and daemon paths share the
  same guard.
- Lock ownership must survive ordinary error paths and release reliably when the
  active dream run finishes.
- Stale lock handling must be conservative and documented.
- Dream run records and service status should expose whether a cycle is active,
  skipped because another cycle is active, or failed.

### Tests

- Add TUI tests for editing provider fields, editing autostart values, saving,
  reloading unsaved changes, and validation failures.
- Add tests proving raw API key values do not appear in config TUI detail text,
  config JSON, provider status JSON, doctor output, or dream error records.
- Add unit tests for the dream-cycle lock covering:
  - lock acquisition and release;
  - second acquisition failure while active;
  - release after provider exceptions;
  - stale lock behavior.
- Add integration tests showing manual CLI/MCP/admin dreaming and daemon
  autostart share the same concurrency guard.
- Add service status tests for active/skipped dream-cycle state.

## Acceptance Criteria

- An admin can configure OpenAI, Gemini, or Anthropic provider settings entirely
  from `hiero config` without editing TOML manually.
- An admin can configure cycle-based autostart settings entirely from
  `hiero config`.
- `hiero config --json`, `hiero status --json`, and `hiero doctor --json` expose
  enough provider/dreaming state for automation without leaking secrets.
- Concurrent dream-cycle attempts cannot process the same completed sessions at
  the same time.
- Full verification passes:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

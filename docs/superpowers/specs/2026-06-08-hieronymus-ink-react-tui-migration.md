# Hieronymus Ink/React TUI Migration Spec

## Goal

Replace the existing Textual-based terminal interfaces with an Ink/React TUI while
keeping the Hieronymus backend in Python.

The migration should let TypeScript own interactive terminal rendering, local UI
state, keyboard handling, dialogs, and visual composition. Python remains the
source of truth for storage, settings, MCP behavior, service lifecycle, dreaming,
rule-crystal validation, memory scoring, migrations, and domain validation.

## Non-Goals

- Do not rewrite the backend in Node.js.
- Do not port SQLite, FTS5, MCP, dreaming providers, settings, or rule-crystal
  validation logic to TypeScript.
- Do not allow the TypeScript UI to write Hieronymus SQLite tables directly.
- Do not let fuzzy memory or UI-side behavior override active rule crystals.
- Do not write tool source code into `/home/inky/Yandex.Disk/Translation`.
- Do not require a web browser or Electron. This is a terminal UI migration.

## Current State

Hieronymus exposes two interactive TUI commands:

- `hiero config`: opens the configuration TUI.
- `hiero admin`: opens the management TUI.

The current Textual UI is located under `src/hieronymus/tui/`.

Important current backend boundaries:

- `src/hieronymus/cli.py` owns command dispatch and currently launches Textual
  for non-JSON `config` and `admin`.
- `src/hieronymus/admin.py` owns `AdminStore`, admin actions, snapshots, stats,
  and detail models.
- `src/hieronymus/admin_models.py` defines admin view models such as
  `AdminRow`, `AdminDetail`, and `AdminSnapshot`.
- `src/hieronymus/tui/config_state.py` already separates much of the config form
  parsing and draft validation from Textual widgets.
- `src/hieronymus/service_http.py` already exposes a local authenticated HTTP
  API, but today it only covers service health, status, and shutdown.

This means the migration does not need to begin with a backend rewrite. It needs
a stable local JSON contract between the Python backend and the TypeScript Ink
frontend.

## Architecture Decision

Use a Python backend plus a TypeScript Ink frontend.

The Ink app is a local client. It renders screens and sends requests to Python.
Python executes all domain behavior and returns typed JSON payloads.

```text
hiero admin / hiero config
        |
        v
Python CLI launcher
        |
        v
TypeScript Ink app
        |
        v
Local JSON boundary
        |
        v
Python backend services
        |
        v
SQLite, dream.conf/settings files, MCP/service/dreaming/domain modules
```

The JSON boundary may be implemented in either of these forms:

1. Local HTTP API on the existing daemon.
2. JSON-RPC over stdio to a short-lived Python bridge process.
3. Structured `hiero ... --json` subprocess calls.

The recommended first implementation is **JSON-RPC over stdio** for the Ink app,
with a later option to move the same contract onto the existing local HTTP
daemon.

Reasoning:

- It avoids making the daemon mandatory for every UI session.
- It avoids teaching the Ink app to discover and authenticate against
  `server.json` at the start of the migration.
- It keeps the TUI process model local and predictable.
- It can reuse existing Python modules directly behind the bridge.
- The contract can later be exposed over HTTP without changing the React
  component model.

## Required Backend Contract

The TypeScript UI must call Python through a typed JSON contract. It must not
import Python internals, parse human CLI output, or access the database directly.

### Envelope

Requests:

```json
{
  "id": "1",
  "method": "admin.snapshot",
  "params": {
    "view": "Crystals",
    "selected_id": null,
    "filters": {}
  }
}
```

Successful responses:

```json
{
  "id": "1",
  "ok": true,
  "result": {}
}
```

Error responses:

```json
{
  "id": "1",
  "ok": false,
  "error": {
    "code": "validation_error",
    "message": "text must not be empty"
  }
}
```

Errors must be safe for display. Provider and dreaming errors must be redacted
using the existing Python secret-redaction logic before crossing the boundary.

### Admin Methods

Minimum admin methods needed for parity with the current management TUI:

- `admin.bootstrap`
  - Returns available views, current default view, stats, service status, and an
    initial snapshot.
- `admin.snapshot`
  - Inputs: `view`, optional `selected_id`, optional `filters`.
  - Returns: rows, selected row, detail pane payload, active filters, stats.
- `admin.filter`
  - Equivalent to `admin.snapshot` with filters, kept as a named method only if
    it simplifies the frontend state machine.
- `admin.add_crystal`
  - Inputs: `series_slug`, `source_language`, `target_language`, `type`,
    `title`, `text`, `tags`.
  - Returns: new crystal id and refreshed snapshot.
- `admin.edit_crystal`
  - Inputs: `id`, `title`, `text`.
  - Returns: action result and refreshed snapshot.
- `admin.merge_crystals`
  - Inputs: `ids`, `title`, `text`.
  - Returns: merged crystal id and refreshed snapshot.
- `admin.split_crystal`
  - Inputs: `id`, `part_one_title`, `part_one_text`, `part_two_title`,
    `part_two_text`.
  - Returns: new ids and refreshed snapshot.
- `admin.supersede_crystal`
  - Inputs: `id`, `replacement_id`.
  - Returns: action result and refreshed snapshot.
- `admin.reinforce_crystal`
  - Inputs: `id`.
  - Returns: action result and refreshed snapshot.
- `admin.decay_crystal`
  - Inputs: `id`.
  - Returns: action result and refreshed snapshot.
- `admin.deprecate_crystal`
  - Inputs: `id`.
  - Returns: action result and refreshed snapshot.
- `admin.delete_crystal`
  - Inputs: `id`, `confirmed`.
  - Returns: action result and refreshed snapshot.
- `admin.list_rule_crystals`
  - Inputs: optional `selected_id`, optional filters.
  - Returns: active and archived rule crystals with linked concepts, language
    tags, story scopes, source credibility, status, and validation scope.
- `admin.archive_rule_crystal`
  - Inputs: rule crystal id, confirmation, and evidence.
  - Returns: action result and refreshed rule crystal snapshot.
- `admin.add_correction_memory`
  - Inputs: correction text, source credibility, optional linked concept ids,
    language tags, story scopes, and semantic tags.
  - Returns: new short-term memory id and refreshed short-term status. It does
    not request immediate dreaming.
- `admin.provenance`
  - Inputs: `crystal_id`.
  - Returns: detail payload suitable for the detail pane.
- `admin.recall_reasons`
  - Inputs: `crystal_id`.
  - Returns: detail payload suitable for the detail pane.
- `admin.run_dream_all`
  - Runs successive capped dreaming cycles until no pending short-term memories
    remain.
  - Uses the same drain-all behavior as a started scheduled or urgent dreaming
    run. Each cycle still respects `max_short_term_memories_per_cycle`.
  - Returns dream run ids and refreshed `Dream Runs` snapshot, or a display-safe
    no-pending-memory result.
- `admin.dream_review`
  - Inputs: `run_id`.
  - Returns: phase-by-phase dream audit payload suitable for the detail pane or
    audit viewer.
- `admin.list_concepts`
  - Inputs: optional `selected_id`, optional filters.
  - Returns: concepts, selected concept, facets, linked crystals, semantic tags,
    and status.
- `admin.add_concept`
  - Inputs: canonical label, optional facets, semantic tags, language tags, and
    story scopes.
  - Returns: new concept id and refreshed concept snapshot.
- `admin.edit_concept`
  - Inputs: concept id, canonical label, notes, status, semantic tags, facets,
    and story scopes.
  - Returns: action result and refreshed concept snapshot.
- `admin.archive_concept`
  - Inputs: concept id and confirmation.
  - Returns: action result and refreshed concept snapshot.
- `admin.reinforce_concept`
  - Inputs: concept id.
  - Returns: concept feedback result and refreshed concept snapshot.
- `admin.rename_concept`
  - Inputs: concept id and new canonical label.
  - Returns: action result, preserving old labels as searchable facets unless
    explicitly superseded.
- `admin.link_crystal_to_concept`
  - Inputs: concept id, crystal id, optional relation type.
  - Returns: refreshed concept and crystal detail.
- `admin.unlink_crystal_from_concept`
  - Inputs: concept id, crystal id.
  - Returns: refreshed concept and crystal detail.
- `admin.remove_short_term_memory`
  - Inputs: short-term memory id and confirmation.
  - Returns: refreshed short-term memory snapshot and short-term status.

### Config Methods

Minimum config methods needed for parity with the current configuration TUI:

- `config.bootstrap`
  - Returns config paths, settings, provider rows, selected provider, form
    values, detail payload, and dreaming status.
- `config.select_provider`
  - Inputs: provider name and current draft values.
  - Returns: updated draft, provider rows, selected provider, form values, and
    detail payload.
- `config.update_draft`
  - Inputs: selected provider and edited provider/dreaming form values.
  - Returns: updated draft, validation state, provider rows, and detail payload.
- `config.save`
  - Inputs: current draft.
  - Returns: saved settings, provider rows, and detail payload.
- `config.reload`
  - Inputs: optional selected provider.
  - Returns: settings reloaded from `dream.conf`, provider rows, form values,
    and detail payload.
- `config.model_suggestions`
  - Inputs: provider profile name and current draft.
  - Returns cached model suggestions for that provider profile from
    `llmcache.tmp` when available, with fallback defaults.
- `config.test_provider_profile`
  - Inputs: provider profile name and current draft.
  - Tests connectivity for the provider profile, fetches models when the
    provider exposes a models endpoint, updates `llmcache.tmp`, and returns a
    redacted result.
- `config.check_workflow`
  - Inputs: selected workflow and current draft.
  - Returns: redacted workflow check result for the selected provider profile,
    model, prompt, and schema constraints.
- `config.dreaming_status`
  - Returns: pending short-term memory counts, active cycle state, last trigger,
    current phase, progress, consecutive `not_enough_memories` skip count, and
    last error.

Config methods must preserve these guarantees:

- Dreaming provider configuration is saved in a scoped plaintext config file at
  `~/.config/hieronymus/dream.conf`.
- `dream.conf` contains only dreaming workflow provider, trigger, prompt, and
  workflow-cap settings for now.
- Discovered provider model lists are not stored in `dream.conf`; they are
  cached in `~/.config/hieronymus/llmcache.tmp`.
- API key values are persisted as plaintext in `dream.conf`.
- API key values are not displayed after save, logged, or returned through the
  bridge except as redacted presence/status fields.
- Provider profile tests and workflow checks use edited in-memory settings, not
  only saved settings.
- Reload discards unsaved edits.
- Save validates through the existing Python settings layer before writing.

Recommended `dream.conf` shape:

```toml
[providers.anthropic-main]
type = "anthropic"
endpoint = "https://api.anthropic.com"
api_key = "plain-text-key"
timeout_seconds = 60

[providers.ollama-local]
type = "openai"
endpoint = "http://localhost:11434/v1"
api_key = ""
timeout_seconds = 120

[workflows.crystallization]
provider = "anthropic-main"
model = "claude-4-6-sonnet"

[workflows.relation_discovery]
enabled = false
provider = "ollama-local"
model = "gemma4-e3b"

[workflows.reinforcement_compaction]
provider = "anthropic-main"
model = "claude-4-5-haiku"
```

First-run defaults:

- create provider profile stubs for Anthropic, OpenAI, Gemini, and Ollama;
- do not create plaintext API keys;
- crystallization points to the Anthropic stub and remains invalid until a model
  and API key are configured;
- LLM-assisted relation discovery is disabled and points to the Ollama stub;
- reinforcement/compaction points to the Ollama stub and remains invalid until a
  model/provider endpoint is configured.

Dreaming is disabled until all required enabled provider-backed workflows are
valid. Deterministic fallback must not silently replace misconfigured dreaming
workflows in scheduled, urgent, or admin-triggered dreaming.
Disabled dreaming blocks conversion and maintenance only; recall still searches
short-term and long-term memory.

## TypeScript Frontend Structure

Recommended source layout:

```text
frontend/
  package.json
  tsconfig.json
  src/
    main.tsx
    rpc/client.ts
    rpc/schema.ts
    app/App.tsx
    app/routes.ts
    admin/AdminScreen.tsx
    admin/AdminTable.tsx
    admin/DetailPane.tsx
    admin/CommandPalette.tsx
    admin/dialogs.tsx
    config/ConfigScreen.tsx
    config/ProviderTable.tsx
    config/ConfigForm.tsx
    ui/FocusableList.tsx
    ui/StatusLine.tsx
    ui/KeyHelp.tsx
```

The exact folder name can be changed, but the separation should remain:

- `rpc/`: transport, generated or hand-maintained TypeScript types, error
  handling.
- `admin/`: management TUI state and components.
- `config/`: configuration TUI state and components.
- `ui/`: shared terminal widgets.
- `app/`: top-level routing between config/admin modes.

Use TypeScript types as a mirror of the Python JSON contract. The TypeScript UI
must validate incoming payloads at runtime with a small schema layer such as
Zod or an equivalent lightweight validator.

## Frontend Dependency Baseline

The current frontend prototype uses Ink 5 and React 18. The refactor should move
to a coherent modern baseline:

- `react` `^19.2`
- `ink` `^7`
- `ink-text-input` `^6`
- `nanostores` `^1.3`
- `@nanostores/react` `^1.1`
- `zod` `^4`
- `unicode-animations` `^1.0`
- `ink-spinner` `^5`
- `ink-select-input` `^6`

Use Nanostores for shared TUI state such as active view, selected row, draft
config, status panes, command palette state, provider profile cache state, and
dreaming progress. Keep Python authoritative for persistence and domain
validation.

Use `unicode-animations` and `ink-spinner` for subtle terminal motion:
progress, working states, phase transitions, and status pulses. Keep animations
optional and degrade cleanly when terminal rendering does not support them.

Use `ink-select-input` for menus and pickers if custom list navigation becomes
too much local code. `ink-tab` is useful conceptually, but its published peer
range does not currently include Ink 7, so do not add it unless compatibility is
verified or replaced.

Keep these dependencies out of the TUI unless a concrete implementation need
appears:

- AI SDK packages such as `ai`, `@ai-sdk/openai`, `@ai-sdk/anthropic`,
  `@ai-sdk/google`, or `@ai-sdk/openai-compatible`; provider logic remains in
  Python.
- MCP/Agent Client Protocol SDKs; the TUI talks to Python through the local JSON
  bridge.
- TOML parsers such as `@iarna/toml` or `yaml`; Python owns `dream.conf`
  parsing and writing.
- broad CLI utility packages such as `simple-git`, `glob`, `open`, `prompts`,
  `clipboardy`, `extract-zip`, or `dotenv` unless a later plan introduces a
  specific UI feature that needs them.

`~/Development/3rd/hermes-agent/ui-tui/packages/hermes-ink/` is a useful local
reference for advanced terminal behavior: alternate screen handling, scroll
boxes, raw ANSI, terminal focus/viewport hooks, selection, links, and text-input
integration. Do not depend on that private package by absolute path in
Hieronymus releases. Either stay on public Ink packages or deliberately vendor a
small, reviewed subset if a future implementation plan needs those features.

## Python Structure

Recommended backend additions:

```text
src/hieronymus/tui_bridge/
  __init__.py
  server.py
  protocol.py
  admin_api.py
  config_api.py
  errors.py
```

Responsibilities:

- `server.py`: JSON-RPC stdio loop, request parsing, response writing.
- `protocol.py`: request/response dataclasses or typed dictionaries.
- `admin_api.py`: wraps `AdminStore` and returns JSON-safe payloads.
- `config_api.py`: wraps settings, provider registry, draft handling, and secret
  redaction.
- `errors.py`: converts Python exceptions into stable display-safe error codes.

The bridge should reuse existing backend modules. It should not duplicate SQL or
settings parsing logic.

## CLI Behavior

The user-facing commands remain stable:

```bash
hiero config
hiero admin
```

After migration:

- `hiero config` launches the Ink config UI.
- `hiero admin` launches the Ink admin UI.
- `hiero config --json` continues to print machine-readable config status.
- `hiero admin --json` continues to print machine-readable admin status.

During migration, Textual and Ink can coexist behind a feature flag:

```bash
HIERONYMUS_TUI=textual hiero admin
HIERONYMUS_TUI=ink hiero admin
```

Default should remain Textual until the Ink admin and config flows pass parity
tests. After parity, default can switch to Ink. Textual should only be removed
after one release cycle with Ink as the default.

## Packaging

The Python package remains the installable application. The TypeScript frontend
is built into a JavaScript bundle during release.

Recommended packaging approach:

- `uv` continues to manage Python dependencies.
- `pnpm` manages frontend dependencies.
- `frontend/dist/` contains the built Ink CLI bundle.
- Python release tooling includes the built frontend bundle in the package.
- The Python CLI launches the bundled Node script with the correct bridge
  process information.

Node.js availability must be handled explicitly:

- If Node is required at runtime, `hiero doctor` should report whether a suitable
  Node version is available.
- If a standalone JS runtime bundle is used, document that runtime requirement.
- If the release process can package a single executable for the Ink frontend,
  prefer that for managed installs, but do not make it a prerequisite for the
  first migration.

## UX Parity Requirements

The Ink admin UI must preserve current behaviors and add the memory surfaces
required by the multilingual memory design:

- Header with the Hieronymus logo/name and current workspace context.
- View switching for Concepts, Concept Facets/Renderings, Crystals, Lessons,
  Rule Crystals, Thought Memories, Short-Term Memories, Short-Term Sessions,
  Dream Runs, Dream Audit, and Audit Log.
- Current short-term memory status pane visible on all admin pages.
  It should show pending short-term memory count, recall availability, dreaming
  enabled/disabled reason, and next scheduled eligibility.
- Current dreaming process state visible on all admin pages. It should show
  `IDLE` when no background work is active, and `Working` with current phase and
  progress while a dream cycle is running. If dreaming config is invalid, it
  should show disabled/invalid status with the failing workflow reason.
- Stats display.
- Table navigation.
- Detail pane updates when selection changes.
- Filter dialog.
- Edit dialog for crystals and lessons.
- Add, merge, split, supersede, reinforce, decay, deprecate, delete.
- Crystal list with manual decay and reinforce.
- Concept list with CRUD, rename, semantic tag editing, facet editing,
  concept-crystal link editing, reinforce feedback, archive, and linked-crystal
  navigation.
- Short-term memory list with remove/archive actions.
- Direct admin edits are allowed for metadata fixes and obvious typo cleanup.
  Semantic corrections that change meaning, rendering, applicability, or rule
  behavior should create correction memories instead of mutating long-term
  memory directly. Correction memories do not request immediate dreaming.
- Rule Crystal list with inspection, archive, and correction-memory creation.
  The admin UI must not expose a manual "promote to rule" action; rule
  activation remains a dreaming decision from clean explicit rule intent.
  Archiving an active rule crystal stops validation from using it immediately;
  replacement rules still enter as correction memories and are processed by
  dreaming.
- Manual dreaming has one admin action: Dream all. Dream all drains pending
  short-term memories through successive capped cycles and includes the final
  small batch.
- Delete confirmation before mutation.
- Provenance inspection.
- Recall reason inspection.
- Dream run list with admin-visible immutable dream audit detail.
- Dream audit detail must show malformed provider items that were rejected or
  recovered best-effort, including parse problems and confidence penalties.
- Command palette scoped by active view.
- Manual dreaming and dream output review.
- Keyboard navigation for every command. Mouse navigation should work where Ink
  and the terminal environment support it.
- Sleek terminal UX with subtle animations where possible. Follow the
  `~/Development/3rd/hermes-agent` style: short 120-200ms transitions, small
  fade/slide/pulse effects for drawers, progress, and focus changes, with no
  animation required where terminal rendering cannot support it cleanly.

The Ink config UI must preserve current behaviors and add the new dreaming
configuration surface:

- Named provider profiles for OpenAI, Anthropic, Gemini, and
  OpenAI-compatible local endpoints such as Ollama. Provider profiles hold
  reusable `type`, `endpoint`/API path where supported, plaintext `api_key`, and
  `timeout_seconds`.
- Workflow selectors for crystallization, optional LLM-assisted relation
  discovery, and reinforcement/compaction. Each workflow selects a named
  provider profile and its own model. For example: crystallization can use
  Anthropic Sonnet, relation discovery can use Ollama Gemma, and
  reinforcement/compaction can use Anthropic Haiku. LLM-assisted relation
  discovery is disabled by default and may default to the reinforcement/
  compaction provider when enabled. Deterministic fallback may remain available
  only as an explicit CLI/debug/test provider, not as a normal scheduled/admin
  dreaming fallback.
- Provider profile test/fetch button and result field. When the provider exposes
  a models endpoint, this fetches and caches model suggestions for that provider
  profile in `llmcache.tmp`.
- Workflow model editors suggest cached models from the selected provider
  profile.
- Test button and result field per provider-backed workflow. The test uses the
  workflow's selected provider profile, model, prompt, and schema constraints,
  with all secrets redacted.
- Dreaming trigger fields: `autostart_enabled`,
  `scheduled_interval_minutes`, `min_pending_short_term_memories`,
  `max_pending_short_term_memories`, `not_enough_memories_cycle_threshold`, and
  cycle/working-set caps, including max short-term memories per dreaming cycle.
  Defaults: 30 minutes, minimum 20 pending short-term memories, urgent maximum
  200 pending short-term memories, 50 short-term memories per cycle, and
  backlog escape after 5 consecutive scheduled
  `not_enough_memories` skips.
- Editable dreaming prompt editors for crystallization, optional LLM-assisted
  relation discovery, and reinforcement/compaction. Each prompt is tied to that
  workflow's selected provider. System format/schema constraints are appended
  automatically and are not user-editable. Hieronymus must ship default prompts
  for provider-backed phases, and those prompts should instruct the provider to
  write ordinary memory prose in English while preserving Japanese, Russian, and
  other non-English forms as facets, renderings, quotations, or metadata.
- Current short-term memory status pane.
- Current dreaming process state pane.
- Unsaved draft state.
- Save.
- Reload.
- Workflow check.
- Secret-safe detail panel.
- Validation errors that do not save invalid settings.

Keyboard bindings should stay as close as possible to the current Textual UI,
but view switching may need dynamic bindings once the admin view count grows:

- Admin: numeric view selection where possible, arrow navigation, `f`, `/`,
  `e`, `a`, `x`, `+`, `-`, `d`, `delete`, `p`, `ctrl+p`, `q`.
- Config: provider navigation, prompt tab navigation, `s`, `r`, `c`, `q`.

Where Ink terminal input differs from Textual behavior, document the difference
in `docs/usage.md` before switching defaults.

## Test Strategy

### Python Contract Tests

Add tests for the bridge methods before building the Ink UI:

- Request dispatch returns stable response envelopes.
- Admin bootstrap returns views, stats, and an initial snapshot.
- Admin actions mutate through `AdminStore` and return refreshed snapshots.
- Delete requires explicit confirmation.
- Config bootstrap returns provider rows and safe settings.
- Config save persists valid settings.
- Config save rejects invalid settings.
- Config workflow checks redact configured secret values.
- Config saves plaintext provider API keys only to
  `~/.config/hieronymus/dream.conf`.
- Config provider profile tests fetch and cache model suggestions when the
  provider exposes a models endpoint, storing them in `llmcache.tmp`.
- Config workflow model editors use cached provider-profile model suggestions
  or safe fallback defaults.
- `hiero doctor` reads `llmcache.tmp`, reports whether selected workflow models
  are still present when the cache knows the provider model list, and tries to
  refresh stale model lists on start. Cache entries are stale after 24 hours.
  If a stale cache refresh fails because the provider is unreachable, doctor
  reports a warning rather than failing.

Dreaming config doctor failures:

- `dream.conf` is invalid TOML or does not match the expected schema;
- a workflow references a missing provider profile;
- a workflow model is not set;
- a provider profile used by an enabled workflow cannot be reached during an
  active check;
- a required API key is missing;
- a configured API key is rejected by the provider, such as HTTP 403;
- a selected model is known not to exist when the provider can verify model
  availability. Local/OpenAI-compatible providers such as Ollama may be
  inconclusive if they cannot verify the model list.

Disabled optional workflows, such as LLM-assisted relation discovery when
`enabled = false`, do not cause doctor failures for missing provider/model/API
settings. Doctor may still report warnings for invalid configured values.
Disabled optional workflow settings are still saved in `dream.conf`, so users
can configure relation discovery while disabled and enable it later without
re-entering provider, model, or prompt settings.
- Config dreaming status returns pending short-term memory counts and active
  phase/progress.
- Config saves provider-backed dreaming prompts, per-workflow provider settings,
  and trigger fields.
- Admin concept methods cover CRUD, rename, facet editing, semantic tags, and
  concept-crystal links.
- Admin short-term memory removal requires confirmation.
- Admin dream review returns immutable phase-by-phase audit data.
- Admin dream review exposes rejected malformed items and recovered malformed
  items with confidence penalties.
- Admin correction-memory creation does not request immediate dreaming.
- Admin Dream all drains all pending short-term memories through successive
  capped cycles.
- Scheduled dreaming records `not_enough_memories` skips below
  `min_pending_short_term_memories` and processes leftovers once
  `not_enough_memories_cycle_threshold` consecutive skips have accumulated.
- Urgent dreaming caused by `max_pending_short_term_memories` still respects the
  max short-term memories per cycle cap.
- Once scheduled, urgent, backlog-escape, or manual dreaming starts, it drains
  pending short-term memories until none remain through successive capped cycles.
- Error responses use stable codes and display-safe messages.

### TypeScript Tests

Add frontend tests for:

- RPC client request/response handling.
- Runtime schema validation.
- Admin screen state transitions.
- Config draft state transitions.
- Config prompt editor tab/state transitions.
- Command palette scoping.
- Dialog submission and cancellation.
- Dreaming status pane rendering for `IDLE` and `Working`.
- Short-term status pane rendering on all admin pages.
- Short-term status pane distinguishes recall availability from dreaming
  enabled/disabled state.

### End-to-End Tests

Add a small set of scripted terminal tests after the bridge and Ink app exist:

- Launch `hiero config` with `HIERONYMUS_TUI=ink`, edit a provider, save, and
  verify `~/.config/hieronymus/dream.conf`.
- Launch `hiero admin` with seeded data, switch views, open detail, and run a
  non-destructive action.
- Confirm that destructive actions require confirmation.
- Confirm that raw API key values persist only in `dream.conf` and never appear
  in captured terminal output, logs, or bridge responses.
- Confirm concept CRUD, short-term memory removal, and dream audit inspection
  are keyboard navigable.
- Confirm Dream all drains all pending short-term memories through successive
  capped cycles, including the final small batch.
- Confirm scheduled dreaming processes a leftover short-term memory batch on the
  run after `not_enough_memories_cycle_threshold` consecutive scheduled skips.

## Migration Phases

### Phase 1: Contract First

Create the Python JSON bridge and tests. Keep Textual as the default UI.

Deliverables:

- `src/hieronymus/tui_bridge/` package.
- Contract tests for admin and config methods, including concepts, short-term
  status, dreaming status, phase prompts, trigger fields, and dream audit.
- No behavior change for `hiero admin` or `hiero config`.

### Phase 2: Ink Config UI

Build the Ink config UI against the bridge. Keep Textual available.

Deliverables:

- `frontend/` project.
- Ink config screen.
- Runtime schemas for config payloads.
- Provider selector, model suggestions, workflow test results, dreaming trigger
  fields, dreaming status pane, short-term status pane, and phase prompt editor.
- Config UI tests.
- `HIERONYMUS_TUI=ink hiero config` launches Ink.

### Phase 3: Ink Admin UI

Build the Ink admin UI against the same bridge.

Deliverables:

- Ink admin screen.
- Admin tables, detail pane, filters, dialogs, command palette, status panes,
  concept editor, short-term memory viewer, crystal actions, and dream audit
  review.
- Admin UI tests.
- `HIERONYMUS_TUI=ink hiero admin` launches Ink.

### Phase 4: Default Switch

Switch the default TUI to Ink after parity tests pass.

Deliverables:

- `hiero config` launches Ink by default.
- `hiero admin` launches Ink by default.
- `HIERONYMUS_TUI=textual` remains available for one release cycle.
- Docs updated for runtime requirements and any keyboard differences.

### Phase 5: Textual Removal

Remove Textual only after one release cycle with Ink as default.

Deliverables:

- Remove Textual dependency.
- Remove `src/hieronymus/tui/` Textual implementation or keep only compatibility
  shims if needed for historical tests.
- Update tests to target the JSON contract and Ink launch behavior.

## Acceptance Criteria

- `hiero config` and `hiero admin` retain their command names.
- Python remains the backend and source of truth.
- TypeScript never writes SQLite or settings files directly.
- Existing `--json` commands keep working.
- Current TUI workflows are available in Ink.
- Config exposes provider selection, workflow testing, model hints, dreaming
  triggers, and provider-backed dreaming prompts.
- Admin exposes concepts, crystals, short-term memories, current short-term
  status, current dreaming state, and dream audits.
- Raw secret values are persisted only in `dream.conf`; they are not displayed,
  logged, or returned through the bridge.
- All backend mutations are performed by existing Python service/domain modules.
- The full Python verification still passes:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

- Frontend verification passes once the frontend exists:

```bash
pnpm --dir frontend test
pnpm --dir frontend build
```

## Risks and Mitigations

### Risk: Contract Drift

The Python bridge and TypeScript schemas can diverge.

Mitigation: keep bridge responses covered by Python snapshot-style contract
tests and validate every frontend response at runtime.

### Risk: Packaging Complexity

Adding TypeScript introduces Node/pnpm into a Python package.

Mitigation: build the frontend during release and ship a compiled artifact.
Keep `uv` as the Python package manager and use `pnpm` only for frontend
development/builds.

### Risk: Terminal Input Differences

Ink keyboard handling may not exactly match Textual for every key sequence.

Mitigation: keep documented bindings stable where possible, test critical keys,
and document any unavoidable differences before switching defaults.

### Risk: Duplicate Business Logic

Frontend convenience code may accidentally reimplement validation or domain
rules.

Mitigation: the frontend may validate shape and required fields for UX, but
Python remains authoritative. Save/action requests must always run through
Python validation and mutation methods.

### Risk: Daemon Coupling

If the frontend relies only on HTTP, the UI may require the daemon even for
simple config edits.

Mitigation: start with stdio JSON-RPC. Reuse the same method contract over HTTP
later if daemon-backed UI behavior becomes desirable.

## Open Decisions

- Runtime strategy: require Node at runtime, package a standalone frontend
  executable, or ship a JS bundle with a documented Node requirement.
- Transport final form: stdio JSON-RPC only, HTTP only, or both sharing the same
  method handlers.
- Frontend test runner: Vitest is the likely default if `pnpm` and TypeScript are
  adopted, but the final choice should match the frontend tooling selected
  during implementation.
- Default-switch timing: one release cycle with `HIERONYMUS_TUI=ink` available
  before changing the default is recommended.

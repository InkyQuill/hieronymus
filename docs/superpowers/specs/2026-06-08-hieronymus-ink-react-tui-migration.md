# Hieronymus Ink/React TUI Migration Spec

## Goal

Replace the existing Textual-based terminal interfaces with an Ink/React TUI while
keeping the Hieronymus backend in Python.

The migration should let TypeScript own interactive terminal rendering, local UI
state, keyboard handling, dialogs, and visual composition. Python remains the
source of truth for storage, settings, MCP behavior, service lifecycle, dreaming,
strict terminology, memory scoring, migrations, and domain validation.

## Non-Goals

- Do not rewrite the backend in Node.js.
- Do not port SQLite, FTS5, MCP, dreaming providers, settings, or strict
  terminology logic to TypeScript.
- Do not allow the TypeScript UI to write Hieronymus SQLite tables directly.
- Do not let fuzzy memory or UI-side behavior override approved termbase entries.
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
SQLite, settings.toml, MCP/service/dreaming/domain modules
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
- `admin.approve_proposal`
  - Inputs: `id`.
  - Returns: action result and refreshed snapshot.
- `admin.reject_proposal`
  - Inputs: `id`.
  - Returns: action result and refreshed snapshot.
- `admin.provenance`
  - Inputs: `crystal_id`.
  - Returns: detail payload suitable for the detail pane.
- `admin.recall_reasons`
  - Inputs: `crystal_id`.
  - Returns: detail payload suitable for the detail pane.
- `admin.run_manual_dreaming`
  - Returns: dream run id and refreshed `Dream Runs` snapshot.
- `admin.dream_review`
  - Inputs: `run_id`.
  - Returns: detail payload suitable for the detail pane.

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
  - Returns: settings reloaded from `settings.toml`, provider rows, form values,
    and detail payload.
- `config.check_provider`
  - Inputs: selected provider and current draft.
  - Returns: redacted provider check result and detail payload.

Config methods must preserve current guarantees:

- API key values are never persisted.
- API key values are never displayed.
- The configured environment variable name may be displayed.
- Provider checks use edited in-memory settings, not only saved settings.
- Reload discards unsaved edits.
- Save validates through the existing Python settings layer before writing.

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

The Ink admin UI must preserve these current behaviors:

- View switching for Concepts, Renderings, Crystals, Lessons, Short-Term
  Sessions, Dream Runs, Proposals, and Audit Log.
- Stats display.
- Table navigation.
- Detail pane updates when selection changes.
- Filter dialog.
- Edit dialog for crystals and lessons.
- Add, merge, split, supersede, reinforce, decay, deprecate, delete.
- Delete confirmation before mutation.
- Proposal approve/reject.
- Provenance inspection.
- Recall reason inspection.
- Command palette scoped by active view.
- Manual dreaming and dream output review.

The Ink config UI must preserve these current behaviors:

- Provider table for deterministic, OpenAI, Gemini, and Anthropic providers.
- Active provider selection.
- Provider fields: `enabled`, `model`, `api_key_env`, `base_url`,
  `timeout_seconds`.
- Dreaming fields: `active_provider`, `autostart_enabled`,
  `min_interval_minutes`, `new_short_term_memory_threshold`,
  `max_cycles_per_autostart`.
- Unsaved draft state.
- Save.
- Reload.
- Provider check.
- Secret-safe detail panel.
- Validation errors that do not save invalid settings.

Keyboard bindings should stay as close as possible to the current Textual UI:

- Admin: `1`-`8`, `f`, `/`, `e`, `a`, `x`, `+`, `-`, `d`, `delete`, `p`,
  `ctrl+p`, `q`.
- Config: `1`-`4`, `s`, `r`, `c`, `q`.

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
- Config provider checks redact configured secret values.
- Error responses use stable codes and display-safe messages.

### TypeScript Tests

Add frontend tests for:

- RPC client request/response handling.
- Runtime schema validation.
- Admin screen state transitions.
- Config draft state transitions.
- Command palette scoping.
- Dialog submission and cancellation.

### End-to-End Tests

Add a small set of scripted terminal tests after the bridge and Ink app exist:

- Launch `hiero config` with `HIERONYMUS_TUI=ink`, edit a provider, save, and
  verify `settings.toml`.
- Launch `hiero admin` with seeded data, switch views, open detail, and run a
  non-destructive action.
- Confirm that destructive actions require confirmation.
- Confirm that raw API key values never appear in captured output.

## Migration Phases

### Phase 1: Contract First

Create the Python JSON bridge and tests. Keep Textual as the default UI.

Deliverables:

- `src/hieronymus/tui_bridge/` package.
- Contract tests for admin and config methods.
- No behavior change for `hiero admin` or `hiero config`.

### Phase 2: Ink Config UI

Build the Ink config UI against the bridge. Keep Textual available.

Deliverables:

- `frontend/` project.
- Ink config screen.
- Runtime schemas for config payloads.
- Config UI tests.
- `HIERONYMUS_TUI=ink hiero config` launches Ink.

### Phase 3: Ink Admin UI

Build the Ink admin UI against the same bridge.

Deliverables:

- Ink admin screen.
- Admin tables, detail pane, filters, dialogs, command palette, and actions.
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
- Raw secret values are not displayed, logged, persisted, or returned through the
  bridge.
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


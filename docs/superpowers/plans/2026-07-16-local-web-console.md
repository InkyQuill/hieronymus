# Local Web Console Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement task-by-task with a test cycle for every task.

**Goal:** Replace OpenTUI Config/Admin with one local Svelte web console opened by `hiero config` and `hiero admin`.

**Architecture:** The existing authenticated service HTTP server serves the packaged Svelte single-page app and a small JSON API under `/api/`. CLI commands ensure the service is running and open a loopback URL in the default browser. Config mutations are scoped to one provider or one config section; they never send or rewrite the full configuration bundle.

**Tech Stack:** Python stdlib HTTP server, Svelte 5, Vite, TypeScript, existing provider registry and configuration modules.

## Global Constraints

- Bind only to the configured loopback service host; every web request requires the existing service token.
- Provider profiles are user-created CRUD records, not a list of built-in provider types.
- Provider type is a creation/edit field; profile ID is unique and supports multiple `openai`-type profiles.
- Successful provider checks refresh that profile's cached models. Workflow model controls are a select only when a cached model list exists, otherwise text input.
- Each screen reads and saves only its own data: provider.conf, dream.conf, ingest.conf, or release.conf.
- `hiero config` and `hiero admin` open the same local server, using `/config` and `/admin` routes.

### Task 1: Package and serve the Svelte application

**Files:**
- Modify: `frontend/package.json`, `frontend/tsconfig.json`, `pyproject.toml`
- Create: `frontend/index.html`, `frontend/src/web/main.ts`, `frontend/src/web/App.svelte`
- Modify: `src/hieronymus/service_http.py`, `tests/test_service_http.py`

- [ ] Write failing service test: `GET /config` with a valid token returns the packaged HTML and unauthenticated requests return 401.
- [ ] Replace the OpenTUI Bun entry point with a Vite Svelte build into `frontend/dist`; package all produced static files.
- [ ] Extend `HieronymusRequestHandler` with safe static-file lookup, SPA fallback for `/config` and `/admin`, and no path traversal.
- [ ] Run `uv run pytest tests/test_service_http.py` and `bun run --cwd frontend build`.

### Task 2: Add narrow web configuration APIs

**Files:**
- Modify: `src/hieronymus/service_http.py`, `src/hieronymus/tui_bridge/config_api.py`
- Modify: `tests/test_service_http.py`, `tests/test_tui_bridge_config.py`

- [ ] Write failing API tests for provider list/create/update/delete/check and model cache refresh.
- [ ] Serve JSON APIs for provider CRUD, provider model discovery, workflow/dreaming config, ingest config, release config, and admin bootstrap/actions.
- [ ] Reuse `ConfigBridge` validation and `ProviderRegistry`; map API errors to 400/404 without exposing secrets.
- [ ] Run focused Python tests after each endpoint group.

### Task 3: Implement the web console shell and provider CRUD

**Files:**
- Create: `frontend/src/web/lib/api.ts`, `frontend/src/web/lib/types.ts`
- Create: `frontend/src/web/components/AppShell.svelte`, `ProviderList.svelte`, `ProviderEditor.svelte`, `ConfirmDialog.svelte`
- Create: `frontend/src/web/routes/ProvidersPage.svelte`
- Create: component tests under `frontend/src/web/**/*.test.ts`

- [ ] Write failing component tests for empty list, profile creation, edit, delete confirmation, and secret redaction.
- [ ] Implement the sidebar routes and Providers page matching the accepted dark editorial console concept.
- [ ] Implement create/edit modal: ID, display name, provider type, endpoint, API key, timeout; show test/refresh-model actions and discovered models.
- [ ] Run component tests and browser interaction tests.

### Task 4: Implement Dreaming, Ingest, Release and Admin routes

**Files:**
- Create: `frontend/src/web/routes/DreamingPage.svelte`, `IngestPage.svelte`, `ReleasePage.svelte`, `AdminPage.svelte`
- Create: respective component tests
- Modify: `frontend/src/web/App.svelte`

- [ ] Write failing tests that prove a workflow uses a model select only when its chosen provider has cached models and otherwise renders a text input.
- [ ] Implement each form with its section-scoped API save endpoint and inline validation.
- [ ] Adapt the existing Admin data/actions into the `/admin` route without spawning an OpenTUI bridge.
- [ ] Run browser tests at desktop and narrow widths.

### Task 5: Switch CLI entry points and remove OpenTUI runtime

**Files:**
- Modify: `src/hieronymus/cli.py`, `tests/test_admin_cli.py`, `tests/test_cli_opentui.py`, `tests/test_cli.py`
- Remove: `frontend/src/admin/**`, `frontend/src/config/**`, `frontend/src/test/opentuiHarness.tsx`, OpenTUI-specific dependencies

- [ ] Write failing CLI tests for `hiero config` opening `/config` and `hiero admin` opening `/admin` after ensuring the service is running.
- [ ] Use Python `webbrowser.open` with a printed URL fallback; retain `--json` behavior.
- [ ] Remove OpenTUI bridge launch and its packaged bundle dependencies.
- [ ] Run `uv run pytest`, `uv run ruff check .`, `uv run ruff format --check .`, frontend tests, production build, and a local package install smoke test.

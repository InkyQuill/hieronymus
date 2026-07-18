# Stable Daemon And Standard Web MCP Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` task by task, `superpowers:systematic-debugging` for lifecycle failures, `superpowers:test-driven-development` for behavior changes, and `superpowers:verification-before-completion` before handoff.

**Goal:** Serve the console, APIs, WebSocket events, lifecycle controls, and standard Streamable HTTP MCP from one reliable process at a stable address.

**Architecture:** A Starlette application mounts FastMCP at exactly `/mcp`. The daemon defaults to `127.0.0.1:9768`, has no authentication layer, and never falls back to an ephemeral port. Direct and proxy MCP registrations share one tool contract. `ServiceManager` owns bounded child lifecycle and diagnostics.

**Tech stack:** Python 3.12, Starlette, Uvicorn, official MCP Python SDK v1, pytest, httpx/TestClient, WebSocket test client.

**Design:** `docs/superpowers/specs/2026-07-18-remediation-and-semantic-rag-design.md`, Plan 2.

**Depends on:** Complete the correctness/data-integrity plan first.

**Worktree safety:** Do not stage `uv.lock` unless this plan intentionally changes dependencies and the lockfile delta contains only those changes. Preserve reports and unrelated untracked files.

---

## Task 1: Add Stable Service Configuration And Remove Token State

**Files:**

- Create: `src/hieronymus/service_config.py`
- Create: `tests/test_service_config.py`
- Modify: `src/hieronymus/service_state.py`
- Modify: `src/hieronymus/service_daemon.py`
- Modify: `src/hieronymus/service_client.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_service_state.py`
- Modify: `tests/test_service_client.py`
- Modify: `tests/test_cli_service.py`
- Modify: `tests/test_service_discovery.py`

- [ ] **Step 1: Write configuration precedence tests**

  Require defaults `127.0.0.1:9768`, then service config, then `HIERONYMUS_HOST`/`HIERONYMUS_PORT`, then explicit daemon arguments. Reject invalid ports and blank hosts. Do not silently coerce port `0` into an ephemeral bind.

- [ ] **Step 2: Write token-removal contracts**

  Assert `ServerState` JSON has no token, `ServiceClient` sends no token header, and `_launch_web_console()` opens a clean route without query parameters.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_service_config.py tests/test_service_state.py tests/test_service_client.py tests/test_cli_service.py -q`

- [ ] **Step 4: Implement `service.conf` loading**

  Use a small typed configuration record. Keep loopback host and port defaults centralized. Make test overrides explicit; remove `allocate_loopback_port()` and its tests.

- [ ] **Step 5: Remove token plumbing**

  Remove token generation, serialization, request headers, cookie fallback expectations, and browser query construction. Keep atomic discovery-state writes.

- [ ] **Step 6: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_service_config.py tests/test_service_state.py tests/test_service_client.py tests/test_cli_service.py tests/test_service_discovery.py -q
  uv run ruff check src/hieronymus/service_config.py src/hieronymus/service_state.py src/hieronymus/service_client.py src/hieronymus/cli.py
  ```

- [ ] **Step 7: Commit**

  ```bash
  git add src/hieronymus/service_config.py src/hieronymus/service_state.py src/hieronymus/service_daemon.py src/hieronymus/service_client.py src/hieronymus/cli.py tests/test_service_config.py tests/test_service_state.py tests/test_service_client.py tests/test_cli_service.py tests/test_service_discovery.py
  git commit -m "feat: configure a stable daemon address"
  ```

## Task 2: Build The ASGI Console And Lifecycle Application

**Files:**

- Create: `src/hieronymus/service_app.py`
- Create: `tests/test_service_app.py`
- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Modify: `src/hieronymus/service_daemon.py`
- Delete after parity: `src/hieronymus/service_http.py`
- Replace after parity: `tests/test_service_http.py`

- [ ] **Step 1: Add direct runtime dependencies**

  Add bounded direct dependencies for Starlette and Uvicorn rather than relying on MCP's transitive dependencies. Add `httpx` to the dev group if TestClient requires an explicit direct test dependency. Regenerate only the lockfile entries caused by these declarations.

- [ ] **Step 2: Port route-contract tests before implementation**

  Recreate all existing HTTP contracts against a `build_app(config, state)` ASGI factory: pages, assets, providers, settings, admin snapshots/actions, health, status, shutdown, body limits, redaction, error payloads, Host/Origin behavior, and not-found responses.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_service_app.py -q`

- [ ] **Step 4: Implement route groups with explicit dependencies**

  Use Starlette `Route`, `Mount`, and typed request helpers. Keep browser authorization limited to same-origin/Host validation; do not add tokens or sessions. Requests without `Origin` must work for non-browser clients where appropriate.

- [ ] **Step 5: Make asset roots deterministic**

  Resolve the packaged `hieronymus/frontend/dist` root only. Permit one explicit development asset-root argument/environment override used by source-checkout tests. Do not scan parent directories.

- [ ] **Step 6: Run old/new parity together**

  Keep the old module until every current service test has an ASGI equivalent. Then delete `service_http.py`, rename/replace the test module, and use `rg` to remove imports.

- [ ] **Step 7: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_service_app.py tests/test_cli_service.py tests/test_service_client.py -q
  uv run ruff check src/hieronymus/service_app.py src/hieronymus/service_daemon.py tests/test_service_app.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add pyproject.toml uv.lock src/hieronymus/service_app.py src/hieronymus/service_daemon.py src/hieronymus/service_http.py tests/test_service_app.py tests/test_service_http.py
  git commit -m "refactor: serve the console through ASGI"
  ```

## Task 3: Mount Standard Streamable HTTP MCP

**Files:**

- Create: `src/hieronymus/mcp_tools.py`
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/mcp_operations.py`
- Modify: `src/hieronymus/service_app.py`
- Modify: `src/hieronymus/service_daemon.py`
- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Create: `tests/test_mcp_http.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Bound the MCP dependency**

  Change the declaration to `mcp>=1.27,<2` and refresh the lockfile. Do not adopt v2 alpha APIs.

- [ ] **Step 2: Write HTTP protocol integration tests**

  Start the ASGI app with a temporary data root and use the official Streamable HTTP client to initialize, list tools, and call representative read/write tools. Assert the endpoint is `/mcp`, not `/mcp/mcp`.

- [ ] **Step 3: Write Origin tests**

  Accept requests without `Origin`; reject a supplied foreign Origin; accept a supplied same-origin value. Keep these tests at the mounted MCP boundary.

- [ ] **Step 4: Verify RED**

  Run: `uv run pytest tests/test_mcp_http.py -q`

- [ ] **Step 5: Separate tool contracts from transport mode**

  Extract registration into a factory that receives a backend implementing every MCP operation. A direct backend calls domain functions with the daemon's validated config. A proxy backend delegates through the compatibility bridge. Preserve all decorator descriptions, schemas, names, defaults, and result payloads.

- [ ] **Step 6: Mount FastMCP correctly**

  Configure `streamable_http_path="/"`, mount the generated application at `/mcp`, and enter its session-manager lifespan from the parent Starlette lifespan. Use stateless JSON responses unless an existing tool demonstrably requires server-to-client MCP sessions.

- [ ] **Step 7: Verify GREEN and tool parity**

  Run:

  ```bash
  uv run pytest tests/test_mcp_http.py tests/test_mcp_server.py -q
  uv run ruff check src/hieronymus/mcp_tools.py src/hieronymus/mcp_server.py src/hieronymus/service_app.py
  ```

- [ ] **Step 8: Commit**

  ```bash
  git add pyproject.toml uv.lock src/hieronymus/mcp_tools.py src/hieronymus/mcp_server.py src/hieronymus/mcp_operations.py src/hieronymus/service_app.py src/hieronymus/service_daemon.py tests/test_mcp_http.py tests/test_mcp_server.py
  git commit -m "feat: expose streamable HTTP MCP"
  ```

## Task 4: Preserve The Stdio Compatibility Shim

**Files:**

- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/daemon_mcp_client.py`
- Modify: `src/hieronymus/mcp_operations.py`
- Modify: `src/hieronymus/service_app.py`
- Modify: `tests/test_daemon_mcp_client.py`
- Modify: `tests/test_mcp_server.py`
- Modify: `tests/test_mcp_http.py`

- [ ] **Step 1: Add end-to-end stdio parity tests**

  Launch `hieronymus-mcp` as a subprocess against a temporary daemon. Initialize over stdio, compare `tools/list` with HTTP, and call representative tools through both transports.

- [ ] **Step 2: Verify RED after the tool extraction**

  Run: `uv run pytest tests/test_daemon_mcp_client.py tests/test_mcp_server.py -q`

- [ ] **Step 3: Register proxy-mode tools**

  Keep the private allow-listed `/api/mcp/<operation>` route without authentication for the compatibility release. `DaemonMcpClient` ensures the fixed-address daemon is running and calls it. Do not duplicate tool schemas or domain implementations.

- [ ] **Step 4: Mark compatibility explicitly**

  Expose stdio/HTTP mode in status diagnostics and add a removal-version note in code comments/docstrings, not a runtime warning on stdout.

- [ ] **Step 5: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_daemon_mcp_client.py tests/test_mcp_server.py tests/test_mcp_http.py -q
  uv run ruff check src/hieronymus/mcp_server.py src/hieronymus/daemon_mcp_client.py src/hieronymus/mcp_operations.py
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/hieronymus/mcp_server.py src/hieronymus/daemon_mcp_client.py src/hieronymus/mcp_operations.py src/hieronymus/service_app.py tests/test_daemon_mcp_client.py tests/test_mcp_server.py tests/test_mcp_http.py
  git commit -m "feat: retain stdio MCP compatibility"
  ```

## Task 5: Repair Process Lifecycle And Diagnostics

**Files:**

- Create: `src/hieronymus/service_logging.py`
- Modify: `src/hieronymus/service_manager.py`
- Modify: `src/hieronymus/service_daemon.py`
- Modify: `src/hieronymus/service_state.py`
- Modify: `tests/test_service_manager.py`
- Modify: `tests/test_service_daemon.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Add child-exit, timeout, and conflict regressions**

  Cover: occupied port; child exits before state; child publishes state but never becomes healthy; startup timeout; process ignores TERM; state belongs to another PID; stop endpoint returns but process remains; restart after failed stop.

- [ ] **Step 2: Add signal and bounded-join tests**

  Run the daemon subprocess, send SIGTERM, and assert state removal plus process exit. Use a fake stuck scheduler to prove shutdown has a deadline.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_service_manager.py tests/test_service_daemon.py tests/test_cli_service.py -q`

- [ ] **Step 4: Implement bounded logging**

  Create a daemon log under the data root. Rotate by size/count with standard-library logging. Preserve early startup stderr by directing it to the same file until logging initializes. Include the path in startup errors.

- [ ] **Step 5: Own and terminate the child**

  Retain `Popen`, poll for early exit, and on failure terminate the process group, wait, then kill within explicit deadlines. Remove discovery state only when PID/state identity matches.

- [ ] **Step 6: Verify stop before restart**

  Poll health/PID after shutdown. Escalate to signals only for the matching process. Return explicit stopped/forced/failed status. Abort restart on failed shutdown.

- [ ] **Step 7: Unify daemon shutdown**

  Uvicorn termination, SIGINT, SIGTERM, and HTTP shutdown trigger one idempotent shutdown coordinator. All worker/scheduler joins use timeouts and log overrun details.

- [ ] **Step 8: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_service_manager.py tests/test_service_daemon.py tests/test_cli_service.py -q
  uv run ruff check src/hieronymus/service_logging.py src/hieronymus/service_manager.py src/hieronymus/service_daemon.py
  ```

- [ ] **Step 9: Commit**

  ```bash
  git add src/hieronymus/service_logging.py src/hieronymus/service_manager.py src/hieronymus/service_daemon.py src/hieronymus/service_state.py tests/test_service_manager.py tests/test_service_daemon.py tests/test_cli_service.py
  git commit -m "fix: bound daemon lifecycle and diagnostics"
  ```

## Task 6: Replace WebSocket Polling With Events

**Files:**

- Modify: `src/hieronymus/service_app.py`
- Modify: `src/hieronymus/daemon_events.py`
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/tui_bridge/admin_api.py`
- Modify: `tests/test_service_app.py`
- Modify: `tests/test_daemon_events.py`
- Modify: `tests/test_dreaming.py`

- [ ] **Step 1: Add real WebSocket tests**

  Assert connect/disconnect, multiple subscribers, slow/dead subscriber isolation, manual dream start/phase/completion/failure events, and secret redaction.

- [ ] **Step 2: Verify RED**

  Run: `uv run pytest tests/test_service_app.py tests/test_daemon_events.py tests/test_dreaming.py -q`

- [ ] **Step 3: Use Starlette WebSocket primitives**

  Replace manual frame logic with accept/send_json/disconnect handling and bounded subscriber queues.

- [ ] **Step 4: Publish phase events at the source**

  Add an optional event sink/callback at the dream orchestration boundary. Publish when phases transition rather than querying `dream_phase_runs` every 200 ms. Keep non-daemon callers working with a no-op sink.

- [ ] **Step 5: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_service_app.py tests/test_daemon_events.py tests/test_dreaming.py -q
  uv run ruff check src/hieronymus/service_app.py src/hieronymus/daemon_events.py src/hieronymus/dreaming.py
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/hieronymus/service_app.py src/hieronymus/daemon_events.py src/hieronymus/dreaming.py src/hieronymus/tui_bridge/admin_api.py tests/test_service_app.py tests/test_daemon_events.py tests/test_dreaming.py
  git commit -m "refactor: publish daemon progress events"
  ```

## Task 7: Update Agent Installers By Capability

**Files:**

- Modify: `src/hieronymus/agent_plugins/base.py`
- Modify: `src/hieronymus/agent_plugins/codex.py`
- Modify: `src/hieronymus/agent_plugins/claude.py`
- Modify: `src/hieronymus/agent_plugins/gemini.py`
- Modify: `src/hieronymus/agent_plugins/opencode.py`
- Modify: `src/hieronymus/agent_plugins/openclaw.py`
- Modify: `src/hieronymus/agent_assets.py`
- Modify: `tests/test_agent_plugin_installers.py`
- Modify: `tests/test_agent_plugins.py`
- Modify: `tests/test_agent_assets.py`
- Modify: `tests/test_cli_agent_install.py`

- [ ] **Step 1: Verify each client's current official config format**

  Record in test names whether the adapter supports Streamable HTTP URLs. Do not infer a common schema. Clients lacking confirmed support remain command-based.

- [ ] **Step 2: Write exact generated-config tests**

  HTTP-capable clients receive `http://127.0.0.1:9768/mcp` using their native schema. Compatibility clients retain `hieronymus-mcp`. Reinstall/update tests must migrate only Hieronymus-owned entries and preserve user fields.

- [ ] **Step 3: Verify RED**

  Run: `uv run pytest tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_agent_assets.py tests/test_cli_agent_install.py -q`

- [ ] **Step 4: Implement an explicit transport capability**

  Put the decision in each adapter or a typed capability record. Do not spread client-name conditionals through the installer.

- [ ] **Step 5: Verify GREEN**

  Run:

  ```bash
  uv run pytest tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_agent_assets.py tests/test_cli_agent_install.py -q
  uv run ruff check src/hieronymus/agent_plugins src/hieronymus/agent_assets.py
  ```

- [ ] **Step 6: Commit**

  ```bash
  git add src/hieronymus/agent_plugins src/hieronymus/agent_assets.py tests/test_agent_plugin_installers.py tests/test_agent_plugins.py tests/test_agent_assets.py tests/test_cli_agent_install.py
  git commit -m "feat: install web MCP where supported"
  ```

## Task 8: Installed-Package And Live-Service Verification

**Files:**

- Modify only for in-scope failures.

- [ ] **Step 1: Run the project verification chain**

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  bun run --cwd frontend format
  bun run --cwd frontend typecheck
  bun run --cwd frontend test
  bun run --cwd frontend build
  uv build
  ```

- [ ] **Step 2: Test the built wheel in isolation**

  Install the wheel into a temporary environment. Start the installed `hiero` daemon with a temporary data root and explicit test port. Probe pages/assets, `/health`, `/status`, `/mcp`, WebSocket events, and stdio compatibility. Stop it and verify no process or state remains.

- [ ] **Step 3: Test fixed-port conflict behavior**

  Hold the configured socket open, run `hiero start`, and verify prompt failure with the address and daemon log path. Confirm no alternate state file is published.

- [ ] **Step 4: Inspect final boundaries**

  Use `rg` to confirm there is no service token/cookie/query path, no `ThreadingHTTPServer`, no manual WebSocket handshake, no port-zero allocation, and no parent-directory asset scan.

- [ ] **Step 5: Commit only necessary verification fixes**

  Do not create an empty commit. Leave durable documentation updates for Plan 5.

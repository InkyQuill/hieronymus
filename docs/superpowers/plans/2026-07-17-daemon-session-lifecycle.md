# Daemon Session Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task. Steps use checkbox syntax.

**Goal:** Close abandoned sessions, make threshold/manual Dreaming observable and controllable in the local web admin, and secure tokenless browser access by Origin.

**Architecture:** The existing daemon scheduler gains a lifecycle coordinator that completes stale sessions and starts threshold-eligible Dreaming. A thread-safe in-process event hub publishes worker progress through one same-origin WebSocket; Svelte refreshes state after each event and reconnect.

**Tech Stack:** Python 3.12, SQLite, `ThreadingHTTPServer`, Svelte 5, TypeScript, pytest, Bun.

## Global Constraints

- Keep one loopback daemon and its one address; do not add a process.
- Idle timeout: 30 minutes; lifecycle cadence: 60 seconds.
- Reuse `min_pending_short_term_memories`; manual Dreaming bypasses it.
- Repeated manual start returns HTTP 200, existing run, and `started: false`.
- Tokenless browser API/WS access requires exact local Origin. MCP, `/status`, `/health`, and `/shutdown` retain daemon-token checks.

### Task 1: Track session activity and expire sessions

**Files:** modify `src/hieronymus/db.py`, `src/hieronymus/migrations/global.sql`, `src/hieronymus/workspace.py`; test `tests/test_workspace.py`.

**Interfaces:** `WorkspaceStore.complete_inactive_sessions(cutoff: datetime) -> tuple[int, ...]`; `complete_session(session_id: int) -> bool` returns `False` when already complete.

- [ ] Write red tests proving a short-memory write updates `task_sessions.last_activity_at`, stale active sessions alone are completed, and repeated completion is a no-op.
- [ ] Run `uv run pytest tests/test_workspace.py -k 'activity or inactive or complete' -q`; expect failure for missing column/method.
- [ ] Add the compatibility `last_activity_at` column, initialise it on start, update it in the memory-write transaction, and atomically complete only `status = 'active'` rows older than the cutoff.
- [ ] Re-run the focused tests; expect PASS.
- [ ] Commit: `feat: track and expire inactive sessions`.

### Task 2: Coordinate closure, threshold Dreaming, and worker events

**Files:** create `src/hieronymus/daemon_events.py`, `src/hieronymus/session_lifecycle.py`; modify `src/hieronymus/service_daemon.py`, `src/hieronymus/dream_autostart.py`, `src/hieronymus/admin.py`, `src/hieronymus/tui_bridge/admin_api.py`; test `tests/test_service_daemon.py`, `tests/test_dream_autostart.py`, `tests/test_admin_actions.py`, `tests/test_tui_bridge_admin.py`.

**Interfaces:**

```python
class AdminEventHub:
    def publish(self, event_type: str, payload: dict[str, object]) -> None: ...

class SessionLifecycle:
    def run_due(self, now: datetime) -> tuple[int, ...]: ...

def run_threshold_now(self) -> DreamRunRecord | None: ...
```

- [ ] Write red tests: stale closure invokes threshold launch; fresh sessions are retained; manual close uses the same threshold check; a duplicate manual request returns the active run with `started is False`; emitted events include `session_closed`, `dream_started`, each phase progress event, and completion/failure.
- [ ] Run `uv run pytest tests/test_service_daemon.py tests/test_dream_autostart.py tests/test_admin_actions.py -k 'inactive or threshold or manual_dream or event' -q`; expect failure.
- [ ] Implement the event hub, lifecycle call before the existing scheduler run, and `DreamAutostart.run_threshold_now()`. Reuse the existing dream lock. Run manual Dreaming in one tracked worker thread; return immediately, publish safe progress/completion/failure payloads, and never create a second run.
- [ ] Add `AdminStore.close_session`/`AdminBridge.close_session`; refresh `Short-Term Sessions`, return a clear already-completed response, and call the same lifecycle threshold path.
- [ ] Re-run the four suites; expect PASS.
- [ ] Commit: `feat: automate session closure and dream runs`.

### Task 3: Serve trusted local browser routes and WebSocket events

**Files:** modify `src/hieronymus/service_http.py`, `src/hieronymus/service_daemon.py`; test `tests/test_service_http.py`, `tests/test_service_daemon.py`.

**Interfaces:** `HieronymusRequestHandler._is_local_browser_origin() -> bool`; `GET /ws/admin`; browser actions `close_session` and `run_manual_dreaming` in `_ADMIN_ACTION_METHODS`.

- [ ] Write red HTTP tests: `GET /api/admin/dashboard` with the exact daemon Origin succeeds without token; absent/mismatched Origin receives 403; internal MCP retains 401 without token; a hostile-origin WebSocket upgrade is rejected; a valid origin receives event frames.
- [ ] Run `uv run pytest tests/test_service_http.py -k 'origin or websocket or token' -q`; expect failure.
- [ ] Remove query/cookie token flow for static navigation/assets. Require exact scheme/host/port Origin for browser API and `/ws/admin`; do not emit permissive CORS headers. Keep token checks for internal routes. Implement RFC-6455 handshake and server-to-client JSON text frames backed by `AdminEventHub`, removing disconnected subscribers safely.
- [ ] Re-run `uv run pytest tests/test_service_http.py tests/test_service_daemon.py -q`; expect PASS.
- [ ] Commit: `feat: stream secured local admin events`.

### Task 4: Add web controls and live refresh

**Files:** create `frontend/src/web/lib/admin-events.svelte.ts`; modify `frontend/src/web/lib/api.ts`, `frontend/src/web/lib/types.ts`, `frontend/src/web/App.svelte`, `frontend/src/web/components/AdminDashboard.svelte`, `frontend/src/web/components/DreamingEditor.svelte`, `frontend/src/web/components/MemoryViews.svelte`, `frontend/src/web/components.css`; test `frontend/src/web/lib/admin-events.svelte.test.ts` and component tests.

**Interfaces:** `startAdminDreaming()` returns `{ started, run_id, cycle_id, status }`; `closeAdminSession(id)` returns `AdminActionResult`; `createAdminEvents(onEvent)` owns one reconnecting `/ws/admin` client.

- [ ] Write red tests for reconnect-triggered refresh, visible phase counter, disabled duplicate run button, successful follow-up toast, and confirmation-gated close for active sessions only.
- [ ] Run `bun test --cwd frontend`; expect failure.
- [ ] Add API wrappers/types and a single event client in `App.svelte`. On event or successful reconnect reload dashboard and current snapshot. Put `Run Dreaming now` in Overview and Dreaming settings; show immediate and completion/failure toasts. Add `Close session` to active Short-Term Sessions and remove the existing incorrect session-to-memory deletion action. Render a non-blocking reconnect indicator and phase progress.
- [ ] Run `npx @sveltejs/mcp svelte-autofixer frontend/src/web/App.svelte --svelte-version 5`, repeat for the three changed components, then `bun test --cwd frontend`; expect no findings and PASS.
- [ ] Commit: `feat: control and monitor dreaming from web admin`.

### Task 5: Verify and hand off

**Files:** tests changed above; add browser-level regression to `tests/test_service_http.py` if the existing harness permits it.

- [ ] Add a regression for no-token same-origin admin, rejected foreign-origin mutation/WS, manual run progress, and final dashboard refresh.
- [ ] Run `uv run pytest`, `uv run ruff check .`, `uv run ruff format --check .`, `bun test --cwd frontend`, `bun run --cwd frontend build`, and `uv build`; every command must exit 0.
- [ ] Run `uv tool install --reinstall .`, `hiero`, and `hiero status`; manually verify closing a session, manual Dreaming, phase updates, and reconnect refresh.
- [ ] Commit test-only changes, if any: `test: cover local admin dream lifecycle`.

## Plan self-review

- Tasks 1–2 cover timeout closure, manual closure, threshold launch, worker safety, and event production.
- Task 3 implements the reviewed Origin boundary without weakening internal token routes.
- Task 4 covers all requested UI controls, WebSocket updates, progress, and reconnect semantics.
- Task 5 covers the stated verification commands and local smoke test.

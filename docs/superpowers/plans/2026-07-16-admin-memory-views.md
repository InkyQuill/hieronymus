# Admin Memory Views Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make memory administration a dedicated, action-oriented web page for browsing, inspecting, and safely curating stored memory.

**Architecture:** Keep `/admin` as the health overview and move snapshot state to `/admin/memory`. The HTTP service exposes an explicit allow-list of existing `AdminBridge` mutations; the Svelte page shows only actions meaningful for the selected view and record, then refreshes the snapshot and invokes the shared toast callback.

**Tech Stack:** Python 3.12, stdlib `http.server`, `AdminBridge`, Svelte 5, TypeScript, Vite, pytest.

## Global Constraints

- Do not expose arbitrary `AdminBridge` method names through HTTP; maintain an explicit action map.
- The table is for finding and selecting records; full content and mutations belong in the detail panel.
- Offer only applicable actions and require confirmation for destructive actions.
- Preserve local-only authentication, JSON error shape, and deterministic terminology behavior.

---

## File Structure

- `src/hieronymus/service_http.py`: bounded admin mutation endpoint and stable JSON errors.
- `tests/test_service_http.py`: memory route and action dispatch tests.
- `frontend/src/web/lib/api.ts` and `types.ts`: typed snapshot-selection and action calls.
- `frontend/src/web/App.svelte`: route and navigation.
- `frontend/src/web/components/AdminDashboard.svelte`: overview only.
- `frontend/src/web/components/MemoryViews.svelte`: memory workspace.
- `frontend/src/web/styles.css`: compact browse/detail layout.

### Task 1: Expose supported memory actions over local HTTP

**Files:**
- Modify: `src/hieronymus/service_http.py`
- Test: `tests/test_service_http.py`

**Interfaces:**
- Consumes: `AdminBridge` methods `reinforce_crystal`, `decay_crystal`, `deprecate_crystal`, `delete_crystal`, `approve_proposal`, `reject_proposal`, `reinforce_concept`, `decay_concept`, `archive_concept`, and `remove_short_term_memory`.
- Produces: `POST /api/admin/actions/<action>` accepting one JSON object and returning bridge JSON or a JSON 400/404 error.

- [ ] **Step 1: Write failing HTTP tests**

```python
def test_admin_memory_route_and_actions(tmp_path: Path) -> None:
    with _running_service(tmp_path, admin_bridge=bridge) as base_url:
        assert _request(f"{base_url}/admin/memory").status == 200
        assert _request_json(
            f"{base_url}/api/admin/actions/reinforce_crystal",
            method="POST", body={"id": "crystal-1"},
        ) == {"ok": True, "id": "crystal-1"}
        assert _request_json(
            f"{base_url}/api/admin/actions/not_a_method", method="POST", body={},
        ).status == 404
```

- [ ] **Step 2: Run the focused test**

Run: `uv run pytest tests/test_service_http.py -k admin_memory_route_and_actions -v`

Expected: FAIL because the POST route and allow-list do not exist.

- [ ] **Step 3: Implement an explicit dispatcher**

```python
_ADMIN_ACTION_METHODS = {
    "reinforce_crystal": "reinforce_crystal", "decay_crystal": "decay_crystal",
    "deprecate_crystal": "deprecate_crystal", "delete_crystal": "delete_crystal",
    "approve_proposal": "approve_proposal", "reject_proposal": "reject_proposal",
    "reinforce_concept": "reinforce_concept", "decay_concept": "decay_concept",
    "archive_concept": "archive_concept", "remove_short_term_memory": "remove_short_term_memory",
}
```

Resolve the path only through this map, reject unknown actions with `404 {"error": "unknown_admin_action"}`, reject invalid bridge parameters with a JSON 400, and return the bridge result unchanged on success.

- [ ] **Step 4: Run focused endpoint tests**

Run: `uv run pytest tests/test_service_http.py -k admin_memory_route_and_actions -v`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/service_http.py tests/test_service_http.py && git commit -m "feat: expose bounded admin memory actions"`

### Task 2: Create the dedicated memory workspace

**Files:**
- Create: `frontend/src/web/components/MemoryViews.svelte`
- Modify: `frontend/src/web/App.svelte`
- Modify: `frontend/src/web/components/AdminDashboard.svelte`
- Modify: `frontend/src/web/lib/api.ts`
- Modify: `frontend/src/web/lib/types.ts`
- Modify: `frontend/src/web/styles.css`

**Interfaces:**
- Consumes: `GET /api/admin/dashboard`, `GET /api/admin/snapshot?view=<view>&selected_id=<id>`, and Task 1 action endpoint.
- Produces: `MemoryViews` with `dashboard` and `onNotice({ kind, message })` props.

- [ ] **Step 1: Add typed API boundaries**

```ts
export type AdminRow = { id: string; label: string; status?: string; kind?: string; scope?: string; confidence?: number; strength?: number };
export async function loadAdminSnapshot(view: string, selectedId?: string): Promise<AdminSnapshot> { /* URLSearchParams */ }
export async function runAdminAction(action: string, params: { id: string }): Promise<{ ok: boolean; message?: string }> { /* POST JSON */ }
```

- [ ] **Step 2: Implement browse, inspect, and curate UI**

`MemoryViews.svelte` owns selected view, selected row, snapshot, loading, mutation state, and pending confirmation. Render a view selector, a compact clickable record table, and a detail pane that shows the full selected record. Use exactly these contextual actions:

```ts
const actionByView = {
  Crystals: ["reinforce_crystal", "decay_crystal", "deprecate_crystal", "delete_crystal"],
  Proposals: ["approve_proposal", "reject_proposal"],
  Concepts: ["reinforce_concept", "decay_concept", "archive_concept"],
  "Short-Term Sessions": ["remove_short_term_memory"],
} as const;
```

Make reinforce/decay and approve/reject immediately available. Put deprecate/delete/archive/remove behind an in-page confirmation that names the selected record. Do not render disabled controls or actions for unsupported views.

- [ ] **Step 3: Route and simplify overview**

Resolve `/admin/memory` before generic `/admin` in `App.svelte`, render `MemoryViews`, and show separate `Overview` and `Memory views` sidebar links. Remove `onMount`, snapshot state, and snapshot API calls from `AdminDashboard.svelte`; retain metrics and service status with a link labelled `Open memory views`.

- [ ] **Step 4: Validate Svelte components and build**

Run: `npx @sveltejs/mcp svelte-autofixer frontend/src/web/App.svelte`

Run: `npx @sveltejs/mcp svelte-autofixer frontend/src/web/components/AdminDashboard.svelte`

Run: `npx @sveltejs/mcp svelte-autofixer frontend/src/web/components/MemoryViews.svelte`

Run: `bun run --cwd frontend typecheck && bun run --cwd frontend test && bun run --cwd frontend build`

Expected: every command exits 0.

- [ ] **Step 5: Commit**

Run: `git add frontend/src/web && git commit -m "feat: add dedicated memory administration page"`

### Task 3: Build, install, and verify the whole local flow

**Files:**
- Modify: `docs/superpowers/plans/2026-07-16-admin-memory-views.md` (check tasks after evidence only)

- [ ] **Step 1: Run project checks**

Run: `uv run pytest && uv run ruff check . && uv run ruff format --check .`

Expected: every command exits 0.

- [ ] **Step 2: Build and test-install**

Run: `uv build && uv tool install --reinstall dist/hieronymus-0.1.0-py3-none-any.whl`

Expected: wheel build and installation exit 0.

- [ ] **Step 3: Restart and probe the actual service**

Run: `hiero restart --json`

Run: `curl --fail --silent http://127.0.0.1:<port>/api/admin/dashboard >/dev/null && curl --fail --silent 'http://127.0.0.1:<port>/api/admin/snapshot?view=Crystals' >/dev/null`

Expected: the service and both endpoints return successfully.

- [ ] **Step 4: Commit and push**

Run: `git add docs/superpowers/plans/2026-07-16-admin-memory-views.md && git commit -m "docs: complete admin memory views plan" && git push origin main`

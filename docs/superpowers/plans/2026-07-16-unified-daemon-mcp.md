# Unified Daemon MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every Hieronymus MCP tool through the one authenticated local daemon that also serves config/admin and runs dreaming.

**Architecture:** Extract the current domain logic and payload serializers from `mcp_server.py` into daemon-owned MCP operations. The stdio MCP process becomes a thin `DaemonMcpClient` wrapper: it ensures the daemon is running, uses `server.json` credentials, and invokes an allow-listed `POST /api/mcp/<operation>` route. The external MCP tool names, parameters, and results remain unchanged.

**Tech Stack:** Python 3.12, FastMCP stdio, stdlib HTTP server/client, SQLite, pytest.

## Global Constraints

- Bind the daemon only to `127.0.0.1` and require its existing token for every MCP RPC request.
- The MCP process must never fall back to direct SQLite/domain-store calls.
- Use an explicit operation registry; reject an unknown operation with `404 {"error": "unknown_mcp_operation"}`.
- Preserve every currently registered MCP tool name, parameter name, result payload, and deterministic terminology rule.
- `DaemonMcpClient` must call `ServiceManager.ensure_running()` before reading daemon state.
- `doctor` remains read-only by default and must not describe a health check as an autofix.

---

## File Structure

- Create `src/hieronymus/mcp_operations.py`: daemon-owned `McpOperations` class, shared serializers, and explicit operation registry.
- Create `src/hieronymus/daemon_mcp_client.py`: lifecycle-aware authenticated RPC client used only by the stdio MCP adapter.
- Modify `src/hieronymus/service_http.py`: dispatch `/api/mcp/<operation>` through the registry.
- Modify `src/hieronymus/service_client.py`: expose a reusable authenticated JSON request primitive for daemon clients.
- Modify `src/hieronymus/mcp_server.py`: retain FastMCP decorators/signatures but delegate each tool to `DaemonMcpClient`.
- Modify `src/hieronymus/doctor.py`: report daemon status as informational status instead of autofix.
- Modify `src/hieronymus/cli_boundaries.py` and `docs/current-baseline.md`: remove the direct-store MCP boundary claim.
- Add `tests/test_daemon_mcp_client.py` and `tests/test_mcp_operations.py`; update `tests/test_service_http.py`, `tests/test_mcp_server.py`, `tests/test_mcp_memory_primitives.py`, `tests/test_mcp_rag.py`, and `tests/test_cli_boundaries.py`.

### Task 1: Build the authenticated MCP RPC transport

**Files:**
- Create: `src/hieronymus/daemon_mcp_client.py`
- Modify: `src/hieronymus/service_client.py`
- Modify: `src/hieronymus/service_http.py`
- Create: `tests/test_daemon_mcp_client.py`
- Modify: `tests/test_service_http.py`

**Interfaces:**
- Produces `DaemonMcpClient(config, manager=None, client=None).invoke(operation: str, params: dict[str, object]) -> dict[str, object]`.
- Produces `POST /api/mcp/<operation>` with a JSON object body.
- Consumes `ServiceManager.ensure_running()`, `read_server_state(config)`, and `ServiceClient.request_json("POST", state, path, payload)`.

- [ ] **Step 1: Write failing client and route tests**

```python
def test_daemon_mcp_client_starts_service_then_posts_operation(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "data")
    manager = FakeManager(state=_state(config))
    client = FakeServiceClient({"result": {"slug": "oso"}})
    result = DaemonMcpClient(config, manager=manager, client=client).invoke(
        "series_create", {"slug": "oso", "title": "Only Sense Online"}
    )
    assert result == {"result": {"slug": "oso"}}
    assert manager.ensure_calls == 1
    assert client.calls == [("POST", "/api/mcp/series_create", {"slug": "oso", "title": "Only Sense Online"})]

def test_mcp_route_rejects_unknown_operation(tmp_path: Path) -> None:
    with _served(tmp_path) as base_url:
        response = _post_error(f"{base_url}/api/mcp/not-real", {})
    assert response.status == 404
    assert response.payload == {"error": "unknown_mcp_operation"}
```

- [ ] **Step 2: Verify the tests are red**

Run: `uv run pytest tests/test_daemon_mcp_client.py tests/test_service_http.py -k 'daemon_mcp or mcp_route' -v`

Expected: import failure for `DaemonMcpClient` and 404 route failure.

- [ ] **Step 3: Implement the client and bounded route**

```python
class DaemonMcpClient:
    def invoke(self, operation: str, params: dict[str, object]) -> dict[str, object]:
        self.manager.ensure_running()
        state = read_server_state(self.config)
        if state is None:
            raise RuntimeError("Hieronymus daemon did not publish state")
        return self.client.request_json("POST", state, f"/api/mcp/{operation}", params)
```

Add `ServiceClient.request_json()` with JSON encoding, token header, and `ServiceClientError` for HTTP/JSON failures. In the HTTP handler authenticate first, use `MCP_OPERATION_HANDLERS.get(operation)`, return `unknown_mcp_operation` for an absent key, and call the handler with `(config, params)`.

- [ ] **Step 4: Verify green**

Run: `uv run pytest tests/test_daemon_mcp_client.py tests/test_service_http.py -k 'daemon_mcp or mcp_route' -v`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/daemon_mcp_client.py src/hieronymus/service_client.py src/hieronymus/service_http.py tests/test_daemon_mcp_client.py tests/test_service_http.py && git commit -m "feat: add authenticated daemon MCP transport"`

### Task 2: Move shared MCP domain operations into the daemon

**Files:**
- Create: `src/hieronymus/mcp_operations.py`
- Test: `tests/test_mcp_operations.py`
- Modify: `src/hieronymus/service_http.py`

**Interfaces:**
- Produces `McpOperations(config).invoke(operation: str, params: dict[str, object]) -> dict[str, object] | list[dict[str, object]]`.
- Produces `MCP_OPERATION_HANDLERS: Mapping[str, Callable[[HieronymusConfig, dict[str, object]], dict[str, object] | list[dict[str, object]]]]`.
- Consumes the existing serializers and domain services currently called by `mcp_server.py`.

- [ ] **Step 1: Write operation parity tests**

```python
def test_operations_preserve_series_session_and_recall_payloads(config: HieronymusConfig) -> None:
    operations = McpOperations(config)
    series = operations.invoke("series_create", {"slug": "oso", "title": "Only Sense Online"})
    session = operations.invoke("session_start", {"series_slug": "oso"})
    recalled = operations.invoke("recall", {"session_id": session["session_id"], "series_slug": "oso", "query": "style"})
    assert series["slug"] == "oso"
    assert session["session_id"] > 0
    assert set(recalled) == {"crystals", "short_term_memories", "rag_hits"}
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_mcp_operations.py -v`

Expected: FAIL because `mcp_operations` does not exist.

- [ ] **Step 3: Extract logic without changing contracts**

Move `_crystal_payload`, `_concept_payload`, `_facet_payload`, `_short_term_memory_payload`, `_recall_payload`, `_series_payload`, `_recent_dream_audit_proposal_payloads`, validation helpers, and the implementations behind all existing MCP tools into `McpOperations`. Register these exact operation keys:

```python
MCP_OPERATION_HANDLERS = {
    "status", "series_create", "series_list", "series_set_language_tags",
    "concept_list", "concept_get", "concept_create", "concept_update", "concept_archive",
    "concept_merge", "concept_rename", "concept_facet_add", "concept_facet_update",
    "concept_facet_list", "concept_facet_set_canonical", "concept_semantic_tags_set",
    "crystal_link_concept", "crystal_story_scopes_set", "crystal_semantic_tags_set",
    "rule_crystals_list", "rule_crystal_archive", "rule_crystal_validate",
    "termbase_contract", "termbase_validate", "termbase_propose", "termbase_approve",
    "memory_search", "rag_import", "rag_search", "memory_add", "session_start",
    "session_complete", "short_term_add", "recall", "feedback", "dream",
    "concept_proposals_list",
}
```

Compatibility wrappers `series_init` and `termbase_*` tool aliases must call their corresponding registered operation rather than add a second handler.

- [ ] **Step 4: Verify unit parity and route execution**

Run: `uv run pytest tests/test_mcp_operations.py tests/test_service_http.py -k 'mcp' -v`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/mcp_operations.py src/hieronymus/service_http.py tests/test_mcp_operations.py tests/test_service_http.py && git commit -m "refactor: move MCP domain operations into daemon"`

### Task 3: Convert stdio MCP tools to daemon delegation

**Files:**
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `tests/test_mcp_server.py`
- Modify: `tests/test_mcp_memory_primitives.py`
- Modify: `tests/test_mcp_rag.py`

**Interfaces:**
- Consumes `DaemonMcpClient.invoke(operation, params)` from Task 1.
- Produces unchanged `hieronymus_*` FastMCP tool signatures and JSON payloads.

- [ ] **Step 1: Change one MCP contract test to require daemon delegation**

```python
def test_mcp_status_uses_daemon_client(monkeypatch, tmp_path: Path) -> None:
    client = FakeDaemonMcpClient({"service": {"running": True}})
    monkeypatch.setattr(mcp_server, "_daemon_client", lambda: client)
    assert mcp_server.hieronymus_status() == {"service": {"running": True}}
    assert client.calls == [("status", {})]
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_mcp_server.py -k status_uses_daemon_client -v`

Expected: FAIL because MCP tools still instantiate direct stores.

- [ ] **Step 3: Replace every direct-store tool body with a typed delegation**

Define `_daemon_client() -> DaemonMcpClient` and `_invoke(operation: str, **params: object) -> Any`. Each existing decorated function keeps its public signature and delegates, for example:

```python
@server.tool()
def hieronymus_session_complete(session_id: int) -> dict[str, int | bool]:
    return cast(dict[str, int | bool], _invoke("session_complete", session_id=session_id))
```

Use the exact operation keys from Task 2. Keep compatibility wrappers calling the public delegated tool to preserve their current argument adaptation. Delete direct-store imports and the direct-store status adapter from this module.

- [ ] **Step 4: Verify all MCP contracts**

Run: `uv run pytest tests/test_mcp_server.py tests/test_mcp_memory_primitives.py tests/test_mcp_rag.py tests/test_mcp_read_learn_compatibility.py -q`

Expected: PASS with unchanged tool discovery and payload assertions.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/mcp_server.py tests/test_mcp_server.py tests/test_mcp_memory_primitives.py tests/test_mcp_rag.py tests/test_mcp_read_learn_compatibility.py && git commit -m "refactor: route stdio MCP tools through daemon"`

### Task 4: Make diagnostics describe the single daemon accurately

**Files:**
- Modify: `src/hieronymus/doctor.py`
- Modify: `src/hieronymus/cli_boundaries.py`
- Modify: `docs/current-baseline.md`
- Modify: `tests/test_doctor.py`
- Modify: `tests/test_cli_boundaries.py`

**Interfaces:**
- Produces a doctor informational finding with code `daemon-running` and message containing PID, port, and data root when healthy.
- Produces no `DIRECT_STORE_MCP_ADAPTER` symbol.

- [ ] **Step 1: Write failing diagnostics tests**

```python
def test_doctor_reports_running_daemon_as_status_not_autofix(config: HieronymusConfig, monkeypatch) -> None:
    monkeypatch.setattr(ServiceManager, "status", lambda _: {"running": True, "pid": 12, "port": 8765, "data_root": str(config.data_root)})
    report = Doctor(config).run()
    assert report["autofixed"] == []
    assert report["warnings"] == []
    assert report["errors"] == []
    assert any(item.code == "daemon-running" and "8765" in item.message for item in report["info"])
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_doctor.py tests/test_cli_boundaries.py -k 'daemon_running_as_status or mcp_direct_store' -v`

Expected: FAIL because the report has no `info` group and direct-store boundary remains.

- [ ] **Step 3: Implement diagnostic contract and documentation update**

Add the `info` report list, append healthy daemon detail there, retain warnings for `no-state` and `unreachable`, remove direct-store boundary metadata, and update `docs/current-baseline.md` to describe the daemon as the owner of MCP operations.

- [ ] **Step 4: Verify green**

Run: `uv run pytest tests/test_doctor.py tests/test_cli_boundaries.py -q`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add src/hieronymus/doctor.py src/hieronymus/cli_boundaries.py docs/current-baseline.md tests/test_doctor.py tests/test_cli_boundaries.py && git commit -m "fix: report unified daemon lifecycle accurately"`

### Task 5: Prove one daemon end-to-end

**Files:**
- Modify: `tests/test_cli_service.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Write the integration regression**

```python
def test_mcp_autostart_is_reused_by_admin_and_doctor(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "data")
    first = DaemonMcpClient(config).invoke("status", {})
    state = read_server_state(config)
    assert first["service"]["running"] is True
    assert state is not None
    assert ServiceManager(config).ensure_running()["started"] is False
    report = Doctor(config).run()
    assert any(str(state.port) in item.message for item in report["info"])
```

- [ ] **Step 2: Verify red, then execute the integration test**

Run: `uv run pytest tests/test_cli_service.py tests/test_mcp_server.py -k mcp_autostart_is_reused_by_admin_and_doctor -v`

Expected before Tasks 1–4: FAIL; after them: PASS.

- [ ] **Step 3: Run full verification, build, install, and check the real daemon**

Run: `uv run pytest`

Run: `uv run ruff check . && uv run ruff format --check .`

Run: `bun run --cwd frontend typecheck && bun run --cwd frontend build`

Run: `uv build && uv tool install --reinstall dist/hieronymus-0.1.0-py3-none-any.whl`

Run: `hiero stop --json; hieronymus-mcp` (invoke `hieronymus_status` from a configured MCP client), then `hiero status --json`

Expected: MCP starts one daemon; config/admin and doctor reuse the same PID/port/data root.

- [ ] **Step 4: Commit and push**

Run: `git add tests/test_cli_service.py tests/test_mcp_server.py docs/superpowers/plans/2026-07-16-unified-daemon-mcp.md && git commit -m "test: verify MCP reuses unified daemon" && git push origin main`

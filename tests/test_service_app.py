from __future__ import annotations

import asyncio
import json
import re
import threading
import time
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest
from starlette.testclient import TestClient, WebSocketDenialResponse
from starlette.websockets import WebSocket, WebSocketDisconnect

from hieronymus.config import HieronymusConfig
from hieronymus.daemon_events import AdminEventHub
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.dreaming import DreamOutput
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.service_app import _cancel_tasks, build_app, status_payload
from hieronymus.service_state import ServerState
from hieronymus.workspace import WorkspaceStore

SERVICE_ORIGIN = "http://127.0.0.1:9768"


def _make_state(config: HieronymusConfig) -> ServerState:
    return ServerState(
        pid=12345,
        host="127.0.0.1",
        port=9768,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        launch_id="test-launch",
    )


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


@pytest.fixture
def asset_root(tmp_path: Path) -> Path:
    root = tmp_path / "dist"
    (root / "assets").mkdir(parents=True)
    (root / "index.html").write_text(
        '<h1>Hieronymus Web Console</h1><script src="/assets/app.js"></script>',
        encoding="utf-8",
    )
    (root / "assets" / "app.js").write_text("const app = 'Hieronymus';", encoding="utf-8")
    return root


@pytest.fixture
def client(config: HieronymusConfig, asset_root: Path) -> TestClient:
    return TestClient(
        build_app(config, _make_state(config), asset_root=asset_root),
        base_url=SERVICE_ORIGIN,
    )


def _browser_headers(origin: str = SERVICE_ORIGIN) -> dict[str, str]:
    return {"Origin": origin}


def test_health_endpoint_returns_daemon_identity(client: TestClient) -> None:
    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {
        "ok": True,
        "service": "hieronymus",
        "version": "0.1.0",
        "pid": 12345,
        "data_root": str(client.app.state.runtime.config.data_root),
        "database_path": str(client.app.state.runtime.config.database_path),
        "launch_id": "test-launch",
    }
    assert "token" not in response.json()


@pytest.mark.parametrize("path", ["/config", "/config/dreaming", "/admin", "/admin/memory"])
def test_console_pages_are_available_without_credentials(client: TestClient, path: str) -> None:
    response = client.get(path)

    assert response.status_code == 200
    assert "Hieronymus Web Console" in response.text
    assert "set-cookie" not in response.headers


def test_web_assets_are_served_without_credentials(client: TestClient) -> None:
    page = client.get("/config")
    asset_path = re.search(r'src="(/assets/[^\"]+\.js)"', page.text)

    assert asset_path is not None
    response = client.get(asset_path.group(1))
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/javascript")
    assert "Hieronymus" in response.text


def test_assets_use_only_the_explicit_root(config: HieronymusConfig, tmp_path: Path) -> None:
    parent_asset = tmp_path / "frontend" / "dist" / "assets" / "leaked.js"
    parent_asset.parent.mkdir(parents=True)
    parent_asset.write_text("leaked", encoding="utf-8")
    explicit_root = tmp_path / "elsewhere"
    explicit_root.mkdir()

    with TestClient(
        build_app(config, _make_state(config), asset_root=explicit_root),
        base_url=SERVICE_ORIGIN,
    ) as local_client:
        response = local_client.get("/assets/leaked.js")

    assert response.status_code == 404
    assert response.json() == {"error": "not_found"}


def test_default_asset_root_is_the_packaged_distribution(
    config: HieronymusConfig, tmp_path: Path
) -> None:
    package_module = tmp_path / "package" / "service_app.py"
    packaged_root = package_module.parent / "frontend" / "dist"
    packaged_root.mkdir(parents=True)
    (packaged_root / "index.html").write_text("packaged console", encoding="utf-8")

    with (
        patch("hieronymus.service_app.__file__", str(package_module)),
        TestClient(build_app(config, _make_state(config)), base_url=SERVICE_ORIGIN) as local_client,
    ):
        response = local_client.get("/config")

    assert response.status_code == 200
    assert response.text == "packaged console"


def test_missing_console_index_has_stable_error(config: HieronymusConfig, tmp_path: Path) -> None:
    empty_root = tmp_path / "empty-dist"
    empty_root.mkdir()

    with TestClient(
        build_app(config, _make_state(config), asset_root=empty_root),
        base_url=SERVICE_ORIGIN,
    ) as local_client:
        response = local_client.get("/config")

    assert response.status_code == 404
    assert response.json() == {"error": "web_console_not_built"}


def test_provider_api_creates_lists_reads_and_deletes_profiles(
    client: TestClient,
) -> None:
    provider = {
        "id": "deepseek",
        "name": "DeepSeek",
        "type": "openai",
        "url": "https://api.deepseek.com/v1",
        "key": "test-key",
        "timeout_seconds": "30",
    }
    created = client.post("/api/providers", json={"provider": provider}, headers=_browser_headers())
    listed = client.get("/api/providers", headers=_browser_headers())
    detailed = client.get("/api/providers/deepseek", headers=_browser_headers())
    deleted = client.delete("/api/providers/deepseek", headers=_browser_headers())

    assert created.status_code == 200
    assert created.json()["provider"]["key_configured"] is True
    assert listed.json()["providers"][0]["id"] == "deepseek"
    assert detailed.json()["provider"]["id"] == "deepseek"
    assert deleted.status_code == 200


def test_provider_check_returns_a_structured_failure(client: TestClient) -> None:
    client.post(
        "/api/providers",
        json={
            "provider": {
                "id": "local-ollama",
                "name": "Local Ollama",
                "type": "ollama",
                "url": "http://127.0.0.1:9",
                "key": "",
                "timeout_seconds": "1",
            }
        },
        headers=_browser_headers(),
    )

    response = client.post("/api/providers/local-ollama/check", json={}, headers=_browser_headers())

    assert response.status_code == 200
    assert response.json()["check"]["ok"] is False
    assert response.json()["check"]["error"] == "model suggestions unavailable"


def test_provider_models_route_returns_model_suggestions(client: TestClient) -> None:
    client.post(
        "/api/providers",
        json={
            "provider": {
                "id": "local-ollama",
                "name": "Local Ollama",
                "type": "ollama",
                "url": "http://127.0.0.1:9",
                "key": "",
                "timeout_seconds": "1",
            }
        },
        headers=_browser_headers(),
    )

    response = client.get("/api/providers/local-ollama/models", headers=_browser_headers())

    assert response.status_code == 400
    assert response.json() == {
        "models": ["gemma4-e3b"],
        "source": "defaults",
        "error": "model suggestions unavailable",
    }


@pytest.mark.parametrize("name", ["dream", "ingest", "release"])
def test_settings_apis_are_scoped_to_their_files(client: TestClient, name: str) -> None:
    response = client.get(f"/api/settings/{name}", headers=_browser_headers())

    assert response.status_code == 200
    assert name in response.json()


def test_dream_settings_can_be_saved(client: TestClient) -> None:
    response = client.post(
        "/api/settings/dream",
        json={
            "dream": {
                "dreaming": {"enabled": True, "schedule_interval_minutes": 45},
                "workflows": {},
            }
        },
        headers=_browser_headers(),
    )

    assert response.status_code == 200
    assert response.json()["dream"]["dreaming"]["enabled"] is True


def test_ingest_and_release_settings_can_be_saved(client: TestClient) -> None:
    ingest = client.post(
        "/api/settings/ingest",
        json={
            "ingest": {
                "short_memory": {
                    "warning_sentence_count": 8,
                    "rejection_sentence_count": 32,
                },
                "learn": {"max_block_chars": 1600},
            }
        },
        headers=_browser_headers(),
    )
    release = client.post(
        "/api/settings/release",
        json={"release": {"update_channel": "dev"}},
        headers=_browser_headers(),
    )

    assert ingest.status_code == 200
    assert ingest.json()["ingest"]["learn"]["max_block_chars"] == 1600
    assert release.status_code == 200
    assert release.json() == {"release": {"update_channel": "dev"}, "error": ""}


def test_admin_dashboard_and_snapshot_return_local_views(client: TestClient) -> None:
    dashboard = client.get("/api/admin/dashboard", headers=_browser_headers())
    snapshot = client.get(
        "/api/admin/snapshot?view=Concepts&selected_id=7", headers=_browser_headers()
    )

    assert dashboard.status_code == 200
    assert dashboard.json()["default_view"] == "Crystals"
    assert snapshot.json()["snapshot"]["view"] == "Concepts"


def test_admin_actions_are_explicitly_allowlisted(
    config: HieronymusConfig, asset_root: Path
) -> None:
    class FakeAdminBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def reinforce_crystal(self, params: dict[str, object]) -> dict[str, object]:
            return {"ok": True, "received": params}

    with (
        patch("hieronymus.service_app.AdminBridge", FakeAdminBridge),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
    ):
        reinforced = local_client.post(
            "/api/admin/actions/reinforce_crystal",
            json={"id": 7},
            headers=_browser_headers(),
        )
        unknown = local_client.post(
            "/api/admin/actions/not_a_method", json={}, headers=_browser_headers()
        )

    assert reinforced.json() == {"ok": True, "received": {"id": 7}}
    assert unknown.status_code == 404
    assert unknown.json() == {"error": "unknown_admin_action"}


def test_mcp_bridge_rejects_unknown_and_executes_known_operations(client: TestClient) -> None:
    unknown = client.post("/api/mcp/not-real", json={})
    created = client.post(
        "/api/mcp/series_create", json={"slug": "oso", "title": "Only Sense Online"}
    )

    assert unknown.status_code == 404
    assert unknown.json() == {"error": "unknown_mcp_operation"}
    assert created.json()["result"]["slug"] == "oso"


def test_browser_api_validates_origin_and_host(client: TestClient) -> None:
    assert client.get("/api/admin/dashboard", headers=_browser_headers()).status_code == 200
    assert (
        client.get(
            "/api/admin/dashboard", headers=_browser_headers("https://evil.example")
        ).status_code
        == 403
    )
    assert (
        client.get(
            "/api/admin/dashboard",
            headers={"Host": "evil.example", "Origin": SERVICE_ORIGIN},
        ).status_code
        == 403
    )


def test_native_api_request_without_origin_is_accepted(client: TestClient) -> None:
    response = client.get("/api/admin/dashboard")

    assert response.status_code == 200


@pytest.mark.parametrize("path", ["/shutdown", "/api/mcp/series_create"])
def test_supplied_foreign_origin_is_rejected_for_native_routes(
    client: TestClient, path: str
) -> None:
    response = client.post(path, json={}, headers=_browser_headers("https://evil.example"))

    assert response.status_code == 403
    assert response.json() == {"error": "forbidden"}


def test_same_origin_fetch_without_origin_is_accepted(client: TestClient) -> None:
    response = client.get(
        "/api/admin/dashboard",
        headers={"Referer": f"{SERVICE_ORIGIN}/admin", "Sec-Fetch-Site": "same-origin"},
    )

    assert response.status_code == 200


def test_admin_websocket_rejects_foreign_origin(client: TestClient) -> None:
    with pytest.raises(WebSocketDenialResponse) as denied:
        with client.websocket_connect(
            "/ws/admin",
            headers={**_browser_headers("https://evil.example"), "Host": "127.0.0.1:9768"},
        ):
            pass
    assert denied.value.status_code == 403


def test_admin_websocket_connects_broadcasts_and_disconnects(client: TestClient) -> None:
    events = client.app.state.runtime.events
    with (
        client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as first,
        client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as second,
    ):
        assert events.subscriber_count == 2
        events.publish("dream_started", {"trigger": "manual"})
        assert first.receive_json()["type"] == "dream_started"
        assert second.receive_json()["type"] == "dream_started"

    deadline = time.monotonic() + 1
    while events.subscriber_count and time.monotonic() < deadline:
        time.sleep(0.01)
    assert events.subscriber_count == 0


def test_admin_websocket_overflow_closes_only_slow_live_subscriber(
    config: HieronymusConfig,
    asset_root: Path,
) -> None:
    app = build_app(config, _make_state(config), asset_root=asset_root)
    app.state.runtime.events = AdminEventHub(default_capacity=1)
    slow_send_started = threading.Event()
    release_slow_send = threading.Event()
    original_send_json = WebSocket.send_json

    async def controlled_send_json(self, data, mode="text"):
        if self.headers.get("x-test-slow") == "true":
            slow_send_started.set()
            while not release_slow_send.is_set():
                await asyncio.sleep(0.001)
        await original_send_json(self, data, mode=mode)

    with (
        patch.object(WebSocket, "send_json", controlled_send_json),
        TestClient(app, base_url=SERVICE_ORIGIN) as local_client,
        local_client.websocket_connect(
            "/ws/admin",
            headers={
                **_browser_headers(),
                "Host": "127.0.0.1:9768",
                "X-Test-Slow": "true",
            },
        ) as slow,
        local_client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as healthy,
    ):
        try:
            app.state.runtime.events.publish("progress", {"sequence": 1})
            assert slow_send_started.wait(timeout=1)
            assert healthy.receive_json()["payload"] == {"sequence": 1}
            app.state.runtime.events.publish("progress", {"sequence": 2})
            assert healthy.receive_json()["payload"] == {"sequence": 2}
            app.state.runtime.events.publish("progress", {"sequence": 3})
            assert healthy.receive_json()["payload"] == {"sequence": 3}

            release_slow_send.set()
            assert slow.receive_json()["payload"] == {"sequence": 1}
            with pytest.raises(WebSocketDisconnect) as closed:
                slow.receive_json()
            assert closed.value.code == 1013

            app.state.runtime.events.publish("progress", {"sequence": 4})
            assert healthy.receive_json()["payload"] == {"sequence": 4}
        finally:
            release_slow_send.set()


def test_status_endpoint_returns_paths_pid_and_active_cycle(
    config: HieronymusConfig, asset_root: Path
) -> None:
    with TestClient(
        build_app(config, _make_state(config), asset_root=asset_root),
        base_url=SERVICE_ORIGIN,
    ) as local_client:
        with dream_cycle_lock(config, owner="manual"):
            payload = local_client.get("/status").json()

    assert payload["running"] is True
    assert payload["pid"] == 12345
    assert payload["host"] == "127.0.0.1"
    assert payload["port"] == 9768
    assert payload["version"] == "0.1.0"
    assert payload["launch_id"] == "test-launch"
    assert payload["data_root"] == str(config.data_root)
    assert payload["database_path"] == str(config.database_path)
    assert "config_path" not in payload
    assert {provider["name"] for provider in payload["providers"]} == {
        "deterministic",
        "openai",
        "gemini",
        "anthropic",
    }
    assert payload["dreaming"]["cycle_active"] is True
    assert payload["dreaming"]["active_cycle"]["owner"] == "manual"
    assert "token" not in payload["dreaming"]["active_cycle"]
    assert payload["mcp_adapter"] == {"available": True, "mode": "local-http"}


def test_status_payload_degrades_when_dependencies_fail(
    config: HieronymusConfig,
) -> None:
    with (
        patch("hieronymus.service_app.DreamAutostart") as autostart_class,
        patch("hieronymus.service_app.ProviderRegistry") as provider_registry,
    ):
        autostart_class.return_value.status.side_effect = RuntimeError("settings broken")
        provider_registry.return_value.status_payload.side_effect = RuntimeError("providers broken")
        payload = status_payload(config, _make_state(config))

    assert payload["dreaming"] == {
        "available": False,
        "pending_short_term_memories": 0,
        "error": "settings broken",
    }
    assert payload["providers"] == []
    assert payload["providers_error"] == "providers broken"


def test_status_survives_obsolete_dream_workflow_config(
    config: HieronymusConfig, asset_root: Path
) -> None:
    config.data_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        "[workflows.crystallization]\n"
        "provider = 'openai'\n"
        "model = 'gpt-4.1-mini'\n"
        "enabled = true\n",
        encoding="utf-8",
    )
    with TestClient(
        build_app(config, _make_state(config), asset_root=asset_root),
        base_url=SERVICE_ORIGIN,
    ) as local_client:
        payload = local_client.get("/status").json()

    assert payload["running"] is True
    assert payload["providers_error"] == ""


def test_shutdown_requests_server_termination(client: TestClient) -> None:
    callback_invocations = 0

    def request_shutdown() -> None:
        nonlocal callback_invocations
        callback_invocations += 1

    client.app.state.runtime.request_shutdown = request_shutdown
    response = client.post("/shutdown", headers={"X-Hieronymus-Expected-Launch-Id": "test-launch"})

    assert response.status_code == 200
    assert response.json() == {"ok": True, "stopping": True}
    assert client.app.state.shutdown_requested.is_set()
    assert callback_invocations == 1


def test_shutdown_refuses_mismatched_launch_precondition(client: TestClient) -> None:
    callback = MagicMock()
    client.app.state.runtime.request_shutdown = callback

    response = client.post(
        "/shutdown", headers={"X-Hieronymus-Expected-Launch-Id": "replacement-launch"}
    )

    assert response.status_code == 412
    assert response.json() == {
        "error": "launch_identity_mismatch",
        "error_type": "launch_identity_mismatch",
    }
    assert not client.app.state.shutdown_requested.is_set()
    callback.assert_not_called()


def test_lifecycle_endpoints_require_no_authentication(client: TestClient) -> None:
    assert client.get("/health").status_code == 200
    assert client.get("/status").status_code == 200
    response = client.post("/shutdown", headers={"X-Hieronymus-Expected-Launch-Id": "test-launch"})
    assert response.status_code == 200


def test_oversized_json_preserves_legacy_empty_object_contract(
    config: HieronymusConfig, asset_root: Path
) -> None:
    received: list[dict[str, object]] = []

    class CapturingConfigBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def save_provider(self, params: dict[str, object]) -> dict[str, object]:
            received.append(params)
            return {"error": "provider must be an object"}

    with (
        patch("hieronymus.service_app.ConfigBridge", CapturingConfigBridge),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
    ):
        response = local_client.post(
            "/api/providers",
            content=json.dumps({"padding": "x" * 1_000_001}),
            headers={**_browser_headers(), "Content-Type": "application/json"},
        )

    assert response.status_code == 400
    assert response.json() == {"error": "provider must be an object"}
    assert received == [{}]


@pytest.mark.parametrize(
    ("body", "content_type"),
    [(b"{", "application/json"), (b"[]", "application/json"), (b"\xff", "application/json")],
)
def test_invalid_json_payloads_become_empty_objects(
    client: TestClient, body: bytes, content_type: str
) -> None:
    response = client.post(
        "/api/providers",
        content=body,
        headers={**_browser_headers(), "Content-Type": content_type},
    )

    assert response.status_code == 400
    assert "error" in response.json()


def test_value_and_key_errors_have_stable_payloads(
    config: HieronymusConfig, asset_root: Path
) -> None:
    def value_error(_: HieronymusConfig, __: dict[str, object]) -> dict[str, object]:
        raise ValueError("bad request")

    def key_error(_: HieronymusConfig, __: dict[str, object]) -> dict[str, object]:
        raise KeyError("missing")

    with (
        patch.dict(
            "hieronymus.service_app.MCP_OPERATION_HANDLERS",
            {"value_error": value_error, "key_error": key_error},
        ),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
    ):
        value_response = local_client.post("/api/mcp/value_error", json={})
        key_response = local_client.post("/api/mcp/key_error", json={})

    assert value_response.status_code == 400
    assert value_response.json() == {"error": "bad request"}
    assert key_response.status_code == 400
    assert key_response.json() == {"error": "'missing'", "error_type": "KeyError"}


def test_sync_service_work_runs_outside_the_asgi_event_loop(
    config: HieronymusConfig, asset_root: Path
) -> None:
    worker_calls: list[str] = []

    def assert_worker_thread(label: str) -> None:
        with pytest.raises(RuntimeError, match="no running event loop"):
            asyncio.get_running_loop()
        worker_calls.append(label)

    class WorkerConfigBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def provider_list(self, _: dict[str, object]) -> dict[str, object]:
            assert_worker_thread("config")
            return {"providers": []}

        def check_saved_provider(self, _: dict[str, object]) -> dict[str, object]:
            assert_worker_thread("provider-network")
            return {"check": {"ok": True}}

    class WorkerAdminBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def dashboard(self, _: dict[str, object]) -> dict[str, object]:
            assert_worker_thread("admin")
            return {"default_view": "Crystals"}

    def worker_mcp(_: HieronymusConfig, __: dict[str, object]) -> dict[str, object]:
        assert_worker_thread("mcp")
        return {"ok": True}

    def worker_status(_: HieronymusConfig, __: ServerState) -> dict[str, object]:
        assert_worker_thread("status")
        return {"running": True}

    with (
        patch("hieronymus.service_app.ConfigBridge", WorkerConfigBridge),
        patch("hieronymus.service_app.AdminBridge", WorkerAdminBridge),
        patch("hieronymus.service_app.status_payload", worker_status),
        patch.dict("hieronymus.service_app.MCP_OPERATION_HANDLERS", {"worker": worker_mcp}),
    ):
        local_client = TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        )
        assert local_client.get("/api/providers").status_code == 200
        assert local_client.post("/api/providers/demo/check", json={}).status_code == 200
        assert local_client.get("/api/admin/dashboard").status_code == 200
        assert local_client.post("/api/mcp/worker", json={}).status_code == 200
        assert local_client.get("/status").status_code == 200

    assert worker_calls == ["config", "provider-network", "admin", "mcp", "status"]


def test_manual_dreaming_failure_redacts_configured_secrets(
    config: HieronymusConfig, asset_root: Path
) -> None:
    config.data_root.mkdir(parents=True)
    config.provider_config_path.write_text(
        "[openai]\nname='OpenAI'\ntype='openai'\nurl='https://example.test'\nkey='super-secret'\n",
        encoding="utf-8",
    )

    class FailingAdminBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def run_manual_dreaming(
            self, _: dict[str, object], *, event_sink=None
        ) -> dict[str, object]:
            raise RuntimeError("provider super-secret failed")

    with (
        patch("hieronymus.service_app.AdminBridge", FailingAdminBridge),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
        local_client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as websocket,
    ):
        response = local_client.post(
            "/api/admin/actions/run_manual_dreaming", json={}, headers=_browser_headers()
        )
        started = websocket.receive_json()
        failed = websocket.receive_json()

    assert response.json() == {"started": True, "status": "running"}
    assert started["type"] == "dream_started"
    assert failed["type"] == "dream_failed"
    assert "super-secret" not in failed["payload"]["error"]


def test_manual_dreaming_publishes_start_phase_and_completion_to_websocket(
    config: HieronymusConfig, asset_root: Path
) -> None:
    class EmptyProvider:
        name = "empty"

        def crystallize(self, context, memories):
            return DreamOutput(crystals=[], concept_proposals=[])

    series = Registry(config).create_series(
        slug="websocket-dream",
        title="WebSocket Dream",
        source_language="ja",
        target_language="ru",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Dream over WebSocket.")
    workspace.complete_session(session.id)

    with (
        patch("hieronymus.admin.resolve_provider", return_value=EmptyProvider()),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
        local_client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as websocket,
    ):
        response = local_client.post(
            "/api/admin/actions/run_manual_dreaming", json={}, headers=_browser_headers()
        )
        events = [websocket.receive_json() for _ in range(4)]

    assert response.json() == {"started": True, "status": "running"}
    assert [event["type"] for event in events] == [
        "dream_started",
        "dream_phase_progress",
        "dream_phase_progress",
        "dream_completed",
    ]
    assert [(event["payload"]["phase"], event["payload"]["status"]) for event in events[1:3]] == [
        ("crystallization", "running"),
        ("crystallization", "completed"),
    ]


def test_manual_dreaming_failure_is_safe_when_sanitization_also_fails(
    config: HieronymusConfig, asset_root: Path
) -> None:
    class FailingAdminBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def run_manual_dreaming(
            self, _: dict[str, object], *, event_sink=None
        ) -> dict[str, object]:
            raise RuntimeError("provider super-secret failed")

    with (
        patch("hieronymus.service_app.AdminBridge", FailingAdminBridge),
        patch(
            "hieronymus.service_app.load_provider_catalog",
            side_effect=RuntimeError("catalog contains super-secret"),
        ),
        TestClient(
            build_app(config, _make_state(config), asset_root=asset_root),
            base_url=SERVICE_ORIGIN,
        ) as local_client,
        local_client.websocket_connect(
            "/ws/admin", headers={**_browser_headers(), "Host": "127.0.0.1:9768"}
        ) as websocket,
    ):
        local_client.post(
            "/api/admin/actions/run_manual_dreaming", json={}, headers=_browser_headers()
        )
        websocket.receive_json()
        failed = websocket.receive_json()

    assert failed["payload"] == {
        "trigger": "manual",
        "error": "manual dreaming failed",
    }


def test_cancel_tasks_awaits_task_cancellation() -> None:
    async def exercise() -> None:
        waiting = asyncio.create_task(asyncio.Event().wait())
        await _cancel_tasks({waiting})
        assert waiting.done()
        assert waiting.cancelled()

    asyncio.run(exercise())


@pytest.mark.parametrize("method", ["get", "post", "delete"])
def test_unknown_routes_have_stable_json_errors(client: TestClient, method: str) -> None:
    response = getattr(client, method)("/not-real")

    assert response.status_code == 404
    assert response.json() == {"error": "not_found", "path": "/not-real"}


@pytest.mark.parametrize(("method", "path"), [("post", "/health"), ("get", "/shutdown")])
def test_known_paths_with_unsupported_methods_keep_not_found_contract(
    client: TestClient, method: str, path: str
) -> None:
    response = getattr(client, method)(path)

    assert response.status_code == 404
    assert response.json() == {"error": "not_found", "path": path}


def test_responses_do_not_emit_credentials(client: TestClient) -> None:
    for response in (client.get("/config"), client.get("/status"), client.post("/shutdown")):
        serialized = response.text.lower()
        assert "authorization" not in serialized
        assert "token" not in serialized
        assert "set-cookie" not in response.headers

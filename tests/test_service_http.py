from __future__ import annotations

import json
import re
import threading
import urllib.error
import urllib.request
from http.cookiejar import CookieJar
from pathlib import Path
from typing import Any
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.service_http import HieronymusHTTPServer, build_server, status_payload
from hieronymus.service_state import ServerState


def _request_json(
    url: str,
    *,
    method: str = "GET",
    token: str = "local-test-token",
) -> dict[str, object]:
    request = urllib.request.Request(url, method=method)
    request.add_header("X-Hieronymus-Token", token)
    with urllib.request.urlopen(request, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


def _post_json(url: str, payload: dict[str, object]) -> dict[str, object]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Hieronymus-Token": "local-test-token",
        },
    )
    with urllib.request.urlopen(request, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


def _assert_unauthorized(url: str, *, method: str = "GET", token: str | None = None) -> None:
    request = urllib.request.Request(url, method=method)
    if token is not None:
        request.add_header("X-Hieronymus-Token", token)
    try:
        urllib.request.urlopen(request, timeout=2)
    except urllib.error.HTTPError as exc:
        assert exc.code == 401
        return
    raise AssertionError("request should have been rejected")


def _make_state(config: HieronymusConfig) -> ServerState:
    return ServerState(
        pid=12345,
        host="127.0.0.1",
        port=0,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )


def _serve(server: HieronymusHTTPServer) -> tuple[threading.Thread, str]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address
    return thread, f"http://{host}:{port}"


def _stop_server(server: HieronymusHTTPServer, thread: threading.Thread) -> None:
    server.shutdown()
    thread.join(timeout=2)
    server.server_close()


def test_health_endpoint_returns_daemon_identity(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    server = build_server(config, state)
    assert isinstance(server, HieronymusHTTPServer)
    thread, base_url = _serve(server)
    try:
        payload = _request_json(f"{base_url}/health")
    finally:
        _stop_server(server, thread)

    assert payload["ok"] is True
    assert payload["service"] == "hieronymus"
    assert payload["version"] == "0.1.0"


def test_config_page_requires_token_then_redirects_to_cookie_session(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        request = urllib.request.Request(f"{base_url}/config?token=local-test-token")
        cookies = CookieJar()
        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(cookies),
            urllib.request.HTTPRedirectHandler(),
        )
        with opener.open(request, timeout=2) as response:
            page = response.read().decode("utf-8")
    finally:
        _stop_server(server, thread)

    assert "Hieronymus Web Console" in page
    assert any(cookie.name == "hieronymus_token" for cookie in cookies)


def test_config_and_admin_memory_routes_serve_the_web_application_after_session_setup(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        pages = []
        for path in ("/config/dreaming", "/admin/memory"):
            request = urllib.request.Request(f"{base_url}{path}")
            request.add_header("X-Hieronymus-Token", "local-test-token")
            with urllib.request.urlopen(request, timeout=2) as response:
                pages.append(response.read().decode("utf-8"))
    finally:
        _stop_server(server, thread)

    assert all("Hieronymus Web Console" in page for page in pages)


def test_web_assets_require_the_same_local_session(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        page_request = urllib.request.Request(f"{base_url}/config")
        page_request.add_header("X-Hieronymus-Token", "local-test-token")
        with urllib.request.urlopen(page_request, timeout=2) as response:
            page = response.read().decode("utf-8")
        asset_path = re.search(r'src="(/assets/[^\"]+\.js)"', page)
        assert asset_path is not None
        request = urllib.request.Request(f"{base_url}{asset_path.group(1)}")
        request.add_header("X-Hieronymus-Token", "local-test-token")
        with urllib.request.urlopen(request, timeout=2) as response:
            content_type = response.headers["Content-Type"]
            asset = response.read().decode("utf-8")
    finally:
        _stop_server(server, thread)

    assert content_type.startswith("text/javascript")
    assert "Hieronymus" in asset


def test_provider_api_creates_and_lists_custom_profiles(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        created = _post_json(
            f"{base_url}/api/providers",
            {
                "provider": {
                    "id": "deepseek",
                    "name": "DeepSeek",
                    "type": "openai",
                    "url": "https://api.deepseek.com/v1",
                    "key": "test-key",
                    "timeout_seconds": "30",
                }
            },
        )
        listed = _request_json(f"{base_url}/api/providers")
    finally:
        _stop_server(server, thread)

    assert created["provider"]["id"] == "deepseek"
    assert created["provider"]["key_configured"] is True
    assert listed["providers"] == [
        {
            "id": "deepseek",
            "key_configured": True,
            "model": "",
            "name": "DeepSeek",
            "timeout_seconds": 30.0,
            "type": "openai",
            "url": "https://api.deepseek.com/v1",
        }
    ]


def test_provider_check_api_returns_a_structured_failure(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        _post_json(
            f"{base_url}/api/providers",
            {
                "provider": {
                    "id": "local-ollama",
                    "name": "Local Ollama",
                    "type": "ollama",
                    "url": "http://127.0.0.1:9",
                    "key": "",
                    "timeout_seconds": "1",
                }
            },
        )
        payload = _post_json(f"{base_url}/api/providers/local-ollama/check", {})
    finally:
        _stop_server(server, thread)

    assert payload["check"]["ok"] is False
    assert payload["check"]["error"] == "model suggestions unavailable"


def test_settings_apis_are_scoped_to_their_configuration_files(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        loaded = _request_json(f"{base_url}/api/settings/dream")
        saved = _post_json(
            f"{base_url}/api/settings/dream",
            {
                "dream": {
                    "dreaming": {"enabled": True, "schedule_interval_minutes": 45},
                    "workflows": {},
                }
            },
        )
    finally:
        _stop_server(server, thread)

    assert "dream" in loaded
    assert saved["dream"]["dreaming"]["enabled"] is True
    assert saved["dream"]["dreaming"]["schedule_interval_minutes"] == 45


def test_admin_dashboard_api_returns_local_admin_snapshot(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        dashboard = _request_json(f"{base_url}/api/admin/dashboard")
    finally:
        _stop_server(server, thread)

    assert dashboard["default_view"] == "Crystals"
    assert dashboard["views"]


def test_admin_snapshot_api_accepts_a_view_parameter(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        snapshot = _request_json(f"{base_url}/api/admin/snapshot?view=Concepts")
    finally:
        _stop_server(server, thread)

    assert snapshot["snapshot"]["view"] == "Concepts"


def test_admin_memory_actions_are_explicitly_allowlisted(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))

    class FakeAdminBridge:
        def __init__(self, _: HieronymusConfig) -> None:
            pass

        def reinforce_crystal(self, params: dict[str, object]) -> dict[str, object]:
            return {"ok": True, "received": params}

    thread, base_url = _serve(server)
    try:
        with patch("hieronymus.service_http.AdminBridge", FakeAdminBridge):
            reinforced = _post_json(f"{base_url}/api/admin/actions/reinforce_crystal", {"id": 7})
            request = urllib.request.Request(
                f"{base_url}/api/admin/actions/not_a_method",
                data=b"{}",
                method="POST",
                headers={
                    "Content-Type": "application/json",
                    "X-Hieronymus-Token": "local-test-token",
                },
            )
            try:
                urllib.request.urlopen(request, timeout=2)
            except urllib.error.HTTPError as error:
                assert error.code == 404
                unknown = json.loads(error.read().decode("utf-8"))
            else:
                raise AssertionError("unknown action should be rejected")
    finally:
        _stop_server(server, thread)

    assert reinforced == {"ok": True, "received": {"id": 7}}
    assert unknown == {"error": "unknown_admin_action"}


def test_mcp_route_rejects_unknown_operation(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        request = urllib.request.Request(
            f"{base_url}/api/mcp/not-real",
            data=b"{}",
            method="POST",
            headers={
                "Content-Type": "application/json",
                "X-Hieronymus-Token": "local-test-token",
            },
        )
        try:
            urllib.request.urlopen(request, timeout=2)
        except urllib.error.HTTPError as error:
            assert error.code == 404
            payload = json.loads(error.read().decode("utf-8"))
        else:
            raise AssertionError("unknown MCP operation should be rejected")
    finally:
        _stop_server(server, thread)

    assert payload == {"error": "unknown_mcp_operation"}


def test_mcp_route_executes_series_operation_in_daemon(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    server = build_server(config, _make_state(config))
    thread, base_url = _serve(server)
    try:
        payload = _post_json(
            f"{base_url}/api/mcp/series_create",
            {"slug": "oso", "title": "Only Sense Online"},
        )
    finally:
        _stop_server(server, thread)

    assert payload == {
        "result": {
            "id": 1,
            "language_tags": [],
            "slug": "oso",
            "source_language": "",
            "target_language": "",
            "title": "Only Sense Online",
        }
    }


def test_status_endpoint_returns_paths_and_pid(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    server = build_server(config, state)
    thread, base_url = _serve(server)
    try:
        payload = _request_json(f"{base_url}/status")
    finally:
        _stop_server(server, thread)

    assert payload["running"] is True
    assert payload["pid"] == 12345
    assert payload["host"] == "127.0.0.1"
    assert payload["port"] == server.server_address[1]
    assert payload["version"] == "0.1.0"
    assert payload["started_at"] == "2026-06-06T12:00:00Z"
    assert payload["data_root"] == str(config.data_root)
    assert payload["database_path"] == str(config.database_path)
    assert payload["config_path"] == str(config.config_root)
    assert [provider["name"] for provider in payload["providers"]] == [
        "deterministic",
        "openai",
        "gemini",
        "anthropic",
    ]
    assert payload["dreaming"]["enabled"] is False
    assert payload["dreaming"]["active_provider"] == "deterministic"
    assert payload["dreaming"]["cycle_active"] is False
    assert payload["dreaming"]["active_cycle"] is None
    assert payload["dreaming"]["last_skipped_at"] is None
    assert payload["dreaming"]["last_skip_reason"] == ""
    assert payload["mcp_adapter"] == {"available": True, "mode": "local-http"}
    assert payload["housekeeping"] == {"last_cycle": None, "pending": False}


def test_status_endpoint_reports_active_dream_cycle(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    server = build_server(config, state)
    thread, base_url = _serve(server)
    try:
        with dream_cycle_lock(config, owner="manual"):
            payload = _request_json(f"{base_url}/status")
    finally:
        _stop_server(server, thread)

    assert payload["dreaming"]["cycle_active"] is True
    assert payload["dreaming"]["active_cycle"]["owner"] == "manual"
    assert payload["dreaming"]["active_cycle"]["pid"] > 0
    assert "started_at" in payload["dreaming"]["active_cycle"]
    assert "token" not in payload["dreaming"]["active_cycle"]


def test_status_payload_degrades_when_dreaming_status_fails(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)

    with patch("hieronymus.service_http.DreamAutostart") as autostart_class:
        autostart_class.return_value.status.side_effect = RuntimeError("settings broken")
        payload = status_payload(config, state)

    assert payload["dreaming"] == {
        "available": False,
        "pending_short_term_memories": 0,
        "error": "settings broken",
    }
    assert payload["housekeeping"] == {"last_cycle": None, "pending": False}


def test_shutdown_endpoint_stops_server(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    server = build_server(config, state)
    thread, base_url = _serve(server)
    try:
        payload: dict[str, Any] = _request_json(f"{base_url}/shutdown", method="POST")
        thread.join(timeout=2)
    finally:
        if thread.is_alive():
            server.shutdown()
            thread.join(timeout=2)
        server.server_close()

    assert payload == {"ok": True, "stopping": True}
    assert not thread.is_alive()


def test_service_endpoints_reject_missing_or_wrong_token(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    server = build_server(config, state)
    thread, base_url = _serve(server)
    try:
        _assert_unauthorized(f"{base_url}/health", token=None)
        _assert_unauthorized(f"{base_url}/status", token="wrong-token")
        _assert_unauthorized(f"{base_url}/shutdown", method="POST", token="wrong-token")
    finally:
        _stop_server(server, thread)

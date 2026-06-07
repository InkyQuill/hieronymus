from __future__ import annotations

import json
import threading
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.service_http import HieronymusHTTPServer, build_server
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
    assert payload["mcp_adapter"] == {"available": True, "mode": "local-http"}
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

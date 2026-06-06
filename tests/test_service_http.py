from __future__ import annotations

import json
import threading
import urllib.request
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_http import HieronymusHTTPServer, build_server
from hieronymus.service_state import ServerState, allocate_loopback_port


def _read_json(url: str) -> dict[str, object]:
    with urllib.request.urlopen(url, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


def test_health_endpoint_returns_daemon_identity(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=allocate_loopback_port(),
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    server = build_server(config, state)
    assert isinstance(server, HieronymusHTTPServer)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        payload = _read_json(f"http://127.0.0.1:{state.port}/health")
    finally:
        server.shutdown()
        thread.join(timeout=2)
        server.server_close()

    assert payload["ok"] is True
    assert payload["service"] == "hieronymus"
    assert payload["version"] == "0.1.0"


def test_status_endpoint_returns_paths_and_pid(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=allocate_loopback_port(),
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    server = build_server(config, state)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        payload = _read_json(f"http://127.0.0.1:{state.port}/status")
    finally:
        server.shutdown()
        thread.join(timeout=2)
        server.server_close()

    assert payload["running"] is True
    assert payload["pid"] == 12345
    assert payload["data_root"] == str(config.data_root)
    assert payload["database_path"] == str(config.database_path)

from __future__ import annotations

from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClientError
from hieronymus.service_discovery import discover_local_service
from hieronymus.service_state import ServerState, write_server_state


def test_discover_local_service_reports_unhealthy_service_state(
    tmp_path: Path,
    monkeypatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=123,
        host="127.0.0.1",
        port=8765,
        version="0.2.0",
        started_at="2026-06-14T00:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="secret",
    )
    write_server_state(config, state)
    monkeypatch.setattr("hieronymus.service_state.is_pid_running", lambda pid: pid == 123)

    def fake_health(self, received_state):
        assert received_state == state
        raise ServiceClientError("bad health payload")

    monkeypatch.setattr("hieronymus.service_client.ServiceClient.health", fake_health)

    assert discover_local_service(config) == {
        "available": False,
        "mode": "direct-local",
        "reason": "local service state exists but health check failed: bad health payload",
    }

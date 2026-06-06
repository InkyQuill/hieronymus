from __future__ import annotations

import os
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_manager import ServiceManager
from hieronymus.service_state import ServerState, read_server_state, write_server_state


class FakeClient:
    def __init__(self, healthy: bool) -> None:
        self.healthy = healthy
        self.shutdown_called = False

    def health(self, state: ServerState) -> dict[str, object]:
        if not self.healthy:
            raise OSError("connection refused")
        return {"ok": True, "service": "hieronymus"}

    def status(self, state: ServerState) -> dict[str, object]:
        return {"running": True, "pid": state.pid}

    def shutdown(self, state: ServerState) -> dict[str, object]:
        self.shutdown_called = True
        return {"ok": True, "stopping": True}


def test_manager_uses_planned_startup_timeout(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    assert manager.startup_timeout == 5.0


def test_status_reports_not_running_without_state(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    status = manager.status()

    assert status["running"] is False
    assert status["reason"] == "no-state"


def test_status_uses_existing_healthy_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=os.getpid(),
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=True))

    status = manager.status()

    assert status["running"] is True
    assert status["pid"] == os.getpid()


def test_stop_without_state_is_clean_result(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    result = manager.stop()

    assert result == {"running": False, "stopped": False, "reason": "not-running"}


def test_stop_calls_shutdown_for_existing_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=12345,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    write_server_state(config, state)
    client = FakeClient(healthy=True)
    manager = ServiceManager(config, client=client)

    result = manager.stop()

    assert client.shutdown_called is True
    assert result["stopped"] is True
    assert read_server_state(config) is None

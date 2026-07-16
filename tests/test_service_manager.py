from __future__ import annotations

import os
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClientError
from hieronymus.service_manager import ServiceManager
from hieronymus.service_state import ServerState, read_server_state, write_server_state


def server_state(
    config: HieronymusConfig,
    *,
    pid: int = 12345,
    token: str = "local-test-token",
) -> ServerState:
    return ServerState(
        pid=pid,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token=token,
    )


class FakeClient:
    def __init__(self, healthy: bool) -> None:
        self.healthy = healthy
        self.shutdown_called = False
        self.status_calls = 0

    def health(self, state: ServerState) -> dict[str, object]:
        if not self.healthy:
            raise OSError("connection refused")
        return {"ok": True, "service": "hieronymus"}

    def status(self, state: ServerState) -> dict[str, object]:
        self.status_calls += 1
        if not self.healthy:
            raise OSError("connection refused")
        return {"running": True, "pid": state.pid}

    def shutdown(self, state: ServerState) -> dict[str, object]:
        self.shutdown_called = True
        return {"ok": True, "stopping": True}


class BadStatusClient(FakeClient):
    def __init__(self) -> None:
        super().__init__(healthy=False)

    def status(self, state: ServerState) -> dict[str, object]:
        raise ServiceClientError("bad status payload")


class ReplacingShutdownClient(FakeClient):
    def __init__(self, config: HieronymusConfig, replacement: ServerState) -> None:
        super().__init__(healthy=True)
        self.config = config
        self.replacement = replacement

    def shutdown(self, state: ServerState) -> dict[str, object]:
        self.shutdown_called = True
        write_server_state(self.config, self.replacement)
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
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=True))

    status = manager.status()

    assert status["running"] is True
    assert status["pid"] == os.getpid()


def test_status_removes_unreachable_live_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=False))

    status = manager.status()

    assert status == {"running": False, "reason": "unreachable"}
    assert read_server_state(config) is None


def test_status_removes_bad_service_client_payload_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=BadStatusClient())

    status = manager.status()

    assert status == {"running": False, "reason": "unreachable"}
    assert read_server_state(config) is None


def test_start_returns_without_spawning_when_service_is_healthy(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=True))

    with patch("hieronymus.service_manager.subprocess.Popen") as popen:
        manager.start()

    popen.assert_not_called()


def test_ensure_running_uses_health_without_requesting_full_status(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    client = FakeClient(healthy=True)
    manager = ServiceManager(config, client=client)

    result = manager.ensure_running()

    assert result["started"] is False
    assert result["status"]["pid"] == os.getpid()
    assert client.status_calls == 0


def test_start_rechecks_status_after_acquiring_start_lock(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    manager = ServiceManager(config, client=FakeClient(healthy=True))
    lock_was_held = False

    @contextmanager
    def fake_start_lock(lock_config: HieronymusConfig) -> Iterator[None]:
        nonlocal lock_was_held
        assert lock_config == config
        lock_was_held = True
        write_server_state(config, state)
        yield

    with (
        patch("hieronymus.service_manager.server_start_lock", fake_start_lock),
        patch("hieronymus.service_manager.subprocess.Popen") as popen,
    ):
        manager.start()

    assert lock_was_held is True
    popen.assert_not_called()


def test_stop_without_state_is_clean_result(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    result = manager.stop()

    assert result == {"running": False, "stopped": False, "reason": "not-running"}


def test_stop_calls_shutdown_for_existing_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config)
    write_server_state(config, state)
    client = FakeClient(healthy=True)
    manager = ServiceManager(config, client=client)

    result = manager.stop()

    assert client.shutdown_called is True
    assert result["stopped"] is True
    assert read_server_state(config) is None


def test_stop_preserves_newer_state_written_during_shutdown(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    old_state = server_state(config, pid=11111, token="old-token")
    new_state = server_state(config, pid=22222, token="new-token")
    write_server_state(config, old_state)
    client = ReplacingShutdownClient(config, new_state)
    manager = ServiceManager(config, client=client)

    result = manager.stop()

    assert client.shutdown_called is True
    assert result["stopped"] is True
    assert read_server_state(config) == new_state

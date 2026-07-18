from __future__ import annotations

import os
import signal
import socket
import subprocess
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClientError
from hieronymus.service_manager import ServiceManager
from hieronymus.service_state import (
    ServerState,
    process_start_identity,
    read_server_state,
    write_server_state,
)


def daemon_log_path(config: HieronymusConfig) -> Path:
    return config.data_root / "daemon.log"


def server_state(
    config: HieronymusConfig,
    *,
    pid: int = 12345,
) -> ServerState:
    return ServerState(
        pid=pid,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        launch_id="test-launch",
    )


class FakeClient:
    def __init__(self, healthy: bool) -> None:
        self.healthy = healthy
        self.shutdown_called = False
        self.status_calls = 0

    def health(self, state: ServerState) -> dict[str, object]:
        if not self.healthy:
            raise OSError("connection refused")
        return {
            "ok": True,
            "service": "hieronymus",
            "pid": state.pid,
            "data_root": state.data_root,
            "database_path": state.database_path,
            "launch_id": state.launch_id,
        }

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

    assert manager.startup_timeout == 10.0


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
    assert read_server_state(config) == state


def test_status_removes_bad_service_client_payload_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=BadStatusClient())

    status = manager.status()

    assert status == {"running": False, "reason": "unreachable"}
    assert read_server_state(config) == state


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


def test_ensure_running_state_returns_the_healthy_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    write_server_state(config, state)
    manager = ServiceManager(config, client=FakeClient(healthy=True))

    assert manager.ensure_running_state() == state


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

    assert result == {
        "running": False,
        "stopped": False,
        "stop_status": "stopped",
        "reason": "not-running",
    }


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
    old_state = server_state(config, pid=11111)
    new_state = server_state(config, pid=22222)
    write_server_state(config, old_state)
    client = ReplacingShutdownClient(config, new_state)
    manager = ServiceManager(config, client=client)

    result = manager.stop()

    assert client.shutdown_called is True
    assert result["stopped"] is True
    assert read_server_state(config) == new_state


class FakeProcess:
    def __init__(self, *, pid: int = 43210, returncode: int | None = None) -> None:
        self.pid = pid
        self.returncode = returncode

    def poll(self) -> int | None:
        return self.returncode

    def wait(self, timeout: float | None = None) -> int:
        if self.returncode is None:
            raise subprocess.TimeoutExpired("daemon", timeout)
        return self.returncode


def test_start_reports_child_exit_before_state_with_log_path(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    process = FakeProcess(returncode=23)
    manager = ServiceManager(config, client=FakeClient(healthy=False), poll_interval=0.001)

    with patch("hieronymus.service_manager.subprocess.Popen", return_value=process):
        with pytest.raises(RuntimeError) as raised:
            manager.start()

    message = str(raised.value)
    assert "exited before publishing state" in message
    assert "exit code 23" in message
    assert str(daemon_log_path(config)) in message


def test_start_distinguishes_published_but_unhealthy_child_exit(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=43210)
    process = FakeProcess(pid=state.pid)
    manager = ServiceManager(config, client=FakeClient(healthy=False), poll_interval=0.001)

    def publish_then_exit(*_args, **_kwargs) -> FakeProcess:
        write_server_state(config, state)
        process.returncode = 24
        return process

    with patch("hieronymus.service_manager.subprocess.Popen", side_effect=publish_then_exit):
        with pytest.raises(RuntimeError, match="published state but exited"):
            manager.start()

    assert read_server_state(config) is None


def test_start_reports_published_but_unhealthy_timeout(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=43210)
    process = FakeProcess(pid=state.pid)
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=False),
        startup_timeout=0.01,
        poll_interval=0.001,
        terminate_timeout=0.01,
    )

    def publish(*_args, **_kwargs) -> FakeProcess:
        write_server_state(config, state)
        return process

    def terminate_group(pid: int, sent_signal: int) -> None:
        assert (pid, sent_signal) == (process.pid, signal.SIGTERM)
        process.returncode = -sent_signal

    with (
        patch("hieronymus.service_manager.subprocess.Popen", side_effect=publish),
        patch("hieronymus.service_manager.os.killpg", side_effect=terminate_group),
    ):
        with pytest.raises(
            RuntimeError,
            match="published state but did not become healthy before startup timeout",
        ):
            manager.start()

    assert read_server_state(config) is None


def test_start_timeout_terminates_owned_process_group_and_reports_log(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    process = FakeProcess()
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=False),
        startup_timeout=0.01,
        poll_interval=0.001,
        terminate_timeout=0.01,
        kill_timeout=0.01,
    )

    def terminate_group(pid: int, sent_signal: int) -> None:
        assert pid == process.pid
        assert sent_signal in {signal.SIGTERM, signal.SIGKILL}
        process.returncode = -sent_signal

    with (
        patch("hieronymus.service_manager.subprocess.Popen", return_value=process),
        patch("hieronymus.service_manager.os.killpg", side_effect=terminate_group) as killpg,
    ):
        with pytest.raises(RuntimeError) as raised:
            manager.start()

    assert "timed out before publishing state" in str(raised.value)
    assert str(daemon_log_path(config)) in str(raised.value)
    killpg.assert_called_once_with(process.pid, signal.SIGTERM)


def test_start_on_occupied_configured_port_fails_without_fallback(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    with socket.socket() as occupied:
        occupied.bind(("127.0.0.1", 0))
        port = occupied.getsockname()[1]
        (config.config_root / "service.conf").write_text(
            f"[service]\nport = {port}\n", encoding="utf-8"
        )
        manager = ServiceManager(config, startup_timeout=3, poll_interval=0.02)

        with pytest.raises(RuntimeError) as raised:
            manager.start()

    assert "exited" in str(raised.value)
    assert str(daemon_log_path(config)) in str(raised.value)
    assert read_server_state(config) is None
    assert f"{port}" in daemon_log_path(config).read_text(encoding="utf-8")


def _start_real_daemon_with_reserved_port(
    data_root: Path, *, attempts: int = 5
) -> tuple[ServiceManager, ServerState]:
    data_root.mkdir(exist_ok=True)
    last_error: RuntimeError | None = None
    for _ in range(attempts):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
            (data_root / "service.conf").write_text(f"[service]\nport = {port}\n", encoding="utf-8")
        manager = ServiceManager(
            HieronymusConfig(data_root=data_root), startup_timeout=3, poll_interval=0.02
        )
        try:
            manager.start()
        except RuntimeError as error:
            last_error = error
            continue
        state = read_server_state(manager.config)
        assert state is not None
        return manager, state
    raise AssertionError(f"daemon startup failed after {attempts} attempts: {last_error}")


def test_start_rejects_older_healthy_daemon_when_spawned_child_loses_bind(
    tmp_path: Path,
) -> None:
    old_manager, old_state = _start_real_daemon_with_reserved_port(tmp_path / "old")
    new_config = HieronymusConfig(data_root=tmp_path / "new")
    new_config.data_root.mkdir()
    (new_config.config_root / "service.conf").write_text(
        f"[service]\nport = {old_state.port}\n", encoding="utf-8"
    )
    new_manager = ServiceManager(new_config, startup_timeout=3, poll_interval=0.02)
    try:
        with pytest.raises(RuntimeError) as raised:
            new_manager.start()

        assert "exited" in str(raised.value)
        assert str(daemon_log_path(new_config)) in str(raised.value)
        assert read_server_state(new_config) is None
        assert old_manager.status()["running"] is True
    finally:
        result = old_manager.stop()
        assert result["stop_status"] in {"stopped", "forced"}


def _spawn_signal_test_process(*, ignore_term: bool) -> subprocess.Popen[str]:
    handler = "signal.signal(signal.SIGTERM, signal.SIG_IGN);" if ignore_term else ""
    process = subprocess.Popen(
        [
            sys.executable,
            "-c",
            f"import signal,time;{handler}print('ready',flush=True);time.sleep(30)",
        ],
        stdout=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    assert process.stdout is not None
    assert process.stdout.readline() == "ready\n"
    return process


def test_stop_forces_matching_process_that_ignores_term(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    process = _spawn_signal_test_process(ignore_term=True)
    try:
        state = server_state(config, pid=process.pid)
        identity = process_start_identity(process.pid)
        if identity is None:
            pytest.skip("safe signal escalation requires Linux procfs process identity")
        state = replace(state, process_identity=identity)
        write_server_state(config, state)
        manager = ServiceManager(
            config,
            client=FakeClient(healthy=True),
            shutdown_timeout=0.03,
            terminate_timeout=0.03,
            kill_timeout=1,
            poll_interval=0.005,
        )

        result = manager.stop()

        assert result["stop_status"] == "forced"
        assert result["stopped"] is True
        assert process.wait(timeout=2) == -signal.SIGKILL
        assert read_server_state(config) is None
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)


def test_stop_never_signals_or_removes_mismatched_process_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    process = _spawn_signal_test_process(ignore_term=False)
    try:
        state = server_state(config, pid=process.pid)
        state = replace(state, process_identity="different-process")
        write_server_state(config, state)
        manager = ServiceManager(
            config,
            client=FakeClient(healthy=True),
            shutdown_timeout=0.01,
            poll_interval=0.002,
        )

        with patch("hieronymus.service_manager.os.killpg") as killpg:
            result = manager.stop()

        assert result["stop_status"] == "failed"
        assert result["reason"] == "process-identity-changed"
        assert process.poll() is None
        assert read_server_state(config) == state
        killpg.assert_not_called()
    finally:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=2)


def test_restart_aborts_after_failed_stop(tmp_path: Path) -> None:
    manager = ServiceManager(HieronymusConfig(data_root=tmp_path / "hieronymus"))
    failed = {"running": True, "stopped": False, "stop_status": "failed"}

    with (
        patch.object(manager, "stop", return_value=failed),
        patch.object(manager, "start") as start,
    ):
        result = manager.restart()

    assert result == {"stopped": failed, "status": None}
    start.assert_not_called()


def test_health_status_rejects_responder_that_does_not_attest_state(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config, pid=os.getpid())
    state = replace(state, launch_id="expected-launch")
    write_server_state(config, state)

    class OtherDaemonClient(FakeClient):
        def health(self, state: ServerState) -> dict[str, object]:
            return {
                "ok": True,
                "service": "hieronymus",
                "pid": state.pid + 1,
                "data_root": state.data_root,
                "database_path": state.database_path,
                "launch_id": "other-launch",
            }

    status = ServiceManager(config, client=OtherDaemonClient(healthy=True)).status()

    assert status == {"running": False, "reason": "identity-mismatch"}


def test_stop_waits_for_attested_graceful_exit_without_procfs(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config)
    state = replace(state, launch_id="portable-launch")
    write_server_state(config, state)
    running = iter([True, False])
    manager = ServiceManager(config, client=FakeClient(healthy=True), poll_interval=0)

    with (
        patch("hieronymus.service_manager.is_pid_running", side_effect=lambda _pid: next(running)),
        patch("hieronymus.service_manager.process_identity_status", return_value="unavailable"),
        patch("hieronymus.service_manager.os.killpg") as killpg,
    ):
        result = manager.stop()

    assert result["stop_status"] == "stopped"
    killpg.assert_not_called()


def test_stop_fails_safely_when_procfs_unavailable_and_process_is_unresponsive(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config)
    state = replace(state, launch_id="portable-launch")
    write_server_state(config, state)
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=False),
        shutdown_timeout=0.005,
        poll_interval=0.001,
    )

    with (
        patch("hieronymus.service_manager.is_pid_running", return_value=True),
        patch("hieronymus.service_manager.process_identity_status", return_value="unavailable"),
        patch("hieronymus.service_manager.os.kill") as kill_process,
        patch("hieronymus.service_manager.os.killpg") as kill_group,
    ):
        result = manager.stop()

    assert result["stop_status"] == "failed"
    assert result["reason"] == "process-identity-unavailable"
    kill_process.assert_not_called()
    kill_group.assert_not_called()


def test_stop_revalidates_pid_identity_after_grace_period_before_sigterm(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config)
    state = replace(state, launch_id="launch")
    write_server_state(config, state)
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=True),
        shutdown_timeout=0.005,
        poll_interval=0.001,
    )

    with (
        patch("hieronymus.service_manager.is_pid_running", return_value=True),
        patch(
            "hieronymus.service_manager.process_identity_status",
            return_value="mismatch",
        ),
        patch("hieronymus.service_manager.os.getpgid", return_value=state.pid),
        patch("hieronymus.service_manager.os.kill") as kill_process,
        patch("hieronymus.service_manager.os.killpg") as kill_group,
    ):
        result = manager.stop()

    assert result["stop_status"] == "failed"
    assert result["reason"] == "process-identity-changed"
    kill_process.assert_not_called()
    kill_group.assert_not_called()


def test_stop_revalidates_pid_identity_immediately_before_sigkill(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = server_state(config)
    write_server_state(config, state)
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=True),
        shutdown_timeout=0.002,
        terminate_timeout=0.002,
        poll_interval=0.001,
    )

    with (
        patch("hieronymus.service_manager.is_pid_running", return_value=True),
        patch(
            "hieronymus.service_manager.process_identity_status",
            side_effect=["match", "match", "mismatch"],
        ),
        patch("hieronymus.service_manager.os.getpgid", return_value=state.pid),
        patch("hieronymus.service_manager.os.killpg") as kill_group,
    ):
        result = manager.stop()

    assert result["stop_status"] == "failed"
    assert result["reason"] == "process-identity-changed"
    kill_group.assert_called_once_with(state.pid, signal.SIGTERM)


def test_startup_cleanup_preserves_original_error_when_child_exits_before_signal(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    process = FakeProcess()
    manager = ServiceManager(
        config,
        client=FakeClient(healthy=False),
        startup_timeout=0.005,
        poll_interval=0.001,
    )

    with (
        patch("hieronymus.service_manager.subprocess.Popen", return_value=process),
        patch("hieronymus.service_manager.os.killpg", side_effect=ProcessLookupError),
    ):
        with pytest.raises(RuntimeError) as raised:
            manager.start()

    assert "startup timed out" in str(raised.value)
    assert str(daemon_log_path(config)) in str(raised.value)

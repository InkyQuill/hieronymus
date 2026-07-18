from __future__ import annotations

import asyncio
import logging
import os
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from unittest.mock import MagicMock, patch

import hieronymus.service_daemon as service_daemon
from hieronymus.config import HieronymusConfig
from hieronymus.service_daemon import DreamAutostartScheduler
from hieronymus.service_state import read_server_state


def test_dream_autostart_scheduler_runs_repeatedly(config: HieronymusConfig) -> None:
    calls = 0
    called_twice = threading.Event()

    class Autostart:
        def __init__(self, _config: HieronymusConfig) -> None:
            pass

        def run_due(self) -> None:
            nonlocal calls
            calls += 1
            if calls >= 2:
                called_twice.set()

    scheduler = DreamAutostartScheduler(config, interval_seconds=0.01, autostart_cls=Autostart)
    scheduler.start()
    try:
        assert called_twice.wait(timeout=1)
    finally:
        scheduler.stop()

    assert calls >= 2


def test_dream_autostart_scheduler_survives_run_due_errors(
    config: HieronymusConfig,
    caplog,
) -> None:
    calls = 0
    recovered = threading.Event()

    class Autostart:
        def __init__(self, _config: HieronymusConfig) -> None:
            pass

        def run_due(self) -> None:
            nonlocal calls
            calls += 1
            if calls == 1:
                raise RuntimeError("autostart failed")
            recovered.set()

    with caplog.at_level(logging.ERROR, logger="hieronymus.service_daemon"):
        scheduler = DreamAutostartScheduler(config, interval_seconds=0.01, autostart_cls=Autostart)
        scheduler.start()
        try:
            assert recovered.wait(timeout=1)
        finally:
            scheduler.stop()

    assert calls >= 2
    assert "Dream autostart run_due failed" in caplog.text
    assert "autostart failed" in caplog.text


def test_dream_autostart_scheduler_stop_waits_for_in_flight_run_due(
    config: HieronymusConfig,
) -> None:
    entered_run_due = threading.Event()
    release_run_due = threading.Event()
    exited_run_due = threading.Event()
    stop_completed = threading.Event()

    class Autostart:
        def __init__(self, _config: HieronymusConfig) -> None:
            pass

        def run_due(self) -> None:
            entered_run_due.set()
            assert release_run_due.wait(timeout=5)
            exited_run_due.set()

    scheduler = DreamAutostartScheduler(config, interval_seconds=0.01, autostart_cls=Autostart)
    scheduler.start()
    try:
        assert entered_run_due.wait(timeout=1)

        stop_thread = threading.Thread(target=lambda: (scheduler.stop(), stop_completed.set()))
        stop_thread.start()
        try:
            assert not stop_completed.wait(timeout=1.2)
            assert not exited_run_due.is_set()

            release_run_due.set()
            stop_thread.join(timeout=1)

            assert stop_completed.is_set()
            assert exited_run_due.is_set()
            assert not scheduler.is_alive()
        finally:
            release_run_due.set()
            stop_thread.join(timeout=1)
    finally:
        release_run_due.set()
        scheduler.stop()


def test_dream_autostart_scheduler_stop_has_deadline_for_stuck_work(
    config: HieronymusConfig, caplog
) -> None:
    entered = threading.Event()
    release = threading.Event()

    class Autostart:
        def __init__(self, _config: HieronymusConfig) -> None:
            pass

        def run_due(self) -> None:
            entered.set()
            release.wait(timeout=2)

    scheduler = DreamAutostartScheduler(
        config, interval_seconds=0.01, join_timeout=0.02, autostart_cls=Autostart
    )
    scheduler.start()
    assert entered.wait(timeout=1)
    started = time.monotonic()
    try:
        with caplog.at_level(logging.ERROR, logger="hieronymus.service_daemon"):
            stopped = scheduler.stop()
        assert time.monotonic() - started < 0.5
        assert stopped is False
        assert "did not stop within" in caplog.text
    finally:
        release.set()
        scheduler.stop()


def test_shutdown_coordinator_is_idempotent_across_request_and_finalize(
    config: HieronymusConfig,
) -> None:
    server = type("Server", (), {"should_exit": False})()
    scheduler = type("Scheduler", (), {"stop": MagicMock(return_value=True)})()
    coordinator = service_daemon.ShutdownCoordinator(config, server, scheduler, None)

    coordinator.request("http")
    coordinator.request("sigterm")
    coordinator.finish("uvicorn")
    coordinator.finish("uvicorn")

    assert server.should_exit is True
    scheduler.stop.assert_called_once()


def test_coordinated_server_preserves_uvicorn_repeated_sigint_semantics(
    config: HieronymusConfig,
) -> None:
    server = service_daemon.CoordinatedServer(service_daemon.uvicorn.Config(lambda *_: None))
    scheduler = type("Scheduler", (), {"stop": MagicMock(return_value=True)})()
    coordinator = service_daemon.ShutdownCoordinator(config, server, scheduler, None)
    server.shutdown_coordinator = coordinator

    server.handle_exit(signal.SIGINT, None)
    server.handle_exit(signal.SIGINT, None)

    assert server._captured_signals == [signal.SIGINT, signal.SIGINT]
    assert server.should_exit is True
    assert server.force_exit is True


def test_coordinated_server_bounds_uvicorn_lifespan_shutdown(
    config: HieronymusConfig, caplog
) -> None:
    server = service_daemon.CoordinatedServer(service_daemon.uvicorn.Config(lambda *_: None))
    scheduler = type("Scheduler", (), {"stop": MagicMock(return_value=True)})()
    coordinator = service_daemon.ShutdownCoordinator(config, server, scheduler, None)
    server.shutdown_coordinator = coordinator
    coordinator.request("test")
    coordinator._deadline = time.monotonic() + 0.01

    async def stuck_shutdown(*_args, **_kwargs) -> None:
        await asyncio.sleep(1)

    started = time.monotonic()
    with (
        patch.object(service_daemon.uvicorn.Server, "shutdown", new=stuck_shutdown),
        caplog.at_level(logging.ERROR, logger="hieronymus.service_daemon"),
    ):
        asyncio.run(server.shutdown())

    assert time.monotonic() - started < 0.5
    assert server.force_exit is True
    assert "lifespan shutdown exceeded" in caplog.text


def _start_daemon_with_reserved_port(
    data_root: Path, *, attempts: int = 5
) -> tuple[subprocess.Popen[str], int]:
    data_root.mkdir(exist_ok=True)
    for _ in range(attempts):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
            (data_root / "service.conf").write_text(f"[service]\nport = {port}\n", encoding="utf-8")
            process = subprocess.Popen(
                [
                    sys.executable,
                    "-m",
                    "hieronymus.service_daemon",
                    "--data-root",
                    str(data_root),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and process.poll() is None:
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/health", timeout=0.1
                ) as response:
                    if response.status == 200:
                        return process, port
            except (OSError, urllib.error.URLError):
                time.sleep(0.02)
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=2)
    raise AssertionError(f"daemon did not start after {attempts} reserved-port attempts")


def test_daemon_sigterm_removes_state_and_exits(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    process, _port = _start_daemon_with_reserved_port(data_root)
    config = HieronymusConfig(data_root=data_root)
    try:
        deadline = time.monotonic() + 5
        while read_server_state(config) is None and process.poll() is None:
            assert time.monotonic() < deadline
            time.sleep(0.02)
        assert read_server_state(config) is not None

        os.kill(process.pid, signal.SIGTERM)

        assert process.wait(timeout=5) == -signal.SIGTERM
        assert read_server_state(config) is None
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)


def test_daemon_logging_rotates_by_size_and_count(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    script = """
import logging
import sys
from pathlib import Path
from hieronymus.config import HieronymusConfig
from hieronymus.service_logging import configure_daemon_logging

config = HieronymusConfig(data_root=Path(sys.argv[1]))
configure_daemon_logging(config)
for _ in range(220):
    logging.getLogger("rotation-test").info("x" * 20_000)
"""

    subprocess.run(
        [sys.executable, "-c", script, str(data_root)],
        check=True,
        timeout=10,
    )

    logs = sorted(data_root.glob("daemon.log*"))
    assert [path.name for path in logs] == [
        "daemon.log",
        "daemon.log.1",
        "daemon.log.2",
        "daemon.log.3",
    ]
    assert all(path.stat().st_size <= 1_100_000 for path in logs)

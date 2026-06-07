from __future__ import annotations

import threading

from hieronymus.config import HieronymusConfig
from hieronymus.service_daemon import DreamAutostartScheduler


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

    scheduler = DreamAutostartScheduler(config, interval_seconds=0.01, autostart_cls=Autostart)
    scheduler.start()
    try:
        assert recovered.wait(timeout=1)
    finally:
        scheduler.stop()

    assert calls >= 2


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

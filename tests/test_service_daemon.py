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

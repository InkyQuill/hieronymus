from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
from datetime import UTC, datetime, timedelta

import pytest

from hieronymus.dream_locks import (
    DreamCycleAlreadyRunning,
    DreamCycleState,
    dream_cycle_lock,
    dream_cycle_paths,
    read_dream_cycle_state,
)


def test_dream_cycle_lock_acquires_and_releases(config):
    with dream_cycle_lock(config, owner="manual") as state:
        assert state.owner == "manual"
        assert state.pid == os.getpid()
        assert read_dream_cycle_state(config).owner == "manual"

    assert read_dream_cycle_state(config) is None


def test_dream_cycle_lock_accepts_positional_owner(config):
    with dream_cycle_lock(config, "manual") as state:
        assert state.owner == "manual"


def test_second_dream_cycle_lock_fails_while_active(config):
    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(DreamCycleAlreadyRunning, match="dream cycle already running"):
            with dream_cycle_lock(config, owner="autostart"):
                raise AssertionError("second lock must not acquire")


def test_dream_cycle_lock_releases_after_exception(config):
    with pytest.raises(RuntimeError, match="provider failed"):
        with dream_cycle_lock(config, owner="manual"):
            raise RuntimeError("provider failed")

    with dream_cycle_lock(config, owner="manual") as state:
        assert state.owner == "manual"


def test_dream_cycle_lock_wait_blocks_until_release(config):
    entered_wait = threading.Event()
    acquired = threading.Event()

    def wait_for_lock() -> None:
        entered_wait.set()
        with dream_cycle_lock(config, "worker", wait=True) as state:
            assert state.owner == "worker"
            acquired.set()

    with dream_cycle_lock(config, owner="manual"):
        thread = threading.Thread(target=wait_for_lock)
        thread.start()
        assert entered_wait.wait(timeout=1)
        assert not acquired.wait(timeout=0.05)

    thread.join(timeout=1)
    assert not thread.is_alive()
    assert acquired.is_set()


def test_dream_cycle_lock_fails_across_processes(config):
    with dream_cycle_lock(config, owner="manual"):
        result = subprocess.run(
            [
                sys.executable,
                "-c",
                (
                    "from pathlib import Path\n"
                    "from hieronymus.config import HieronymusConfig\n"
                    "from hieronymus.dream_locks import "
                    "DreamCycleAlreadyRunning, dream_cycle_lock\n"
                    f"config = HieronymusConfig(data_root=Path({str(config.data_root)!r}))\n"
                    "try:\n"
                    "    with dream_cycle_lock(config, 'subprocess'):\n"
                    "        print('acquired')\n"
                    "except DreamCycleAlreadyRunning as error:\n"
                    "    print(str(error))\n"
                    "    raise SystemExit(2)\n"
                ),
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    assert result.returncode == 2
    assert "dream cycle already running" in result.stdout


def test_stale_state_with_dead_pid_is_cleaned_conservatively(config):
    paths = dream_cycle_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    paths.state_json.write_text(
        (
            '{"owner":"manual","pid":-1,"started_at":"'
            + (datetime.now(UTC) - timedelta(hours=1)).isoformat()
            + '","token":"stale"}'
        ),
        encoding="utf-8",
    )

    assert read_dream_cycle_state(config) is None
    assert not paths.state_json.exists()


def test_stale_cleanup_does_not_remove_replaced_state(config, monkeypatch):
    paths = dream_cycle_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    stale = DreamCycleState(
        owner="manual",
        pid=-1,
        started_at=(datetime.now(UTC) - timedelta(hours=1)).isoformat(),
        token="stale",
    )
    fresh = DreamCycleState(
        owner="autostart",
        pid=os.getpid(),
        started_at=datetime.now(UTC).isoformat(),
        token="fresh",
    )
    paths.state_json.write_text(
        json.dumps(stale.to_json_dict(), sort_keys=True),
        encoding="utf-8",
    )

    calls = 0

    def replace_state_after_stale_check(pid: int) -> bool:
        nonlocal calls
        calls += 1
        if calls == 1:
            paths.state_json.write_text(
                json.dumps(fresh.to_json_dict(), sort_keys=True),
                encoding="utf-8",
            )
        return pid > 0

    monkeypatch.setattr(
        "hieronymus.dream_locks.is_pid_running",
        replace_state_after_stale_check,
    )

    assert read_dream_cycle_state(config) is None
    assert json.loads(paths.state_json.read_text(encoding="utf-8"))["token"] == "fresh"

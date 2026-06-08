from __future__ import annotations

import os
import time
from datetime import UTC, datetime, timedelta

import pytest

from hieronymus.dream_locks import (
    DreamCycleAlreadyRunning,
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
    with dream_cycle_lock(config, owner="manual"):
        started = time.monotonic()
        with pytest.raises(DreamCycleAlreadyRunning):
            with dream_cycle_lock(config, owner="manual", wait=False):
                pass
        assert time.monotonic() - started < 1


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

from __future__ import annotations

import fcntl
import os
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import (
    ServerState,
    allocate_loopback_port,
    cleanup_stale_state,
    is_pid_running,
    read_server_state,
    remove_server_state,
    runtime_paths,
    server_start_lock,
    write_server_state,
)


def test_runtime_paths_stay_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    paths = runtime_paths(config)

    assert paths.config_root == tmp_path / "hieronymus"
    assert paths.server_json == tmp_path / "hieronymus" / "server.json"
    assert paths.server_pid == tmp_path / "hieronymus" / "server.pid"
    assert paths.server_lock == tmp_path / "hieronymus" / "server.lock"


def test_server_state_round_trips_as_json(tmp_path: Path) -> None:
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

    assert read_server_state(config) == state


def test_cleanup_stale_state_removes_dead_pid_files(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = ServerState(
        pid=99999999,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )
    paths = runtime_paths(config)
    write_server_state(config, state)
    paths.server_lock.write_text("99999999", encoding="utf-8")

    removed = cleanup_stale_state(config)

    assert removed is True
    assert read_server_state(config) is None
    assert not paths.server_pid.exists()
    assert paths.server_lock.exists()


def test_remove_server_state_preserves_different_owner(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    old_state = ServerState(
        pid=11111,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="old-token",
    )
    new_state = ServerState(
        pid=22222,
        host="127.0.0.1",
        port=32200,
        version="0.1.0",
        started_at="2026-06-06T12:00:01Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="new-token",
    )
    paths = runtime_paths(config)
    write_server_state(config, new_state)
    paths.server_lock.write_text("22222", encoding="utf-8")

    remove_server_state(config, expected_state=old_state)

    assert read_server_state(config) == new_state
    assert paths.server_pid.exists()
    assert paths.server_lock.exists()


def test_remove_server_state_removes_matching_owner(tmp_path: Path) -> None:
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
    paths = runtime_paths(config)
    write_server_state(config, state)
    paths.server_lock.write_text("12345", encoding="utf-8")

    remove_server_state(config, expected_state=state)

    assert read_server_state(config) is None
    assert not paths.server_pid.exists()
    assert paths.server_lock.exists()


def test_server_start_lock_holds_advisory_lock_without_unlinking(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    paths = runtime_paths(config)

    with server_start_lock(config), paths.server_lock.open("a+", encoding="utf-8") as lock_file:
        try:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            blocked = True
        else:
            blocked = False
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    assert blocked is True
    assert paths.server_lock.exists()


def test_current_process_pid_is_running() -> None:
    assert is_pid_running(os.getpid()) is True


def test_allocate_loopback_port_returns_connectable_port_number() -> None:
    port = allocate_loopback_port()

    assert 1024 < port < 65536

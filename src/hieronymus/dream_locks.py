from __future__ import annotations

import functools
import json
import os
import threading
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import is_pid_running

try:
    import fcntl
except ImportError:  # pragma: no cover - exercised only on non-Unix platforms.
    fcntl = None


@dataclass(frozen=True)
class DreamCyclePaths:
    config_root: Path
    lock_file: Path
    state_json: Path


@dataclass(frozen=True)
class DreamCycleState:
    owner: str
    pid: int
    started_at: str
    token: str

    def to_json_dict(self) -> dict[str, object]:
        return asdict(self)

    @classmethod
    def from_json_dict(cls, payload: dict[str, object]) -> DreamCycleState:
        return cls(
            owner=str(payload["owner"]),
            pid=int(payload["pid"]),
            started_at=str(payload["started_at"]),
            token=str(payload["token"]),
        )


class DreamCycleAlreadyRunning(ValueError):
    def __init__(self, state: DreamCycleState | None = None) -> None:
        self.state = state
        detail = f" by {state.owner} pid {state.pid}" if state is not None else ""
        super().__init__(f"dream cycle already running{detail}")


class PlatformNotSupportedError(RuntimeError):
    """Raised when dream-cycle file locking is unavailable on this platform."""


def dream_cycle_paths(config: HieronymusConfig) -> DreamCyclePaths:
    root = config.config_root
    return DreamCyclePaths(
        config_root=root,
        lock_file=root / "dream-cycle.lock",
        state_json=root / "dream-cycle.json",
    )


@functools.lru_cache(maxsize=256)
def _local_lock(path: Path) -> threading.Lock:
    return threading.Lock()


def _lock_file(file_descriptor: int, *, wait: bool) -> None:
    if fcntl is None:
        raise PlatformNotSupportedError(
            "dream cycle locks require Unix fcntl file locking on this platform"
        )
    flags = fcntl.LOCK_EX if wait else fcntl.LOCK_EX | fcntl.LOCK_NB
    fcntl.flock(file_descriptor, flags)


def _unlock_file(file_descriptor: int) -> None:
    if fcntl is None:
        return
    fcntl.flock(file_descriptor, fcntl.LOCK_UN)


def _write_state(paths: DreamCyclePaths, state: DreamCycleState) -> None:
    tmp = paths.state_json.with_name(f"{paths.state_json.name}.tmp-{os.getpid()}")
    tmp.write_text(
        json.dumps(state.to_json_dict(), ensure_ascii=False, sort_keys=True),
        encoding="utf-8",
    )
    tmp.replace(paths.state_json)


def _read_state_file(paths: DreamCyclePaths) -> DreamCycleState | None:
    try:
        payload = json.loads(paths.state_json.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    try:
        return DreamCycleState.from_json_dict(payload)
    except (KeyError, TypeError, ValueError):
        return None


def read_dream_cycle_state(config: HieronymusConfig) -> DreamCycleState | None:
    paths = dream_cycle_paths(config)
    state = _read_state_file(paths)
    if state is None:
        return None
    if is_pid_running(state.pid):
        return state
    _remove_state_if_unchanged(paths, state)
    return None


def _remove_state_if_unchanged(
    paths: DreamCyclePaths,
    expected: DreamCycleState,
) -> None:
    current = _read_state_file(paths)
    if current is None or current.token != expected.token or current.pid != expected.pid:
        return
    try:
        paths.state_json.unlink()
    except FileNotFoundError:
        pass


@contextmanager
def dream_cycle_lock(
    config: HieronymusConfig,
    owner: str,
    *,
    wait: bool = False,
) -> Iterator[DreamCycleState]:
    paths = dream_cycle_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    local_lock = _local_lock(paths.lock_file)
    local_acquired = local_lock.acquire(blocking=wait)
    if not local_acquired:
        raise DreamCycleAlreadyRunning(read_dream_cycle_state(config))

    try:
        lock_file = paths.lock_file.open("a+", encoding="utf-8")
        try:
            state = DreamCycleState(
                owner=owner,
                pid=os.getpid(),
                started_at=datetime.now(UTC).isoformat(),
                token=uuid.uuid4().hex,
            )
            file_locked = False
            try:
                try:
                    _lock_file(lock_file.fileno(), wait=wait)
                    file_locked = True
                except BlockingIOError as error:
                    raise DreamCycleAlreadyRunning(read_dream_cycle_state(config)) from error
                _write_state(paths, state)
                yield state
            finally:
                try:
                    _remove_state_if_unchanged(paths, state)
                finally:
                    if file_locked:
                        _unlock_file(lock_file.fileno())
        finally:
            lock_file.close()
    finally:
        local_lock.release()

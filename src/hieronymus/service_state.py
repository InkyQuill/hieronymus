from __future__ import annotations

import json
import os
import socket
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

from hieronymus.config import HieronymusConfig


@dataclass(frozen=True)
class RuntimePaths:
    config_root: Path
    server_json: Path
    server_pid: Path
    server_lock: Path


@dataclass(frozen=True)
class ServerState:
    pid: int
    host: str
    port: int
    version: str
    started_at: str
    data_root: str
    database_path: str
    token: str

    @property
    def base_url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def to_json_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def from_json_dict(cls, payload: dict[str, Any]) -> ServerState:
        return cls(
            pid=int(payload["pid"]),
            host=str(payload["host"]),
            port=int(payload["port"]),
            version=str(payload["version"]),
            started_at=str(payload["started_at"]),
            data_root=str(payload["data_root"]),
            database_path=str(payload["database_path"]),
            token=str(payload["token"]),
        )


def runtime_paths(config: HieronymusConfig) -> RuntimePaths:
    root = config.config_root
    return RuntimePaths(
        config_root=root,
        server_json=root / "server.json",
        server_pid=root / "server.pid",
        server_lock=root / "server.lock",
    )


def is_pid_running(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def read_server_state(config: HieronymusConfig) -> ServerState | None:
    path = runtime_paths(config).server_json
    if not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    try:
        return ServerState.from_json_dict(payload)
    except (KeyError, TypeError, ValueError):
        return None


def write_server_state(config: HieronymusConfig, state: ServerState) -> None:
    paths = runtime_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    tmp = paths.server_json.with_name(f"{paths.server_json.name}.tmp-{os.getpid()}")
    tmp.write_text(
        json.dumps(state.to_json_dict(), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    tmp.replace(paths.server_json)
    paths.server_pid.write_text(f"{state.pid}\n", encoding="utf-8")


def remove_server_state(config: HieronymusConfig) -> None:
    paths = runtime_paths(config)
    for path in (paths.server_json, paths.server_pid, paths.server_lock):
        try:
            path.unlink()
        except FileNotFoundError:
            pass


def cleanup_stale_state(config: HieronymusConfig) -> bool:
    state = read_server_state(config)
    if state is None:
        return False
    if is_pid_running(state.pid):
        return False
    remove_server_state(config)
    return True


def allocate_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])

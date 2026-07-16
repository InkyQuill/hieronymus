from __future__ import annotations

import subprocess
import sys
import time
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_state import (
    ServerState,
    cleanup_stale_state,
    read_server_state,
    remove_server_state,
    server_start_lock,
)


class ClientProtocol(Protocol):
    def health(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError

    def status(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        raise NotImplementedError


class ServiceManager:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        client: ClientProtocol | None = None,
        startup_timeout: float = 10.0,
        poll_interval: float = 0.1,
    ) -> None:
        self.config = config
        self.client = client if client is not None else ServiceClient()
        self.startup_timeout = startup_timeout
        self.poll_interval = poll_interval

    def status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            self.client.health(state)
            return self.client.status(state)
        except (OSError, ServiceClientError):
            remove_server_state(self.config, expected_state=state)
            return {"running": False, "reason": "unreachable"}

    def ensure_running(self) -> dict[str, Any]:
        current = self._health_status()
        if current.get("running") is True:
            return {"started": False, "status": current}
        self.start()
        return {"started": True, "status": self._health_status()}

    def start(self) -> None:
        current = self._health_status()
        if current.get("running") is True:
            return
        with server_start_lock(self.config):
            current = self._health_status()
            if current.get("running") is True:
                return
            self.config.data_root.mkdir(parents=True, exist_ok=True)
            subprocess.Popen(
                [
                    sys.executable,
                    "-m",
                    "hieronymus.service_daemon",
                    "--data-root",
                    str(self.config.data_root),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            deadline = time.monotonic() + self.startup_timeout
            last_state: ServerState | None = None
            while time.monotonic() < deadline:
                state = read_server_state(self.config)
                if state is not None:
                    last_state = state
                    try:
                        self.client.health(state)
                    except (OSError, ServiceClientError):
                        pass
                    else:
                        return
                time.sleep(self.poll_interval)
            if last_state is not None:
                remove_server_state(self.config, expected_state=last_state)
            raise RuntimeError("hieronymus service daemon did not become healthy")

    def _health_status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            self.client.health(state)
        except (OSError, ServiceClientError):
            remove_server_state(self.config, expected_state=state)
            return {"running": False, "reason": "unreachable"}
        return {
            "running": True,
            "pid": state.pid,
            "host": state.host,
            "port": state.port,
            "version": state.version,
            "data_root": state.data_root,
            "database_path": state.database_path,
        }

    def stop(self) -> dict[str, Any]:
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "stopped": False, "reason": "not-running"}
        try:
            self.client.shutdown(state)
        except (OSError, ServiceClientError):
            remove_server_state(self.config, expected_state=state)
            return {"running": False, "stopped": False, "reason": "unreachable"}
        remove_server_state(self.config, expected_state=state)
        return {"running": False, "stopped": True}

    def restart(self) -> dict[str, Any]:
        stopped = self.stop()
        self.start()
        return {"stopped": stopped, "status": self.status()}

from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_logging import daemon_log_path, open_early_daemon_log
from hieronymus.service_state import (
    ServerState,
    cleanup_stale_state,
    is_pid_running,
    read_server_state,
    remove_server_state,
    server_start_lock,
    state_matches_process,
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
        shutdown_timeout: float = 2.0,
        terminate_timeout: float = 2.0,
        kill_timeout: float = 1.0,
    ) -> None:
        self.config = config
        self.client = client if client is not None else ServiceClient()
        self.startup_timeout = startup_timeout
        self.poll_interval = poll_interval
        self.shutdown_timeout = shutdown_timeout
        self.terminate_timeout = terminate_timeout
        self.kill_timeout = kill_timeout
        self._process: subprocess.Popen[object] | None = None

    def status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            self.client.health(state)
            return self.client.status(state)
        except (OSError, ServiceClientError):
            return {"running": False, "reason": "unreachable"}

    def ensure_running(self) -> dict[str, Any]:
        state, started = self._ensure_running_state()
        return {
            "started": started,
            "status": {
                "running": True,
                "pid": state.pid,
                "host": state.host,
                "port": state.port,
                "version": state.version,
                "data_root": state.data_root,
                "database_path": state.database_path,
            },
        }

    def ensure_running_state(self) -> ServerState:
        return self._ensure_running_state()[0]

    def _ensure_running_state(self) -> tuple[ServerState, bool]:
        state = self._healthy_state()
        if state is not None:
            return state, False
        self.start()
        state = self._healthy_state()
        if state is None:
            raise RuntimeError("hieronymus service daemon did not publish state")
        return state, True

    def start(self) -> None:
        current = self._health_status()
        if current.get("running") is True:
            return
        with server_start_lock(self.config):
            current = self._health_status()
            if current.get("running") is True:
                return
            self.config.data_root.mkdir(parents=True, exist_ok=True)
            existing = read_server_state(self.config)
            if existing is not None and is_pid_running(existing.pid):
                raise RuntimeError(
                    "refusing to replace live daemon state with an unverified process identity"
                )
            self._start_child_and_wait()

    def _start_child_and_wait(self) -> None:
        log_path = daemon_log_path(self.config)
        with open_early_daemon_log(self.config) as early_log:
            process = subprocess.Popen(
                [
                    sys.executable,
                    "-m",
                    "hieronymus.service_daemon",
                    "--data-root",
                    str(self.config.data_root),
                ],
                stdout=early_log,
                stderr=early_log,
                start_new_session=True,
            )
        self._process = process
        deadline = time.monotonic() + self.startup_timeout
        last_state: ServerState | None = None
        while time.monotonic() < deadline:
            state = read_server_state(self.config)
            if state is not None and state.pid == process.pid:
                last_state = state
                try:
                    self.client.health(state)
                except (OSError, ServiceClientError):
                    pass
                else:
                    return
            returncode = process.poll()
            if returncode is not None:
                self._remove_owned_state(process, last_state)
                if last_state is None:
                    detail = "exited before publishing state"
                else:
                    detail = "published state but exited before becoming healthy"
                raise RuntimeError(
                    f"hieronymus service daemon {detail} (exit code {returncode}); "
                    f"see daemon log: {log_path}"
                )
            time.sleep(self.poll_interval)
        self._terminate_owned_child(process)
        self._remove_owned_state(process, last_state)
        if last_state is None:
            detail = "startup timed out before publishing state"
        else:
            detail = "published state but did not become healthy before startup timeout"
        raise RuntimeError(f"hieronymus service daemon {detail}; see daemon log: {log_path}")

    def _remove_owned_state(
        self, process: subprocess.Popen[object], state: ServerState | None
    ) -> None:
        if state is not None and state.pid == process.pid:
            remove_server_state(self.config, expected_state=state)

    def _terminate_owned_child(self, process: subprocess.Popen[object]) -> bool:
        if process.poll() is not None:
            return True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=self.terminate_timeout)
            return True
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            try:
                process.wait(timeout=self.kill_timeout)
            except subprocess.TimeoutExpired:
                return False
            return True

    def _health_status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            self.client.health(state)
        except (OSError, ServiceClientError):
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

    def _healthy_state(self) -> ServerState | None:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return None
        try:
            self.client.health(state)
        except (OSError, ServiceClientError):
            return None
        return state

    def stop(self) -> dict[str, Any]:
        state = read_server_state(self.config)
        if state is None:
            return {
                "running": False,
                "stopped": False,
                "stop_status": "stopped",
                "reason": "not-running",
            }
        try:
            self.client.shutdown(state)
        except (OSError, ServiceClientError):
            pass
        if not is_pid_running(state.pid):
            remove_server_state(self.config, expected_state=state)
            return {"running": False, "stopped": True, "stop_status": "stopped"}
        current = read_server_state(self.config)
        if current is not None and current != state:
            return {
                "running": True,
                "stopped": False,
                "stop_status": "failed",
                "reason": "state-owner-changed",
            }
        if not state_matches_process(self.config, state):
            return {
                "running": True,
                "stopped": False,
                "stop_status": "failed",
                "reason": "process-identity-mismatch",
            }
        if self._wait_until_process_stops(state, self.shutdown_timeout):
            remove_server_state(self.config, expected_state=state)
            return {"running": False, "stopped": True, "stop_status": "stopped"}
        current = read_server_state(self.config)
        if current is not None and current != state:
            return {
                "running": True,
                "stopped": False,
                "stop_status": "failed",
                "reason": "state-owner-changed",
            }
        self._signal_state_process(state, signal.SIGTERM)
        if not self._wait_until_process_stops(state, self.terminate_timeout):
            if not state_matches_process(self.config, state):
                return {
                    "running": True,
                    "stopped": False,
                    "stop_status": "failed",
                    "reason": "process-identity-changed",
                }
            self._signal_state_process(state, signal.SIGKILL)
            if not self._wait_until_process_stops(state, self.kill_timeout):
                return {
                    "running": True,
                    "stopped": False,
                    "stop_status": "failed",
                    "reason": "process-did-not-exit",
                }
        remove_server_state(self.config, expected_state=state)
        return {"running": False, "stopped": True, "stop_status": "forced"}

    def _wait_until_process_stops(self, state: ServerState, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not is_pid_running(state.pid):
                return True
            time.sleep(self.poll_interval)
        return not is_pid_running(state.pid)

    @staticmethod
    def _signal_state_process(state: ServerState, sent_signal: signal.Signals) -> None:
        try:
            process_group = os.getpgid(state.pid)
            if process_group == state.pid:
                os.killpg(process_group, sent_signal)
            else:
                os.kill(state.pid, sent_signal)
        except ProcessLookupError:
            pass

    def restart(self) -> dict[str, Any]:
        stopped = self.stop()
        if stopped.get("stop_status") == "failed":
            return {"stopped": stopped, "status": None}
        self.start()
        return {"stopped": stopped, "status": self.status()}

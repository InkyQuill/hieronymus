from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_deadlines import MANAGER_SHUTDOWN_TIMEOUT
from hieronymus.service_logging import daemon_log_path, open_early_daemon_log
from hieronymus.service_state import (
    ServerState,
    cleanup_stale_state,
    is_pid_running,
    process_identity_status,
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
        shutdown_timeout: float = MANAGER_SHUTDOWN_TIMEOUT,
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
            health = self.client.health(state)
            if not self._health_attests_state(health, state):
                return {"running": False, "reason": "identity-mismatch"}
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
                    health = self.client.health(state)
                except (OSError, ServiceClientError):
                    pass
                else:
                    if self._health_attests_state(health, state):
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
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return self._reap_missing_child(process)
        try:
            process.wait(timeout=self.terminate_timeout)
            return True
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                return self._reap_missing_child(process)
            try:
                process.wait(timeout=self.kill_timeout)
            except subprocess.TimeoutExpired:
                return False
            return True

    def _reap_missing_child(self, process: subprocess.Popen[object]) -> bool:
        try:
            process.wait(timeout=self.kill_timeout)
        except subprocess.TimeoutExpired:
            return process.poll() is not None
        return True

    def _health_status(self) -> dict[str, Any]:
        cleanup_stale_state(self.config)
        state = read_server_state(self.config)
        if state is None:
            return {"running": False, "reason": "no-state"}
        try:
            health = self.client.health(state)
        except (OSError, ServiceClientError):
            return {"running": False, "reason": "unreachable"}
        if not self._health_attests_state(health, state):
            return {"running": False, "reason": "identity-mismatch"}
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
            health = self.client.health(state)
        except (OSError, ServiceClientError):
            return None
        if not self._health_attests_state(health, state):
            return None
        return state

    @staticmethod
    def _health_attests_state(health: dict[str, Any], state: ServerState) -> bool:
        return (
            state.launch_id is not None
            and health.get("pid") == state.pid
            and health.get("data_root") == state.data_root
            and health.get("database_path") == state.database_path
            and health.get("launch_id") == state.launch_id
        )

    def stop(self) -> dict[str, Any]:
        state = read_server_state(self.config)
        if state is None:
            return {
                "running": False,
                "stopped": False,
                "stop_status": "stopped",
                "reason": "not-running",
            }
        attested = False
        health_received = False
        try:
            health = self.client.health(state)
            health_received = True
            attested = self._health_attests_state(health, state)
        except (OSError, ServiceClientError):
            pass
        if health_received and not attested:
            return {
                "running": True,
                "stopped": False,
                "stop_status": "failed",
                "reason": "health-identity-mismatch",
            }
        if not attested:
            health_unavailable = True
        else:
            health_unavailable = False
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
        if health_unavailable:
            identity = process_identity_status(self.config, state)
            if identity != "match":
                return {
                    "running": True,
                    "stopped": False,
                    "stop_status": "failed",
                    "reason": f"process-identity-{identity}",
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
        signal_result = self._signal_state_process(state, signal.SIGTERM)
        if signal_result != "sent":
            if signal_result == "gone":
                remove_server_state(self.config, expected_state=state)
                return {"running": False, "stopped": True, "stop_status": "stopped"}
            reason = (
                "process-identity-unavailable"
                if signal_result == "unavailable"
                else "process-identity-changed"
            )
            if health_unavailable and signal_result == "mismatch":
                reason = "process-identity-mismatch"
            return {
                "running": True,
                "stopped": False,
                "stop_status": "failed",
                "reason": reason,
            }
        if not self._wait_until_process_stops(state, self.terminate_timeout):
            signal_result = self._signal_state_process(state, signal.SIGKILL)
            if signal_result not in {"sent", "gone"}:
                return {
                    "running": True,
                    "stopped": False,
                    "stop_status": "failed",
                    "reason": (
                        "process-identity-unavailable"
                        if signal_result == "unavailable"
                        else "process-identity-changed"
                    ),
                }
            if signal_result == "sent" and not self._wait_until_process_stops(
                state, self.kill_timeout
            ):
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

    def _signal_state_process(self, state: ServerState, sent_signal: signal.Signals) -> str:
        if read_server_state(self.config) != state:
            return "mismatch"
        identity = process_identity_status(self.config, state)
        if identity != "match":
            return identity
        try:
            process_group = os.getpgid(state.pid)
            if read_server_state(self.config) != state:
                return "mismatch"
            identity = process_identity_status(self.config, state)
            if identity != "match":
                return identity
            if process_group == state.pid:
                os.killpg(process_group, sent_signal)
            else:
                os.kill(state.pid, sent_signal)
        except ProcessLookupError:
            return "gone"
        return "sent"

    def restart(self) -> dict[str, Any]:
        stopped = self.stop()
        if stopped.get("stop_status") == "failed":
            return {"stopped": stopped, "status": None}
        self.start()
        return {"stopped": stopped, "status": self.status()}

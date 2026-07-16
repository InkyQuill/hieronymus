from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient
from hieronymus.service_manager import ServiceManager
from hieronymus.service_state import ServerState, read_server_state


class _Manager(Protocol):
    def ensure_running(self) -> dict[str, object]: ...


class _Client(Protocol):
    def request_json(
        self,
        method: str,
        state: ServerState,
        path: str,
        payload: dict[str, object],
    ) -> dict[str, object]: ...


class DaemonMcpClient:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        manager: _Manager | None = None,
        client: _Client | None = None,
        state_reader: Callable[[HieronymusConfig], ServerState | None] = read_server_state,
    ) -> None:
        self.config = config
        self.manager = manager if manager is not None else ServiceManager(config)
        self.client = client if client is not None else ServiceClient()
        self.state_reader = state_reader

    def invoke(self, operation: str, params: dict[str, object]) -> dict[str, object]:
        self.manager.ensure_running()
        state = self.state_reader(self.config)
        if state is None:
            raise RuntimeError("Hieronymus daemon did not publish state")
        return self.client.request_json("POST", state, f"/api/mcp/{operation}", params)

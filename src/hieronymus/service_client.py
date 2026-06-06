from __future__ import annotations

import json
import urllib.request
from typing import Any

from hieronymus.service_state import ServerState


class ServiceClient:
    def __init__(self, timeout: float = 2.0) -> None:
        self.timeout = timeout

    def health(self, state: ServerState) -> dict[str, Any]:
        return self._json("GET", state, "/health")

    def status(self, state: ServerState) -> dict[str, Any]:
        return self._json("GET", state, "/status")

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        return self._json("POST", state, "/shutdown")

    def _json(self, method: str, state: ServerState, path: str) -> dict[str, Any]:
        request = urllib.request.Request(f"{state.base_url}{path}", method=method)
        with urllib.request.urlopen(request, timeout=self.timeout) as response:
            payload = json.loads(response.read().decode("utf-8"))
        if not isinstance(payload, dict):
            raise ValueError(f"expected JSON object from {path}")
        return payload

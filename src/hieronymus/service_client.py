from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

from hieronymus.service_state import ServerState


class ServiceClientError(RuntimeError):
    pass


class ServiceClient:
    def __init__(self, timeout: float = 2.0) -> None:
        self.timeout = timeout

    def health(self, state: ServerState) -> dict[str, Any]:
        payload = self._json("GET", state, "/health")
        if payload.get("ok") is not True or payload.get("service") != "hieronymus":
            raise ServiceClientError("unexpected health response from service")
        return payload

    def status(self, state: ServerState) -> dict[str, Any]:
        return self._json("GET", state, "/status")

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        return self._json("POST", state, "/shutdown")

    def _json(self, method: str, state: ServerState, path: str) -> dict[str, Any]:
        request = urllib.request.Request(f"{state.base_url}{path}", method=method)
        request.add_header("X-Hieronymus-Token", state.token)
        try:
            response_context = urllib.request.urlopen(request, timeout=self.timeout)
        except urllib.error.HTTPError as exc:
            raise ServiceClientError(f"HTTP {exc.code} response from {path}") from exc
        with response_context as response:
            try:
                payload = json.loads(response.read().decode("utf-8"))
            except (json.JSONDecodeError, UnicodeDecodeError) as exc:
                raise ServiceClientError(f"invalid JSON response from {path}") from exc
        if not isinstance(payload, dict):
            raise ServiceClientError(f"expected JSON object from {path}")
        return payload

from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

from hieronymus.service_state import ServerState


class ServiceClientError(RuntimeError):
    def __init__(
        self,
        message: str,
        *,
        status: int | None = None,
        error_type: str = "",
    ) -> None:
        super().__init__(message)
        self.status = status
        self.error_type = error_type


class ServiceClient:
    def __init__(self, timeout: float = 2.0) -> None:
        self.timeout = timeout

    def health(self, state: ServerState) -> dict[str, Any]:
        payload = self.request_json("GET", state, "/health")
        if payload.get("ok") is not True or payload.get("service") != "hieronymus":
            raise ServiceClientError("unexpected health response from service")
        return payload

    def status(self, state: ServerState) -> dict[str, Any]:
        return self.request_json("GET", state, "/status")

    def shutdown(self, state: ServerState) -> dict[str, Any]:
        return self.request_json("POST", state, "/shutdown")

    def request_json(
        self,
        method: str,
        state: ServerState,
        path: str,
        payload: dict[str, object] | None = None,
    ) -> dict[str, Any]:
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(f"{state.base_url}{path}", data=data, method=method)
        request.add_header("X-Hieronymus-Token", state.token)
        if data is not None:
            request.add_header("Content-Type", "application/json")
        try:
            response_context = urllib.request.urlopen(request, timeout=self.timeout)
        except urllib.error.HTTPError as exc:
            try:
                error_payload = json.loads(exc.read().decode("utf-8"))
            except (json.JSONDecodeError, UnicodeDecodeError):
                error_payload = {}
            message = error_payload.get("error") if isinstance(error_payload, dict) else None
            error_type = (
                str(error_payload.get("error_type", "")) if isinstance(error_payload, dict) else ""
            )
            raise ServiceClientError(
                str(message or f"HTTP {exc.code} response from {path}"),
                status=exc.code,
                error_type=error_type,
            ) from exc
        with response_context as response:
            try:
                payload = json.loads(response.read().decode("utf-8"))
            except (json.JSONDecodeError, UnicodeDecodeError) as exc:
                raise ServiceClientError(f"invalid JSON response from {path}") from exc
        if not isinstance(payload, dict):
            raise ServiceClientError(f"expected JSON object from {path}")
        return payload

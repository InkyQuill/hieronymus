from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import fields, is_dataclass
from typing import Any, NamedTuple


class RpcRequest(NamedTuple):
    id: str
    method: str
    params: dict[str, object]


class RpcError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


def parse_request(line: str) -> RpcRequest:
    try:
        payload = json.loads(line)
    except json.JSONDecodeError as error:
        raise RpcError("invalid_json", f"invalid JSON: {error.msg}") from error
    if type(payload) is not dict:
        raise RpcError("invalid_request", "request must be an object")
    request_id = payload.get("id")
    method = payload.get("method")
    params = payload.get("params", {})
    if type(request_id) is not str or not request_id:
        raise RpcError("invalid_request", "id must be a non-empty string")
    if type(method) is not str or not method:
        raise RpcError("invalid_request", "method must be a non-empty string")
    if type(params) is not dict:
        raise RpcError("invalid_request", "params must be an object")
    return RpcRequest(id=request_id, method=method, params=params)


def dataclass_to_json(value: Any) -> Any:
    if is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: dataclass_to_json(getattr(value, field.name))
            for field in fields(value)
            if not field.name.startswith("_")
        }
    if isinstance(value, tuple):
        return [dataclass_to_json(item) for item in value]
    if isinstance(value, list):
        return [dataclass_to_json(item) for item in value]
    if isinstance(value, dict):
        return {str(key): dataclass_to_json(item) for key, item in value.items()}
    return value


def success_response(request_id: str, result: dict[str, object]) -> dict[str, object]:
    return {"id": request_id, "ok": True, "result": dataclass_to_json(result)}


def error_response(
    request_id: str | None,
    error: RpcError,
    *,
    redact: Callable[[str], str] | None = None,
) -> dict[str, object]:
    from hieronymus.tui_bridge.errors import error_payload

    return {
        "id": request_id,
        "ok": False,
        "error": error_payload(error, redact=redact),
    }

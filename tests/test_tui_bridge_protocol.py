from dataclasses import dataclass

from hieronymus.settings import default_settings
from hieronymus.tui_bridge.errors import error_payload
from hieronymus.tui_bridge.protocol import (
    RpcError,
    RpcRequest,
    dataclass_to_json,
    error_response,
    parse_request,
    success_response,
)


@dataclass(frozen=True)
class Child:
    name: str


@dataclass(frozen=True)
class Parent:
    id: int
    child: Child
    tags: tuple[str, ...]
    _cache_key: str = "private-state"


def test_parse_request_accepts_valid_json_rpc_object() -> None:
    request = parse_request('{"id":"1","method":"admin.snapshot","params":{"view":"Crystals"}}')

    assert request == RpcRequest(
        id="1",
        method="admin.snapshot",
        params={"view": "Crystals"},
    )


def test_parse_request_rejects_non_object_params() -> None:
    response = error_response("1", RpcError("invalid_request", "params must be an object"))

    assert response == {
        "id": "1",
        "ok": False,
        "error": {"code": "invalid_request", "message": "params must be an object"},
    }


def test_success_response_wraps_result() -> None:
    assert success_response("9", {"ready": True}) == {
        "id": "9",
        "ok": True,
        "result": {"ready": True},
    }


def test_dataclass_to_json_recurses_without_private_state() -> None:
    assert dataclass_to_json(Parent(id=4, child=Child("x"), tags=("a", "b"))) == {
        "id": 4,
        "child": {"name": "x"},
        "tags": ["a", "b"],
    }


def test_error_payload_redacts_configured_secret_values(monkeypatch) -> None:
    settings = default_settings()
    monkeypatch.setenv("OPENAI_API_KEY", "raw-secret-value")

    payload = error_payload(
        ValueError("provider rejected raw-secret-value"),
        settings=settings,
    )

    assert payload == {
        "code": "validation_error",
        "message": "provider rejected [redacted]",
    }

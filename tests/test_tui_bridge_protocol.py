from dataclasses import dataclass

import pytest

import hieronymus.tui_bridge as tui_bridge
from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import default_dream_config, save_dream_config
from hieronymus.provider_config import ProviderCatalog, ProviderProfile, save_provider_catalog
from hieronymus.tui_bridge.errors import error_payload
from hieronymus.tui_bridge.protocol import (
    RpcError,
    RpcRequest,
    dataclass_to_json,
    error_response,
    parse_request,
    success_response,
)
from hieronymus.tui_bridge.server import dispatch


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


@pytest.mark.parametrize(
    ("line", "code", "message"),
    [
        ("{", "invalid_json", "invalid JSON: Expecting property name enclosed in double quotes"),
        ("[]", "invalid_request", "request must be an object"),
        ('{"method":"admin.snapshot"}', "invalid_request", "id must be a non-empty string"),
        ('{"id":"","method":"admin.snapshot"}', "invalid_request", "id must be a non-empty string"),
        ('{"id":"1"}', "invalid_request", "method must be a non-empty string"),
        ('{"id":"1","method":""}', "invalid_request", "method must be a non-empty string"),
    ],
)
def test_parse_request_rejects_invalid_request_shapes(
    line: str,
    code: str,
    message: str,
) -> None:
    with pytest.raises(RpcError) as error_info:
        parse_request(line)

    assert error_info.value.code == code
    assert error_info.value.message == message


def test_parse_request_rejects_non_object_params() -> None:
    with pytest.raises(RpcError) as error_info:
        parse_request('{"id":"1","method":"admin.snapshot","params":["Crystals"]}')

    response = error_response("1", error_info.value)

    assert response == {
        "id": "1",
        "ok": False,
        "error": {"code": "invalid_request", "message": "params must be an object"},
    }


def test_success_response_wraps_json_safe_result() -> None:
    assert success_response(
        "9",
        {"ready": True, "parent": Parent(id=4, child=Child("x"), tags=("a", "b"))},
    ) == {
        "id": "9",
        "ok": True,
        "result": {
            "ready": True,
            "parent": {"id": 4, "child": {"name": "x"}, "tags": ["a", "b"]},
        },
    }


def test_dataclass_to_json_recurses_without_private_state() -> None:
    assert dataclass_to_json(Parent(id=4, child=Child("x"), tags=("a", "b"))) == {
        "id": 4,
        "child": {"name": "x"},
        "tags": ["a", "b"],
    }


def test_error_payload_redacts_configured_secret_values() -> None:
    dream_config = default_dream_config()

    payload = error_payload(
        ValueError("provider rejected raw-secret-value"),
        dream_config=dream_config,
        redact=lambda text: text.replace("raw-secret-value", "[redacted]"),
    )

    assert payload == {
        "code": "validation_error",
        "message": "provider rejected [redacted]",
    }


def test_error_response_can_redact_configured_secret_values() -> None:
    response = error_response(
        "1",
        RpcError("invalid_request", "provider rejected raw-secret-value"),
        redact=lambda text: text.replace("raw-secret-value", "[redacted]"),
    )

    assert response == {
        "id": "1",
        "ok": False,
        "error": {
            "code": "invalid_request",
            "message": "provider rejected [redacted]",
        },
    }


def test_dispatch_error_redacts_secret_from_provider_catalog(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "openai": ProviderProfile(
                    name="OpenAI",
                    type="openai",
                    url="https://api.example.test/v1",
                    key="raw-secret-value",
                )
            }
        ),
    )
    save_dream_config(
        config,
        default_dream_config(),
    )
    monkeypatch.setattr(
        "hieronymus.tui_bridge.server._redactor_or_none",
        lambda config: lambda text: text.replace("raw-secret-value", "[redacted]"),
    )

    response = dispatch(
        config,
        {"id": "1", "method": "provider rejected raw-secret-value", "params": {}},
    )

    assert response["ok"] is False
    assert "raw-secret-value" not in repr(response)
    assert "[redacted]" in repr(response)


def test_tui_bridge_does_not_export_missing_server_entrypoint() -> None:
    assert tui_bridge.__all__ == []
    assert not hasattr(tui_bridge, "run_stdio")

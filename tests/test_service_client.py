from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.service_client import ServiceClient, ServiceClientError
from hieronymus.service_state import ServerState


class FakeResponse:
    def __init__(self, body: bytes) -> None:
        self.body = body

    def __enter__(self) -> FakeResponse:
        return self

    def __exit__(self, *args: object) -> None:
        return None

    def read(self) -> bytes:
        return self.body


def _make_state(config: HieronymusConfig) -> ServerState:
    return ServerState(
        pid=12345,
        host="127.0.0.1",
        port=32199,
        version="0.1.0",
        started_at="2026-06-06T12:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="local-test-token",
    )


def test_service_client_sends_state_token_header(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)
    seen: dict[str, Any] = {}

    def fake_urlopen(request: Any, timeout: float) -> FakeResponse:
        seen["timeout"] = timeout
        seen["token"] = request.get_header("X-hieronymus-token")
        seen["url"] = request.full_url
        return FakeResponse(b'{"ok": true, "service": "hieronymus"}')

    monkeypatch.setattr("urllib.request.urlopen", fake_urlopen)

    payload = ServiceClient(timeout=1.5).health(state)

    assert payload == {"ok": True, "service": "hieronymus"}
    assert seen == {
        "timeout": 1.5,
        "token": "local-test-token",
        "url": "http://127.0.0.1:32199/health",
    }


def test_service_client_rejects_wrong_health_identity(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = _make_state(config)

    def fake_urlopen(request: Any, timeout: float) -> FakeResponse:
        return FakeResponse(b'{"ok": true, "service": "other"}')

    monkeypatch.setattr("urllib.request.urlopen", fake_urlopen)

    with pytest.raises(ServiceClientError, match="unexpected health response"):
        ServiceClient().health(state)

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.daemon_mcp_client import DaemonMcpClient
from hieronymus.service_state import ServerState


@dataclass
class FakeManager:
    ensure_calls: int = 0

    def ensure_running(self) -> dict[str, object]:
        self.ensure_calls += 1
        return {"started": self.ensure_calls == 1}


@dataclass
class FakeClient:
    calls: list[tuple[str, str, dict[str, object]]]

    def request_json(
        self,
        method: str,
        _: ServerState,
        path: str,
        payload: dict[str, object],
    ) -> dict[str, object]:
        self.calls.append((method, path, payload))
        return {"slug": "oso"}


def test_daemon_mcp_client_starts_service_then_posts_operation(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "data")
    state = ServerState(
        pid=1,
        host="127.0.0.1",
        port=8765,
        version="0.1.0",
        started_at="2026-07-16T00:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="test-token",
    )
    manager = FakeManager()
    client = FakeClient(calls=[])

    result = DaemonMcpClient(
        config,
        manager=manager,
        client=client,
        state_reader=lambda _: state,
    ).invoke("series_create", {"slug": "oso", "title": "Only Sense Online"})

    assert result == {"slug": "oso"}
    assert manager.ensure_calls == 1
    assert client.calls == [
        (
            "POST",
            "/api/mcp/series_create",
            {"slug": "oso", "title": "Only Sense Online"},
        )
    ]

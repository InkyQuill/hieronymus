from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.agent_hooks import main
from hieronymus.config import HieronymusConfig
from hieronymus.service_state import ServerState, write_server_state


def test_hook_session_start_outputs_missing_context_when_no_project_file(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "event": "session-start",
        "handled": False,
        "reason": "no .hieronymus.json context found",
        "service": {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        },
    }


def test_hook_session_start_outputs_context_when_project_file_exists(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    (tmp_path / ".hieronymus.json").write_text(
        json.dumps(
            {
                "series_slug": "oso",
                "source_language": "ja",
                "target_language": "en",
                "task_type": "translation",
                "volume": "1",
                "chapter": "2",
            }
        ),
        encoding="utf-8",
    )

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "event": "session-start",
        "handled": True,
        "series_slug": "oso",
        "source_language": "ja",
        "target_language": "en",
        "task_type": "translation",
        "volume": "1",
        "chapter": "2",
        "service": {
            "available": False,
            "mode": "direct-local",
            "reason": "no running local service discovered",
        },
    }


def test_hook_session_start_json_includes_missing_service_discovery(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["service"] == {
        "available": False,
        "mode": "direct-local",
        "reason": "no running local service discovered",
    }


def test_hook_session_start_json_includes_discovered_service(
    tmp_path: Path,
    monkeypatch,
) -> None:
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    state = ServerState(
        pid=123,
        host="127.0.0.1",
        port=8765,
        version="0.2.0",
        started_at="2026-06-14T00:00:00Z",
        data_root=str(config.data_root),
        database_path=str(config.database_path),
        token="secret",
    )
    write_server_state(config, state)
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(data_root))
    monkeypatch.setattr("hieronymus.service_state.is_pid_running", lambda pid: pid == 123)

    def fake_health(self, received_state):
        assert received_state == state
        return {"ok": True, "service": "hieronymus", "version": "0.2.0"}

    monkeypatch.setattr("hieronymus.service_client.ServiceClient.health", fake_health)

    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path), "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["service"] == {
        "available": True,
        "mode": "local-http",
        "base_url": "http://127.0.0.1:8765",
        "pid": 123,
        "version": "0.2.0",
        "data_root": str(data_root),
        "database_path": str(data_root / "hieronymus.sqlite"),
    }


def test_hook_session_start_human_output_is_concise(tmp_path: Path) -> None:
    result = CliRunner().invoke(main, ["session-start", "--cwd", str(tmp_path)])

    assert result.exit_code == 0
    assert result.output == "no .hieronymus.json context found\n"


def test_hook_session_end_outputs_json() -> None:
    result = CliRunner().invoke(main, ["session-end", "--json"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {"event": "session-end", "handled": True}

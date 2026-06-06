from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main


def test_install_without_app_lists_candidates_json(tmp_path: Path, monkeypatch) -> None:
    codex_home = tmp_path / "home" / ".codex"
    codex_home.mkdir(parents=True)
    monkeypatch.setenv("HOME", str(tmp_path / "home"))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    codex = next(item for item in payload["candidates"] if item["target"] == "codex")
    assert codex["available"] is True
    assert codex["installed"] is False


def test_install_list_human_output_marks_available_targets(tmp_path: Path, monkeypatch) -> None:
    (tmp_path / "home" / ".claude").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(tmp_path / "home"))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "list"],
    )

    assert result.exit_code == 0
    assert "Claude Code / Claude Desktop: available, not installed" in result.output

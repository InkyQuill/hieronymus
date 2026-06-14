from __future__ import annotations

import json
import tomllib
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


def test_install_codex_json_installs_but_dry_run_does_not_mutate(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    data_root = tmp_path / "hieronymus"
    monkeypatch.setenv("HOME", str(home))
    runner = CliRunner()

    dry_run = runner.invoke(
        main,
        ["--data-root", str(data_root), "install", "codex", "--dry-run", "--json"],
    )

    assert dry_run.exit_code == 0
    dry_payload = json.loads(dry_run.output)
    assert dry_payload["dry_run"] is True
    assert dry_payload["result_kind"] == "installable"
    assert not (data_root / "agent-plugins" / "codex").exists()
    assert not (home / ".codex" / "config.toml").exists()

    result = runner.invoke(
        main,
        ["--data-root", str(data_root), "install", "codex", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["dry_run"] is False
    assert payload["result_kind"] == "installed"
    assert payload["availability"]["installed"] is True
    assert (data_root / "agent-plugins" / "codex" / ".codex-plugin" / "plugin.json").exists()
    config_payload = tomllib.loads((home / ".codex" / "config.toml").read_text(encoding="utf-8"))
    assert config_payload["mcp_servers"]["hieronymus"]["command"] == "hieronymus-mcp"
    assert config_payload["plugins"]["hieronymus"]["path"] == str(
        data_root / "agent-plugins" / "codex"
    )

    human_result = runner.invoke(
        main,
        ["--data-root", str(data_root), "install", "codex"],
    )

    assert human_result.exit_code == 0
    assert "Installed Codex integration" in human_result.output
    assert "Applied changes:" in human_result.output
    assert "Result: installed." in human_result.output


def test_install_reserved_provider_json_reports_reserved(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".pi").mkdir(parents=True)
    data_root = tmp_path / "hieronymus"
    monkeypatch.setenv("HOME", str(home))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "install", "pi", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["result_kind"] == "reserved"
    assert payload["steps"] == []
    assert not (data_root / "agent-plugins" / "pi").exists()
    assert not (home / ".pi" / "config.json").exists()


def test_install_mimocode_alias_json_reports_mimo_reserved(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".mimocode").mkdir(parents=True)
    data_root = tmp_path / "hieronymus"
    monkeypatch.setenv("HOME", str(home))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "install", "mimocode", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["target"] == "mimo"
    assert payload["result_kind"] == "reserved"
    assert payload["steps"] == []
    assert not (data_root / "agent-plugins" / "mimo").exists()
    assert not (home / ".config" / "mimocode").exists()


def test_install_reserved_provider_human_output_remains_plan(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".pi").mkdir(parents=True)
    data_root = tmp_path / "hieronymus"
    monkeypatch.setenv("HOME", str(home))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "install", "pi"],
    )

    assert result.exit_code == 0
    assert "Planning Pi integration" in result.output
    assert "Planned changes:" in result.output
    assert "reserved" in result.output
    assert "Result: reserved; no config was written" in result.output
    assert not (data_root / "agent-plugins" / "pi").exists()
    assert not (home / ".pi" / "config.json").exists()


def test_install_malformed_config_returns_clean_error(tmp_path: Path, monkeypatch) -> None:
    home = tmp_path / "home"
    (home / ".gemini").mkdir(parents=True)
    (home / ".gemini" / "settings.json").write_text(
        '{"mcpServers": []}\n',
        encoding="utf-8",
    )
    monkeypatch.setenv("HOME", str(home))

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "install", "gemini"],
    )

    assert result.exit_code == 1
    assert "expected object at mcpServers" in result.output
    assert "Traceback" not in result.output

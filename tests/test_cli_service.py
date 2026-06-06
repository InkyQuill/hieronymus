from __future__ import annotations

import json
import subprocess
from pathlib import Path
from unittest.mock import patch

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.presentation import GREETING_ICON, render_greeting


def test_render_greeting_contains_identity_and_tagline() -> None:
    rendered = render_greeting("0.1.0")

    assert rendered == f"{GREETING_ICON} Hieronymus v0.1.0\nRemembers things for you."


def test_hiero_console_alias_runs_existing_command(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = subprocess.run(
        [
            "uv",
            "run",
            "hiero",
            "--data-root",
            str(data_root),
            "init-series",
            "oso",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode == 0
    assert json.loads(result.stdout) == {
        "slug": "oso",
        "database_path": str(data_root / "hieronymus.sqlite"),
    }


def test_cli_help_mentions_service_commands() -> None:
    result = CliRunner().invoke(main, ["help"])

    assert result.exit_code == 0
    assert "hiero status" in result.output
    assert "hiero install codex --dry-run" in result.output


def test_status_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "status", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {"reason": "no-state", "running": False}


def test_stop_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.stop.return_value = {
            "running": False,
            "stopped": False,
            "reason": "not-running",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "stop", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "reason": "not-running",
        "running": False,
        "stopped": False,
    }


def test_restart_json_returns_manager_payload(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.restart.return_value = {
            "stopped": {"running": False, "stopped": True},
            "status": {"running": True, "pid": 1000, "port": 32199},
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "restart", "--json"],
        )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "status": {"pid": 1000, "port": 32199, "running": True},
        "stopped": {"running": False, "stopped": True},
    }


def test_config_json_returns_paths_and_tui_placeholder(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config", "--json"],
    )

    assert result.exit_code == 0
    assert json.loads(result.output) == {
        "config_root": str(data_root),
        "database_path": str(data_root / "hieronymus.sqlite"),
        "tui": "not-available-in-this-pass",
    }


def test_admin_json_returns_tui_placeholder(tmp_path: Path) -> None:
    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "admin", "--json"],
    )

    assert result.exit_code == 0
    assert json.loads(result.output) == {"tui": "not-available-in-this-pass"}


def test_doctor_json_has_expected_sections(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        result = runner.invoke(
            main,
            ["--data-root", str(tmp_path / "hieronymus"), "doctor", "--json"],
        )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert sorted(payload.keys()) == ["autofixed", "errors", "warnings"]


def test_no_subcommand_ensures_service_and_prints_greeting(tmp_path: Path) -> None:
    runner = CliRunner()

    with patch("hieronymus.cli.ServiceManager") as manager_class:
        manager_class.return_value.ensure_running.return_value = {
            "started": True,
            "status": {"running": True, "pid": 1000, "port": 32199},
        }
        result = runner.invoke(main, ["--data-root", str(tmp_path / "hieronymus")])

    assert result.exit_code == 0
    assert "🪶 Hieronymus v" in result.output
    assert "running: yes" in result.output
    assert "port: 32199" in result.output

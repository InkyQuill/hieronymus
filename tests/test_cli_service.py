from __future__ import annotations

import json
import subprocess
from pathlib import Path

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

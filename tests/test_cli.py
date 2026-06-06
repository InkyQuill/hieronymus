import json
import subprocess
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main


def test_init_series_outputs_json_and_creates_database(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload == {
        "slug": "only-sense-online",
        "database_path": str(data_root / "series" / "only-sense-online.sqlite"),
    }
    assert (data_root / "series" / "only-sense-online.sqlite").exists()


def test_unknown_series_returns_clean_click_error(tmp_path):
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "remember",
            "missing-series",
            "--kind",
            "translation_rationale",
            "--text",
            "Use concise system messages.",
        ],
    )

    assert result.exit_code == 1
    assert "Error: unknown series: missing-series" in result.output
    assert "Traceback" not in result.output


def test_data_root_rejects_existing_file_without_traceback(tmp_path):
    data_root = tmp_path / "data-root-file"
    data_root.write_text("not a directory", encoding="utf-8")

    result = subprocess.run(
        [
            "uv",
            "run",
            "hieronymus",
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode != 0
    assert "Invalid value for '--data-root'" in result.stderr
    assert "Traceback" not in result.stderr


def test_console_entrypoint_init_series_outputs_json(tmp_path):
    data_root = tmp_path / "hieronymus"

    result = subprocess.run(
        [
            "uv",
            "run",
            "hieronymus",
            "--data-root",
            str(data_root),
            "init-series",
            "only-sense-online",
            "--title",
            "Only Sense Online",
        ],
        check=False,
        cwd=Path.cwd(),
        text=True,
        capture_output=True,
    )

    assert result.returncode == 0
    payload = json.loads(result.stdout)
    assert payload == {
        "slug": "only-sense-online",
        "database_path": str(data_root / "series" / "only-sense-online.sqlite"),
    }

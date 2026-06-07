import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.registry import Registry


def _seed_series(data_root: Path) -> None:
    Registry(HieronymusConfig(data_root=data_root)).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )


def test_admin_json_reports_available_tui_and_counts(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    _seed_series(data_root)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(data_root), "admin", "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["tui"] == "available"
    assert payload["counts"]["series"] == 1
    assert payload["counts"]["crystals"] == 0
    assert payload["service"]["running"] is False
    assert payload["views"] == [
        "Concepts",
        "Renderings",
        "Crystals",
        "Lessons",
        "Short-Term Sessions",
        "Dream Runs",
        "Proposals",
        "Audit Log",
    ]


def test_admin_launch_invokes_textual_app(monkeypatch, tmp_path: Path) -> None:
    launched: dict[str, object] = {}

    class FakeApp:
        def __init__(self, config):
            launched["database_path"] = str(config.database_path)

        def run(self):
            launched["ran"] = True

    monkeypatch.setattr("hieronymus.cli.HieronymusAdminApp", FakeApp)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(tmp_path), "admin"])

    assert result.exit_code == 0
    assert launched == {
        "database_path": str(tmp_path / "hieronymus.sqlite"),
        "ran": True,
    }

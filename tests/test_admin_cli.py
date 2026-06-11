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
        "Dream Audits",
        "Audit Log",
    ]


def test_admin_json_survives_malformed_dream_config(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    _seed_series(data_root)
    data_root.mkdir(parents=True, exist_ok=True)
    (data_root / "dream.conf").write_text("[dreaming\n", encoding="utf-8")
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(data_root), "admin", "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["tui"] == "available"
    assert payload["dream_status"]["state"] == "DISABLED"
    assert "dream.conf is not valid TOML" in payload["dream_status"]["reason"]
    assert "dream.conf is not valid TOML" in payload["dream_config_error"]


def test_admin_launch_invokes_opentui(monkeypatch, tmp_path: Path) -> None:
    launched: dict[str, object] = {}

    def fake_launch_opentui(mode, *, data_root):
        launched["mode"] = mode
        launched["data_root"] = data_root

    monkeypatch.setattr("hieronymus.cli._launch_opentui", fake_launch_opentui)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(tmp_path), "admin"])

    assert result.exit_code == 0
    assert launched == {
        "mode": "admin",
        "data_root": tmp_path,
    }

from __future__ import annotations

import tomllib
from pathlib import Path
from unittest.mock import patch

from hieronymus.agent_plugins import resolve_plugin
from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, report_to_json


def test_doctor_reports_available_uninstalled_agent(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {"running": False}
        report = Doctor(config).run()
    payload = report_to_json(report)

    assert any(
        finding["code"] == "agent-plugin-available"
        and "Codex is available but Hieronymus is not installed" in finding["message"]
        for finding in payload["warnings"]
    )


def test_doctor_does_not_warn_for_installed_agent(
    tmp_path: Path,
    monkeypatch,
) -> None:
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    resolve_plugin("codex").install(config)

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {"running": False}
        report = Doctor(config).run()
    payload = report_to_json(report)

    assert tomllib.loads((home / ".codex" / "config.toml").read_text(encoding="utf-8"))[
        "hieronymus"
    ]["managed"]
    assert not any(
        finding["code"] == "agent-plugin-available" and "Codex is available" in finding["message"]
        for finding in payload["warnings"]
    )

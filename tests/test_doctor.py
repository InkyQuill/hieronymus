from __future__ import annotations

from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding
from hieronymus.settings import load_settings, save_settings


def test_doctor_reports_missing_config_root_as_autofixable(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert (
        DoctorFinding(
            level="warning",
            code="config-root-missing",
            message=f"Config root does not exist: {config.config_root}",
            autofixed=False,
        )
        in report["warnings"]
    )


def test_doctor_autofix_creates_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=True)

    assert config.config_root.is_dir()
    assert report["autofixed"][0].code == "config-root-created"


def test_doctor_reports_database_file_when_present(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.database_path.write_text("not sqlite", encoding="utf-8")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert report["errors"][0].code == "database-unreadable"


def test_doctor_reports_missing_active_provider_env(
    tmp_path: Path,
    monkeypatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    monkeypatch.delenv("MISSING_OPENAI_KEY", raising=False)
    settings = load_settings(config)
    openai = replace(settings.providers["openai"], enabled=True, api_key_env="MISSING_OPENAI_KEY")
    save_settings(
        config,
        settings.with_provider("openai", openai).with_dreaming(
            replace(settings.dreaming, active_provider="openai")
        ),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert report["errors"][0].code == "provider-env-missing"
    assert "MISSING_OPENAI_KEY" in report["errors"][0].message

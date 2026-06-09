from __future__ import annotations

from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding, report_to_json
from hieronymus.llm_cache import CachedModels, ModelCacheEntry, save_model_cache
from hieronymus.settings import DreamingSettings, ProviderSettings, load_settings, save_settings


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

    finding = next(error for error in report["errors"] if error.code == "provider-env-missing")
    assert "MISSING_OPENAI_KEY" in finding.message


def test_doctor_warns_when_llm_model_cache_refresh_failed(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="anthropic",
                models=("claude-3-5-haiku-latest",),
                fetched_at="2026-06-09T12:00:00+00:00",
                error="model suggestions unavailable",
            )
        ),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert (
        DoctorFinding(
            level="warning",
            code="llm-model-cache-refresh-failed",
            message="Model cache refresh failed for provider profile: anthropic",
        )
        in report["warnings"]
    )


def test_doctor_json_does_not_include_raw_api_key_value(config, monkeypatch):
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="HIERONYMUS_OPENAI_KEY",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai"))
    )
    save_settings(config, settings)

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    assert "provider-configured" in repr(payload)
    assert "raw-secret-value" not in repr(payload)

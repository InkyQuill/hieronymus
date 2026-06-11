from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding, report_to_json
from hieronymus.dream_config import ProviderProfile
from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    dream_profile_cache_identity,
    model_cache_identity,
    save_model_cache,
)
from hieronymus.settings import DreamingSettings, ProviderSettings, load_settings, save_settings


def write_dream_config(config: HieronymusConfig, raw_config: str) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text(raw_config, encoding="utf-8")


def run_doctor_without_daemon(config: HieronymusConfig):
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        return Doctor(config).run(autofix=False)


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


def test_doctor_warns_when_dreaming_is_disabled(config) -> None:
    report = run_doctor_without_daemon(config)

    assert (
        DoctorFinding(
            level="warning",
            code="dreaming_disabled",
            message="Dreaming is disabled",
        )
        in report["warnings"]
    )


def test_doctor_reports_invalid_dream_conf(config) -> None:
    write_dream_config(config, "[dreaming\n")

    report = run_doctor_without_daemon(config)

    assert (
        DoctorFinding(
            level="error",
            code="dream_conf_invalid",
            message="dream.conf invalid",
        )
        in report["errors"]
    )


@pytest.mark.parametrize(
    ("raw_config", "code", "severity", "message"),
    [
        (
            "[dreaming]\n"
            "enabled = true\n"
            "[workflows.crystallization]\n"
            "provider='missing'\n"
            "model='x'\n"
            "enabled=true\n",
            "dream_provider_profile_missing",
            "error",
            "Referenced provider profile is missing",
        ),
        (
            "[dreaming]\n"
            "enabled = true\n"
            "[workflows.crystallization]\n"
            "provider='anthropic'\n"
            "model=''\n"
            "enabled=true\n",
            "dream_model_not_set",
            "error",
            "Model not set for workflow",
        ),
        (
            "[dreaming]\n"
            "enabled = true\n"
            "[providers.anthropic]\n"
            "type='anthropic'\n"
            "api_key=''\n"
            "[workflows.crystallization]\n"
            "provider='anthropic'\n"
            "model='x'\n"
            "enabled=true\n",
            "dream_api_key_missing",
            "error",
            "API key missing for provider profile",
        ),
    ],
)
def test_doctor_reports_dream_conf_readiness_errors(
    config,
    raw_config: str,
    code: str,
    severity: str,
    message: str,
) -> None:
    write_dream_config(config, raw_config)

    report = run_doctor_without_daemon(config)

    assert DoctorFinding(level=severity, code=code, message=message) in report[f"{severity}s"]


def test_doctor_reports_multiple_dream_readiness_errors_after_config_validation_error(
    config,
) -> None:
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.openai]\n"
        "type='openai'\n"
        "api_key=''\n"
        "[workflows.crystallization]\n"
        "provider='missing'\n"
        "model='x'\n"
        "enabled=true\n"
        "[workflows.reinforcement_compaction]\n"
        "provider='openai'\n"
        "model='gpt-4.1-mini'\n"
        "enabled=true\n",
    )

    report = run_doctor_without_daemon(config)

    assert (
        DoctorFinding(
            level="error",
            code="dream_provider_profile_missing",
            message="Referenced provider profile is missing",
        )
        in report["errors"]
    )
    assert (
        DoctorFinding(
            level="error",
            code="dream_api_key_missing",
            message="API key missing for provider profile",
        )
        in report["errors"]
    )
    assert all(error.code != "dream_conf_invalid" for error in report["errors"])


def test_doctor_ignores_disabled_optional_dream_workflows(config) -> None:
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[workflows.relation_discovery]\n"
        "provider='missing'\n"
        "model=''\n"
        "enabled=false\n",
    )

    report = run_doctor_without_daemon(config)

    failing_codes = {finding.code for finding in report["errors"]}
    assert "dream_provider_profile_missing" not in failing_codes
    assert "dream_model_not_set" not in failing_codes


def test_doctor_warns_when_configured_dream_model_missing_from_cache(config) -> None:
    profile = ProviderProfile(type="ollama", endpoint="http://localhost:11434")
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.crystallization]\n"
        "provider='ollama'\n"
        "model='missing-model'\n"
        "enabled=true\n",
    )
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="ollama",
                models=("present-model",),
                fetched_at=datetime.now(UTC).isoformat(),
                identity=dream_profile_cache_identity("ollama", profile),
            )
        ),
    )

    report = run_doctor_without_daemon(config)

    assert (
        DoctorFinding(
            level="warning",
            code="dream_model_missing",
            message="Configured model was not found in provider cache",
        )
        in report["warnings"]
    )


def test_doctor_ignores_dream_model_cache_for_obsolete_provider_profile(config) -> None:
    current_profile = ProviderProfile(type="ollama", endpoint="http://localhost:11434")
    old_profile = ProviderProfile(type="ollama", endpoint="http://localhost:11435")
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.crystallization]\n"
        "provider='ollama'\n"
        "model='missing-model'\n"
        "enabled=true\n",
    )
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="ollama",
                models=("present-model",),
                fetched_at=datetime.now(UTC).isoformat(),
                error="provider returned HTTP 403",
                identity=dream_profile_cache_identity("ollama", old_profile),
            )
        ),
    )

    report = run_doctor_without_daemon(config)

    warning_codes = {warning.code for warning in report["warnings"]}
    error_codes = {error.code for error in report["errors"]}
    assert current_profile != old_profile
    assert "dream_model_missing" not in warning_codes
    assert "dream_provider_unreachable" not in warning_codes
    assert "dream_api_key_rejected" not in error_codes


def test_doctor_ignores_stale_dream_model_cache(config) -> None:
    profile = ProviderProfile(type="ollama", endpoint="http://localhost:11434")
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.crystallization]\n"
        "provider='ollama'\n"
        "model='missing-model'\n"
        "enabled=true\n",
    )
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="ollama",
                models=("present-model",),
                fetched_at=(datetime.now(UTC) - timedelta(hours=24)).isoformat(),
                identity=dream_profile_cache_identity("ollama", profile),
            )
        ),
    )

    report = run_doctor_without_daemon(config)

    assert all(warning.code != "dream_model_missing" for warning in report["warnings"])


@pytest.mark.parametrize(
    ("error", "code", "severity", "message"),
    [
        (
            "provider returned HTTP 403",
            "dream_api_key_rejected",
            "error",
            "API key rejected with 403",
        ),
        (
            "network error",
            "dream_provider_unreachable",
            "warning",
            "Provider in use cannot be reached",
        ),
    ],
)
def test_doctor_reports_fresh_dream_provider_cache_errors(
    config,
    error: str,
    code: str,
    severity: str,
    message: str,
) -> None:
    profile = ProviderProfile(type="ollama", endpoint="http://localhost:11434")
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.crystallization]\n"
        "provider='ollama'\n"
        "model='gemma4-e3b'\n"
        "enabled=true\n",
    )
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="ollama",
                models=(),
                fetched_at=datetime.now(UTC).isoformat(),
                error=error,
                identity=dream_profile_cache_identity("ollama", profile),
            )
        ),
    )

    report = run_doctor_without_daemon(config)

    assert DoctorFinding(level=severity, code=code, message=message) in report[f"{severity}s"]


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
                fetched_at=datetime.now(UTC).isoformat(),
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


def test_doctor_ignores_stale_llm_model_cache_refresh_failure(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="anthropic",
                models=("claude-3-5-haiku-latest",),
                fetched_at=(datetime.now(UTC) - timedelta(hours=24)).isoformat(),
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

    assert all(warning.code != "llm-model-cache-refresh-failed" for warning in report["warnings"])


def test_doctor_ignores_llm_model_cache_error_for_obsolete_settings(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    saved = load_settings(config)
    settings_a = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            api_key_env="OPENAI_KEY_A",
            base_url="https://a.example.test/v1",
        ),
    )
    settings_b = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            api_key_env="OPENAI_KEY_B",
            base_url="https://b.example.test/v1",
        ),
    )
    save_settings(config, settings_b)
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="openai",
                models=("gpt-4.1-mini",),
                fetched_at=datetime.now(UTC).isoformat(),
                error="missing environment variable: OPENAI_KEY_A",
                identity=model_cache_identity("openai", settings_a.providers["openai"]),
            )
        ),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert all(warning.code != "llm-model-cache-refresh-failed" for warning in report["warnings"])


def test_doctor_ignores_malformed_llm_model_cache(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.llm_cache_path.write_text("{not json", encoding="utf-8")

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    assert all(warning.code != "llm-model-cache-refresh-failed" for warning in report["warnings"])


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

from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import patch

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding, report_to_json
from hieronymus.dream_config import (
    WorkflowProfile,
    default_dream_config,
    save_dream_config,
)
from hieronymus.dream_providers import ProviderProfile as RuntimeProviderProfile
from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    dream_profile_cache_identity,
    save_model_cache,
)
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderDefaults,
    ProviderProfile,
    save_provider_catalog,
)


def write_dream_config(config: HieronymusConfig, raw_config: str) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text(raw_config, encoding="utf-8")


def save_provider_profile(
    config: HieronymusConfig,
    profile_id: str,
    profile: ProviderProfile,
    *,
    default_model: str = "",
) -> None:
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={profile_id: profile},
            defaults=ProviderDefaults(provider=profile_id, model=default_model),
        ),
    )


def catalog_profile(
    *,
    provider_type: str,
    url: str,
    key: str = "",
    name: str = "Provider",
) -> ProviderProfile:
    return ProviderProfile(name=name, type=provider_type, url=url, key=key)


def runtime_profile(profile: ProviderProfile) -> RuntimeProviderProfile:
    provider_type = "gemini" if profile.type == "google" else profile.type
    return RuntimeProviderProfile(
        type=provider_type,
        endpoint=profile.url,
        api_key=profile.key,
        timeout_seconds=profile.timeout_seconds,
    )


def run_doctor_without_daemon(config: HieronymusConfig):
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        return Doctor(config).run(autofix=False)


def test_doctor_reports_running_daemon_as_information(config) -> None:
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": True,
            "pid": 12,
            "port": 8765,
            "data_root": str(config.data_root),
        }
        report = Doctor(config).run(autofix=False)

    finding = next(item for item in report["info"] if item.code == "daemon-running")
    assert finding.autofixed is False
    assert "pid 12" in finding.message
    assert "port 8765" in finding.message
    assert str(config.data_root) in finding.message


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


def test_doctor_reports_invalid_provider_conf_without_follow_on_readiness_errors(
    config,
) -> None:
    save_dream_config(
        config,
        replace(default_dream_config(), enabled=True).with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )
    config.provider_config_path.write_text("[openai\n", encoding="utf-8")

    report = run_doctor_without_daemon(config)

    assert (
        DoctorFinding(
            level="error",
            code="provider_conf_invalid",
            message="provider.conf invalid",
        )
        in report["errors"]
    )
    assert all(error.code != "dream_provider_profile_missing" for error in report["errors"])
    assert all(error.code != "dream_api_key_missing" for error in report["errors"])


def test_doctor_accepts_deterministic_dream_workflow_without_provider_profile(
    config,
) -> None:
    save_dream_config(
        config,
        replace(default_dream_config(), enabled=True)
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="deterministic", model="deterministic", enabled=True),
        )
        .with_workflow(
            "reinforcement_compaction",
            WorkflowProfile(provider="ollama", model="gemma4-e3b", enabled=False),
        ),
    )

    report = run_doctor_without_daemon(config)

    assert all(error.code != "dream_provider_profile_missing" for error in report["errors"])
    assert all(error.code != "dream_api_key_missing" for error in report["errors"])


@pytest.mark.parametrize(
    ("raw_config", "code", "severity", "message"),
    [
        (
            "[dreaming]\n"
            "enabled = true\n"
            "[workflows.knowledge_crystals]\n"
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
            "[workflows.knowledge_crystals]\n"
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
            "[workflows.knowledge_crystals]\n"
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
    if code == "dream_model_not_set":
        save_provider_profile(
            config,
            "anthropic",
            catalog_profile(
                provider_type="anthropic",
                url="https://api.anthropic.com",
                key="secret",
            ),
        )
    if code == "dream_api_key_missing":
        save_provider_profile(
            config,
            "anthropic",
            catalog_profile(provider_type="anthropic", url="https://api.anthropic.com"),
        )
    write_dream_config(config, raw_config)

    report = run_doctor_without_daemon(config)

    assert DoctorFinding(level=severity, code=code, message=message) in report[f"{severity}s"]


def test_doctor_reports_multiple_dream_readiness_errors_after_config_validation_error(
    config,
) -> None:
    save_provider_profile(
        config,
        "openai",
        catalog_profile(provider_type="openai", url="https://api.openai.com/v1"),
    )
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.openai]\n"
        "type='openai'\n"
        "api_key=''\n"
        "[workflows.knowledge_crystals]\n"
        "provider='missing'\n"
        "model='x'\n"
        "enabled=true\n"
        "[workflows.reinforcement]\n"
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
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "anthropic": catalog_profile(
                    provider_type="anthropic",
                    url="https://api.anthropic.com",
                    key="secret",
                ),
                "ollama": catalog_profile(
                    provider_type="ollama",
                    url="http://localhost:11434",
                ),
            },
        ),
    )
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
    profile = catalog_profile(provider_type="ollama", url="http://localhost:11434")
    save_provider_profile(config, "ollama", profile)
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.knowledge_crystals]\n"
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
                identity=dream_profile_cache_identity("ollama", runtime_profile(profile)),
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
    current_profile = catalog_profile(provider_type="ollama", url="http://localhost:11434")
    old_profile = RuntimeProviderProfile(type="ollama", endpoint="http://localhost:11435")
    save_provider_profile(config, "ollama", current_profile)
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.knowledge_crystals]\n"
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
    profile = catalog_profile(provider_type="ollama", url="http://localhost:11434")
    save_provider_profile(config, "ollama", profile)
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.knowledge_crystals]\n"
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
                identity=dream_profile_cache_identity("ollama", runtime_profile(profile)),
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
    profile = catalog_profile(provider_type="ollama", url="http://localhost:11434")
    save_provider_profile(config, "ollama", profile)
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "[providers.ollama]\n"
        "type='ollama'\n"
        "endpoint='http://localhost:11434'\n"
        "[workflows.knowledge_crystals]\n"
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
                identity=dream_profile_cache_identity("ollama", runtime_profile(profile)),
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


def test_doctor_reports_missing_active_provider_api_key(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_provider_profile(
        config,
        "openai",
        catalog_profile(provider_type="openai", url="https://api.openai.com/v1"),
    )
    save_dream_config(
        config,
        replace(
            default_dream_config().with_workflow(
                "crystallization",
                WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
            ),
            enabled=True,
        ),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        report = Doctor(config).run(autofix=False)

    finding = next(error for error in report["errors"] if error.code == "dream_api_key_missing")
    assert finding.message == "API key missing for provider profile"


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


def test_doctor_ignores_llm_model_cache_error_for_obsolete_provider_profile(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    old_profile = RuntimeProviderProfile(
        type="openai",
        endpoint="https://a.example.test/v1",
        api_key="secret-a",
    )
    save_provider_profile(
        config,
        "openai",
        catalog_profile(
            provider_type="openai",
            url="https://b.example.test/v1",
            key="secret-b",
        ),
    )
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="openai",
                models=("gpt-4.1-mini",),
                fetched_at=datetime.now(UTC).isoformat(),
                error="model suggestions unavailable",
                identity=dream_profile_cache_identity("openai", old_profile),
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


def test_doctor_json_does_not_include_raw_api_key_value(config):
    save_provider_profile(
        config,
        "openai",
        catalog_profile(
            provider_type="openai",
            url="https://api.openai.com/v1",
            key="raw-secret-value",
        ),
    )
    save_dream_config(
        config,
        default_dream_config().with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )

    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        payload = report_to_json(Doctor(config).run())

    assert "dream-conf-loaded" in repr(payload)
    assert "raw-secret-value" not in repr(payload)

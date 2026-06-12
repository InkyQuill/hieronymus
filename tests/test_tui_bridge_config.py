import json
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.doctor import Doctor, DoctorFinding
from hieronymus.dream_config import (
    ProviderProfile,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    save_dream_config,
)
from hieronymus.dream_providers import HTTPResponse, ProviderRegistry
from hieronymus.ingest_config import load_ingest_config
from hieronymus.llm_cache import dream_profile_cache_identity, load_model_cache
from hieronymus.release_config import load_release_config
from hieronymus.tui_bridge.config_api import ConfigBridge


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def test_config_bootstrap_returns_one_remote_provider_selector(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.bootstrap({})

    assert [choice["name"] for choice in payload["provider_choices"]] == [
        "openai",
        "gemini",
        "anthropic",
    ]
    assert payload["selected_provider"] == "anthropic"
    assert payload["form_values"]["provider"]["api_path"] == "https://api.anthropic.com"
    assert payload["release"] == {
        "update_channel": "stable",
        "update_target": "latest",
    }
    assert payload["form_values"]["release"]["update_channel"] == "stable"
    assert payload["draft"]["release"]["update_channel"] == "stable"
    assert "deterministic" not in [choice["name"] for choice in payload["provider_choices"]]
    openai = payload["provider_choices"][0]
    assert openai["supports_api_path"] is True
    assert "supports_base_url" not in openai


def test_config_bootstrap_exposes_ingest_config_defaults(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).bootstrap({})

    assert payload["config_paths"]["ingest_config_path"].endswith("ingest.conf")
    assert payload["ingest"]["short_memory"]["warning_sentence_count"] == 6
    assert payload["ingest"]["short_memory"]["rejection_sentence_count"] == 30
    assert payload["ingest"]["learn"]["max_block_chars"] == 1200


def test_config_bootstrap_exposes_redacted_dream_config_and_model_cache(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="raw-secret-value",
                timeout_seconds=12.0,
            ),
        ),
    )

    payload = ConfigBridge(config).bootstrap({})

    assert payload["dreaming"]["min_pending_short_term_memories"] == 20
    assert payload["providers"]["openai"] == {
        "type": "openai",
        "endpoint": "https://llm.example.test/v1",
        "api_key": "***",
        "timeout_seconds": 12.0,
    }
    assert payload["workflows"]["crystallization"]["provider"] == "anthropic"
    assert payload["model_cache"] == {"providers": {}}


def test_config_payload_redacts_api_key_and_preserves_existing_secret_on_save(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    save_dream_config(
        config,
        default_dream_config()
        .with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="raw-secret-value",
                timeout_seconds=12.0,
            ),
        )
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )
    bridge = ConfigBridge(config)

    bootstrap = bridge.bootstrap({})
    updated = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key": "***",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "12",
            },
        }
    )
    save_payload = bridge.save({"draft": updated["draft"]})

    assert "raw-secret-value" not in repr(bootstrap)
    assert "raw-secret-value" not in json.dumps(bootstrap, sort_keys=True)
    assert "raw-secret-value" not in repr(updated)
    assert "raw-secret-value" not in json.dumps(updated, sort_keys=True)
    assert save_payload["validation"]["ok"] is True
    assert load_dream_config(config).providers["openai"].api_key == "raw-secret-value"


def test_config_save_accepts_unchanged_bootstrap_draft(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    bootstrap = bridge.bootstrap({})

    payload = bridge.save({"draft": bootstrap["draft"]})

    dream_config = load_dream_config(config)
    assert bootstrap["selected_provider"] == "anthropic"
    assert payload["validation"]["ok"] is True
    assert dream_config.workflows["crystallization"].provider == "anthropic"


def test_config_save_rejects_legacy_partial_draft(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).save(
        {"draft": {"dreaming": {"active_provider": "openai"}, "providers": {}}}
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }


def test_config_update_draft_rejects_legacy_partial_draft(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).update_draft(
        {"draft": {"dreaming": {"active_provider": "openai"}, "providers": {}}}
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }


def test_config_check_provider_rejects_legacy_partial_draft(tmp_path: Path) -> None:
    class Registry:
        def check_profile(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach provider check")

    payload = ConfigBridge(_config(tmp_path), registry=Registry()).check_provider(
        {"draft": {"dreaming": {"active_provider": "openai"}, "providers": {}}}
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }
    assert payload["check_result"] == {}


def test_config_model_suggestions_rejects_legacy_partial_draft(tmp_path: Path) -> None:
    class Registry:
        def list_profile_model_suggestions(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach model suggestions")

    payload = ConfigBridge(_config(tmp_path), registry=Registry()).model_suggestions(
        {"draft": {"dreaming": {"active_provider": "openai"}, "providers": {}}}
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }
    assert payload["suggestions"] == {}


def test_config_bootstrap_survives_malformed_dream_config(tmp_path: Path) -> None:
    config = _config(tmp_path)
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text("[dreaming\n", encoding="utf-8")

    payload = ConfigBridge(config).bootstrap({})

    assert payload["selected_provider"] == "anthropic"
    assert payload["draft"]["providers"]["anthropic"]["enabled"] is True
    assert payload["dreaming"]["min_pending_short_term_memories"] == 20
    assert payload["providers"]["openai"]["api_key"] == ""
    assert payload["workflows"]["crystallization"]["provider"] == "anthropic"
    assert payload["model_cache"] == {"providers": {}}
    assert payload["validation"]["ok"] is False
    assert any("dream.conf is not valid TOML" in error for error in payload["validation"]["errors"])
    assert "dream.conf is not valid TOML" in payload["detail"]


def test_config_bootstrap_survives_malformed_ingest_config(tmp_path: Path) -> None:
    config = _config(tmp_path)
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text("[short_memory\n", encoding="utf-8")

    payload = ConfigBridge(config).bootstrap({})

    assert payload["ingest"]["short_memory"]["warning_sentence_count"] == 6
    assert payload["ingest"]["short_memory"]["rejection_sentence_count"] == 30
    assert payload["ingest"]["learn"]["max_block_chars"] == 1200
    assert payload["validation"]["ok"] is False
    assert any(
        "ingest.conf is not valid TOML" in error for error in payload["validation"]["errors"]
    )
    assert "ingest.conf is not valid TOML" in payload["detail"]


def test_config_select_provider_enables_only_selected_remote_provider(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.select_provider({"provider": "gemini", "draft": {}})

    assert payload["selected_provider"] == "gemini"
    providers = payload["draft"]["providers"]
    assert providers["gemini"]["enabled"] is True
    assert providers["openai"]["enabled"] is False
    assert providers["anthropic"]["enabled"] is False
    assert payload["draft"]["dreaming"]["active_provider"] == "gemini"


def test_config_update_draft_uses_api_path_alias_for_base_url(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1",
                "api_key": "plain-secret",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "12.5",
            },
            "dreaming": {
                "autostart_enabled": "yes",
                "min_interval_minutes": "9",
                "new_short_term_memory_threshold": "3",
                "max_cycles_per_autostart": "2",
            },
        }
    )

    assert payload["validation"]["ok"] is True
    assert payload["form_values"]["provider"]["api_path"] == "https://llm.example.test/v1"
    assert payload["draft"]["providers"]["openai"]["base_url"] == "https://llm.example.test/v1"


def test_config_save_persists_valid_selected_provider(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "gemini",
            "provider": {
                "model": "gemini-2.5-flash",
                "api_key": "plain-secret",
                "api_path": "",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "30",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
        }
    )["draft"]

    payload = bridge.save({"draft": draft})

    dream_config = load_dream_config(config)
    assert payload["validation"]["ok"] is True
    assert dream_config.workflows["crystallization"].provider == "gemini"
    assert dream_config.workflows["crystallization"].model == "gemini-2.5-flash"


def test_config_save_persists_dream_and_ingest_config_without_settings(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1",
                "api_key": "plain-secret",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "12.5",
            },
            "dreaming": {
                "autostart_enabled": "yes",
                "min_interval_minutes": "9",
                "new_short_term_memory_threshold": "3",
                "max_cycles_per_autostart": "2",
            },
            "ingest": {
                "warning_sentence_count": "7",
                "rejection_sentence_count": "31",
                "max_block_chars": "1300",
            },
        }
    )["draft"]

    payload = bridge.save({"selected_provider": "openai", "draft": draft})

    dream_config = load_dream_config(config)
    ingest_config = load_ingest_config(config)
    assert payload["validation"]["ok"] is True
    assert dream_config.providers["openai"].api_key == "plain-secret"
    assert dream_config.providers["openai"].endpoint == "https://llm.example.test/v1"
    assert dream_config.workflows["crystallization"].provider == "openai"
    assert dream_config.workflows["crystallization"].model == "gpt-4.1"
    assert dream_config.enabled is True
    assert dream_config.schedule_interval_minutes == 9
    assert ingest_config.short_memory.warning_sentence_count == 7
    assert ingest_config.short_memory.rejection_sentence_count == 31
    assert ingest_config.learn.max_block_chars == 1300
    assert not config.settings_path.exists()


def test_config_save_persists_release_update_channel(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key": "plain-secret",
                "api_path": "https://api.openai.com/v1",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "30",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
            "release": {
                "update_channel": "dev",
            },
        }
    )["draft"]

    payload = bridge.save({"draft": draft})

    assert payload["validation"]["ok"] is True
    assert payload["release"]["update_channel"] == "dev"
    assert payload["release"]["update_target"] == "main"
    assert load_release_config(config).update_channel == "dev"


def test_config_save_rejects_invalid_release_update_channel(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))
    draft = bridge.bootstrap({})["draft"]
    draft["release"] = {"update_channel": "nightly"}

    payload = bridge.save({"draft": draft})

    assert payload["validation"] == {
        "ok": False,
        "errors": ["updates.channel must be one of: dev, stable"],
    }


def test_config_bootstrap_survives_malformed_release_config(tmp_path: Path) -> None:
    config = _config(tmp_path)
    config.config_root.mkdir(parents=True)
    config.release_config_path.write_text("[updates\n", encoding="utf-8")

    payload = ConfigBridge(config).bootstrap({})

    assert payload["release"]["update_channel"] == "stable"
    assert payload["validation"]["ok"] is False
    assert any(
        "release.conf is not valid TOML" in error for error in payload["validation"]["errors"]
    )
    assert "release.conf is not valid TOML" in payload["detail"]


def test_config_save_applies_selected_provider_to_valid_draft(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key": "plain-secret",
                "api_path": "https://api.openai.com/v1",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "30",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
        }
    )["draft"]

    payload = bridge.save({"selected_provider": "gemini", "draft": draft})

    dream_config = load_dream_config(config)
    assert payload["validation"]["ok"] is True
    assert payload["selected_provider"] == "gemini"
    assert dream_config.workflows["crystallization"].provider == "gemini"


def test_config_save_rejects_invalid_dreaming_threshold(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))
    draft = bridge.bootstrap({})["draft"]
    dream = draft["dream"]
    dream["dreaming"] = {**dream["dreaming"], "schedule_interval_minutes": 0}

    payload = bridge.save({"draft": draft})

    assert payload["validation"] == {
        "ok": False,
        "errors": ["schedule_interval_minutes must be at least 1"],
    }


def test_config_check_provider_redacts_error(tmp_path: Path) -> None:
    class Registry:
        def list_model_suggestions(self, *args, **kwargs):
            return {"provider": "openai", "models": [], "source": "unavailable", "error": ""}

        def check_profile(self, *args, **kwargs):
            class Result:
                def to_json_dict(self):
                    return {
                        "name": "openai",
                        "ok": False,
                        "model": "gpt-4.1-mini",
                        "error": "provider returned raw-secret-value",
                        "latency_ms": 10,
                    }

            return Result()

    config = _config(tmp_path)
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("openai", ProviderProfile(type="openai", api_key="raw-secret-value"))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )
    bridge = ConfigBridge(config, registry=Registry())

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})

    assert payload["check_result"]["error"] == "provider returned [redacted]"


def test_config_check_provider_success_updates_model_cache(
    tmp_path: Path,
) -> None:
    class Transport:
        def post_json(self, *args, **kwargs):
            return HTTPResponse(status=200, body="{}")

        def get_json(self, *args, **kwargs):
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"gpt-test"},{"id":"gpt-test-mini"}]}',
            )

    config = _config(tmp_path)
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("openai", ProviderProfile(type="openai", api_key="raw-secret-value"))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )
    bridge = ConfigBridge(config, registry=ProviderRegistry(Transport()))

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})

    assert payload["check_result"]["ok"] is True
    assert payload["suggestions"]["models"] == ["gpt-test", "gpt-test-mini"]
    assert payload["model_cache"]["providers"]["openai"]["models"] == [
        "gpt-test",
        "gpt-test-mini",
    ]
    assert load_model_cache(config).providers["openai"].models == (
        "gpt-test",
        "gpt-test-mini",
    )


def test_config_check_dream_profile_updates_cache_consumed_by_doctor(
    tmp_path: Path,
) -> None:
    class Transport:
        def post_json(self, *args, **kwargs):
            return HTTPResponse(status=200, body="{}")

        def get_json(self, *args, **kwargs):
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"gpt-test"},{"id":"gpt-test-mini"}]}',
            )

    config = _config(tmp_path)
    profile = ProviderProfile(
        type="openai",
        endpoint="https://llm.example.test/v1",
        api_key="raw-secret-value",
    )
    save_dream_config(
        config,
        replace(default_dream_config(), enabled=True)
        .with_provider("openai", profile)
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="missing-model", enabled=True),
        ),
    )
    bridge = ConfigBridge(config, registry=ProviderRegistry(Transport()))

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})
    report = _run_doctor_without_daemon(config)

    entry = load_model_cache(config).providers["openai"]
    assert payload["check_result"]["ok"] is True
    assert payload["model_cache"]["providers"]["openai"]["identity"] == entry.identity
    assert entry.identity == dream_profile_cache_identity("openai", profile)
    assert "raw-secret-value" not in entry.identity
    assert (
        DoctorFinding(
            level="warning",
            code="dream_model_missing",
            message="Configured model was not found in provider cache",
        )
        in report["warnings"]
    )


def test_config_check_dream_profile_failure_updates_cache_consumed_by_doctor(
    tmp_path: Path,
) -> None:
    class Transport:
        def post_json(self, *args, **kwargs):
            return HTTPResponse(status=403, body="forbidden")

        def get_json(self, *args, **kwargs):
            raise AssertionError("failed check should not fetch model suggestions")

    config = _config(tmp_path)
    profile = ProviderProfile(
        type="openai",
        endpoint="https://llm.example.test/v1",
        api_key="raw-secret-value",
    )
    save_dream_config(
        config,
        replace(default_dream_config(), enabled=True)
        .with_provider("openai", profile)
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )
    bridge = ConfigBridge(config, registry=ProviderRegistry(Transport()))

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})
    report = _run_doctor_without_daemon(config)

    entry = load_model_cache(config).providers["openai"]
    assert payload["check_result"]["ok"] is False
    assert entry.error == "provider returned HTTP 403"
    assert entry.identity == dream_profile_cache_identity("openai", profile)
    assert "raw-secret-value" not in entry.identity
    assert (
        DoctorFinding(
            level="error",
            code="dream_api_key_rejected",
            message="API key rejected with 403",
        )
        in report["errors"]
    )


def test_config_check_provider_returns_validation_for_malformed_providers_container(
    tmp_path: Path,
) -> None:
    class Registry:
        def check(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach provider check")

    bridge = ConfigBridge(_config(tmp_path), registry=Registry())

    payload = bridge.check_provider(
        {
            "selected_provider": "openai",
            "draft": {"providers": "bad"},
        }
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }
    assert payload["check_result"] == {}


def test_config_model_suggestions_returns_validation_for_malformed_timeout(
    tmp_path: Path,
) -> None:
    class Registry:
        def list_model_suggestions(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach model suggestions")

    bridge = ConfigBridge(_config(tmp_path), registry=Registry())

    payload = bridge.model_suggestions(
        {
            "selected_provider": "openai",
            "provider": {"timeout_seconds": "slow"},
        }
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["providers.openai.timeout_seconds must be a number"],
    }
    assert payload["suggestions"] == {}


def test_config_model_suggestions_returns_validation_for_malformed_dreaming_container(
    tmp_path: Path,
) -> None:
    class Registry:
        def list_model_suggestions(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach model suggestions")

    bridge = ConfigBridge(_config(tmp_path), registry=Registry())

    payload = bridge.model_suggestions(
        {
            "selected_provider": "openai",
            "draft": {"dreaming": ["bad"]},
        }
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["draft must include dream, ingest, and release"],
    }
    assert payload["suggestions"] == {}


def test_config_model_suggestions_fall_back_to_defaults(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.model_suggestions({"selected_provider": "anthropic", "draft": {}})

    assert payload["suggestions"]["provider"] == "anthropic"
    assert "claude-3-5-haiku-latest" in payload["suggestions"]["models"]


def _run_doctor_without_daemon(config: HieronymusConfig):
    with patch("hieronymus.doctor.ServiceManager") as manager_class:
        manager_class.return_value.status.return_value = {
            "running": False,
            "reason": "no-state",
        }
        return Doctor(config).run(autofix=False)

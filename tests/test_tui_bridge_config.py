from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import ProviderProfile, default_dream_config, save_dream_config
from hieronymus.dream_providers import HTTPResponse, ProviderRegistry
from hieronymus.llm_cache import load_model_cache
from hieronymus.settings import load_settings
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
    assert payload["selected_provider"] == "openai"
    assert payload["form_values"]["provider"]["api_path"] == "https://api.openai.com/v1"
    assert "deterministic" not in [choice["name"] for choice in payload["provider_choices"]]
    openai = payload["provider_choices"][0]
    assert openai["supports_api_path"] is True
    assert "supports_base_url" not in openai


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
                "api_key_env": "HIERONYMUS_OPENAI_KEY",
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
                "api_key_env": "GEMINI_API_KEY",
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

    settings = load_settings(config)
    assert payload["validation"]["ok"] is True
    assert settings.dreaming.active_provider == "gemini"
    assert settings.providers["gemini"].enabled is True
    assert settings.providers["openai"].enabled is False


def test_config_save_applies_selected_provider_to_valid_draft(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key_env": "OPENAI_API_KEY",
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

    settings = load_settings(config)
    assert payload["validation"]["ok"] is True
    assert payload["selected_provider"] == "gemini"
    assert settings.dreaming.active_provider == "gemini"
    assert settings.providers["gemini"].enabled is True
    assert settings.providers["openai"].enabled is False


def test_config_save_rejects_invalid_dreaming_threshold(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key_env": "OPENAI_API_KEY",
                "api_path": "https://api.openai.com/v1",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "0",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
        }
    )["draft"]

    payload = bridge.save({"draft": draft})

    assert payload["validation"] == {
        "ok": False,
        "errors": ["min_interval_minutes must be at least 1"],
    }


def test_config_check_provider_redacts_error(tmp_path: Path, monkeypatch) -> None:
    class Registry:
        def list_model_suggestions(self, *args, **kwargs):
            return {"provider": "openai", "models": [], "source": "unavailable", "error": ""}

        def check(self, *args, **kwargs):
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
    monkeypatch.setenv("OPENAI_API_KEY", "raw-secret-value")
    bridge = ConfigBridge(config, registry=Registry())

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})

    assert payload["check_result"]["error"] == "provider returned [redacted]"
    assert "raw-secret-value" not in repr(payload)


def test_config_check_provider_success_updates_model_cache(
    tmp_path: Path,
    monkeypatch,
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
    monkeypatch.setenv("OPENAI_API_KEY", "raw-secret-value")
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


def test_config_check_provider_returns_validation_for_malformed_api_key_env(
    tmp_path: Path,
) -> None:
    class Registry:
        def check(self, *args, **kwargs):
            raise AssertionError("invalid draft should not reach provider check")

    bridge = ConfigBridge(_config(tmp_path), registry=Registry())

    payload = bridge.check_provider(
        {
            "selected_provider": "openai",
            "draft": {"providers": {"openai": {"api_key_env": 123}}},
        }
    )

    assert payload["validation"] == {
        "ok": False,
        "errors": ["providers.openai.api_key_env must be a string"],
    }
    assert payload["check_result"] == {}


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
        "errors": ["providers must be a table"],
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
            "draft": {"providers": {"openai": {"timeout_seconds": "slow"}}},
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
        "errors": ["dreaming must be a table"],
    }
    assert payload["suggestions"] == {}


def test_config_model_suggestions_fall_back_to_defaults(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.model_suggestions({"selected_provider": "anthropic", "draft": {}})

    assert payload["suggestions"]["provider"] == "anthropic"
    assert "claude-3-5-haiku-latest" in payload["suggestions"]["models"]

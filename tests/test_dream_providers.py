from __future__ import annotations

import json
import urllib.error
from dataclasses import dataclass, replace
from datetime import UTC, datetime, timedelta

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    ProviderProfile,
    WorkflowProfile,
    default_dream_config,
    save_dream_config,
)
from hieronymus.dream_providers import (
    ANTHROPIC_API_VERSION,
    HTTPResponse,
    ProviderCheckResult,
    ProviderRegistry,
    UrllibTransport,
    resolve_profile_provider,
    resolve_provider,
)
from hieronymus.llm_cache import CachedModels, ModelCacheEntry, load_model_cache, save_model_cache
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.settings import (
    DreamingSettings,
    ProviderSettings,
    load_settings,
    save_settings,
)


@dataclass
class FakeTransport:
    response: HTTPResponse
    requests: list[dict[str, object]]

    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse:
        self.requests.append(
            {"url": url, "headers": headers, "payload": payload, "timeout": timeout}
        )
        return self.response


def _save_provider_profile(
    config: HieronymusConfig,
    name: str,
    profile: ProviderProfile,
    *,
    model: str = "model",
) -> None:
    save_dream_config(
        config,
        default_dream_config()
        .with_provider(name, profile)
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider=name, model=model, enabled=True),
        ),
    )


def test_registry_lists_real_providers() -> None:
    registry = ProviderRegistry()

    assert [provider.name for provider in registry.list()] == [
        "deterministic",
        "openai",
        "gemini",
        "anthropic",
    ]
    openai = next(provider for provider in registry.list() if provider.name == "openai")
    assert openai.display_name == "OpenAI compatible"


def test_provider_status_uses_dream_config_profiles(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="",
                timeout_seconds=12.5,
            ),
        ),
    )

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "API key missing for provider profile"
    assert openai["base_url"] == "https://llm.example.test/v1"
    assert openai["timeout_seconds"] == 12.5
    assert "api_key_env" not in openai


def test_provider_status_rejects_whitespace_model(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("openai", ProviderProfile(type="openai", api_key="secret-openai"))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="   ", enabled=True),
        ),
    )

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "model is empty for provider profile"


def test_provider_status_rejects_whitespace_api_key(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("openai", ProviderProfile(type="openai", api_key="   "))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "API key missing for provider profile"


def test_provider_status_marks_empty_profile_key_missing(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("openai", ProviderProfile(type="openai", api_key=""))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "API key missing for provider profile"


def test_provider_status_reports_key_presence_without_key_value(config):
    save_dream_config(
        config,
        default_dream_config()
        .with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://api.example.test/v1",
                api_key="sk-live-secret-value",
            ),
        )
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-4.1-mini", enabled=True),
        ),
    )

    payload = ProviderRegistry().status_payload(config)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["api_key_present"] is True
    assert "sk-live-secret-value" not in repr(payload)


def test_provider_status_ignores_unsaved_in_memory_settings(config):
    saved = load_settings(config)
    draft = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            enabled=True,
            api_key_env="DRAFT_OPENAI_KEY",
            model="draft-model",
            base_url="https://draft.example.test/v1",
        ),
    )
    save_dream_config(
        config,
        default_dream_config()
        .with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://dream.example.test/v1",
                api_key="dream-secret",
            ),
        )
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="dream-model", enabled=True),
        ),
    )

    payload = ProviderRegistry().status_payload(config, settings=draft)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["model"] == "dream-model"
    assert openai["api_key_present"] is True
    assert load_settings(config).providers["openai"].model == "gpt-4.1-mini"


def test_urllib_transport_converts_url_error(monkeypatch) -> None:
    def fail(*args, **kwargs):
        raise urllib.error.URLError("network unavailable")

    monkeypatch.setattr("urllib.request.urlopen", fail)

    response = UrllibTransport().post_json(
        "https://api.example.test/v1",
        headers={},
        payload={},
        timeout=1,
    )

    assert response == HTTPResponse(status=0, body="network error")


def test_provider_check_uses_plaintext_profile_key(config):
    class Transport:
        def __init__(self):
            self.calls = []

        def post_json(self, url, *, headers, payload, timeout):
            self.calls.append((url, headers, payload, timeout))
            return type("Response", (), {"status": 200, "body": "{}"})()

    save_dream_config(
        config,
        default_dream_config()
        .with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="secret-test-key",
                timeout_seconds=12.5,
            ),
        )
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="openai", model="gpt-profile", enabled=True),
        ),
    )
    transport = Transport()

    result = ProviderRegistry(transport=transport).check(config, "openai")

    assert result.ok is True
    assert result.model == "gpt-profile"
    assert transport.calls[0][0] == "https://llm.example.test/v1/chat/completions"
    assert transport.calls[0][1]["Authorization"] == "Bearer secret-test-key"
    assert transport.calls[0][3] == 12.5
    assert "secret-test-key" not in repr(result.to_json_dict())


def test_openai_model_suggestions_use_models_endpoint(tmp_path, monkeypatch) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"gpt-4.1-mini"},{"id":"gpt-4.1"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config)
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-openai",
        ),
    )
    transport = Transport()

    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["gpt-4.1", "gpt-4.1-mini"],
        "source": "api",
        "error": "",
    }
    assert transport.requests[0]["url"] == "https://api.openai.com/v1/models"
    assert "secret-openai" not in repr(result.to_json_dict())


def test_deterministic_model_suggestions_ignore_malformed_settings(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text("[dreaming\n", encoding="utf-8")

    result = ProviderRegistry().list_model_suggestions(config, "deterministic")

    assert result.to_json_dict() == {
        "provider": "deterministic",
        "models": [""],
        "source": "defaults",
        "error": "",
    }
    assert not config.llm_cache_path.exists()


def test_anthropic_model_suggestions_ignore_malformed_settings_and_cache_defaults(
    tmp_path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text("[dreaming\n", encoding="utf-8")

    result = ProviderRegistry().list_model_suggestions(config, "anthropic")

    assert result.to_json_dict() == {
        "provider": "anthropic",
        "models": ["claude-3-5-haiku-latest", "claude-3-7-sonnet-latest"],
        "source": "defaults",
        "error": "",
    }
    assert load_model_cache(config).providers["anthropic"].models == tuple(result.models)


def test_model_suggestions_use_fresh_cache_without_network(tmp_path, monkeypatch) -> None:
    class PrimingTransport:
        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"cached-a"},{"id":"cached-b"}]}',
            )

    class CachedTransport:
        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for cached suggestions")

        def get_json(self, *args, **kwargs):
            raise AssertionError("get_json should not be used for cached suggestions")

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config)
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-openai",
        ),
    )

    ProviderRegistry(PrimingTransport()).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    result = ProviderRegistry(CachedTransport()).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["cached-a", "cached-b"],
        "source": "llmcache.tmp",
        "error": "",
    }


def test_model_suggestions_refresh_after_cached_error_resolves(
    tmp_path,
    monkeypatch,
) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"fresh-after-error"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config)
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(type="openai", endpoint="https://api.openai.com/v1", api_key=""),
    )
    transport = Transport()

    first = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-openai",
        ),
    )
    second = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert first.error == "API key missing for provider profile"
    assert second.to_json_dict() == {
        "provider": "openai",
        "models": ["fresh-after-error"],
        "source": "api",
        "error": "",
    }
    assert transport.requests[0]["url"] == "https://api.openai.com/v1/models"
    assert load_model_cache(config).providers["openai"].error == ""


def test_model_suggestions_return_api_result_when_cache_save_fails(
    tmp_path,
    monkeypatch,
) -> None:
    class Transport:
        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"fresh-without-cache"}]}',
            )

    def fail_save(*args, **kwargs):
        raise PermissionError("cache is not writable")

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config)
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-openai",
        ),
    )
    monkeypatch.setattr("hieronymus.dream_providers.save_model_cache", fail_save)

    result = ProviderRegistry(Transport()).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["fresh-without-cache"],
        "source": "api",
        "error": "",
    }


def test_model_suggestions_refresh_and_save_stale_cache(tmp_path, monkeypatch) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"fresh-model"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(
        config,
        CachedModels().with_entry(
            ModelCacheEntry(
                provider="openai",
                models=("stale-model",),
                fetched_at=(datetime.now(UTC) - timedelta(hours=24)).isoformat(),
            )
        ),
    )
    settings = load_settings(config)
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-openai",
        ),
    )
    transport = Transport()

    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["fresh-model"],
        "source": "api",
        "error": "",
    }
    assert transport.requests[0]["url"] == "https://api.openai.com/v1/models"
    assert load_model_cache(config).providers["openai"].models == ("fresh-model",)


def test_model_suggestions_refresh_when_openai_base_url_changes(
    tmp_path,
    monkeypatch,
) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            if url == "https://a.example.test/v1/models":
                return HTTPResponse(
                    status=200,
                    body='{"data":[{"id":"model-from-a"}]}',
                )
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"model-from-b"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    saved = load_settings(config)
    settings_a = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            base_url="https://a.example.test/v1",
        ),
    )
    settings_b = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            base_url="https://b.example.test/v1",
        ),
    )
    transport = Transport()
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://a.example.test/v1",
            api_key="secret-openai",
        ),
    )

    first = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings_a,
    )
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://b.example.test/v1",
            api_key="secret-openai",
        ),
    )
    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings_b,
    )

    assert first.models == ["model-from-a"]
    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["model-from-b"],
        "source": "api",
        "error": "",
    }
    assert [request["url"] for request in transport.requests] == [
        "https://a.example.test/v1/models",
        "https://b.example.test/v1/models",
    ]


def test_model_suggestions_refresh_when_openai_api_key_env_changes(
    tmp_path,
    monkeypatch,
) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            if headers["Authorization"] == "Bearer secret-a":
                return HTTPResponse(
                    status=200,
                    body='{"data":[{"id":"model-from-key-a"}]}',
                )
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"model-from-key-b"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    saved = load_settings(config)
    settings_a = saved.with_provider(
        "openai",
        replace(saved.providers["openai"], api_key_env="OPENAI_KEY_A"),
    )
    settings_b = saved.with_provider(
        "openai",
        replace(saved.providers["openai"], api_key_env="OPENAI_KEY_B"),
    )
    transport = Transport()
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-a",
        ),
    )

    first = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings_a,
    )
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://api.openai.com/v1",
            api_key="secret-b",
        ),
    )
    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings_b,
    )

    assert first.models == ["model-from-key-a"]
    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["model-from-key-b"],
        "source": "api",
        "error": "",
    }
    assert [request["headers"]["Authorization"] for request in transport.requests] == [
        "Bearer secret-a",
        "Bearer secret-b",
    ]


def test_model_suggestions_refresh_when_gemini_api_key_env_changes(
    tmp_path,
    monkeypatch,
) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            if headers["x-goog-api-key"] == "secret-a":
                return HTTPResponse(
                    status=200,
                    body='{"models":[{"name":"models/gemini-from-key-a"}]}',
                )
            return HTTPResponse(
                status=200,
                body='{"models":[{"name":"models/gemini-from-key-b"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    saved = load_settings(config)
    settings_a = saved.with_provider(
        "gemini",
        replace(saved.providers["gemini"], api_key_env="GEMINI_KEY_A"),
    )
    settings_b = saved.with_provider(
        "gemini",
        replace(saved.providers["gemini"], api_key_env="GEMINI_KEY_B"),
    )
    transport = Transport()
    _save_provider_profile(
        config,
        "gemini",
        ProviderProfile(
            type="gemini",
            endpoint="https://generativelanguage.googleapis.com",
            api_key="secret-a",
        ),
    )

    first = ProviderRegistry(transport).list_model_suggestions(
        config,
        "gemini",
        settings=settings_a,
    )
    _save_provider_profile(
        config,
        "gemini",
        ProviderProfile(
            type="gemini",
            endpoint="https://generativelanguage.googleapis.com",
            api_key="secret-b",
        ),
    )
    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "gemini",
        settings=settings_b,
    )

    assert first.models == ["gemini-from-key-a"]
    assert result.to_json_dict() == {
        "provider": "gemini",
        "models": ["gemini-from-key-b"],
        "source": "api",
        "error": "",
    }
    assert [request["headers"]["x-goog-api-key"] for request in transport.requests] == [
        "secret-a",
        "secret-b",
    ]


def test_anthropic_model_suggestions_cache_default_hints(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    result = ProviderRegistry().list_model_suggestions(config, "anthropic")

    assert result.source == "defaults"
    assert load_model_cache(config).providers["anthropic"].models == tuple(result.models)


def test_anthropic_model_suggestions_return_defaults_when_cache_save_fails(
    tmp_path,
    monkeypatch,
) -> None:
    def fail_save(*args, **kwargs):
        raise FileExistsError("cache temp path already exists")

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    monkeypatch.setattr("hieronymus.dream_providers.save_model_cache", fail_save)

    result = ProviderRegistry().list_model_suggestions(config, "anthropic")

    assert result.to_json_dict() == {
        "provider": "anthropic",
        "models": ["claude-3-5-haiku-latest", "claude-3-7-sonnet-latest"],
        "source": "defaults",
        "error": "",
    }


def test_deterministic_check_passes_without_network(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    result = ProviderRegistry().check(config, "deterministic")

    assert result == ProviderCheckResult(
        name="deterministic",
        ok=True,
        model="",
        error="",
        latency_ms=None,
    )


def test_status_payload_includes_default_ollama_profile(config: HieronymusConfig) -> None:
    payload = ProviderRegistry().status_payload(config)

    ollama = next(row for row in payload if row["name"] == "ollama")
    assert ollama["configured"] is True
    assert ollama["api_key_present"] is False
    assert ollama["base_url"] == "http://localhost:11434"
    assert ollama["model"] == "gemma4-e3b"


def test_ollama_profile_check_uses_dream_config_without_api_key(
    config: HieronymusConfig,
) -> None:
    transport = FakeTransport(HTTPResponse(status=200, body=json.dumps({"id": "ok"})), [])

    result = ProviderRegistry(transport=transport).check(config, "ollama")

    assert result.ok is True
    assert transport.requests[0]["url"] == "http://localhost:11434/api/chat"
    assert transport.requests[0]["headers"] == {}
    assert transport.requests[0]["payload"]["model"] == "gemma4-e3b"


def test_non_ollama_profile_type_requires_api_key(config: HieronymusConfig) -> None:
    save_dream_config(
        config,
        default_dream_config()
        .with_provider("local_openai", ProviderProfile(type="openai", api_key=""))
        .with_workflow(
            "crystallization",
            WorkflowProfile(provider="local_openai", model="gpt-local", enabled=True),
        ),
    )

    payload = ProviderRegistry().status_payload(config)
    local_openai = next(row for row in payload if row["name"] == "local_openai")

    assert local_openai["configured"] is False
    assert local_openai["error"] == "API key missing for provider profile"


def test_openai_check_uses_plaintext_profile_key(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    _save_provider_profile(
        config,
        "openai",
        ProviderProfile(
            type="openai",
            endpoint="https://llm.example.test/v1",
            api_key="secret-test-key",
            timeout_seconds=12.5,
        ),
        model="gpt-4.1-mini",
    )
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"id": "ok"})),
        [],
    )

    result = ProviderRegistry(transport=transport).check(config, "openai")

    assert result.ok is True
    assert transport.requests[0]["url"] == "https://llm.example.test/v1/chat/completions"
    assert transport.requests[0]["headers"]["Authorization"] == "Bearer secret-test-key"
    assert transport.requests[0]["timeout"] == 12.5


def test_gemini_check_uses_api_key_header_without_url_secret(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    _save_provider_profile(
        config,
        "gemini",
        ProviderProfile(
            type="gemini",
            endpoint="https://generativelanguage.googleapis.com",
            api_key="secret-gemini",
        ),
        model="gemini-2.5-flash",
    )
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"id": "ok"})),
        [],
    )

    result = ProviderRegistry(transport=transport).check(config, "gemini")

    assert result.ok is True
    assert (
        transport.requests[0]["url"] == "https://generativelanguage.googleapis.com/v1beta/models/"
        "gemini-2.5-flash:generateContent"
    )
    assert "secret-gemini" not in transport.requests[0]["url"]
    assert transport.requests[0]["headers"]["x-goog-api-key"] == "secret-gemini"


def test_resolve_provider_rejects_disabled_provider(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    try:
        resolve_provider(config, "openai")
    except ValueError as exc:
        assert str(exc) == "dream provider is disabled: openai"
    else:
        raise AssertionError("disabled provider should fail")


def test_resolve_profile_provider_rejects_missing_profile(config: HieronymusConfig) -> None:
    with pytest.raises(ValueError, match="referenced provider profile is missing: missing"):
        resolve_profile_provider(config, "missing", model="model")


def test_resolve_profile_provider_rejects_missing_model(config: HieronymusConfig) -> None:
    with pytest.raises(
        ValueError,
        match="model must not be empty for provider profile: openai",
    ):
        resolve_profile_provider(config, "openai", model=" ")


def test_resolve_profile_provider_requires_plaintext_api_key_for_non_ollama(
    config: HieronymusConfig,
) -> None:
    with pytest.raises(ValueError, match="API key missing for provider profile: openai"):
        resolve_profile_provider(config, "openai", model="gpt-4.1-mini")


@pytest.mark.parametrize(
    ("profile_name", "profile", "model", "response_body", "expected"),
    [
        (
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://openai.example.test/v1",
                api_key="plain-openai",
                timeout_seconds=6.5,
            ),
            "gpt-profile",
            lambda: json.dumps({"choices": [{"message": {"content": json.dumps(_llm_payload())}}]}),
            {
                "url": "https://openai.example.test/v1/chat/completions",
                "header": "Authorization",
                "value": "Bearer plain-openai",
            },
        ),
        (
            "anthropic",
            ProviderProfile(
                type="anthropic",
                endpoint="https://anthropic.example.test",
                api_key="plain-anthropic",
                timeout_seconds=7.5,
            ),
            "claude-profile",
            lambda: json.dumps({"content": [{"type": "text", "text": json.dumps(_llm_payload())}]}),
            {
                "url": "https://anthropic.example.test/v1/messages",
                "header": "x-api-key",
                "value": "plain-anthropic",
            },
        ),
        (
            "gemini",
            ProviderProfile(
                type="gemini",
                endpoint="https://gemini.example.test",
                api_key="plain-gemini",
                timeout_seconds=8.5,
            ),
            "gemini-profile",
            lambda: json.dumps(
                {"candidates": [{"content": {"parts": [{"text": json.dumps(_llm_payload())}]}}]}
            ),
            {
                "url": "https://gemini.example.test/v1beta/models/gemini-profile:generateContent",
                "header": "x-goog-api-key",
                "value": "plain-gemini",
            },
        ),
    ],
)
def test_resolve_profile_provider_uses_plaintext_api_key(
    config: HieronymusConfig,
    profile_name: str,
    profile: ProviderProfile,
    model: str,
    response_body,
    expected: dict[str, str],
) -> None:
    save_dream_config(config, default_dream_config().with_provider(profile_name, profile))
    transport = FakeTransport(HTTPResponse(status=200, body=response_body()), [])

    provider = resolve_profile_provider(
        config,
        profile_name,
        model=model,
        transport=transport,
    )
    output = provider.crystallize(_context(), [_memory()])

    request = transport.requests[0]
    assert output.crystals[0].title == "Compact UI Labels"
    assert request["url"] == expected["url"]
    assert request["headers"][expected["header"]] == expected["value"]
    if "model" in request["payload"]:
        assert request["payload"]["model"] == model
    else:
        assert model in request["url"]
    assert request["timeout"] == profile.timeout_seconds


def test_resolve_profile_provider_supports_native_ollama_without_api_key(
    config: HieronymusConfig,
) -> None:
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "ollama",
            ProviderProfile(
                type="ollama",
                endpoint="http://ollama.example.test",
                timeout_seconds=4.5,
            ),
        ),
    )
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"message": {"content": json.dumps(_llm_payload())}}),
        ),
        [],
    )

    provider = resolve_profile_provider(config, "ollama", model="gemma4-e3b", transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    request = transport.requests[0]
    assert output.concept_proposals[0].concept_text == "Sense"
    assert request["url"] == "http://ollama.example.test/api/chat"
    assert request["headers"] == {}
    assert request["payload"]["model"] == "gemma4-e3b"
    assert request["payload"]["stream"] is False
    assert request["timeout"] == 4.5


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )


def _memory() -> ShortTermMemoryRecord:
    return ShortTermMemoryRecord(
        id=7,
        session_id=3,
        source_role="user",
        kind="style",
        text="Use compact UI labels for inventory skill names.",
        source_ref="chapter 1",
        metadata={},
    )


def _typed_context() -> TranslationContext:
    return TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        task_type="translation",
        volume="1",
        chapter="2",
        tags=("legacy-tag",),
        language_tags=("ja", "en", "fr"),
        story_scopes=("arc:academy",),
        semantic_tags=("ui", "term"),
    )


def _typed_memory() -> ShortTermMemoryRecord:
    return ShortTermMemoryRecord(
        id=8,
        session_id=3,
        source_role="user",
        kind="correction",
        text="Use Sense as a game-system term.",
        source_ref="chapter 2",
        metadata={},
        language_tags=("ja", "en"),
        story_scopes=("arc:academy",),
        semantic_tags=("term",),
        source_credibility="user_rule",
        rule_intent="terminology",
        soft_origin="inline-correction",
    )


def _llm_payload() -> dict[str, object]:
    return {
        "crystals": [
            {
                "crystal_type": "lesson",
                "title": "Compact UI Labels",
                "text": "Use compact UI labels for inventory skill names.",
                "strength": 0.7,
                "confidence": 0.8,
                "source_memory_ids": [7],
            }
        ],
        "concept_proposals": [
            {
                "series_slug": "only-sense-online",
                "source_language": "ja",
                "target_language": "en",
                "concept_text": "Sense",
                "source_form": "センス",
                "canonical_rendering": "Sense",
                "approved_variants": ["Sense"],
                "forbidden_variants": ["Senses"],
                "rationale": "Existing series terminology.",
            }
        ],
    }


def test_openai_provider_crystallizes_structured_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="OPENAI_API_KEY",
                base_url="https://api.openai.test/v1",
                timeout_seconds=12.5,
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"choices": [{"message": {"content": json.dumps(_llm_payload())}}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].title == "Compact UI Labels"
    assert output.concept_proposals[0].source_form == "センス"
    assert transport.requests[0]["url"] == "https://api.openai.test/v1/chat/completions"
    assert transport.requests[0]["timeout"] == 12.5


def test_openai_provider_prompt_includes_typed_context_and_memory_metadata(
    tmp_path,
    monkeypatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="OPENAI_API_KEY",
                base_url="https://api.openai.test/v1",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"choices": [{"message": {"content": json.dumps(_llm_payload())}}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    provider.crystallize(_typed_context(), [_typed_memory()])

    request_payload = transport.requests[0]["payload"]
    prompt = json.loads(request_payload["messages"][0]["content"])
    assert prompt["context"] == {
        "series_slug": "only-sense-online",
        "source_language": "ja",
        "target_language": "en",
        "task_type": "translation",
        "volume": "1",
        "chapter": "2",
        "tags": ["legacy-tag"],
        "language_tags": ["ja", "en", "fr"],
        "story_scopes": ["arc:academy"],
        "semantic_tags": ["ui", "term"],
    }
    assert prompt["memories"] == [
        {
            "id": 8,
            "source_role": "user",
            "kind": "correction",
            "text": "Use Sense as a game-system term.",
            "source_ref": "chapter 2",
            "language_tags": ["ja", "en"],
            "story_scopes": ["arc:academy"],
            "semantic_tags": ["term"],
            "source_credibility": "user_rule",
            "rule_intent": "terminology",
            "soft_origin": "inline-correction",
        }
    ]


def test_gemini_provider_crystallizes_structured_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "gemini",
            ProviderSettings(
                enabled=True,
                model="gemini-2.5-flash",
                api_key_env="GEMINI_API_KEY",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="gemini"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("GEMINI_API_KEY", "secret-gemini")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps(
                {"candidates": [{"content": {"parts": [{"text": json.dumps(_llm_payload())}]}}]}
            ),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].source_memory_ids == [7]
    assert (
        transport.requests[0]["url"] == "https://generativelanguage.googleapis.com/v1beta/models/"
        "gemini-2.5-flash:generateContent"
    )
    assert "secret-gemini" not in transport.requests[0]["url"]
    assert transport.requests[0]["headers"]["x-goog-api-key"] == "secret-gemini"


def test_anthropic_provider_crystallizes_structured_response(
    tmp_path,
    monkeypatch,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "anthropic",
            ProviderSettings(
                enabled=True,
                model="claude-3-5-haiku-latest",
                api_key_env="ANTHROPIC_API_KEY",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="anthropic"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "secret-anthropic")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"content": [{"type": "text", "text": json.dumps(_llm_payload())}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].confidence == 0.8
    assert transport.requests[0]["headers"]["x-api-key"] == "secret-anthropic"
    assert transport.requests[0]["headers"]["anthropic-version"] == ANTHROPIC_API_VERSION


@pytest.mark.parametrize(
    ("provider_name", "settings", "response_body", "expected_url"),
    [
        (
            "gemini",
            ProviderSettings(
                enabled=True,
                model="gemini-2.5-flash",
                api_key_env="GEMINI_API_KEY",
                base_url="https://untrusted.example.test",
            ),
            json.dumps(
                {"candidates": [{"content": {"parts": [{"text": json.dumps(_llm_payload())}]}}]}
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/"
            "gemini-2.5-flash:generateContent",
        ),
        (
            "anthropic",
            ProviderSettings(
                enabled=True,
                model="claude-3-5-haiku-latest",
                api_key_env="ANTHROPIC_API_KEY",
                base_url="https://untrusted.example.test",
            ),
            json.dumps({"content": [{"type": "text", "text": json.dumps(_llm_payload())}]}),
            "https://api.anthropic.com/v1/messages",
        ),
    ],
)
def test_legacy_gemini_and_anthropic_ignore_configured_base_url(
    tmp_path,
    monkeypatch,
    provider_name: str,
    settings: ProviderSettings,
    response_body: str,
    expected_url: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    saved = (
        load_settings(config)
        .with_provider(provider_name, settings)
        .with_dreaming(DreamingSettings(active_provider=provider_name))
    )
    save_settings(config, saved)
    monkeypatch.setenv(settings.api_key_env, f"secret-{provider_name}")
    transport = FakeTransport(HTTPResponse(status=200, body=response_body), [])

    provider = resolve_provider(config, transport=transport)
    provider.crystallize(_context(), [_memory()])

    assert transport.requests[0]["url"] == expected_url
    assert "untrusted.example.test" not in transport.requests[0]["url"]


def test_llm_provider_rejects_invalid_json_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="OPENAI_API_KEY",
                base_url="https://api.openai.test/v1",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"choices": [{"message": {"content": "nope"}}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)

    try:
        provider.crystallize(_context(), [_memory()])
    except ValueError as exc:
        assert str(exc) == "openai response did not contain valid dream JSON"
    else:
        raise AssertionError("invalid JSON should fail")


@pytest.mark.parametrize(
    ("provider_name", "body", "expected_error"),
    [
        (
            "openai",
            "{not json",
            "openai response did not match provider envelope",
        ),
        (
            "gemini",
            json.dumps({"candidates": []}),
            "gemini response did not match provider envelope",
        ),
        (
            "anthropic",
            json.dumps({"content": [{"type": "image"}]}),
            "anthropic response did not match provider envelope",
        ),
    ],
)
def test_llm_provider_normalizes_malformed_provider_envelope(
    tmp_path,
    monkeypatch,
    provider_name: str,
    body: str,
    expected_error: str,
) -> None:
    config = _configured_llm_provider(tmp_path, monkeypatch, provider_name)
    transport = FakeTransport(HTTPResponse(status=200, body=body), [])
    provider = resolve_provider(config, transport=transport)

    with pytest.raises(ValueError) as exc_info:
        provider.crystallize(_context(), [_memory()])

    assert str(exc_info.value) == expected_error
    assert body not in str(exc_info.value)
    assert "secret-" not in str(exc_info.value)
    assert "http" not in str(exc_info.value).lower()


def test_llm_provider_rejects_null_crystal_title(tmp_path, monkeypatch) -> None:
    payload = _llm_payload()
    crystals = payload["crystals"]
    assert isinstance(crystals, list)
    crystal = crystals[0]
    assert isinstance(crystal, dict)
    crystal["title"] = None

    _assert_openai_payload_schema_error(tmp_path, monkeypatch, payload)


def test_llm_provider_rejects_numeric_approved_variant(tmp_path, monkeypatch) -> None:
    payload = _llm_payload()
    proposals = payload["concept_proposals"]
    assert isinstance(proposals, list)
    proposal = proposals[0]
    assert isinstance(proposal, dict)
    proposal["approved_variants"] = [1]

    _assert_openai_payload_schema_error(tmp_path, monkeypatch, payload)


def test_llm_provider_rejects_string_source_memory_id(tmp_path, monkeypatch) -> None:
    payload = _llm_payload()
    crystals = payload["crystals"]
    assert isinstance(crystals, list)
    crystal = crystals[0]
    assert isinstance(crystal, dict)
    crystal["source_memory_ids"] = ["7"]

    _assert_openai_payload_schema_error(tmp_path, monkeypatch, payload)


def _assert_openai_payload_schema_error(tmp_path, monkeypatch, payload: dict[str, object]) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="OPENAI_API_KEY",
                base_url="https://api.openai.test/v1",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai"))
    )
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"choices": [{"message": {"content": json.dumps(payload)}}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)

    try:
        provider.crystallize(_context(), [_memory()])
    except ValueError as exc:
        assert str(exc) == "openai response did not match dream schema"
    else:
        raise AssertionError("invalid dream schema should fail")


def _configured_llm_provider(tmp_path, monkeypatch, provider_name: str) -> HieronymusConfig:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    provider_settings = {
        "openai": ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://api.openai.test/v1",
        ),
        "gemini": ProviderSettings(
            enabled=True,
            model="gemini-2.5-flash",
            api_key_env="GEMINI_API_KEY",
        ),
        "anthropic": ProviderSettings(
            enabled=True,
            model="claude-3-5-haiku-latest",
            api_key_env="ANTHROPIC_API_KEY",
        ),
    }[provider_name]
    settings = (
        load_settings(config)
        .with_provider(provider_name, provider_settings)
        .with_dreaming(DreamingSettings(active_provider=provider_name))
    )
    save_settings(config, settings)
    monkeypatch.setenv(provider_settings.api_key_env, f"secret-{provider_name}")
    return config

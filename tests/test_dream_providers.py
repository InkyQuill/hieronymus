from __future__ import annotations

import json
import urllib.error
from dataclasses import dataclass, replace

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import (
    ANTHROPIC_API_VERSION,
    HTTPResponse,
    ProviderCheckResult,
    ProviderRegistry,
    UrllibTransport,
    resolve_provider,
)
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


def test_provider_status_marks_missing_env_for_enabled_provider(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="MISSING_OPENAI_KEY",
            base_url="https://api.openai.com/v1",
        ),
    )
    save_settings(config, settings)
    monkeypatch.delenv("MISSING_OPENAI_KEY", raising=False)

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["enabled"] is True
    assert openai["configured"] is False
    assert openai["error"] == "missing environment variable: MISSING_OPENAI_KEY"


def test_provider_status_rejects_whitespace_model(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=False,
            model="   ",
            api_key_env="OPENAI_API_KEY",
            base_url="https://api.openai.com/v1",
        ),
    )
    save_settings(config, settings)

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "model is empty"


def test_provider_status_rejects_whitespace_api_key_env(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=False,
            model="gpt-4.1-mini",
            api_key_env="   ",
            base_url="https://api.openai.com/v1",
        ),
    )
    save_settings(config, settings)

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "api_key_env is empty"


def test_provider_status_marks_empty_env_value_missing(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://api.openai.com/v1",
        ),
    )
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "")

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "missing environment variable: OPENAI_API_KEY"


def test_provider_status_reports_key_presence_without_key_value(config, monkeypatch):
    monkeypatch.setenv("HIERONYMUS_OPENAI_TEST_KEY", "sk-live-secret-value")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_TEST_KEY",
            base_url="https://api.example.test/v1",
        ),
    )
    save_settings(config, settings)

    payload = ProviderRegistry().status_payload(config)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["api_key_env"] == "HIERONYMUS_OPENAI_TEST_KEY"
    assert openai["api_key_present"] is True
    assert "sk-live-secret-value" not in repr(payload)


def test_provider_status_can_use_unsaved_in_memory_settings(config, monkeypatch):
    monkeypatch.setenv("DRAFT_OPENAI_KEY", "draft-secret")
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

    payload = ProviderRegistry().status_payload(config, settings=draft)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["model"] == "draft-model"
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


def test_provider_check_uses_unsaved_in_memory_settings(config, monkeypatch):
    class Transport:
        def __init__(self):
            self.calls = []

        def post_json(self, url, *, headers, payload, timeout):
            self.calls.append((url, headers, payload, timeout))
            return type("Response", (), {"status": 200, "body": "{}"})()

    monkeypatch.setenv("DRAFT_OPENAI_KEY", "draft-secret")
    saved = load_settings(config)
    draft = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            enabled=True,
            api_key_env="DRAFT_OPENAI_KEY",
            model="draft-model",
            base_url="https://draft.example.test/v1",
            timeout_seconds=7.5,
        ),
    )
    transport = Transport()

    result = ProviderRegistry(transport=transport).check(config, "openai", settings=draft)

    assert result.ok is True
    assert result.model == "draft-model"
    assert transport.calls[0][0] == "https://draft.example.test/v1/chat/completions"
    assert transport.calls[0][3] == 7.5
    assert "draft-secret" not in repr(result.to_json_dict())


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


def test_openai_check_uses_temporary_key_without_saving_secret(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://llm.example.test/v1",
            timeout_seconds=12.5,
        ),
    )
    save_settings(config, settings)
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"id": "ok"})),
        [],
    )

    result = ProviderRegistry(transport=transport).check(
        config,
        "openai",
        temporary_api_key="secret-test-key",
    )

    assert result.ok is True
    assert transport.requests[0]["url"] == "https://llm.example.test/v1/chat/completions"
    assert transport.requests[0]["headers"]["Authorization"] == "Bearer secret-test-key"
    assert transport.requests[0]["timeout"] == 12.5
    assert "secret-test-key" not in config.settings_path.read_text(encoding="utf-8")


def test_gemini_check_uses_api_key_header_without_url_secret(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "gemini",
        ProviderSettings(
            enabled=True,
            model="gemini-2.5-flash",
            api_key_env="GEMINI_API_KEY",
        ),
    )
    save_settings(config, settings)
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"id": "ok"})),
        [],
    )

    result = ProviderRegistry(transport=transport).check(
        config,
        "gemini",
        temporary_api_key="secret-gemini",
    )

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

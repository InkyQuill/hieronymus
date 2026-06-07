from __future__ import annotations

import json
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import (
    HTTPResponse,
    ProviderCheckResult,
    ProviderRegistry,
    resolve_provider,
)
from hieronymus.settings import ProviderSettings, load_settings, save_settings


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
    assert "secret-test-key" not in config.settings_path.read_text(encoding="utf-8")


def test_resolve_provider_rejects_disabled_provider(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    try:
        resolve_provider(config, "openai")
    except ValueError as exc:
        assert str(exc) == "dream provider is disabled: openai"
    else:
        raise AssertionError("disabled provider should fail")

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.dreaming import DeterministicDreamProvider, DreamProvider
from hieronymus.settings import ProviderSettings, load_settings

_DEFAULT_TIMEOUT_SECONDS = 30.0


@dataclass(frozen=True)
class HTTPResponse:
    status: int
    body: str


class HTTPTransport(Protocol):
    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse: ...


class UrllibTransport:
    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse:
        data = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            url,
            data=data,
            headers={**headers, "Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                body = response.read().decode("utf-8", errors="replace")
                return HTTPResponse(status=response.status, body=body)
        except urllib.error.HTTPError as error:
            body = error.read().decode("utf-8", errors="replace")
            return HTTPResponse(status=error.code, body=body)


@dataclass(frozen=True)
class ProviderMetadata:
    name: str
    display_name: str
    requires_api_key: bool
    supports_base_url: bool


@dataclass(frozen=True)
class ProviderCheckResult:
    name: str
    ok: bool
    model: str
    error: str = ""
    latency_ms: int | None = None

    def to_json_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "ok": self.ok,
            "model": self.model,
            "error": self.error,
            "latency_ms": self.latency_ms,
        }


class ProviderRegistry:
    def __init__(self, transport: HTTPTransport | None = None) -> None:
        self._transport = transport or UrllibTransport()
        self._providers = (
            ProviderMetadata(
                name="deterministic",
                display_name="Deterministic",
                requires_api_key=False,
                supports_base_url=False,
            ),
            ProviderMetadata(
                name="openai",
                display_name="OpenAI compatible",
                requires_api_key=True,
                supports_base_url=True,
            ),
            ProviderMetadata(
                name="gemini",
                display_name="Gemini",
                requires_api_key=True,
                supports_base_url=False,
            ),
            ProviderMetadata(
                name="anthropic",
                display_name="Anthropic",
                requires_api_key=True,
                supports_base_url=False,
            ),
        )

    def list(self) -> list[ProviderMetadata]:
        return list(self._providers)

    def metadata(self, name: str) -> ProviderMetadata:
        for provider in self._providers:
            if provider.name == name:
                return provider
        raise ValueError(f"unsupported dream provider: {name}")

    def status_payload(self, config: HieronymusConfig) -> list[dict[str, object]]:
        settings = load_settings(config)
        statuses = []
        for metadata in self._providers:
            provider = settings.providers.get(metadata.name, ProviderSettings())
            configured, error = _configured_status(metadata.name, provider)
            statuses.append(
                {
                    "name": metadata.name,
                    "display_name": metadata.display_name,
                    "enabled": provider.enabled,
                    "configured": configured,
                    "model": provider.model,
                    "api_key_env": provider.api_key_env,
                    "base_url": provider.base_url,
                    "error": error,
                }
            )
        return statuses

    def check(
        self,
        config: HieronymusConfig,
        name: str,
        temporary_api_key: str | None = None,
    ) -> ProviderCheckResult:
        self.metadata(name)
        if name == "deterministic":
            return ProviderCheckResult(name="deterministic", ok=True, model="")

        settings = load_settings(config)
        provider = settings.providers.get(name, ProviderSettings())
        key = temporary_api_key or os.environ.get(provider.api_key_env)
        if not key:
            return ProviderCheckResult(
                name=name,
                ok=False,
                model=provider.model,
                error=f"missing environment variable: {provider.api_key_env}",
            )

        started = time.perf_counter()
        try:
            response = self._check_remote(name, provider, key)
        except Exception:
            latency_ms = round((time.perf_counter() - started) * 1000)
            return ProviderCheckResult(
                name=name,
                ok=False,
                model=provider.model,
                error="provider check failed",
                latency_ms=latency_ms,
            )
        latency_ms = round((time.perf_counter() - started) * 1000)
        if 200 <= response.status < 300:
            return ProviderCheckResult(
                name=name,
                ok=True,
                model=provider.model,
                latency_ms=latency_ms,
            )
        return ProviderCheckResult(
            name=name,
            ok=False,
            model=provider.model,
            error=f"provider returned HTTP {response.status}",
            latency_ms=latency_ms,
        )

    def _check_remote(
        self,
        name: str,
        provider: ProviderSettings,
        api_key: str,
    ) -> HTTPResponse:
        if name == "openai":
            base_url = (provider.base_url or "https://api.openai.com/v1").rstrip("/")
            return self._transport.post_json(
                f"{base_url}/chat/completions",
                headers={"Authorization": f"Bearer {api_key}"},
                payload={
                    "model": provider.model,
                    "messages": [{"role": "user", "content": "Reply with ok."}],
                    "max_tokens": 1,
                },
                timeout=_DEFAULT_TIMEOUT_SECONDS,
            )
        if name == "gemini":
            return self._transport.post_json(
                (
                    "https://generativelanguage.googleapis.com/v1beta/models/"
                    f"{provider.model}:generateContent?key={api_key}"
                ),
                headers={},
                payload={
                    "contents": [
                        {
                            "role": "user",
                            "parts": [{"text": "Reply with ok."}],
                        }
                    ]
                },
                timeout=_DEFAULT_TIMEOUT_SECONDS,
            )
        if name == "anthropic":
            return self._transport.post_json(
                "https://api.anthropic.com/v1/messages",
                headers={
                    "x-api-key": api_key,
                    "anthropic-version": "2023-06-01",
                },
                payload={
                    "model": provider.model,
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "Reply with ok."}],
                },
                timeout=_DEFAULT_TIMEOUT_SECONDS,
            )
        raise ValueError(f"unsupported dream provider: {name}")


def resolve_provider(
    config: HieronymusConfig,
    name: str | None = None,
    *,
    transport: HTTPTransport | None = None,
) -> DreamProvider:
    settings = load_settings(config)
    provider_name = name or settings.dreaming.active_provider
    ProviderRegistry(transport=transport).metadata(provider_name)
    provider = settings.providers.get(provider_name, ProviderSettings())
    if not provider.enabled:
        raise ValueError(f"dream provider is disabled: {provider_name}")
    if provider_name == "deterministic":
        return DeterministicDreamProvider()
    raise ValueError(f"dream provider is not implemented: {provider_name}")


def _configured_status(name: str, provider: ProviderSettings) -> tuple[bool, str]:
    if name == "deterministic":
        return True, ""
    if not provider.model.strip():
        return False, "model is empty"
    if not provider.api_key_env.strip():
        return False, "api_key_env is empty"
    if provider.enabled and not os.environ.get(provider.api_key_env):
        return False, f"missing environment variable: {provider.api_key_env}"
    return True, ""

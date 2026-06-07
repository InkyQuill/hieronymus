from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.dreaming import (
    DeterministicDreamProvider,
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamProvider,
)
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
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


def _dream_prompt(
    context: TranslationContext,
    memories: list[ShortTermMemoryRecord],
) -> str:
    memory_payload = [
        {
            "id": memory.id,
            "source_role": memory.source_role,
            "kind": memory.kind,
            "text": memory.text,
            "source_ref": memory.source_ref,
        }
        for memory in memories
    ]
    return json.dumps(
        {
            "instruction": (
                "Return only JSON with keys crystals and concept_proposals. "
                "Use only provided source memory ids. Do not add markdown."
            ),
            "context": {
                "series_slug": context.series_slug,
                "source_language": context.source_language,
                "target_language": context.target_language,
                "task_type": context.task_type,
                "volume": context.volume,
                "chapter": context.chapter,
                "tags": list(context.tags),
            },
            "memories": memory_payload,
            "schema": {
                "crystals": [
                    {
                        "crystal_type": "lesson|concept|erudition",
                        "title": "string",
                        "text": "string",
                        "strength": 0.7,
                        "confidence": 0.8,
                        "source_memory_ids": [1],
                    }
                ],
                "concept_proposals": [
                    {
                        "series_slug": context.series_slug,
                        "source_language": context.source_language,
                        "target_language": context.target_language,
                        "concept_text": "string",
                        "source_form": "string",
                        "canonical_rendering": "string",
                        "approved_variants": ["string"],
                        "forbidden_variants": ["string"],
                        "rationale": "string",
                    }
                ],
            },
        },
        ensure_ascii=False,
    )


def _parse_dream_json(provider_name: str, raw_text: str) -> DreamOutput:
    try:
        payload = json.loads(raw_text)
    except json.JSONDecodeError as error:
        raise ValueError(f"{provider_name} response did not contain valid dream JSON") from error

    if type(payload) is not dict:
        raise ValueError(f"{provider_name} response did not match dream schema")
    crystals_payload = payload.get("crystals")
    proposals_payload = payload.get("concept_proposals")
    if type(crystals_payload) is not list or type(proposals_payload) is not list:
        raise ValueError(f"{provider_name} response did not match dream schema")

    try:
        crystals = [_dream_crystal_from_payload(item) for item in crystals_payload]
        proposals = [_dream_proposal_from_payload(item) for item in proposals_payload]
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"{provider_name} response did not match dream schema") from error
    return DreamOutput(crystals=crystals, concept_proposals=proposals)


def _dream_crystal_from_payload(payload: object) -> DreamCrystalCandidate:
    item = _require_dict(payload)
    return DreamCrystalCandidate(
        crystal_type=_require_str(item, "crystal_type"),
        title=_require_str(item, "title"),
        text=_require_str(item, "text"),
        strength=_require_float(item, "strength"),
        confidence=_require_float(item, "confidence"),
        source_memory_ids=_require_int_list(item, "source_memory_ids"),
    )


def _dream_proposal_from_payload(payload: object) -> DreamConceptProposal:
    item = _require_dict(payload)
    return DreamConceptProposal(
        series_slug=_require_str(item, "series_slug"),
        source_language=_require_str(item, "source_language"),
        target_language=_require_str(item, "target_language"),
        concept_text=_require_str(item, "concept_text"),
        source_form=_require_str(item, "source_form"),
        canonical_rendering=_require_str(item, "canonical_rendering"),
        approved_variants=_require_str_list(item, "approved_variants"),
        forbidden_variants=_require_str_list(item, "forbidden_variants"),
        rationale=_require_str(item, "rationale"),
    )


def _require_dict(payload: object) -> dict[str, Any]:
    if type(payload) is not dict:
        raise ValueError("schema item must be an object")
    return payload


def _require_field(payload: dict[str, Any], key: str) -> object:
    if key not in payload:
        raise ValueError("schema field is required")
    return payload[key]


def _require_str(payload: dict[str, Any], key: str) -> str:
    value = _require_field(payload, key)
    if type(value) is not str:
        raise ValueError("schema field must be a string")
    return value


def _require_float(payload: dict[str, Any], key: str) -> float:
    value = _require_field(payload, key)
    if type(value) not in (int, float):
        raise ValueError("schema field must be a number")
    return float(value)


def _require_int_list(payload: dict[str, Any], key: str) -> list[int]:
    value = _require_field(payload, key)
    if type(value) is not list:
        raise ValueError("schema field must be a list")
    if not all(type(item) is int for item in value):
        raise ValueError("schema field must be a list of integers")
    return value


def _require_str_list(payload: dict[str, Any], key: str) -> list[str]:
    value = _require_field(payload, key)
    if type(value) is not list:
        raise ValueError("schema field must be a list")
    if not all(type(item) is str for item in value):
        raise ValueError("schema field must be a list of strings")
    return value


class OpenAIDreamProvider:
    name = "openai"

    def __init__(
        self,
        settings: ProviderSettings,
        api_key: str,
        transport: HTTPTransport,
    ) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        base_url = (self.settings.base_url or "https://api.openai.com/v1").rstrip("/")
        response = self.transport.post_json(
            f"{base_url}/chat/completions",
            headers={"Authorization": f"Bearer {self.api_key}"},
            payload={
                "model": self.settings.model,
                "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
                "temperature": 0.1,
                "response_format": {"type": "json_object"},
            },
            timeout=_DEFAULT_TIMEOUT_SECONDS,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"openai returned HTTP {response.status}")
        payload = json.loads(response.body)
        text = payload["choices"][0]["message"]["content"]
        return _parse_dream_json("openai", str(text))


class GeminiDreamProvider:
    name = "gemini"

    def __init__(
        self,
        settings: ProviderSettings,
        api_key: str,
        transport: HTTPTransport,
    ) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        response = self.transport.post_json(
            (
                "https://generativelanguage.googleapis.com/v1beta/models/"
                f"{self.settings.model}:generateContent?key={self.api_key}"
            ),
            headers={},
            payload={
                "contents": [
                    {
                        "role": "user",
                        "parts": [{"text": _dream_prompt(context, memories)}],
                    }
                ],
                "generationConfig": {
                    "temperature": 0.1,
                    "responseMimeType": "application/json",
                },
            },
            timeout=_DEFAULT_TIMEOUT_SECONDS,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"gemini returned HTTP {response.status}")
        payload = json.loads(response.body)
        text = payload["candidates"][0]["content"]["parts"][0]["text"]
        return _parse_dream_json("gemini", str(text))


class AnthropicDreamProvider:
    name = "anthropic"

    def __init__(
        self,
        settings: ProviderSettings,
        api_key: str,
        transport: HTTPTransport,
    ) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        response = self.transport.post_json(
            "https://api.anthropic.com/v1/messages",
            headers={
                "x-api-key": self.api_key,
                "anthropic-version": "2023-06-01",
            },
            payload={
                "model": self.settings.model,
                "max_tokens": 2000,
                "temperature": 0.1,
                "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
            },
            timeout=_DEFAULT_TIMEOUT_SECONDS,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"anthropic returned HTTP {response.status}")
        payload = json.loads(response.body)
        text = payload["content"][0]["text"]
        return _parse_dream_json("anthropic", str(text))


def resolve_provider(
    config: HieronymusConfig,
    name: str | None = None,
    *,
    transport: HTTPTransport | None = None,
) -> DreamProvider:
    settings = load_settings(config)
    provider_name = name or settings.dreaming.active_provider
    ProviderRegistry(transport=transport).metadata(provider_name)
    provider_settings = settings.providers.get(provider_name, ProviderSettings())
    if not provider_settings.enabled:
        raise ValueError(f"dream provider is disabled: {provider_name}")
    if provider_name == "deterministic":
        return DeterministicDreamProvider()
    api_key = os.environ.get(provider_settings.api_key_env, "")
    if not api_key:
        raise ValueError(
            f"missing environment variable for {provider_name}: {provider_settings.api_key_env}"
        )
    active_transport = transport or UrllibTransport()
    if provider_name == "openai":
        return OpenAIDreamProvider(provider_settings, api_key, active_transport)
    if provider_name == "gemini":
        return GeminiDreamProvider(provider_settings, api_key, active_transport)
    if provider_name == "anthropic":
        return AnthropicDreamProvider(provider_settings, api_key, active_transport)
    raise ValueError(f"unsupported dream provider: {provider_name}")


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

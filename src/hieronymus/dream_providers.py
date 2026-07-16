from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any, Protocol

from anthropic import Anthropic
from google import genai
from google.genai import types as google_types
from ollama import Client as OllamaClient
from openai import OpenAI

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import DreamConfig, WorkflowProfile, load_dream_config
from hieronymus.dream_workflows import CRYSTALLIZATION_PHASE, resolve_effective_workflow
from hieronymus.dreaming import (
    DeterministicDreamProvider,
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamProvider,
)
from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    dream_profile_cache_identity,
    load_model_cache,
    save_model_cache,
)
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.provider_config import (
    ProviderProfile as CatalogProviderProfile,
)
from hieronymus.provider_config import (
    load_provider_catalog,
)

ANTHROPIC_API_VERSION = "2023-06-01"
ANTHROPIC_API_BASE_URL = "https://api.anthropic.com"
GEMINI_API_BASE_URL = "https://generativelanguage.googleapis.com"


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

    def get_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
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
        except urllib.error.URLError:
            return HTTPResponse(status=0, body="network error")

    def get_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        timeout: float,
    ) -> HTTPResponse:
        request = urllib.request.Request(
            url,
            headers={**headers, "Accept": "application/json"},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                body = response.read().decode("utf-8", errors="replace")
                return HTTPResponse(status=response.status, body=body)
        except urllib.error.HTTPError as error:
            body = error.read().decode("utf-8", errors="replace")
            return HTTPResponse(status=error.code, body=body)
        except urllib.error.URLError:
            return HTTPResponse(status=0, body="network error")


class ProviderClient(Protocol):
    def list_models(self) -> list[str]: ...

    def check(self, model: str) -> None: ...

    def generate_dream(self, model: str, prompt: str) -> str: ...


class ProviderClientFactory(Protocol):
    def create(self, profile: ProviderProfile) -> ProviderClient: ...


@dataclass(frozen=True)
class ProviderMetadata:
    name: str
    display_name: str
    requires_api_key: bool
    supports_base_url: bool


@dataclass(frozen=True)
class ProviderProfile:
    type: str
    endpoint: str = ""
    api_key: str = ""
    timeout_seconds: float = 30.0


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


@dataclass(frozen=True)
class ProviderRuntimeSettings:
    model: str
    base_url: str
    timeout_seconds: float


class OpenAIProviderClient:
    def __init__(self, profile: ProviderProfile) -> None:
        self._client = OpenAI(
            api_key=profile.api_key,
            base_url=profile.endpoint or None,
            timeout=profile.timeout_seconds,
        )

    def list_models(self) -> list[str]:
        return [model.id for model in self._client.models.list().data]

    def check(self, model: str) -> None:
        self._client.chat.completions.create(
            model=model,
            messages=[{"role": "user", "content": "Reply with ok."}],
            max_tokens=1,
        )

    def generate_dream(self, model: str, prompt: str) -> str:
        response = self._client.chat.completions.create(
            model=model,
            messages=[{"role": "user", "content": prompt}],
            temperature=0.1,
            response_format={"type": "json_object"},
        )
        text = response.choices[0].message.content
        if not isinstance(text, str):
            raise ValueError("openai response did not match provider envelope")
        return text


class GoogleProviderClient:
    def __init__(self, profile: ProviderProfile) -> None:
        http_options = (
            google_types.HttpOptions(base_url=profile.endpoint) if profile.endpoint else None
        )
        self._client = genai.Client(api_key=profile.api_key, http_options=http_options)

    def list_models(self) -> list[str]:
        return [
            model.name.removeprefix("models/")
            for model in self._client.models.list()
            if model.name and "generateContent" in (model.supported_actions or [])
        ]

    def check(self, model: str) -> None:
        self._client.models.generate_content(
            model=model,
            contents="Reply with ok.",
            config=google_types.GenerateContentConfig(max_output_tokens=1),
        )

    def generate_dream(self, model: str, prompt: str) -> str:
        response = self._client.models.generate_content(
            model=model,
            contents=prompt,
            config=google_types.GenerateContentConfig(
                temperature=0.1,
                response_mime_type="application/json",
            ),
        )
        if not isinstance(response.text, str):
            raise ValueError("gemini response did not match provider envelope")
        return response.text


class AnthropicProviderClient:
    def __init__(self, profile: ProviderProfile) -> None:
        self._client = Anthropic(
            api_key=profile.api_key,
            base_url=profile.endpoint or None,
            timeout=profile.timeout_seconds,
        )

    def list_models(self) -> list[str]:
        return [model.id for model in self._client.models.list()]

    def check(self, model: str) -> None:
        self._client.messages.create(
            model=model,
            max_tokens=1,
            messages=[{"role": "user", "content": "Reply with ok."}],
        )

    def generate_dream(self, model: str, prompt: str) -> str:
        response = self._client.messages.create(
            model=model,
            max_tokens=2000,
            temperature=0.1,
            messages=[{"role": "user", "content": prompt}],
        )
        for block in response.content:
            text = getattr(block, "text", None)
            if isinstance(text, str):
                return text
        raise ValueError("anthropic response did not match provider envelope")


class OllamaProviderClient:
    def __init__(self, profile: ProviderProfile) -> None:
        headers = {"Authorization": f"Bearer {profile.api_key}"} if profile.api_key else None
        self._client = OllamaClient(
            host=profile.endpoint or None,
            headers=headers,
            timeout=profile.timeout_seconds,
        )

    def list_models(self) -> list[str]:
        return [model.model for model in self._client.list().models]

    def check(self, model: str) -> None:
        self._client.chat(
            model=model,
            messages=[{"role": "user", "content": "Reply with ok."}],
            stream=False,
            options={"num_predict": 1},
        )

    def generate_dream(self, model: str, prompt: str) -> str:
        response = self._client.chat(
            model=model,
            messages=[{"role": "user", "content": prompt}],
            stream=False,
            format="json",
            options={"temperature": 0.1},
        )
        text = response.message.content
        if not isinstance(text, str):
            raise ValueError("ollama response did not match provider envelope")
        return text


class DefaultProviderClientFactory:
    def create(self, profile: ProviderProfile) -> ProviderClient:
        match profile.type:
            case "openai":
                return OpenAIProviderClient(profile)
            case "google" | "gemini":
                return GoogleProviderClient(profile)
            case "anthropic":
                return AnthropicProviderClient(profile)
            case "ollama":
                return OllamaProviderClient(profile)
            case _:
                raise ValueError(f"unsupported provider type: {profile.type}")


class SdkDreamProvider:
    def __init__(
        self,
        name: str,
        settings: ProviderRuntimeSettings,
        client: ProviderClient,
    ) -> None:
        self.name = name
        self.settings = settings
        self._client = client

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        raw_text = self._client.generate_dream(
            self.settings.model,
            _dream_prompt(context, memories),
        )
        return _parse_dream_json(self.name, raw_text)


@dataclass(frozen=True)
class ModelSuggestionResult:
    provider: str
    models: list[str]
    source: str
    error: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": self.models,
            "source": self.source,
            "error": self.error,
        }


class ProviderRegistry:
    def __init__(
        self,
        transport: HTTPTransport | None = None,
        *,
        client_factory: ProviderClientFactory | None = None,
    ) -> None:
        self._transport = transport or UrllibTransport()
        self._client_factory = client_factory or (
            DefaultProviderClientFactory() if transport is None else None
        )
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
                supports_base_url=True,
            ),
            ProviderMetadata(
                name="anthropic",
                display_name="Anthropic",
                requires_api_key=True,
                supports_base_url=True,
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
        dream_config = load_dream_config(config)
        provider_catalog = load_provider_catalog(config)
        statuses = []
        profile_names = [metadata.name for metadata in self._providers]
        profile_names.extend(
            name for name in provider_catalog.providers if name not in set(profile_names)
        )
        for name in profile_names:
            metadata = self._metadata_or_profile_default(name)
            if name == "deterministic":
                statuses.append(
                    {
                        "name": name,
                        "display_name": metadata.display_name,
                        "configured": True,
                        "model": "",
                        "api_key_present": False,
                        "base_url": "",
                        "timeout_seconds": 0.0,
                        "error": "",
                    }
                )
                continue
            catalog_profile = provider_catalog.providers.get(name)
            profile = (
                _catalog_profile_to_runtime(catalog_profile)
                if catalog_profile is not None
                else None
            )
            if profile is None:
                configured = False
                error = "provider profile missing"
                model = ""
                api_key_present = False
                base_url = ""
                timeout_seconds = 0.0
            else:
                model = _model_for_profile(dream_config, name)
                if not model and provider_catalog.defaults.provider == name:
                    model = provider_catalog.defaults.model
                api_key_present = bool(profile.api_key.strip())
                configured, error = _configured_profile_status(profile, model)
                base_url = profile.endpoint
                timeout_seconds = profile.timeout_seconds
            statuses.append(
                {
                    "name": name,
                    "display_name": metadata.display_name,
                    "configured": configured,
                    "model": model,
                    "api_key_present": api_key_present,
                    "base_url": base_url,
                    "timeout_seconds": timeout_seconds,
                    "error": error,
                }
            )
        return statuses

    def list_model_suggestions(
        self,
        config: HieronymusConfig,
        name: str,
    ) -> ModelSuggestionResult:
        if name == "deterministic":
            return ModelSuggestionResult(
                provider=name,
                models=_default_model_suggestions(name),
                source="defaults",
            )

        provider_catalog = load_provider_catalog(config)
        catalog_profile = provider_catalog.providers.get(name)
        if catalog_profile is None:
            return ModelSuggestionResult(
                provider=name,
                models=_default_model_suggestions(name),
                source="defaults",
                error=f"provider profile missing: {name}",
            )
        profile = _catalog_profile_to_runtime(catalog_profile)
        return self.list_profile_model_suggestions(config, name, profile)

    def list_profile_model_suggestions(
        self,
        config: HieronymusConfig,
        profile_name: str,
        profile: ProviderProfile,
    ) -> ModelSuggestionResult:
        identity = dream_profile_cache_identity(profile_name, profile)
        cache = load_model_cache(config)
        entry = cache.providers.get(profile_name)
        if (
            entry is not None
            and not entry.error
            and entry.identity == identity
            and not entry.is_stale()
        ):
            return ModelSuggestionResult(
                provider=profile_name,
                models=list(entry.models),
                source=config.llm_cache_path.name,
                error=entry.error,
            )

        result = self._list_uncached_profile_model_suggestions(profile_name, profile)
        _save_model_cache_best_effort(
            config,
            cache.with_entry(
                ModelCacheEntry(
                    provider=profile_name,
                    models=tuple(result.models),
                    fetched_at=datetime.now(UTC).isoformat(),
                    error=result.error,
                    identity=identity,
                )
            ),
        )
        return result

    def check_profile_connection(
        self,
        config: HieronymusConfig,
        profile_name: str,
        profile: ProviderProfile,
    ) -> ModelSuggestionResult:
        """Probe a live endpoint and refresh its cached model metadata."""
        result = self._list_uncached_profile_model_suggestions(profile_name, profile)
        cache = load_model_cache(config)
        _save_model_cache_best_effort(
            config,
            cache.with_entry(
                ModelCacheEntry(
                    provider=profile_name,
                    models=tuple(result.models),
                    fetched_at=datetime.now(UTC).isoformat(),
                    error=result.error,
                    identity=dream_profile_cache_identity(profile_name, profile),
                )
            ),
        )
        return result

    def _list_uncached_profile_model_suggestions(
        self,
        profile_name: str,
        profile: ProviderProfile,
    ) -> ModelSuggestionResult:
        defaults = _default_model_suggestions(profile.type)
        api_key = profile.api_key.strip()
        if profile.type != "ollama" and not api_key:
            return ModelSuggestionResult(
                provider=profile_name,
                models=defaults,
                source="defaults",
                error="API key missing for provider profile",
            )

        settings = _profile_provider_settings(profile, model=defaults[0])
        try:
            if self._client_factory is not None:
                models = sorted(set(self._client_factory.create(profile).list_models()))
            else:
                response = self._list_remote_models(profile.type, settings, api_key)
                if not 200 <= response.status < 300:
                    raise ValueError("model suggestions request failed")
                models = _parse_model_suggestions(profile.type, response.body)
            if not models:
                raise ValueError("empty model suggestions")
        except Exception:
            return ModelSuggestionResult(
                provider=profile_name,
                models=defaults,
                source="defaults",
                error="model suggestions unavailable",
            )
        return ModelSuggestionResult(provider=profile_name, models=models, source="api")

    def check(
        self,
        config: HieronymusConfig,
        name: str,
        *,
        model: str | None = None,
    ) -> ProviderCheckResult:
        if name == "deterministic":
            return ProviderCheckResult(name="deterministic", ok=True, model="")

        dream_config = load_dream_config(config)
        provider_catalog = load_provider_catalog(config)
        catalog_profile = provider_catalog.providers.get(name)
        resolved_model = model if model is not None else _model_for_profile(dream_config, name)
        if not resolved_model and provider_catalog.defaults.provider == name:
            resolved_model = provider_catalog.defaults.model
        if catalog_profile is None:
            return ProviderCheckResult(
                name=name,
                ok=False,
                model=resolved_model,
                error=f"provider profile missing: {name}",
            )
        profile = _catalog_profile_to_runtime(catalog_profile)
        return self.check_profile(config, name, profile, model=resolved_model)

    def _metadata_or_profile_default(self, name: str) -> ProviderMetadata:
        for provider in self._providers:
            if provider.name == name:
                return provider
        return ProviderMetadata(
            name=name,
            display_name=name.replace("_", " ").title(),
            requires_api_key=True,
            supports_base_url=True,
        )

    def check_profile(
        self,
        config: HieronymusConfig,
        profile_name: str,
        profile: ProviderProfile,
        *,
        model: str,
    ) -> ProviderCheckResult:
        resolved_model = model.strip()
        if not resolved_model:
            result = ProviderCheckResult(
                name=profile_name,
                ok=False,
                model="",
                error=f"model must not be empty for provider profile: {profile_name}",
            )
            self._cache_profile_check_failure(config, profile_name, profile, result)
            return result
        api_key = profile.api_key.strip()
        if profile.type != "ollama" and not api_key:
            result = ProviderCheckResult(
                name=profile_name,
                ok=False,
                model=resolved_model,
                error=f"API key missing for provider profile: {profile_name}",
            )
            self._cache_profile_check_failure(config, profile_name, profile, result)
            return result

        settings = _profile_provider_settings(profile, resolved_model)
        started = time.perf_counter()
        try:
            if self._client_factory is not None:
                self._client_factory.create(profile).check(resolved_model)
                response = None
            else:
                response = self._check_profile_remote(profile, settings, api_key=api_key)
        except Exception:
            latency_ms = round((time.perf_counter() - started) * 1000)
            result = ProviderCheckResult(
                name=profile_name,
                ok=False,
                model=resolved_model,
                error="provider check failed",
                latency_ms=latency_ms,
            )
            self._cache_profile_check_failure(config, profile_name, profile, result)
            return result

        latency_ms = round((time.perf_counter() - started) * 1000)
        if response is None or 200 <= response.status < 300:
            return ProviderCheckResult(
                name=profile_name,
                ok=True,
                model=resolved_model,
                latency_ms=latency_ms,
            )

        result = ProviderCheckResult(
            name=profile_name,
            ok=False,
            model=resolved_model,
            error=f"provider returned HTTP {response.status}",
            latency_ms=latency_ms,
        )
        self._cache_profile_check_failure(config, profile_name, profile, result)
        return result

    def _cache_profile_check_failure(
        self,
        config: HieronymusConfig,
        profile_name: str,
        profile: ProviderProfile,
        result: ProviderCheckResult,
    ) -> None:
        if result.ok:
            return
        cache = load_model_cache(config)
        _save_model_cache_best_effort(
            config,
            cache.with_entry(
                ModelCacheEntry(
                    provider=profile_name,
                    models=(),
                    fetched_at=datetime.now(UTC).isoformat(),
                    error=result.error,
                    identity=dream_profile_cache_identity(profile_name, profile),
                )
            ),
        )

    def _check_profile_remote(
        self,
        profile: ProviderProfile,
        provider: ProviderRuntimeSettings,
        *,
        api_key: str,
    ) -> HTTPResponse:
        if profile.type in {"openai", "google", "gemini", "anthropic"}:
            name = "gemini" if profile.type == "google" else profile.type
            return self._check_remote(name, provider, api_key)
        if profile.type == "ollama":
            if _is_openai_compatible_ollama_endpoint(profile.endpoint):
                return self._check_remote("openai", provider, api_key or "ollama")
            base_url = (provider.base_url or "http://localhost:11434").rstrip("/")
            return self._transport.post_json(
                f"{base_url}/api/chat",
                headers={},
                payload={
                    "model": provider.model,
                    "messages": [{"role": "user", "content": "Reply with ok."}],
                    "stream": False,
                },
                timeout=provider.timeout_seconds,
            )
        raise ValueError(f"unsupported provider type: {profile.type}")

    def _check_remote(
        self,
        name: str,
        provider: ProviderRuntimeSettings,
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
                timeout=provider.timeout_seconds,
            )
        if name in {"google", "gemini"}:
            base_url = (provider.base_url or "https://generativelanguage.googleapis.com").rstrip(
                "/"
            )
            return self._transport.post_json(
                f"{base_url}/v1beta/models/{provider.model}:generateContent",
                headers={"x-goog-api-key": api_key},
                payload={
                    "contents": [
                        {
                            "role": "user",
                            "parts": [{"text": "Reply with ok."}],
                        }
                    ]
                },
                timeout=provider.timeout_seconds,
            )
        if name == "anthropic":
            base_url = (provider.base_url or "https://api.anthropic.com").rstrip("/")
            return self._transport.post_json(
                f"{base_url}/v1/messages",
                headers={
                    "x-api-key": api_key,
                    "anthropic-version": ANTHROPIC_API_VERSION,
                },
                payload={
                    "model": provider.model,
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "Reply with ok."}],
                },
                timeout=provider.timeout_seconds,
            )
        raise ValueError(f"unsupported dream provider: {name}")

    def _list_remote_models(
        self,
        name: str,
        provider: ProviderRuntimeSettings,
        api_key: str,
    ) -> HTTPResponse:
        if name == "openai":
            base_url = (provider.base_url or "https://api.openai.com/v1").rstrip("/")
            return self._transport.get_json(
                f"{base_url}/models",
                headers={"Authorization": f"Bearer {api_key}"},
                timeout=provider.timeout_seconds,
            )
        if name in {"google", "gemini"}:
            base_url = (provider.base_url or "https://generativelanguage.googleapis.com").rstrip(
                "/"
            )
            return self._transport.get_json(
                f"{base_url}/v1beta/models",
                headers={"x-goog-api-key": api_key},
                timeout=provider.timeout_seconds,
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
            "language_tags": list(memory.language_tags),
            "story_scopes": list(memory.story_scopes),
            "semantic_tags": list(memory.semantic_tags),
            "source_credibility": memory.source_credibility,
            "rule_intent": memory.rule_intent,
            "soft_origin": memory.soft_origin,
        }
        for memory in memories
    ]
    return json.dumps(
        {
            "instruction": (
                "Return only JSON with keys crystals and concept_proposals. "
                "Use English memory prose by default; Japanese, Russian, or other "
                "languages may appear only as terms, names, renderings, quotes, or "
                "metadata. Long-term crystals must be 1-2 sentences. Short-term "
                "memories must be 1-6 sentences. Use only provided source memory ids. "
                "Do not add markdown."
            ),
            "context": {
                "series_slug": context.series_slug,
                "source_language": context.source_language,
                "target_language": context.target_language,
                "task_type": context.task_type,
                "volume": context.volume,
                "chapter": context.chapter,
                "tags": list(context.tags),
                "language_tags": list(context.language_tags),
                "story_scopes": list(context.story_scopes),
                "semantic_tags": list(context.semantic_tags),
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


def _default_model_suggestions(name: str) -> list[str]:
    defaults = {
        "openai": ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
        "google": ["gemini-2.5-flash", "gemini-2.5-pro"],
        "gemini": ["gemini-2.5-flash", "gemini-2.5-pro"],
        "anthropic": ["claude-3-5-haiku-latest", "claude-3-7-sonnet-latest"],
        "deterministic": [""],
        "ollama": ["gemma4-e3b"],
    }
    return list(defaults.get(name, []))


def _parse_model_suggestions(name: str, body: str) -> list[str]:
    payload = json.loads(body)
    if type(payload) is not dict:
        return []
    if name == "openai":
        data = payload.get("data")
        if type(data) is not list:
            return []
        return sorted(
            item["id"] for item in data if type(item) is dict and type(item.get("id")) is str
        )
    if name in {"google", "gemini"}:
        data = payload.get("models")
        if type(data) is not list:
            return []
        return sorted(
            item["name"].removeprefix("models/")
            for item in data
            if type(item) is dict and type(item.get("name")) is str
        )
    return []


def _save_model_cache_best_effort(config: HieronymusConfig, cache: CachedModels) -> None:
    try:
        save_model_cache(config, cache)
    except OSError:
        return


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


def _provider_envelope_error(provider_name: str) -> ValueError:
    return ValueError(f"{provider_name} response did not match provider envelope")


def _provider_envelope_payload(provider_name: str, body: str) -> dict[str, Any]:
    try:
        payload = json.loads(body)
    except json.JSONDecodeError as error:
        raise _provider_envelope_error(provider_name) from error
    if type(payload) is not dict:
        raise _provider_envelope_error(provider_name)
    return payload


def _openai_envelope_text(body: str) -> str:
    try:
        text = _provider_envelope_payload("openai", body)["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as error:
        raise _provider_envelope_error("openai") from error
    if type(text) is not str:
        raise _provider_envelope_error("openai")
    return text


def _gemini_envelope_text(body: str) -> str:
    try:
        text = _provider_envelope_payload("gemini", body)["candidates"][0]["content"]["parts"][0][
            "text"
        ]
    except (KeyError, IndexError, TypeError) as error:
        raise _provider_envelope_error("gemini") from error
    if type(text) is not str:
        raise _provider_envelope_error("gemini")
    return text


def _anthropic_envelope_text(body: str) -> str:
    try:
        text = _provider_envelope_payload("anthropic", body)["content"][0]["text"]
    except (KeyError, IndexError, TypeError) as error:
        raise _provider_envelope_error("anthropic") from error
    if type(text) is not str:
        raise _provider_envelope_error("anthropic")
    return text


def _ollama_envelope_text(body: str) -> str:
    try:
        text = _provider_envelope_payload("ollama", body)["message"]["content"]
    except (KeyError, TypeError) as error:
        raise _provider_envelope_error("ollama") from error
    if type(text) is not str:
        raise _provider_envelope_error("ollama")
    return text


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
        settings: ProviderRuntimeSettings,
        api_key: str,
        transport: HTTPTransport,
        *,
        name: str = "openai",
    ) -> None:
        self.name = name
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
            timeout=self.settings.timeout_seconds,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"openai returned HTTP {response.status}")
        return _parse_dream_json("openai", _openai_envelope_text(response.body))


class GeminiDreamProvider:
    name = "gemini"

    def __init__(
        self,
        settings: ProviderRuntimeSettings,
        api_key: str,
        transport: HTTPTransport,
        *,
        base_url: str | None = None,
    ) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport
        self.base_url = (base_url or GEMINI_API_BASE_URL).rstrip("/")

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        response = self.transport.post_json(
            f"{self.base_url}/v1beta/models/{self.settings.model}:generateContent",
            headers={"x-goog-api-key": self.api_key},
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
            timeout=self.settings.timeout_seconds,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"gemini returned HTTP {response.status}")
        return _parse_dream_json("gemini", _gemini_envelope_text(response.body))


class AnthropicDreamProvider:
    name = "anthropic"

    def __init__(
        self,
        settings: ProviderRuntimeSettings,
        api_key: str,
        transport: HTTPTransport,
        *,
        base_url: str | None = None,
    ) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport
        self.base_url = (base_url or ANTHROPIC_API_BASE_URL).rstrip("/")

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        response = self.transport.post_json(
            f"{self.base_url}/v1/messages",
            headers={
                "x-api-key": self.api_key,
                "anthropic-version": ANTHROPIC_API_VERSION,
            },
            payload={
                "model": self.settings.model,
                "max_tokens": 2000,
                "temperature": 0.1,
                "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
            },
            timeout=self.settings.timeout_seconds,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"anthropic returned HTTP {response.status}")
        return _parse_dream_json("anthropic", _anthropic_envelope_text(response.body))


class OllamaDreamProvider:
    name = "ollama"

    def __init__(
        self,
        settings: ProviderRuntimeSettings,
        transport: HTTPTransport,
    ) -> None:
        self.settings = settings
        self.transport = transport

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        base_url = (self.settings.base_url or "http://localhost:11434").rstrip("/")
        response = self.transport.post_json(
            f"{base_url}/api/chat",
            headers={},
            payload={
                "model": self.settings.model,
                "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
                "stream": False,
                "format": "json",
                "options": {"temperature": 0.1},
            },
            timeout=self.settings.timeout_seconds,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"ollama returned HTTP {response.status}")
        return _parse_dream_json("ollama", _ollama_envelope_text(response.body))


def resolve_profile_provider(
    config: HieronymusConfig,
    profile_name: str,
    *,
    model: str,
    transport: HTTPTransport | None = None,
    client_factory: ProviderClientFactory | None = None,
) -> DreamProvider:
    provider_catalog = load_provider_catalog(config)
    catalog_profile = provider_catalog.providers.get(profile_name)
    if catalog_profile is None:
        raise ValueError(f"referenced provider profile is missing: {profile_name}")
    profile = _catalog_profile_to_runtime(catalog_profile)
    return _provider_from_profile(
        profile_name,
        profile,
        model=model,
        transport=transport,
        client_factory=client_factory,
    )


def _provider_from_profile(
    profile_name: str,
    profile: ProviderProfile,
    *,
    model: str,
    transport: HTTPTransport | None = None,
    client_factory: ProviderClientFactory | None = None,
) -> DreamProvider:
    resolved_model = model.strip()
    api_key = profile.api_key.strip()
    if not resolved_model:
        raise ValueError(f"model must not be empty for provider profile: {profile_name}")
    if profile.type != "ollama" and not api_key:
        raise ValueError(f"API key missing for provider profile: {profile_name}")

    if client_factory is None and transport is None:
        client_factory = DefaultProviderClientFactory()
    if client_factory is not None:
        return SdkDreamProvider(
            "gemini" if profile.type == "gemini" else profile.type,
            _profile_provider_settings(profile, resolved_model),
            client_factory.create(profile),
        )

    active_transport = transport or UrllibTransport()
    settings = _profile_provider_settings(profile, resolved_model)
    if profile.type == "openai":
        return OpenAIDreamProvider(settings, api_key, active_transport)
    if profile.type in {"google", "gemini"}:
        return GeminiDreamProvider(
            settings,
            api_key,
            active_transport,
            base_url=settings.base_url,
        )
    if profile.type == "anthropic":
        return AnthropicDreamProvider(
            settings,
            api_key,
            active_transport,
            base_url=settings.base_url,
        )
    if profile.type == "ollama":
        if _is_openai_compatible_ollama_endpoint(profile.endpoint):
            return OpenAIDreamProvider(
                settings, api_key or "ollama", active_transport, name="ollama"
            )
        return OllamaDreamProvider(settings, active_transport)
    raise ValueError(f"unsupported provider type for {profile_name}: {profile.type}")


def _profile_provider_settings(profile: ProviderProfile, model: str) -> ProviderRuntimeSettings:
    endpoint = profile.endpoint.strip() or _default_profile_endpoint(profile.type)
    return ProviderRuntimeSettings(
        model=model,
        base_url=endpoint,
        timeout_seconds=profile.timeout_seconds,
    )


def _default_profile_endpoint(provider_type: str) -> str:
    if provider_type == "openai":
        return "https://api.openai.com/v1"
    if provider_type in {"google", "gemini"}:
        return "https://generativelanguage.googleapis.com"
    if provider_type == "anthropic":
        return "https://api.anthropic.com"
    if provider_type == "ollama":
        return "http://localhost:11434"
    return ""


def _is_openai_compatible_ollama_endpoint(endpoint: str) -> bool:
    return endpoint.strip().rstrip("/").endswith("/v1")


def resolve_provider(
    config: HieronymusConfig,
    name: str | None = None,
    *,
    transport: HTTPTransport | None = None,
    client_factory: ProviderClientFactory | None = None,
) -> DreamProvider:
    dream_config = load_dream_config(config)
    if name == "deterministic":
        return DeterministicDreamProvider()
    provider_catalog = load_provider_catalog(config)
    if name is not None:
        if name not in provider_catalog.providers:
            ProviderRegistry(transport=transport, client_factory=client_factory).metadata(name)
        workflow = _workflow_for_profile(dream_config, name)
        model = workflow.model if workflow is not None else _model_for_profile(dream_config, name)
        if not model and provider_catalog.defaults.provider == name:
            model = provider_catalog.defaults.model
        return resolve_profile_provider(
            config,
            name,
            model=model,
            transport=transport,
            client_factory=client_factory,
        )

    if not dream_config.enabled:
        return DeterministicDreamProvider()

    workflow_name = _runtime_workflow_name(dream_config)
    if workflow_name is None:
        return DeterministicDreamProvider()
    workflow = resolve_effective_workflow(dream_config, provider_catalog, workflow_name)
    if workflow.provider == "deterministic":
        return DeterministicDreamProvider()
    catalog_profile = provider_catalog.providers.get(workflow.provider)
    if catalog_profile is None:
        raise ValueError(f"referenced provider profile is missing: {workflow.provider}")
    profile = _catalog_profile_to_runtime(catalog_profile)
    if not _profile_runtime_configured(profile, workflow.model):
        return DeterministicDreamProvider()
    return _provider_from_profile(
        workflow.provider,
        profile,
        model=workflow.model,
        transport=transport,
        client_factory=client_factory,
    )


def _configured_profile_status(profile: ProviderProfile, model: str) -> tuple[bool, str]:
    if profile.type != "ollama" and not profile.api_key.strip():
        return False, "API key missing for provider profile"
    if not model.strip():
        return False, "model is empty for provider profile"
    return True, ""


def _model_for_profile(dream_config: DreamConfig, profile_name: str) -> str:
    for workflow in dream_config.workflows.values():
        if workflow.enabled and workflow.provider == profile_name and workflow.model.strip():
            return workflow.model
    for workflow in dream_config.workflows.values():
        if workflow.provider == profile_name and workflow.model.strip():
            return workflow.model
    return ""


def _runtime_workflow(dream_config: DreamConfig) -> WorkflowProfile | None:
    workflow = dream_config.workflows.get(CRYSTALLIZATION_PHASE)
    if workflow is not None and workflow.enabled:
        return workflow
    for workflow in dream_config.workflows.values():
        if workflow.enabled:
            return workflow
    return None


def _runtime_workflow_name(dream_config: DreamConfig) -> str | None:
    workflow = dream_config.workflows.get(CRYSTALLIZATION_PHASE)
    if workflow is not None and workflow.enabled:
        return CRYSTALLIZATION_PHASE
    for name, workflow in dream_config.workflows.items():
        if workflow.enabled:
            return name
    return None


def _workflow_for_profile(
    dream_config: DreamConfig,
    profile_name: str,
) -> WorkflowProfile | None:
    for workflow in dream_config.workflows.values():
        if workflow.enabled and workflow.provider == profile_name and workflow.model.strip():
            return workflow
    for workflow in dream_config.workflows.values():
        if workflow.provider == profile_name and workflow.model.strip():
            return workflow
    return None


def _profile_runtime_configured(profile: ProviderProfile, model: str) -> bool:
    if not model.strip():
        return False
    return profile.type == "ollama" or bool(profile.api_key.strip())


def _catalog_profile_to_runtime(profile: CatalogProviderProfile) -> ProviderProfile:
    return ProviderProfile(
        type=profile.type,
        endpoint=profile.url,
        api_key=profile.key,
        timeout_seconds=profile.timeout_seconds,
    )

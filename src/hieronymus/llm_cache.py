from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from typing import Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import ProviderProfile

CACHE_TTL = timedelta(hours=24)


@dataclass(frozen=True)
class ModelCacheEntry:
    provider: str
    models: tuple[str, ...]
    fetched_at: str
    error: str = ""
    identity: str = ""

    def is_stale(self, now: datetime | None = None) -> bool:
        try:
            fetched_at = _parse_datetime(self.fetched_at)
        except ValueError:
            return True
        current = now or datetime.now(UTC)
        if current.tzinfo is None:
            current = current.replace(tzinfo=UTC)
        if fetched_at > current:
            return True
        return current - fetched_at >= CACHE_TTL

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": list(self.models),
            "fetched_at": self.fetched_at,
            "error": self.error,
            "identity": self.identity,
        }


@dataclass(frozen=True)
class CachedModels:
    providers: dict[str, ModelCacheEntry] = field(default_factory=dict)

    def with_entry(self, entry: ModelCacheEntry) -> CachedModels:
        return replace(self, providers={**self.providers, entry.provider: entry})

    def to_payload(self) -> dict[str, object]:
        return {
            "providers": {
                provider: entry.to_payload() for provider, entry in self.providers.items()
            }
        }


def load_model_cache(config: HieronymusConfig) -> CachedModels:
    if not config.llm_cache_path.exists():
        return CachedModels()
    try:
        payload = json.loads(config.llm_cache_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return CachedModels()
    return _cache_from_payload(payload)


def save_model_cache(config: HieronymusConfig, cache: CachedModels) -> None:
    atomic_write_text(
        config.llm_cache_path,
        json.dumps(
            cache.to_payload(),
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )


def model_cache_identity(name: str) -> str:
    payload = {"provider": name}
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def dream_profile_cache_identity(name: str, provider: ProviderProfile) -> str:
    payload = {
        "provider": name,
        "type": provider.type,
        "endpoint": provider.endpoint.rstrip("/"),
        "api_key_sha256": _secret_fingerprint(provider.api_key),
    }
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def _secret_fingerprint(value: str) -> str:
    if not value:
        return ""
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _cache_from_payload(payload: Any) -> CachedModels:
    if type(payload) is not dict:
        return CachedModels()
    providers = payload.get("providers")
    if type(providers) is not dict:
        return CachedModels()
    entries = {}
    for provider, raw_entry in providers.items():
        if type(provider) is not str or type(raw_entry) is not dict:
            continue
        entry = _entry_from_payload(provider, raw_entry)
        if entry is not None:
            entries[provider] = entry
    return CachedModels(providers=entries)


def _entry_from_payload(provider: str, payload: dict[str, Any]) -> ModelCacheEntry | None:
    raw_models = payload.get("models")
    raw_fetched_at = payload.get("fetched_at")
    raw_error = payload.get("error", "")
    raw_identity = payload.get("identity", "")
    if (
        type(raw_models) is not list
        or type(raw_fetched_at) is not str
        or type(raw_error) is not str
        or type(raw_identity) is not str
    ):
        return None
    try:
        _parse_datetime(raw_fetched_at)
    except ValueError:
        return None
    models = tuple(model for model in raw_models if type(model) is str)
    if not models and not raw_error:
        return None
    return ModelCacheEntry(
        provider=provider,
        models=models,
        fetched_at=raw_fetched_at,
        error=raw_error,
        identity=raw_identity,
    )


def _parse_datetime(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed

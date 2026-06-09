from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from typing import Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

CACHE_TTL = timedelta(hours=24)


@dataclass(frozen=True)
class ModelCacheEntry:
    provider: str
    models: tuple[str, ...]
    fetched_at: str
    error: str = ""

    def is_stale(self, now: datetime | None = None) -> bool:
        try:
            fetched_at = _parse_datetime(self.fetched_at)
        except ValueError:
            return True
        current = now or datetime.now(UTC)
        if current.tzinfo is None:
            current = current.replace(tzinfo=UTC)
        return current - fetched_at >= CACHE_TTL

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": list(self.models),
            "fetched_at": self.fetched_at,
            "error": self.error,
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
    payload = json.loads(config.llm_cache_path.read_text(encoding="utf-8"))
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
    raw_provider = payload.get("provider", provider)
    raw_models = payload.get("models")
    raw_fetched_at = payload.get("fetched_at")
    raw_error = payload.get("error", "")
    if (
        type(raw_provider) is not str
        or type(raw_models) is not list
        or type(raw_fetched_at) is not str
        or type(raw_error) is not str
    ):
        return None
    models = tuple(model for model in raw_models if type(model) is str)
    return ModelCacheEntry(
        provider=raw_provider,
        models=models,
        fetched_at=raw_fetched_at,
        error=raw_error,
    )


def _parse_datetime(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed

from __future__ import annotations

from datetime import UTC, datetime, timedelta

from hieronymus.config import HieronymusConfig
from hieronymus.llm_cache import CachedModels, ModelCacheEntry, load_model_cache, save_model_cache


def test_model_cache_round_trips_provider_models_through_llmcache_tmp(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    cache = CachedModels().with_entry(
        ModelCacheEntry(
            provider="openai",
            models=("gpt-4.1", "gpt-4.1-mini"),
            fetched_at="2026-06-09T12:00:00+00:00",
            error="",
        )
    )

    save_model_cache(config, cache)
    loaded = load_model_cache(config)

    assert config.llm_cache_path == config.config_root / "llmcache.tmp"
    assert loaded == cache


def test_model_cache_entry_is_stale_after_24_hours_exactly() -> None:
    fetched_at = datetime(2026, 6, 8, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="anthropic",
        models=("claude-3-5-haiku-latest",),
        fetched_at=fetched_at.isoformat(),
    )

    assert entry.is_stale(fetched_at + timedelta(hours=24)) is True


def test_model_cache_entry_is_not_stale_before_24_hours() -> None:
    fetched_at = datetime(2026, 6, 8, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="gemini",
        models=("gemini-2.5-flash",),
        fetched_at=fetched_at.isoformat(),
    )

    assert entry.is_stale(fetched_at + timedelta(hours=24) - timedelta(microseconds=1)) is False

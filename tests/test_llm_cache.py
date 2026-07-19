from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import patch

from hieronymus.config import HieronymusConfig
from hieronymus.llm_cache import CachedModels, ModelCacheEntry, load_model_cache, save_model_cache


def _cache() -> CachedModels:
    return CachedModels().with_entry(
        ModelCacheEntry(
            provider="openai",
            models=("gpt-4.1", "gpt-4.1-mini"),
            fetched_at="2026-06-09T12:00:00+00:00",
            error="",
        )
    )


def _legacy_cache_path(config: HieronymusConfig) -> Path:
    return config.data_root / ("llmcache" + ".tmp")


def test_model_cache_round_trips_provider_models_through_json_cache(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    cache = _cache()

    save_model_cache(config, cache)
    loaded = load_model_cache(config)

    assert config.llm_cache_path == config.data_root / "llm-cache.json"
    assert loaded == cache


def test_load_model_cache_atomically_adopts_legacy_cache(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")

    loaded = load_model_cache(config)

    assert loaded == _cache()
    assert config.llm_cache_path.exists()
    assert not legacy_path.exists()


def test_load_model_cache_leaves_new_cache_unchanged(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(config, _cache())
    original = config.llm_cache_path.read_bytes()

    assert load_model_cache(config) == _cache()
    assert config.llm_cache_path.read_bytes() == original


def test_load_model_cache_prefers_new_cache_when_both_exist(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    old_cache = _cache()
    new_cache = CachedModels().with_entry(
        ModelCacheEntry(
            provider="gemini",
            models=("gemini-2.5-flash",),
            fetched_at="2026-06-09T12:00:00+00:00",
        )
    )
    save_model_cache(config, new_cache)
    legacy_path = _legacy_cache_path(config)
    legacy_path.write_text(json.dumps(old_cache.to_payload()), encoding="utf-8")

    assert load_model_cache(config) == new_cache
    assert legacy_path.exists()


def test_load_model_cache_does_not_adopt_malformed_legacy_cache(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text("{not json", encoding="utf-8")

    assert load_model_cache(config) == CachedModels()
    assert legacy_path.exists()
    assert not config.llm_cache_path.exists()


def test_load_model_cache_preserves_data_when_atomic_adoption_is_interrupted(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")

    with patch.object(Path, "replace", side_effect=OSError("interrupted")):
        loaded = load_model_cache(config)

    assert loaded == _cache()
    assert legacy_path.exists()
    assert not config.llm_cache_path.exists()


def test_load_model_cache_uses_cache_adopted_by_a_concurrent_process(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")

    def concurrent_adoption(path: Path, target: Path) -> None:
        target.write_bytes(path.read_bytes())
        path.unlink()
        raise OSError("source was adopted concurrently")

    monkeypatch.setattr(Path, "replace", concurrent_adoption)

    assert load_model_cache(config) == _cache()


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


def test_model_cache_entry_with_future_fetched_at_is_stale() -> None:
    now = datetime(2026, 6, 9, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="openai",
        models=("gpt-4.1-mini",),
        fetched_at=(now + timedelta(seconds=1)).isoformat(),
    )

    assert entry.is_stale(now) is True


def test_load_model_cache_tolerates_invalid_json(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text("{not json", encoding="utf-8")

    assert load_model_cache(config) == CachedModels()


def test_load_model_cache_normalizes_provider_to_map_key(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "gemini",'
        '"models": ["gpt-4.1-mini"],'
        '"fetched_at": "2026-06-09T12:00:00+00:00",'
        '"error": ""'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config).providers["openai"].provider == "openai"


def test_load_model_cache_skips_bad_datetime_entries(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "openai",'
        '"models": ["gpt-4.1-mini"],'
        '"fetched_at": "not-a-date",'
        '"error": "model suggestions unavailable"'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config) == CachedModels()


def test_load_model_cache_skips_success_entries_without_valid_models(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "openai",'
        '"models": [1, null],'
        '"fetched_at": "2026-06-09T12:00:00+00:00",'
        '"error": ""'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config) == CachedModels()

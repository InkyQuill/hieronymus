import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfigError,
    ProviderProfile,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
    save_dream_config,
)


def test_dream_config_paths_live_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.dream_config_path == config.config_root / "dream.conf"
    assert config.llm_cache_path == config.config_root / "llmcache.tmp"


def test_default_dream_config_matches_memory_spec(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    dream_config = load_dream_config(config)

    assert dream_config.enabled is False
    assert dream_config.schedule_interval_minutes == 30
    assert dream_config.min_pending_short_term_memories == 20
    assert dream_config.max_pending_short_term_memories == 200
    assert dream_config.max_short_term_memories_per_cycle == 50
    assert dream_config.not_enough_memories_cycle_threshold == 5
    assert dream_config.workflows["crystallization"].provider == "anthropic"
    assert dream_config.workflows["relation_discovery"].enabled is False
    assert dream_config.workflows["reinforcement_compaction"].provider == "ollama"


def test_save_dream_config_writes_plaintext_api_key_and_redacts_json(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_provider(
        "anthropic",
        ProviderProfile(
            type="anthropic",
            endpoint="https://api.anthropic.com",
            api_key="secret-value",
            timeout_seconds=30.0,
        ),
    )

    save_dream_config(config, dream_config)

    raw = config.dream_config_path.read_text(encoding="utf-8")
    assert "secret-value" in raw
    payload = tomllib.loads(raw)
    assert payload["providers"]["anthropic"]["api_key"] == "secret-value"
    redacted = redacted_dream_config_payload(dream_config)
    assert redacted["providers"]["anthropic"]["api_key"] == "***"


def test_load_dream_config_rejects_invalid_threshold_order(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        "[dreaming]\n"
        "enabled = true\n"
        "schedule_interval_minutes = 30\n"
        "min_pending_short_term_memories = 20\n"
        "max_pending_short_term_memories = 10\n"
        "max_short_term_memories_per_cycle = 50\n"
        "not_enough_memories_cycle_threshold = 5\n",
        encoding="utf-8",
    )

    with pytest.raises(DreamConfigError, match="max_pending_short_term_memories"):
        load_dream_config(config)


def test_workflow_references_existing_provider(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_workflow(
        "crystallization",
        WorkflowProfile(provider="missing", model="model", enabled=True),
    )

    with pytest.raises(DreamConfigError, match="referenced provider profile is missing"):
        save_dream_config(config, dream_config)

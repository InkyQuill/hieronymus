import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.ingest_config import (
    IngestConfig,
    IngestConfigError,
    LearnLimits,
    ShortMemoryLimits,
    default_ingest_config,
    load_ingest_config,
    save_ingest_config,
)


def test_ingest_config_path_lives_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.ingest_config_path == config.config_root / "ingest.conf"


def test_default_ingest_config_preserves_current_behavior(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    ingest_config = load_ingest_config(config)

    assert ingest_config == default_ingest_config()
    assert ingest_config.short_memory.warning_sentence_count == 6
    assert ingest_config.short_memory.rejection_sentence_count == 30
    assert ingest_config.short_memory.warning_symbol_count == 0
    assert ingest_config.short_memory.rejection_symbol_count == 0
    assert ingest_config.learn.max_block_chars == 1200


def test_save_load_ingest_config_round_trips_plaintext_limits(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    ingest_config = IngestConfig(
        short_memory=ShortMemoryLimits(
            warning_sentence_count=5,
            rejection_sentence_count=12,
            warning_symbol_count=1000,
            rejection_symbol_count=3000,
        ),
        learn=LearnLimits(max_block_chars=900),
    )

    save_ingest_config(config, ingest_config)

    raw = config.ingest_config_path.read_text(encoding="utf-8")
    assert "warning_sentence_count = 5" in raw
    assert "rejection_sentence_count = 12" in raw
    assert "warning_symbol_count = 1000" in raw
    assert "rejection_symbol_count = 3000" in raw
    assert "max_block_chars = 900" in raw
    assert tomllib.loads(raw) == ingest_config.to_payload()
    assert load_ingest_config(config) == ingest_config


def test_load_ingest_config_rejects_invalid_sentence_threshold_order(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(
        "[short_memory]\nwarning_sentence_count = 30\nrejection_sentence_count = 6\n",
        encoding="utf-8",
    )

    with pytest.raises(IngestConfigError, match="rejection_sentence_count"):
        load_ingest_config(config)

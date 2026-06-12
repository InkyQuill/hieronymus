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


def test_load_ingest_config_rejects_invalid_symbol_threshold_order(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(
        "[short_memory]\nwarning_symbol_count = 100\nrejection_symbol_count = 50\n",
        encoding="utf-8",
    )

    with pytest.raises(IngestConfigError, match="rejection_symbol_count"):
        load_ingest_config(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[unknown]\nvalue = 1\n",
            "unknown ingest config setting: unknown",
        ),
        (
            "[short_memory]\nextra = 1\n",
            "unknown ingest config setting: short_memory.extra",
        ),
        (
            "[learn]\nextra = 1\n",
            "unknown ingest config setting: learn.extra",
        ),
    ],
)
def test_load_ingest_config_rejects_unknown_keys(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(raw_config, encoding="utf-8")

    with pytest.raises(IngestConfigError, match=error):
        load_ingest_config(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[short_memory]\nwarning_sentence_count = true\n",
            "short_memory.warning_sentence_count must be an integer",
        ),
        (
            "[short_memory]\nrejection_symbol_count = 1.5\n",
            "short_memory.rejection_symbol_count must be an integer",
        ),
        (
            "[learn]\nmax_block_chars = '1200'\n",
            "learn.max_block_chars must be an integer",
        ),
    ],
)
def test_load_ingest_config_rejects_non_integer_values(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(raw_config, encoding="utf-8")

    with pytest.raises(IngestConfigError, match=error):
        load_ingest_config(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[short_memory]\nwarning_sentence_count = 0\n",
            "short_memory.warning_sentence_count must be at least 1",
        ),
        (
            "[short_memory]\nrejection_sentence_count = 0\n",
            "short_memory.rejection_sentence_count must be at least 1",
        ),
        (
            "[short_memory]\nwarning_symbol_count = -1\n",
            "short_memory.warning_symbol_count must be at least 0",
        ),
        (
            "[short_memory]\nrejection_symbol_count = -1\n",
            "short_memory.rejection_symbol_count must be at least 0",
        ),
        (
            "[learn]\nmax_block_chars = 0\n",
            "learn.max_block_chars must be at least 1",
        ),
    ],
)
def test_load_ingest_config_rejects_minimum_value_failures(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(raw_config, encoding="utf-8")

    with pytest.raises(IngestConfigError, match=error):
        load_ingest_config(config)

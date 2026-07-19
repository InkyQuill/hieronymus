from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.semantic_config import (
    DEFAULT_SEMANTIC_MODEL,
    SemanticConfig,
    SemanticConfigError,
    load_semantic_config,
)


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def test_semantic_config_uses_local_multilingual_defaults(tmp_path: Path) -> None:
    semantic = load_semantic_config(_config(tmp_path), environ={})

    assert semantic == SemanticConfig(
        enabled=True,
        provider="local",
        model=DEFAULT_SEMANTIC_MODEL,
        revision=None,
        dimensions=384,
        batch_size=32,
        candidate_multiplier=4,
        fusion_constant=60,
    )


def test_hieronymus_config_exposes_semantic_storage_paths(tmp_path: Path) -> None:
    config = _config(tmp_path)

    assert config.semantic_config_path == config.data_root / "semantic.conf"
    assert config.semantic_index_root == config.data_root / "semantic-index"
    assert config.embedding_cache_root == config.data_root / "embedding-models"


def test_semantic_config_file_overrides_defaults(tmp_path: Path) -> None:
    config = _config(tmp_path)
    config.data_root.mkdir(parents=True)
    config.semantic_config_path.write_text(
        """
[semantic]
enabled = false
provider = "future-remote"
model = "provider/model"
revision = "2026-07-19"
dimensions = 768
batch_size = 16
candidate_multiplier = 6
fusion_constant = 80
""".strip(),
        encoding="utf-8",
    )

    assert load_semantic_config(config, environ={}) == SemanticConfig(
        enabled=False,
        provider="future-remote",
        model="provider/model",
        revision="2026-07-19",
        dimensions=768,
        batch_size=16,
        candidate_multiplier=6,
        fusion_constant=80,
    )


def test_semantic_environment_overrides_file(tmp_path: Path) -> None:
    config = _config(tmp_path)
    config.data_root.mkdir(parents=True)
    config.semantic_config_path.write_text(
        '[semantic]\nenabled = true\nprovider = "local"\nbatch_size = 8\n',
        encoding="utf-8",
    )

    semantic = load_semantic_config(
        config,
        environ={
            "HIERONYMUS_SEMANTIC_ENABLED": "off",
            "HIERONYMUS_SEMANTIC_PROVIDER": "remote",
            "HIERONYMUS_SEMANTIC_MODEL": "remote/model",
            "HIERONYMUS_SEMANTIC_REVISION": "r2",
            "HIERONYMUS_SEMANTIC_DIMENSIONS": "1024",
            "HIERONYMUS_SEMANTIC_BATCH_SIZE": "24",
            "HIERONYMUS_SEMANTIC_CANDIDATE_MULTIPLIER": "7",
            "HIERONYMUS_SEMANTIC_FUSION_CONSTANT": "90",
        },
    )

    assert semantic == SemanticConfig(
        enabled=False,
        provider="remote",
        model="remote/model",
        revision="r2",
        dimensions=1024,
        batch_size=24,
        candidate_multiplier=7,
        fusion_constant=90,
    )


@pytest.mark.parametrize(
    ("payload", "message"),
    [
        ("[semantic]\nenabled = 1\n", "semantic.enabled must be a boolean"),
        ("[semantic]\nunknown = 1\n", "unknown semantic config setting"),
        ("[semantic]\nbatch_size = 0\n", "semantic.batch_size must be between"),
        ("[semantic]\ncandidate_multiplier = 101\n", "candidate_multiplier"),
        ("[semantic]\nfusion_constant = 0\n", "fusion_constant"),
        ("[semantic]\ndimensions = true\n", "dimensions must be an integer"),
        ("semantic = 1\n", "semantic must be a table"),
    ],
)
def test_semantic_config_rejects_invalid_file_values(
    tmp_path: Path, payload: str, message: str
) -> None:
    config = _config(tmp_path)
    config.data_root.mkdir(parents=True)
    config.semantic_config_path.write_text(payload, encoding="utf-8")

    with pytest.raises(SemanticConfigError, match=message):
        load_semantic_config(config, environ={})


@pytest.mark.parametrize(
    ("name", "value"),
    [
        ("HIERONYMUS_SEMANTIC_ENABLED", "sometimes"),
        ("HIERONYMUS_SEMANTIC_DIMENSIONS", "384.0"),
        ("HIERONYMUS_SEMANTIC_BATCH_SIZE", "0"),
    ],
)
def test_semantic_config_rejects_invalid_environment_values(
    tmp_path: Path, name: str, value: str
) -> None:
    with pytest.raises(SemanticConfigError, match=name):
        load_semantic_config(_config(tmp_path), environ={name: value})


def test_disabled_semantic_config_is_explicit_fts_only_mode(tmp_path: Path) -> None:
    semantic = load_semantic_config(
        _config(tmp_path), environ={"HIERONYMUS_SEMANTIC_ENABLED": "false"}
    )

    assert semantic.enabled is False
    assert semantic.fts_only is True

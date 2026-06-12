from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.release_config import (
    ReleaseConfig,
    ReleaseConfigError,
    load_release_config,
    save_release_config,
)


def test_release_config_path_lives_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.release_config_path == config.config_root / "release.conf"


def test_default_release_config_uses_stable_channel(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    release_config = load_release_config(config)

    assert release_config.update_channel == "stable"
    assert release_config.update_target == "latest"
    assert release_config.allows_dev_updates is False


def test_save_release_config_round_trips_dev_channel(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    save_release_config(config, ReleaseConfig(update_channel="dev"))

    assert load_release_config(config).update_channel == "dev"
    assert 'channel = "dev"' in config.release_config_path.read_text(encoding="utf-8")


def test_load_release_config_rejects_unknown_channel(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.release_config_path.write_text('[updates]\nchannel = "nightly"\n', encoding="utf-8")

    with pytest.raises(ReleaseConfigError, match="updates.channel"):
        load_release_config(config)

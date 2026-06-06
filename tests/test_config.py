from pathlib import Path

from hieronymus.config import load_config


def test_load_config_uses_explicit_data_root() -> None:
    config = load_config("~/hieronymus-test")

    assert config.data_root == Path("~/hieronymus-test").expanduser()
    assert config.registry_path == config.data_root / "registry.sqlite"
    assert config.series_dir == config.data_root / "series"

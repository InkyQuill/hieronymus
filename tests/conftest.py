from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "memory")

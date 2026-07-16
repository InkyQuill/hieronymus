import os
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.service_manager import ServiceManager


@pytest.fixture(scope="session", autouse=True)
def setup_path():
    # Prepend mise and uv paths to environment PATH
    home = Path.home()
    mise_bin = home / ".local/bin"
    mise_shims = home / ".local/share/mise/shims"

    old_path = os.environ.get("PATH", "")
    new_paths = [str(mise_shims), str(mise_bin)]

    os.environ["PATH"] = os.pathsep.join(new_paths) + os.pathsep + old_path
    yield


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "memory")


@pytest.fixture(autouse=True)
def stop_daemons_started_for_test(tmp_path_factory: pytest.TempPathFactory):
    yield
    for state_path in tmp_path_factory.getbasetemp().rglob("server.json"):
        ServiceManager(HieronymusConfig(data_root=state_path.parent)).stop()

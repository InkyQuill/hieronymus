from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    @property
    def database_path(self) -> Path:
        return self.data_root / "hieronymus.sqlite"


def load_config(data_root: str | Path | None = None) -> HieronymusConfig:
    if data_root is not None:
        root = Path(data_root).expanduser()
    elif env_root := os.environ.get("HIERONYMUS_DATA_ROOT"):
        root = Path(env_root).expanduser()
    else:
        root = Path.home() / ".config" / "hieronymus"
    return HieronymusConfig(data_root=root)

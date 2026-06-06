from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    @property
    def registry_path(self) -> Path:
        return self.data_root / "registry.sqlite"

    @property
    def series_dir(self) -> Path:
        return self.data_root / "series"


def load_config(data_root: str | None = None) -> HieronymusConfig:
    raw_root = data_root or os.environ.get("HIERONYMUS_DATA_ROOT")
    root = Path(raw_root).expanduser() if raw_root else Path.home() / ".hieronymus"
    return HieronymusConfig(data_root=root)

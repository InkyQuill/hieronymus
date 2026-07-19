from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    def __post_init__(self) -> None:
        object.__setattr__(self, "data_root", self.data_root.expanduser().resolve())

    @property
    def database_path(self) -> Path:
        return self.data_root / "hieronymus.sqlite"

    @property
    def dream_config_path(self) -> Path:
        return self.data_root / "dream.conf"

    @property
    def provider_config_path(self) -> Path:
        return self.data_root / "provider.conf"

    @property
    def ingest_config_path(self) -> Path:
        return self.data_root / "ingest.conf"

    @property
    def release_config_path(self) -> Path:
        return self.data_root / "release.conf"

    @property
    def llm_cache_path(self) -> Path:
        return self.data_root / "llm-cache.json"

    @property
    def backups_root(self) -> Path:
        return self.data_root / "backups"

    @property
    def agent_plugins_root(self) -> Path:
        return self.data_root / "agent-plugins"


def load_config(data_root: str | Path | None = None) -> HieronymusConfig:
    if data_root is not None:
        root = Path(data_root)
    elif env_root := os.environ.get("HIERONYMUS_DATA_ROOT", "").strip():
        root = Path(env_root)
    elif xdg_root := os.environ.get("XDG_CONFIG_HOME", "").strip():
        root = Path(xdg_root) / "hieronymus"
    else:
        root = Path.home() / ".config" / "hieronymus"
    return HieronymusConfig(data_root=root)

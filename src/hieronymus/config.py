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

    @property
    def config_root(self) -> Path:
        return self.data_root

    @property
    def settings_path(self) -> Path:
        return self.config_root / "settings.toml"

    @property
    def dream_config_path(self) -> Path:
        return self.config_root / "dream.conf"

    @property
    def llm_cache_path(self) -> Path:
        return self.config_root / "llmcache.tmp"

    @property
    def backups_root(self) -> Path:
        return self.config_root / "backups"

    @property
    def agent_plugins_root(self) -> Path:
        return self.config_root / "agent-plugins"


def load_config(data_root: str | Path | None = None) -> HieronymusConfig:
    if data_root is not None:
        root = Path(data_root).expanduser()
    elif env_root := os.environ.get("HIERONYMUS_DATA_ROOT"):
        root = Path(env_root).expanduser()
    else:
        root = Path.home() / ".config" / "hieronymus"
    return HieronymusConfig(data_root=root)

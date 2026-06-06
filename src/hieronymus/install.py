from __future__ import annotations

from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.agent_plugins.base import (
    InstallPlan,
    InstallStep,
    atomic_write_text,
    backup_file,
)
from hieronymus.config import HieronymusConfig

__all__ = [
    "InstallPlan",
    "InstallStep",
    "atomic_write_text",
    "backup_file",
    "known_targets",
    "plan_install",
    "resolve_target",
]


def known_targets() -> list[str]:
    return [plugin.name for plugin in available_plugins()]


def resolve_target(name: str):
    return resolve_plugin(name)


def plan_install(config: HieronymusConfig, target_name: str) -> InstallPlan:
    return resolve_plugin(target_name).plan(config)

from __future__ import annotations

from dataclasses import dataclass

from hieronymus.agent_plugins import available_plugins, resolve_plugin
from hieronymus.agent_plugins.base import (
    AgentPlugin,
    InstallPlan,
    InstallStep,
    atomic_write_text,
    backup_file,
)
from hieronymus.config import HieronymusConfig


@dataclass(frozen=True)
class InstallTarget:
    name: str
    display_name: str
    detect_path: str
    config_path: str
    protocol_note: str
    docs: str

    @property
    def detect_paths(self) -> tuple[str, ...]:
        return (self.detect_path,)

    @property
    def config_paths(self) -> tuple[str, ...]:
        return (self.config_path,)


def _target_from_plugin(plugin: AgentPlugin) -> InstallTarget:
    return InstallTarget(
        name=plugin.name,
        display_name=plugin.display_name,
        detect_path=plugin.detect_path,
        config_path=plugin.config_path,
        protocol_note=plugin.protocol_note,
        docs=plugin.docs,
    )


TARGETS = [_target_from_plugin(plugin) for plugin in available_plugins()]


__all__ = [
    "InstallTarget",
    "InstallPlan",
    "InstallStep",
    "TARGETS",
    "agent_install_candidates",
    "atomic_write_text",
    "backup_file",
    "known_targets",
    "plan_install",
    "resolve_target",
]


def known_targets() -> list[str]:
    return [target.name for target in TARGETS]


def resolve_target(name: str) -> InstallTarget:
    normalized = name.lower()
    for target in TARGETS:
        if target.name == normalized:
            return target
    resolve_plugin(name)
    raise AssertionError("resolve_plugin returned for an unknown target")


def agent_install_candidates(config: HieronymusConfig) -> list[dict[str, object]]:
    return [plugin.availability(config).to_json_dict() for plugin in available_plugins()]


def plan_install(config: HieronymusConfig, target_name: str) -> InstallPlan:
    return resolve_plugin(target_name).plan(config)

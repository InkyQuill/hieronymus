from __future__ import annotations

from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import BaseAgentPlugin, InstallPlan, write_plugin_assets
from hieronymus.config import HieronymusConfig


class CodexPlugin(BaseAgentPlugin):
    name = "codex"
    display_name = "Codex"
    detect_paths = ("~/.codex",)
    config_paths = ("~/.codex/config.toml",)
    protocol_note = "Codex integration installs Hieronymus skills, MCP config, and hooks."

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        _ = force
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        plan = self.plan(config)
        return InstallPlan(
            target=plan.target,
            display_name=plan.display_name,
            protocol_note=plan.protocol_note,
            docs=plan.docs,
            result_kind="installed",
            steps=plan.steps,
            availability=self.availability(config),
        )

from __future__ import annotations

from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import (
    BaseAgentPlugin,
    InstallPlan,
    expand_user,
    get_object_section,
    load_json_object,
    patch_json_config,
    set_managed_entry,
    write_plugin_assets,
)
from hieronymus.config import HieronymusConfig


class GeminiPlugin(BaseAgentPlugin):
    name = "gemini"
    display_name = "Gemini CLI"
    detect_paths = ("~/.gemini",)
    config_paths = ("~/.gemini/settings.json",)
    protocol_note = "Gemini CLI integration installs Hieronymus MCP and extension configuration."
    installs_managed_config = True
    required_asset_paths = (
        "gemini-extension.json",
        "mcp/hieronymus.mcp.json",
        "skills/hieronymus-recall/SKILL.md",
    )

    def has_expected_config(self, config: HieronymusConfig) -> bool:
        config_path = expand_user(self.config_paths[0])
        try:
            payload = load_json_object(config_path)
        except (OSError, ValueError):
            return False
        marker = payload.get("hieronymus")
        mcp_servers = payload.get("mcpServers")
        extensions = payload.get("extensions")
        return (
            isinstance(marker, dict)
            and marker.get("managed") is True
            and isinstance(mcp_servers, dict)
            and mcp_servers.get("hieronymus") == {"command": "hieronymus-mcp", "args": []}
            and isinstance(extensions, dict)
            and extensions.get("hieronymus") == {"path": str(config.agent_plugins_root / self.name)}
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        config_path = expand_user(self.config_paths[0])
        payload = load_json_object(config_path)
        set_managed_entry(
            get_object_section(payload, "mcpServers", config_path),
            "hieronymus",
            {"command": "hieronymus-mcp", "args": []},
            path=config_path,
            force=force,
        )
        set_managed_entry(
            get_object_section(payload, "extensions", config_path),
            "hieronymus",
            {"path": str(config.agent_plugins_root / self.name)},
            path=config_path,
            force=force,
        )
        payload["hieronymus"] = {"managed": True, "version": "0.1.0"}
        write_plugin_assets(config, self.name, render_agent_plugin_assets(self.name))
        patch_json_config(config, config_path, agent=self.name, payload=payload)
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

from __future__ import annotations

from hieronymus.agent_assets import render_agent_plugin_assets
from hieronymus.agent_plugins.base import (
    HIERONYMUS_MCP_URL,
    BaseAgentPlugin,
    InstallPlan,
    McpTransport,
    expand_user,
    get_object_section,
    is_hieronymus_managed,
    load_toml_object,
    patch_toml_config,
    set_managed_entry,
    write_plugin_assets,
)
from hieronymus.config import HieronymusConfig


class CodexPlugin(BaseAgentPlugin):
    name = "codex"
    display_name = "Codex"
    detect_paths = ("~/.codex",)
    config_paths = ("~/.codex/config.toml",)
    protocol_note = "Codex integration installs Hieronymus skills, MCP config, and hooks."
    installs_managed_config = True
    mcp_transport = McpTransport.STREAMABLE_HTTP
    required_asset_paths = (
        ".codex-plugin/plugin.json",
        ".mcp.json",
        "skills/hieronymus-recall/SKILL.md",
    )

    def mcp_server_entry(self) -> dict[str, object]:
        return {"url": HIERONYMUS_MCP_URL}

    def has_expected_config(self, config: HieronymusConfig) -> bool:
        config_path = expand_user(self.config_paths[0])
        try:
            payload = load_toml_object(config_path)
        except (OSError, ValueError):
            return False
        marker = payload.get("hieronymus")
        if not isinstance(marker, dict) or marker.get("managed") is not True:
            return False
        mcp_servers = payload.get("mcp_servers")
        plugins = payload.get("plugins")
        return (
            isinstance(mcp_servers, dict)
            and mcp_servers.get("hieronymus") == self.mcp_server_entry()
            and isinstance(plugins, dict)
            and plugins.get("hieronymus") == {"path": str(config.agent_plugins_root / self.name)}
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        config_path = expand_user(self.config_paths[0])
        payload = load_toml_object(config_path)
        managed = is_hieronymus_managed(payload)
        set_managed_entry(
            get_object_section(payload, "mcp_servers", config_path),
            "hieronymus",
            self.mcp_server_entry(),
            path=config_path,
            force=force,
            owned_existing_values=({"command": "hieronymus-mcp", "args": []},),
            managed=managed,
        )
        set_managed_entry(
            get_object_section(payload, "plugins", config_path),
            "hieronymus",
            {"path": str(config.agent_plugins_root / self.name)},
            path=config_path,
            force=force,
        )
        payload["hieronymus"] = {"managed": True, "version": "0.1.0"}
        write_plugin_assets(
            config,
            self.name,
            render_agent_plugin_assets(self.name, mcp_server_entry=self.mcp_server_entry()),
        )
        patch_toml_config(config, config_path, agent=self.name, payload=payload)
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

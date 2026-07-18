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
    mcp_transport = McpTransport.STREAMABLE_HTTP
    required_asset_paths = (
        "gemini-extension.json",
        "mcp/hieronymus.mcp.json",
        "skills/hieronymus-recall/SKILL.md",
    )

    def mcp_server_entry(self) -> dict[str, object]:
        return {"httpUrl": HIERONYMUS_MCP_URL}

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
            and mcp_servers.get("hieronymus") == self.mcp_server_entry()
            and isinstance(extensions, dict)
            and extensions.get("hieronymus") == {"path": str(config.agent_plugins_root / self.name)}
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        config_path = expand_user(self.config_paths[0])
        payload = load_json_object(config_path)
        managed = is_hieronymus_managed(payload)
        set_managed_entry(
            get_object_section(payload, "mcpServers", config_path),
            "hieronymus",
            self.mcp_server_entry(),
            path=config_path,
            force=force,
            owned_existing_values=({"command": "hieronymus-mcp", "args": []},),
            managed=managed,
        )
        set_managed_entry(
            get_object_section(payload, "extensions", config_path),
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

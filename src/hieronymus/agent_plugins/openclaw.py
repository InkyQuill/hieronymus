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
    remove_owned_legacy_entry,
    set_managed_entry,
    write_plugin_assets,
)
from hieronymus.config import HieronymusConfig


class OpenClawPlugin(BaseAgentPlugin):
    name = "openclaw"
    display_name = "OpenClaw"
    detect_paths = ("~/.openclaw",)
    config_paths = ("~/.openclaw/openclaw.json",)
    protocol_note = "OpenClaw integration installs Hieronymus MCP and plugin configuration."
    installs_managed_config = True
    mcp_transport = McpTransport.STREAMABLE_HTTP
    required_asset_paths = (
        "openclaw/plugin.json",
        "mcp/hieronymus.mcp.json",
        "skills/hieronymus-recall/SKILL.md",
    )

    def mcp_server_entry(self) -> dict[str, object]:
        return {"transport": "streamable-http", "url": HIERONYMUS_MCP_URL}

    def has_expected_config(self, config: HieronymusConfig) -> bool:
        config_path = expand_user(self.config_paths[0])
        try:
            payload = load_json_object(config_path)
        except (OSError, ValueError):
            return False
        marker = payload.get("hieronymus")
        mcp = payload.get("mcp")
        plugins = payload.get("plugins")
        return (
            isinstance(marker, dict)
            and marker.get("managed") is True
            and isinstance(mcp, dict)
            and isinstance(mcp.get("servers"), dict)
            and mcp["servers"].get("hieronymus") == self.mcp_server_entry()
            and isinstance(plugins, dict)
            and plugins.get("hieronymus") == {"path": str(config.agent_plugins_root / self.name)}
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        config_path = expand_user(self.config_paths[0])
        payload = load_json_object(config_path)
        managed = is_hieronymus_managed(payload)
        remove_owned_legacy_entry(
            payload,
            "mcpServers",
            path=config_path,
            managed=managed,
            expected={"command": "hieronymus-mcp", "args": []},
        )
        mcp = get_object_section(payload, "mcp", config_path)
        set_managed_entry(
            get_object_section(mcp, "servers", config_path),
            "hieronymus",
            self.mcp_server_entry(),
            path=config_path,
            force=force,
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
            render_agent_plugin_assets(self.name),
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

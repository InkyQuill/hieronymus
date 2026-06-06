from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class OpenCodePlugin(BaseAgentPlugin):
    name = "opencode"
    display_name = "opencode"
    detect_paths = ("~/.config/opencode",)
    config_paths = ("~/.config/opencode/plugin.json",)
    protocol_note = "opencode integration is reserved for future MCP plugin configuration support."

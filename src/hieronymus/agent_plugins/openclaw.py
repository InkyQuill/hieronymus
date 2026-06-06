from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class OpenClawPlugin(BaseAgentPlugin):
    name = "openclaw"
    display_name = "OpenClaw"
    detect_paths = ("~/.openclaw",)
    config_paths = ("~/.openclaw/openclaw.json",)
    protocol_note = "OpenClaw integration is reserved for future MCP configuration support."

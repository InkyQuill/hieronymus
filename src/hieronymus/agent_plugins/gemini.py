from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class GeminiPlugin(BaseAgentPlugin):
    name = "gemini"
    display_name = "Gemini CLI"
    detect_paths = ("~/.gemini",)
    config_paths = ("~/.gemini/settings.json",)
    protocol_note = "Gemini CLI integration is reserved for future MCP configuration support."

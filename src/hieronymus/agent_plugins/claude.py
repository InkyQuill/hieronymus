from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class ClaudePlugin(BaseAgentPlugin):
    name = "claude"
    display_name = "Claude Code / Claude Desktop"
    detect_paths = ("~/.claude", "~/.claude.json")
    config_paths = ("~/.claude.json",)
    protocol_note = (
        "Claude Code integration uses MCP; host-specific hooks are deferred to a later pass."
    )

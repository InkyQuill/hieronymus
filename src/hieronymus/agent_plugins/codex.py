from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class CodexPlugin(BaseAgentPlugin):
    name = "codex"
    display_name = "Codex"
    detect_paths = ("~/.codex",)
    config_paths = ("~/.codex/config.toml",)
    protocol_note = "Codex integration installs Hieronymus skills, MCP config, and hooks."

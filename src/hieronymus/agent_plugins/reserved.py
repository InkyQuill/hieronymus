from __future__ import annotations

from hieronymus.agent_plugins.base import BaseAgentPlugin


class PiPlugin(BaseAgentPlugin):
    name = "pi"
    display_name = "Pi"
    detect_paths = ("~/.pi",)
    config_paths = ("~/.pi/config.json",)
    protocol_note = "Pi is a reserved future integration target."


class HermesPlugin(BaseAgentPlugin):
    name = "hermes"
    display_name = "Hermes"
    detect_paths = ("~/.hermes",)
    config_paths = ("~/.hermes/config.json",)
    protocol_note = "Hermes is a reserved future integration target."

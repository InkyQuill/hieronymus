from __future__ import annotations

from dataclasses import replace

from hieronymus.agent_plugins.base import AgentAvailability, BaseAgentPlugin, InstallPlan
from hieronymus.config import HieronymusConfig


class ReservedAgentPlugin(BaseAgentPlugin):
    reserved_reason = "No safe host protocol is implemented for this target."

    def availability(self, config: HieronymusConfig) -> AgentAvailability:
        availability = super().availability(config)
        return replace(availability, installed=False, reason="reserved target")

    def plan(self, config: HieronymusConfig) -> InstallPlan:
        self._require_non_empty_paths()
        return InstallPlan(
            target=self.name,
            display_name=self.display_name,
            protocol_note=self.protocol_note or self.reserved_reason,
            docs=self.docs,
            result_kind="reserved",
            steps=[],
            availability=self.availability(config),
        )

    def install(self, config: HieronymusConfig, *, force: bool = False) -> InstallPlan:
        _ = force
        return self.plan(config)


class PiPlugin(ReservedAgentPlugin):
    name = "pi"
    display_name = "Pi"
    detect_paths = ("~/.pi",)
    config_paths = ("~/.pi/config.json",)
    protocol_note = "Pi is reserved because no safe host protocol is implemented for this target."


class HermesPlugin(ReservedAgentPlugin):
    name = "hermes"
    display_name = "Hermes"
    detect_paths = ("~/.hermes",)
    config_paths = ("~/.hermes/config.json",)
    protocol_note = (
        "Hermes is reserved because no safe host protocol is implemented for this target."
    )


class MimoPlugin(ReservedAgentPlugin):
    name = "mimo"
    aliases = ("xiaomi-mimo", "xiaomi_mimo", "mimocode")
    display_name = "Xiaomi MiMo"
    detect_paths = ("~/.mimocode", "~/.config/mimocode")
    config_paths = ("~/.config/mimocode",)
    protocol_note = (
        "Xiaomi MiMo is detected through ~/.mimocode and ~/.config/mimocode, "
        "but Hieronymus does not write MiMo configuration until a stable "
        "noninteractive MCP or plugin configuration contract is implemented."
    )

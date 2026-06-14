from __future__ import annotations

from hieronymus.agent_plugins.base import AgentPlugin


def available_plugins() -> list[AgentPlugin]:
    from hieronymus.agent_plugins.claude import ClaudePlugin
    from hieronymus.agent_plugins.codex import CodexPlugin
    from hieronymus.agent_plugins.gemini import GeminiPlugin
    from hieronymus.agent_plugins.openclaw import OpenClawPlugin
    from hieronymus.agent_plugins.opencode import OpenCodePlugin
    from hieronymus.agent_plugins.reserved import HermesPlugin, MimoPlugin, PiPlugin

    return [
        ClaudePlugin(),
        CodexPlugin(),
        OpenClawPlugin(),
        OpenCodePlugin(),
        GeminiPlugin(),
        MimoPlugin(),
        PiPlugin(),
        HermesPlugin(),
    ]


def resolve_plugin(name: str) -> AgentPlugin:
    wanted = name.lower()
    for plugin in available_plugins():
        aliases = tuple(alias.lower() for alias in plugin.aliases)
        if plugin.name.lower() == wanted or wanted in aliases:
            return plugin
    supported = ", ".join(plugin.name for plugin in available_plugins())
    raise ValueError(f"Unsupported agent target {name!r}; supported targets: {supported}")

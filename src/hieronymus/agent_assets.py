from __future__ import annotations

import json

MCP_CONFIG = {
    "mcpServers": {
        "hieronymus": {
            "command": "hieronymus-mcp",
            "args": [],
            "env": {},
        }
    }
}

CODEX_HOOKS = {
    "hooks": [
        {
            "event": "SessionStart",
            "command": "python",
            "args": ["-m", "hieronymus.agent_hooks", "session-start"],
        },
        {
            "event": "Stop",
            "command": "python",
            "args": ["-m", "hieronymus.agent_hooks", "session-end"],
        },
    ]
}

BOUNDARY_TEXT = """Strict concept contracts are mandatory. Crystals and lessons are advisory.
Do not approve terminology proposals yourself; record proposals for human approval instead."""

RECALL_SKILL = f"""---
name: hieronymus-recall
description: Recall Hieronymus memory before translation, review, terminology, or docs work.
---

# Hieronymus Recall

Use this skill before memory-sensitive work. Start or identify a task session, call
`hieronymus_recall`, and keep strict concept contracts separate from advisory crystals.

{BOUNDARY_TEXT}
Cite influential crystals when they shape a translation or review decision.
"""

LEARN_SKILL = f"""---
name: hieronymus-learn
description: Commit material into short-term memory for later dreaming and crystallization.
---

# Hieronymus Learn

Use when the user says to absorb, remember, study, ingest, import, or learn from a source.
Call `hieronymus_learn` with provenance. Do not promote strict terminology directly; dreaming
can produce crystals, lessons, erudition, and proposals later.

{BOUNDARY_TEXT}
"""

READ_SKILL = f"""---
name: hieronymus-read
description: Inspect material for the current task without committing the whole source to memory.
---

# Hieronymus Read

Use for casual lookup, extraction, summaries, or temporary understanding. Call `hieronymus_read`.
By default, do not store every block as short-term memory.

{BOUNDARY_TEXT}
"""

TRANSLATE_SKILL = f"""---
name: hieronymus-translate
description: Translate with Hieronymus strict terminology and advisory crystals.
---

# Hieronymus Translate

{BOUNDARY_TEXT}
Apply approved concept contracts first. Use crystals and lessons only as context, and record
uncertainty or discoveries as short-term memories.
"""

REVIEW_SKILL = f"""---
name: hieronymus-review
description: Review translation output using strict terminology and mentor-grade observations.
---

# Hieronymus Review

{BOUNDARY_TEXT}
Check strict validation findings first. Identify whether crystals helped or misled. Record recurring
issues, contradictions, and correction patterns as mentor short-term memories.
"""

ORCHESTRATE_SKILL = f"""---
name: hieronymus-orchestrate
description: Coordinate Hieronymus task sessions, recall, validation, feedback, and dreaming.
---

# Hieronymus Orchestrate

{BOUNDARY_TEXT}
Create a task session, recall before work, collect short-term memories, record feedback events, and
trigger or defer dreaming based on configuration or user instruction.
"""


def _json(payload: object) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def asset_map() -> dict[str, str]:
    return {
        "skills/hieronymus-recall/SKILL.md": RECALL_SKILL,
        "skills/hieronymus-learn/SKILL.md": LEARN_SKILL,
        "skills/hieronymus-read/SKILL.md": READ_SKILL,
        "skills/hieronymus-translate/SKILL.md": TRANSLATE_SKILL,
        "skills/hieronymus-review/SKILL.md": REVIEW_SKILL,
        "skills/hieronymus-orchestrate/SKILL.md": ORCHESTRATE_SKILL,
        "mcp/hieronymus.mcp.json": _json(MCP_CONFIG),
        "hooks/hooks.codex.json": _json(CODEX_HOOKS),
    }


def render_agent_plugin_assets(target: str) -> dict[str, str]:
    assets = asset_map()
    plugin_json = {
        "name": "hieronymus",
        "version": "0.1.0",
        "description": "Translation memory, termbase, recall, and dreaming workflows for agents.",
        "skills": "./skills/",
        "mcpServers": "./mcp/hieronymus.mcp.json",
    }

    if target == "codex":
        assets[".mcp.json"] = _json(MCP_CONFIG)
        assets[".codex-plugin/plugin.json"] = _json(
            {
                **plugin_json,
                "author": {
                    "email": "me@inkyquill.net",
                    "name": "Pavel Obruchnikov",
                },
                "interface": {
                    "capabilities": [
                        "translation memory recall",
                        "terminology boundary guidance",
                        "literary translation workflow skills",
                    ],
                    "category": "productivity",
                    "defaultPrompt": (
                        "Use Hieronymus skills and MCP tools when translation memory, "
                        "terminology boundaries, or literary workflow context matter."
                    ),
                    "developerName": "Pavel Obruchnikov",
                    "displayName": "Hieronymus",
                    "longDescription": (
                        "Hieronymus packages local-first translation memory workflows, "
                        "strict terminology boundary guidance, and MCP recall for literary "
                        "translation agents."
                    ),
                    "shortDescription": (
                        "Local-first translation memory and terminology workflows for agents."
                    ),
                },
                "mcpServers": "./.mcp.json",
            }
        )
        return assets
    if target == "claude":
        assets[".claude-plugin/plugin.json"] = _json(plugin_json)
        return assets
    if target == "gemini":
        assets["gemini-extension.json"] = _json(
            {
                "name": "hieronymus",
                "version": "0.1.0",
                "contextFileName": "AGENTS.md",
                "mcpServers": MCP_CONFIG["mcpServers"],
            }
        )
        return assets
    if target == "opencode":
        assets["opencode/plugin.json"] = _json(
            {
                "name": "hieronymus",
                "version": "0.1.0",
                "mcp": "./mcp/hieronymus.mcp.json",
            }
        )
        return assets
    if target == "openclaw":
        assets["openclaw/plugin.json"] = _json(plugin_json)
        return assets

    raise ValueError(f"Unsupported agent plugin target: {target}")

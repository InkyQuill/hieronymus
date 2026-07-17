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

SOURCE_ROLE_TEXT = """`source_role` is a freeform provenance label. It does not determine a
crystal type, confidence, or priority: dreaming chooses those from the evidence, text,
`source_credibility`, and `rule_intent`. Omit it for an ordinary agent note (it defaults to
`agent`), or use a useful label such as `user`, `mentor`, `reviewer`, `source-text`, or `system`."""

BOOTSTRAP_SKILL = f"""---
name: hieronymus-bootstrap
description: Use at the start of a project session with Hieronymus skills or MCP tools.
---

# Hieronymus Bootstrap

Use this skill first. It explains the installed workflow and prevents malformed memory writes.

## Skill map

| Need | Skill |
| --- | --- |
| Retrieve relevant memory | `hieronymus-recall` |
| Read sources into RAG and conclusions into memory | `hieronymus-read` |
| Study or import material | `hieronymus-learn` |
| Preserve a direct correction | `hieronymus-remember` |
| Translate with terminology boundaries | `hieronymus-translate` |
| Review with expert observations | `hieronymus-review` |
| Coordinate session, recall, feedback, and dreaming | `hieronymus-orchestrate` |

## MCP memory contract

Start a session before writing short-term memory. Use `hieronymus_short_term_add` for one block
or `hieronymus_short_term_add_batch` for up to 500 independently valid blocks. Every item needs
`kind` and `text`; `source_role` is optional.

{SOURCE_ROLE_TEXT}

For example, use `source_role="agent"` for an ordinary note and `source_role="user"` for a
direct correction. Any non-empty provenance label is accepted, including `source_role="system"`.

## Report observed problems

Whenever you notice a Hieronymus bug, missing capability, bad recall, rejected valid input,
ambiguous skill instruction, or confusing MCP response, append it to `./hiero_report.md` in the
current project. Record the date, workflow or tool, concise reproduction/context, observed result,
expected result, and relevant IDs or non-secret evidence. Keep working unless the problem blocks
the user. Never put API keys, tokens, or private source text in the report.

{BOUNDARY_TEXT}
"""

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
The agent does the judgment: split material into small observed facts, attach source credibility,
language tags, story scopes, and semantic tags, then call `hieronymus_short_term_add`. source_role
is optional provenance metadata; it does not classify the resulting crystal.

MCP tools are storage and retrieval primitives, not judgment engines. There is no supported Learn
judgment MCP tool; use this skill workflow plus `hieronymus_short_term_add`. Do not promote strict
terminology directly; dreaming can produce crystals, lessons, erudition, and proposals later.

{BOUNDARY_TEXT}
"""

READ_SKILL = f"""---
name: hieronymus-read
description: >
  Read source material into RAG and preserve concise agent conclusions in short-term memory.
---

# Hieronymus Read

Use for reading files, lookup, summaries, or temporary understanding. First import each source file
into project RAG with `hieronymus_rag_import`: RAG retains the source material itself for later
retrieval.

Do not copy file text or long extracts into short-term memory. Instead, after reading, record the
agent's own conclusions with `hieronymus_short_term_add_batch`: learned terminology, concepts,
important facts, implications, uncertainties, and connections to the current work. Each
short-term memory block must contain 1–6 sentences. Create as many separate blocks as necessary
to cover every important term, concept, and detail. The size limit applies to each block,
never to the total set.
Accumulate up to 500 validated blocks per request. Continue making batches until every important
detail is covered; a book commonly needs hundreds of conclusion blocks.

MCP tools are storage and retrieval primitives, not judgment engines. There is no supported Read
judgment MCP tool; use this skill workflow plus `hieronymus_short_term_add_batch`.

source_role is optional provenance metadata and does not classify the resulting crystal.

RAG stores the direct source; short-term memory stores the agent's indirect understanding of it.

{BOUNDARY_TEXT}
"""

REMEMBER_SKILL = f"""---
name: hieronymus-remember
description: Record user corrections as high-credibility short-term memory.
---

# Hieronymus Remember

Use when the user corrects terminology, style, facts, or workflow rules. The agent does the
judgment: turn the correction into a short short-term memory, preserve scope and tags, and call
`hieronymus_short_term_add`.

For high-credibility user rules, phrase the memory as `User told me to ...`, use source_role `user`,
kind `correction`, source_credibility `user_rule`, and a specific rule_intent when known.

MCP tools are storage and retrieval primitives, not judgment engines. Do not create or promote rule
crystals manually; dreaming handles crystallization later.

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
issues, contradictions, and correction patterns as short-term memories. Use an optional
source_role such as `reviewer` when the provenance will help later audit.
"""

ORCHESTRATE_SKILL = f"""---
name: hieronymus-orchestrate
description: Coordinate Hieronymus task sessions, recall, validation, feedback, and dreaming.
---

# Hieronymus Orchestrate

{BOUNDARY_TEXT}
Create a task session, recall before work, collect short-term memories, record feedback events, and
trigger or defer dreaming based on configuration or user instruction. source_role is optional
provenance metadata; it never decides how dreaming categorizes the memory.
"""


def _json(payload: object) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def asset_map() -> dict[str, str]:
    return {
        "skills/hieronymus-bootstrap/SKILL.md": BOOTSTRAP_SKILL,
        "skills/hieronymus-recall/SKILL.md": RECALL_SKILL,
        "skills/hieronymus-learn/SKILL.md": LEARN_SKILL,
        "skills/hieronymus-read/SKILL.md": READ_SKILL,
        "skills/hieronymus-remember/SKILL.md": REMEMBER_SKILL,
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

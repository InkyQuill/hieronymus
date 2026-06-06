# Hieronymus Agent Workflows

Hieronymus agent integrations package skills, MCP configuration, and host-specific plugin manifests
for local coding agents. The generated assets teach agents to recall memory, distinguish strict terms
from advisory memory, write short-term observations, learn material deliberately, and read material
casually.

This task only defines the asset bundle. Installer behavior, host config patching, backups,
availability checks, and idempotent install status are handled by later agent integration work.

## Integrations

The common bundle includes:

- Hieronymus workflow skills under `skills/`.
- MCP config under `mcp/hieronymus.mcp.json`.
- Codex hook config under `hooks/hooks.codex.json`.

`render_agent_plugin_assets(target)` adds one target-specific manifest:

- `codex`: `.codex-plugin/plugin.json`, including the Codex hooks entry.
- `claude`: `.claude-plugin/plugin.json`.
- `gemini`: `gemini-extension.json`.
- `opencode`: `opencode/plugin.json`.

The MCP config invokes `hieronymus-mcp`. Codex hooks currently invoke
`python -m hieronymus.agent_hooks session-start` and
`python -m hieronymus.agent_hooks session-end`; the hook module is implemented in a later task.

## Learn vs Read

`learn` is deliberate ingestion. It stores source blocks as short-term memories with provenance so
later dreaming can distill crystals, lessons, erudition, and terminology proposals.

`read` is temporary inspection. It extracts useful concepts and terms for the current task without
committing the whole source by default. Agents should use `read` for casual lookup, summaries, and
one-off context unless the user asks Hieronymus to learn the material.

## Strict vs Advisory

Strict concept contracts are mandatory. Approved termbase entries and strict validation findings take
priority over fuzzy recall and stylistic memory.

Crystals and lessons are advisory. They can influence translation and review choices, but they must
not silently override an approved concept contract.

Agents must not approve terminology proposals themselves. They may record proposals, uncertainty,
conflicts, and supporting evidence, then leave approval to the human workflow.

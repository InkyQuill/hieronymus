# Hieronymus Agent Workflows

Hieronymus agent integrations package skills, MCP configuration, and host-specific plugin manifests
for local coding agents. The generated assets teach agents to recall memory, distinguish active rule
crystals from advisory memory, write short-term observations, learn material deliberately, and read
material casually.

Installers write the asset bundle under the global Hieronymus config root, patch host config files
with backups, and report installed status only when both assets and host config entries are present.

## Integrations

The common bundle includes:

- Hieronymus workflow skills under `skills/`.
- MCP config under `mcp/hieronymus.mcp.json`.
- Codex hook config under `hooks/hooks.codex.json`.

`render_agent_plugin_assets(target)` adds one target-specific manifest:

- `codex`: `.codex-plugin/plugin.json`, plus a root `.mcp.json` for Codex plugin validation.
- `claude`: `.claude-plugin/plugin.json`.
- `gemini`: `gemini-extension.json`.
- `opencode`: `opencode/plugin.json`.
- `openclaw`: `openclaw/plugin.json`.

The MCP config invokes `hieronymus-mcp`. Codex hooks currently invoke
`python -m hieronymus.agent_hooks session-start` and
`python -m hieronymus.agent_hooks session-end`; the hook asset is packaged separately for later
installer wiring and is not referenced from the Codex plugin manifest.

## Learn vs Read

`learn` is deliberate ingestion. The agent splits source material into observed facts or compact
blocks, records source credibility, language tags, story scopes, and semantic tags, then stores the
result as short-term memories. Later dreaming can distill crystals, lessons, erudition, and
legacy compatibility proposals.

Legacy `remember` and memory-add integrations are compatibility wrappers around the same short-term
queue. They preserve the old searchable entry shape for callers, but the stored memory remains
short-term until dreaming converts, supersedes, or discards it.

Legacy memory search keeps raw short-term IDs when they are unambiguous. If a long-term crystal and
short-term memory would otherwise share the same legacy `id`, the crystal is returned with a negative
compatibility ID. Vague concept suggestions in proposal lists also use negative compatibility IDs;
only positive proposal IDs are actionable legacy compatibility proposals.

`read` is temporary inspection. The agent summarizes source text into small short-term extracts only
when an extract is useful for the current task. Agents should use `read` for casual lookup,
summaries, and one-off context unless the user asks Hieronymus to learn the material.

Read, Learn, and Remember are agent judgment workflows. MCP tools are storage and retrieval
primitives, not judgment engines. Read and Learn are no longer exposed as judgment-heavy MCP tools;
agents should use the skill workflows plus `hieronymus_short_term_add`.

`remember` records corrections as short-term memory. For high-credibility user rules, phrase the
memory as "User told me to ...", use user-rule credibility, and let dreaming crystallize it later.

## Rule Crystals vs Advisory

Active rule crystals are mandatory. Legacy termbase contract and validation wrappers source their
requirements from active rule crystals, and those rules take priority over fuzzy recall and stylistic
memory.

Crystals and lessons are advisory. They can influence translation and review choices, but they must
not silently override an active rule crystal.

Agents must not approve legacy terminology proposals themselves. They may record proposals,
uncertainty, conflicts, and supporting evidence, then leave approval to the human workflow.

## Installing Agent Integrations

Use `hiero install --json` or `hiero install list` to see detected agent hosts and whether the
Hieronymus plugin is installed.

Use `hiero install codex --dry-run` to inspect planned changes. Use `hiero install codex` to write
plugin assets and patch the host config with backups under `~/.config/hieronymus/backups`.

Supported install targets in this pass:

- `claude`
- `codex`
- `openclaw`
- `opencode`
- `gemini`

Reserved detectable targets:

- `pi`
- `hermes`

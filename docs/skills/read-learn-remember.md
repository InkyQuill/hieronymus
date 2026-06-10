# Read, Learn, And Remember Skills

Read, Learn, and Remember are agent skills. MCP tools are storage and retrieval primitives, not
judgment engines. The agent decides what is worth recording, how credible it is, and which tags or
scopes apply; Hieronymus stores the resulting short-term memories and later dreaming decides whether
they become long-term crystals.

## Read

Use Read for casual inspection, lookup, and temporary understanding. Summarize source text into small
short-term extracts only when an extract is useful for the current task. Do not store the whole source
by default.

Preferred storage primitive:

```text
hieronymus_short_term_add
```

Use concise text, source references, and relevant language tags, story scopes, and semantic tags.
There is no supported Read MCP judgment tool; use the skill workflow plus `hieronymus_short_term_add`.

## Learn

Use Learn when the user asks the agent to absorb, study, ingest, import, or learn material. Split the
source into observed facts or compact blocks, record source credibility, and attach language tags,
story scopes, and semantic tags before writing short-term memory.

Learn must not create long-term crystals directly. Dreaming is the path from learned short-term
memory to lessons, erudition, rule crystals, and terminology proposals. There is no supported Learn
MCP judgment tool; use the skill workflow plus `hieronymus_short_term_add`.

## Remember

Use Remember for corrections from the user. Record corrections as short-term memories, not direct
rule promotions.

For high-credibility user rules, phrase the memory as:

```text
User told me to ...
```

Use `source_role="user"`, `kind="correction"`, `source_credibility="user_rule"`, and a specific
`rule_intent` when known. Keep the memory short, scoped, and tagged so dreaming can crystallize it
later without silently overriding approved termbase entries.

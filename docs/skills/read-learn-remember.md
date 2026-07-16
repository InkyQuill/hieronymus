# Read, Learn, And Remember Skills

Read, Learn, and Remember are agent skills. MCP tools are storage and retrieval primitives, not
judgment engines. The agent decides what is worth recording, how credible it is, and which tags or
scopes apply; Hieronymus stores the resulting short-term memories and later dreaming decides whether
they become long-term crystals.

## Read

Use Read for file inspection, lookup, and temporary understanding. Import each source file into RAG
so direct source content remains retrievable. Do not store source text or long extracts in
short-term memory. Instead, record the agent's own conclusions: terms, concepts, significant facts,
implications, uncertainties, and useful connections.

Each short-term-memory block contains 1–6 sentences. Create as many separate blocks as necessary to
cover every important term, concept, and detail; the limit applies to a block, not to the total
amount remembered. RAG stores the direct source, while short-term memory stores indirect
understanding of it.

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
memory to lessons, erudition, concept/facet updates, and rule crystals. There is no supported Learn
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
later without silently overriding active rule crystals.

---
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

`source_role` is a freeform provenance label. It does not determine a
crystal type, confidence, or priority: dreaming chooses those from the evidence, text,
`source_credibility`, and `rule_intent`. Omit it for an ordinary agent note (it defaults to
`agent`), or use a useful label such as `user`, `mentor`, `reviewer`, `source-text`, or `system`.

For example, use `source_role="agent"` for an ordinary note and `source_role="user"` for a
direct correction. Any non-empty provenance label is accepted, including `source_role="system"`.

## Report observed problems

Whenever you notice a Hieronymus bug, missing capability, bad recall, rejected valid input,
ambiguous skill instruction, or confusing MCP response, append it to `./hiero_report.md` in the
current project. Record the date, workflow or tool, concise reproduction/context, observed result,
expected result, and relevant IDs or non-secret evidence. Keep working unless the problem blocks
the user. Never put API keys, tokens, or private source text in the report.

Strict concept contracts are mandatory. Crystals and lessons are advisory.
Do not approve terminology proposals yourself; record proposals for human approval instead.

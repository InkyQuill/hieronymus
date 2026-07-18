---
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

Strict concept contracts are mandatory. Crystals and lessons are advisory.
Do not approve terminology proposals yourself; record proposals for human approval instead.

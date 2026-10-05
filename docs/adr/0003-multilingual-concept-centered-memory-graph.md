# 0003 — Multilingual concept identity

Status: accepted; structured terminology storage superseded by ADR 0011.
Consolidated 2026-10-05. ADR 0010 owns schema/import mechanics.

## Context

Rigid translation-direction columns cannot describe a multilingual work, and
strings alone cannot distinguish one character's aliases from another character.
A shared durable identity needs language- and context-specific descriptions.

## Decision

Series are language-neutral containers with registered languages. Concepts are
durable identity anchors; facets hold scoped names, renderings, descriptions and
notes. Renaming preserves concept identity. Freeform semantic tags describe meaning
and story labels can boost relevance; neither substitutes for resolved identity,
chronology or applicability.

Store compact searchable memory primarily in English while retaining exact source
forms, renderings and quotations in their languages. Agent skills judge what to
read/learn/remember; MCP supplies explicit storage/retrieval primitives. Dreaming
consolidates evidence in bounded phases with immutable audit.

Terminology authority lives in structured `term_rules`/`term_rule_forms` under
[ADR 0011](0011-deterministic-terminology-and-graded-memory.md). Rule crystals are
searchable projections, not a competing authority. The original crystal-only
storage decision is historical.

## Consequences

Languages and scoped forms can evolve without conflating identity or losing aliases.
Migration preserves supported old records and provenance; current truth/viewpoint
checks remain distinct from relevance. See [business logic](../business-logic.md)
for the reader-facing vocabulary and workflow.

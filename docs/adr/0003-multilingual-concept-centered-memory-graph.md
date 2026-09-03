# Transition to Multilingual, Concept-Centered Memory Graph

> **Status note (2026-09-03):** this ADR describes the Python-era data-model
> migration. It remains the historical definition of the concept/facet/crystal
> data model the Rust database upgrade must preserve, but normative schema
> ownership now sits with [ADR 0010](0010-data-locations-schema-ownership-and-upgrade.md)
> (import/upgrade boundary) and [ADR 0011](0011-deterministic-terminology-and-graded-memory.md)
> (structured `term_rules`/`term_rule_forms` authority). Decision 4
> (rule-crystal-only terminology storage) is superseded by ADR 0011.

## Context
The initial Hieronymus memory design carried rigid translation-direction boundaries (source-target language pairs), had no first-class representation of concepts, and treated deterministic terminology validation as a separate strict termbase system. Additionally, dreaming was restricted to a single crystallization pass.

## Decision
We refactored the memory model to support an language-neutral, concept-centered, and multi-phase dreaming cycle:
1. **Language-Neutral Series & Tags**: Series are language-neutral, associating language tags, story scopes (relevance boosts), and semantic tags via side-tables rather than hardcoded columns.
2. **First-Class Concepts**: Concepts are represented as durable identity anchors with distinct lifecycles (`candidate`, `established`, `archived`, `merged`).
3. **Concept Facets**: Multilingual renderings, names, descriptions, and notes are modeled as scoped facets linked to concepts.
4. **Rule Crystals**: Separate strict-term validation is replaced by "rule" crystals linked to concepts, allowing contextual disambiguation and warnings for ambiguous occurrences.
5. **Multi-Phase Dreaming**: Dreaming runs as a background process containing distinct phase workflows (crystallization, relation discovery, reinforcement/compaction) operating on a bounded affected memory set and generating immutable audit records.
6. **English-First Memory**: Standard memory prose is written in English to keep recall searchable across translation directions. Non-English values are preserved specifically in facets, quotations, and metadata.

## Consequences
- The database schema is upgraded idempotently through a global SQLite migration mapping legacy pair columns and terms into concepts, facets, and rule crystals.
- Recall queries search both short-term memory and long-term crystals, returning a unified ranked list.
- MCP tools expose low-level storage primitives, while agent judgment workflows (Read/Learn/Remember) are moved to client-side agent skills.

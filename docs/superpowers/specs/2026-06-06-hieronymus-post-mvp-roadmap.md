# Hieronymus Post-MVP Roadmap

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

The MVP proved the core shape: strict termbase validation, fuzzy memory search, CLI commands,
and MCP tools. Post-MVP turns Hieronymus from a per-series translation-memory utility into a
global translation-memory system with scoped strict concepts, human-like memory housekeeping,
agent workflows, and later a management TUI.

## Roadmap Order

1. **Memory Housekeeping and Dreaming**
   - Replace per-series database isolation with a global scoped store.
   - Add task sessions, short-term memory, activation traces, long-term crystals, reinforcement,
     decay, dream cycles, and LLM-generated crystal proposals.
   - Redesign strict termbase entries as scoped concepts/renderings.

2. **Agent Skills and Workflows**
   - Define how translators, reviewers, mentors, and orchestrators use Hieronymus.
   - Specify recall, short-term memory writes, influence citation, concept proposals, validation,
     and dream triggering.
   - Add hooks and skills for coding agents such as Claude, Codex, and opencode so they can call
     the MCP for project memories and translation knowledge.
   - Keep agents productive before investing in a full management UI.

3. **Management TUI**
   - Build the human control surface after real agent workflows produce memories, crystals,
     lessons, and proposals.
   - Manage entries, inspect provenance, reinforce/decay knowledge, review dream outputs, and
     approve strict concept/rendering changes.

## Architectural Direction

Hieronymus should become one global MCP-backed memory system. Storage remains local-first under
`HIERONYMUS_DATA_ROOT`, but knowledge objects carry explicit scope instead of living in separate
series databases.

Strict translation knowledge and fuzzy advisory knowledge stay separate:

- **Strict concepts and renderings** are deterministic and enforceable.
- **Crystals** are fuzzy long-term memories and can guide agents, but they do not become mandatory
  unless they produce an accepted strict concept/rendering proposal.

## Global Scoped Store

The post-MVP store should be one SQLite database under the configured data root. The current MVP has
no production memories, so this is a schema rewrite rather than a difficult data migration.

Knowledge objects should carry scope metadata:

- global
- series
- volume
- chapter
- source language
- target language
- task/session
- tags or domains

Retrieval starts from the current context and can include global lessons or global concept groups
only when their scope applies.

## Strict Concepts

A strict concept represents an entity, term, phrase, style convention, or cultural idea. Concepts
are series-local by default because the same source form can be rendered differently across works
or language pairs.

Some concepts can join explicit global concept groups, especially cultural terms, genre/trope
concepts, honorific strategies, and reader-facing explanation conventions. Cross-series linking is
explicit; similar names or terms are not merged silently.

Renderings are attached to concepts and are scoped by:

- source language and target language, such as `ja -> en`, `ja -> ru`, or `en -> ru`
- series, volume, or chapter
- canonical rendering
- approved variants
- forbidden variants
- evidence and notes

Validation uses the active translation context. Series-local decisions win unless an explicit
scoped override exists.

## Crystals

A crystal is any long-term fuzzy memory entry. It is a compact 1-3 sentence statement distilled
from short-term memory. Crystal types include:

- **Lesson:** reusable behavioral rule.
- **Concept crystal:** advisory statement about an entity or idea. This is separate from a strict
  translation concept.
- **Erudition:** contextual knowledge, often linked to a concept.

Crystals have strength, confidence, provenance, scope, status, and cycle metadata. They guide recall
and prompts but remain advisory unless promoted into strict concepts/renderings.

## Spec Set

This roadmap is supported by three follow-up specs:

- `2026-06-06-hieronymus-memory-dreaming.md`
- `2026-06-06-hieronymus-agent-workflows.md`
- `2026-06-06-hieronymus-management-tui.md`

## Non-Goals

- Do not build the TUI before agent workflows can generate real memories and dream outputs.
- Do not let LLM-generated output silently become strict termbase truth.
- Do not merge series-local concepts into global concepts without explicit linking or promotion.
- Do not use wall-clock aging for memory decay; use cycle-based accounting.

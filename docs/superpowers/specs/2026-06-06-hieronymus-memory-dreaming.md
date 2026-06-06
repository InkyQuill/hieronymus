# Hieronymus Memory Housekeeping and Dreaming

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

Memory housekeeping and dreaming turn Hieronymus into a global translation-memory brain. The system
should recall useful long-term knowledge into a task workspace, collect short-term evidence during
translation and review, and periodically crystallize that evidence into long-term advisory memory,
lessons, concepts, and strict term/rendering proposals.

## Core Model

Hieronymus has two fuzzy memory layers:

- **Long-term memory:** durable crystals with strength, confidence, scope, status, provenance, and
  cycle metadata.
- **Short-term memory:** task/session activation surface containing recalled crystals, translations,
  audits, validation findings, user corrections, mentor observations, and derivations.

Short-term memories are not scored immediately. Dreaming uses them as evidence to update long-term
memory.

## Crystals

A **crystal** is any long-term memory entry. It is a compact 1-3 sentence statement derived from one
or more short-term memories. One large short-term memory can produce several crystals.

Crystal types:

- **Lesson:** a reusable rule for agent behavior.
  - Example: "If a Japanese cultural term may be unfamiliar to average English readers, consider
    defining it in narration or a footnote."
- **Concept crystal:** an advisory statement about an entity or idea. This is separate from a strict
  translation concept.
  - Example: "Gantz is a martial arts user."
- **Erudition:** contextual knowledge linked to a concept.
  - Example: "Gantz does not wear much armor and relies on speed and evasion."

All crystal types are advisory by default. They can influence prompts and later soft validation, but
only strict concepts/renderings create hard validation failures.

## Scores

Use at least two scores:

- **Strength:** how likely the crystal is to be recalled and treated as useful.
- **Confidence:** how reliable the content appears to be.

These should not be collapsed into one number. A narrow user preference can have high confidence but
limited recall strength. A broad style lesson can have high strength but moderate confidence.

## Recall

Recall performs weighted search over long-term crystals for the current context:

- series
- volume/chapter
- source language
- target language
- task type
- query text
- tags/domains

Recall copies selected long-term crystals into the task workspace as short-term activations. Each
activation records:

- source crystal id
- recall query/context
- reason or rank metadata
- cycle id
- task/session id

Recall does not increase strength or confidence by itself. It only prevents immediate decay in the
next dream cycle.

## Short-Term Memory

Short-term memory is scoped to a task/session, not global. A task/session can represent a chapter
translation, review pass, terminology review, import pass, or user correction session.

Short-term memory source roles:

- **mundane:** translator outputs, ordinary observations, local notes.
- **mentor:** audit/review observations, higher-confidence corrections.
- **user:** explicit user decisions or preferences, highest-confidence fuzzy input.
- **system:** validation findings, tool outputs, recall traces.

These roles affect how strongly dreaming weighs the evidence, but they do not directly create strict
truth.

## Feedback Events

Use events to record what happened to recalled or generated knowledge:

- recalled
- cited
- used_in_translation
- passed_review
- caused_correction
- contradicted_by_user
- confirmed_by_user
- deleted_by_user
- superseded

Explicit user feedback applies immediately to the affected crystal where possible. Passive workflow
signals are recorded and applied during dreaming.

## Dream Cycles

Dreaming is explicit and configurable:

- manual "think/dream" action
- after N completed task sessions
- after short-term memory reaches a configured threshold

Dreaming is cycle-based, not wall-clock based. Decay is applied per cycle.

During a dream cycle, the system:

1. Loads completed task/session workspaces and activation traces.
2. Sends selected short-term memories to an LLM such as Gemini.
3. Receives candidate crystals, lessons, crystal links, and strict concept/rendering proposals.
4. Validates output against schema and scope rules.
5. Inserts or updates long-term crystals with provenance.
6. Reinforces crystals with strong positive evidence.
7. Decays long-term crystals that were not activated or reinforced during the cycle window.
8. Archives noisy or consumed short-term fragments.

## LLM Boundaries

The LLM is used for crystallization, not authority.

It may create active advisory crystals with confidence and provenance. It may also propose strict
concepts, renderings, aliases, or links. Strict proposals remain non-mandatory until accepted by the
user or an explicit workflow.

LLM output must be structured and validated. Invalid output is stored as a failed dream artifact for
inspection, not applied silently.

## Lessons

Lessons are structured crystals plus rendered prompt text.

Suggested fields:

- title
- canonical instruction text
- applies_when
- avoid
- examples
- scope
- source crystal ids
- strength
- confidence
- status

Lessons start as prompt guidance. If reinforced, they may become soft validation heuristics. They do
not create hard failures.

Lessons can be:

- series-local
- global candidates
- active global lessons

Dreaming can create global lesson candidates from repeated cross-series evidence. Activation as a
global lesson requires user confirmation or strong repeated successful use.

## Strict Concepts and Proposals

Dreaming may propose:

- new strict concepts
- new source forms
- canonical renderings for a language pair
- approved variants
- forbidden variants
- crystal links
- strict concept links
- scope overrides

These remain proposals. They do not become mandatory contract entries without explicit acceptance.

## Decay

Long-term crystals decay when they are not recalled, used, confirmed, or reinforced during the cycle
window. Recall alone protects from immediate decay but does not strengthen the crystal.

Decay should reduce strength before confidence. A memory can be reliable but rarely useful.

## Open Implementation Decisions

- Exact score ranges and update formulas.
- Whether soft validation heuristics live in crystal tables or separate rule tables.
- How large a short-term batch can be before dreaming chunks it.
- How to display dream provenance before the TUI exists.

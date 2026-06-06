# Hieronymus Agent Workflows

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

Agent workflows define how translators, reviewers, mentors, and orchestrators use Hieronymus after
the memory/dreaming model exists. Agents should use Hieronymus as a memory and consistency layer,
not as an autonomous editor.

## Roles

### Orchestrator

Creates task sessions for translation and review units. It supplies:

- series
- volume/chapter
- source language
- target language
- task type
- source and translated text or file references

The orchestrator calls recall before translation/review, passes strict contracts and relevant
crystals to agents, records task outcomes, and decides whether to trigger dreaming.

### Translator

Uses strict concept contracts as mandatory constraints. Uses crystals and lessons as advisory
context.

Translator agents should:

- obey strict renderings and forbidden variants
- cite influential crystals when practical
- write short-term memories for discoveries, uncertainty, and local observations
- propose strict concepts/renderings instead of silently creating rules

### Reviewer

Checks meaning, completeness, style, and strict terminology. Reviewer observations become short-term
memories, usually stronger than ordinary translator observations.

### Mentor/Audit Agent

Writes higher-confidence short-term memories:

- recurring mistakes
- style observations
- contradictions
- validated inferences
- correction patterns

Mentor memories weigh more during dreaming than mundane memories.

### User

User decisions become high-confidence short-term memories immediately. User feedback reinforces,
decays, redirects, or supersedes crystals. It does not necessarily make a fuzzy crystal permanently
true.

## Workflow

1. Orchestrator creates task session.
2. Recall loads relevant long-term crystals into short-term activation.
3. Strict concept contract is generated for the raw text and language pair.
4. Translator works with strict contract plus advisory crystals/lessons.
5. Translator writes short-term memories and proposals as needed.
6. Validation checks hard strict concept/rendering rules.
7. Reviewer/mentor audits translation and writes short-term observations.
8. User corrections, if any, become high-confidence short-term memories.
9. Orchestrator marks task session complete.
10. Dreaming runs manually or after configured thresholds.

## Agent Skill Requirements

The agent skills should teach:

- when to recall
- how to read strict contracts
- how to distinguish mandatory terms from advisory crystals
- how to cite influential crystals
- how to write short-term memories
- how to create concept/rendering proposals
- how to record user decisions
- how to trigger or defer dreaming
- how to treat lessons as guidance, not hard rules unless promoted to soft validation

## Coding Agent Hooks

Hieronymus should provide integration hooks for coding agents such as Claude, Codex, opencode, and
similar local coding assistants. These hooks should make the MCP memory layer easy to use from an
agent session without requiring the agent to know Hieronymus internals.

Hook responsibilities:

- expose the active project, series, source language, and target language context where available
- call recall before translation, review, terminology, or documentation work
- make relevant crystals and strict concepts available to the agent
- record important user corrections as high-confidence short-term memories
- record task outcomes and influence citations for later dreaming
- avoid writing raw source text into long-term memory unless a learning workflow explicitly asks for it

Coding-agent hooks should stay thin. They should call MCP tools and skills; they should not
reimplement storage, scoring, dreaming, or validation.

## Learning and Reading Skills

The agent workflow needs two ingestion modes: thorough learning and casual reading.

### Learn Skill

The `learn` skill commits material into short-term memory for later dreaming. It is used when the
user wants Hieronymus to absorb a document, chapter, review, conversation, style guide, glossary, or
other substantial source.

The skill should:

- split input into coherent blocks
- preserve source references and provenance
- classify blocks by source role, such as mundane, mentor, user, or system
- write short-term memories into the active task/session workspace
- mark the material as eligible for dreaming and crystallization
- avoid making strict concepts/renderings active directly

Dreaming later turns these short-term memories into crystals, lessons, erudition, and strict
concept/rendering proposals.

### Read Skill

The `read` skill inspects material without committing the whole source into memory. It is used for
casual lookup, temporary understanding, or one-off extraction.

The skill should:

- read the material for the current task
- extract useful concepts, source forms, candidate renderings, and uncertainty
- return structured findings to the agent
- optionally create explicit proposals if the user or workflow asks for them
- avoid storing every block as short-term memory by default

The `read` skill can still produce a small number of deliberate short-term observations, but it must
not behave like `learn`. Its default posture is "understand now, do not remember everything."

### Skill Split

Use `learn` when the user says to absorb, remember, study, ingest, import, or learn from a source.
Use `read` when the user asks to inspect, summarize, extract terms, check a file, or understand a
source for the current task only.

## Prompt Contracts

Each role should have a concise prompt contract.

Translator contract:

- strict concepts are mandatory
- crystals are advisory
- do not invent approved terminology
- record uncertainty and discoveries

Reviewer contract:

- check strict validation findings first
- identify whether crystals helped or misled
- write mentor short-term memories for recurring issues

Orchestrator contract:

- create task session
- recall before work
- collect short-term memories
- trigger dreaming based on config/manual instruction

Coding-agent hook contract:

- establish current project and translation context
- call recall before memory-sensitive work
- use `read` for temporary extraction
- use `learn` for material that should feed future dreaming
- do not bypass MCP APIs for memory writes

## Integration With Memory/Dreaming

Agent actions should produce event traces:

- recalled
- cited
- used
- accepted
- corrected
- user confirmed
- user rejected

These traces feed dream cycles. Mere recall protects from decay but does not reinforce strength.

## Non-Goals

- Do not make agents auto-approve strict concept/rendering proposals.
- Do not require the Management TUI before agents can use the service.
- Do not treat lessons as hard validation failures by default.

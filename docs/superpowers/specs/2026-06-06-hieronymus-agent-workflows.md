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


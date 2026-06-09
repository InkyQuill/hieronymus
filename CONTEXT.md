# Hieronymus

Hieronymus is a local-first memory system for literary translation. Its language
centres on how translation observations become durable memory and optional
terminology constraints.

## Language

**Concept**:
A durable multilingual identity anchor that gathers facets, notes, and related
memories around one meaningful translation subject. A concept is not forgotten
through ordinary memory decay, and its identity is usually broader than any one
story scope.
_Avoid_: Proposal, strict term, raw tag

**Concept Facet**:
A language-scoped or story-scoped piece of information attached to a concept,
such as a name, rendering, description, or note.
_Avoid_: Alias, form, translation

**Semantic Tag**:
A freeform label attached to a concept, facet, crystal, or memory to describe
meaningful context such as talent, subskill, location, faction, or ability.
Semantic tags are normally managed by dreaming but remain user-editable.
_Avoid_: Concept, story scope, rule crystal

**Concept Rename**:
An identity-preserving change to a concept's canonical label. Previous labels
remain searchable facets unless explicitly superseded as wrong.
_Avoid_: Merge, split, translation change

**Dream Audit**:
The complete record of what a dreaming cycle inspected, decided, changed, and
why, so memory behavior can be traced and corrected later. Dream audits are
visible from the admin interface and are immutable; corrections append new
events instead of changing prior records. Dream audits include a structured
trace and may include redacted bounded raw provider payloads.
_Avoid_: Debug log, summary, changelog

**Crystal**:
An atomic long-term memory record: one character fact, rendering preference,
style lesson, world rule, uncertainty, or reusable translation observation.
Crystals are normally one or two sentences and are written primarily in English
for searchability, with non-English text preserved as renderings, quotations, or
metadata where relevant.
_Avoid_: Concept, summary, source document

**Story Scope**:
Freeform work-position labels that describe where a memory applies within the
translated material, such as a book, chapter, route, episode, scene, side story,
or story bible. Story scope can apply to concepts, concept facets, and
individual crystals, and normally boosts recall relevance rather than excluding
non-matching memories.
_Avoid_: Source citation, tag, language scope

**Supersession**:
An explicit memory relationship where one crystal replaces another because the
newer or better memory should take precedence.
_Avoid_: Decay, scope conflict, archive

**Short-Term Memory**:
A session-scoped observation captured before dreaming decides whether and how it
should become durable memory. Short-term memory can still be recalled directly
before dreaming processes it, and is normally a small extract of one to six
sentences rather than a raw source chunk. Short-term memory is written primarily
in English unless the non-English text itself is the memory.
_Avoid_: Crystal, note, source text

**Correction Memory**:
A short-term memory that records a user or admin correction for dreaming to
process, often with rule intent, source credibility, linked concepts, language
tags, story scopes, semantic tags, and an admin-correction origin.
_Avoid_: Direct edit, rule promotion, audit event

**Semantic Correction**:
A correction that changes the meaning, applicability, rendering, or rule
behavior of memory. Semantic corrections enter as correction memories so
dreaming can reconcile them with audit.
_Avoid_: Typo fix, metadata edit, direct mutation

**Thought Memory**:
A low-confidence long-term crystal proposed by dreaming when it notices a
potentially useful inference that did not come directly from source material or
user input. Thought memories are recallable by default but clearly marked as
inferred and ranked below comparable source-backed memories.
_Avoid_: Rule crystal, source evidence, hallucination

**Dreaming**:
The process that turns completed short-term memories into long-term crystals,
advisory concept updates, or rule crystals, then reconciles, consolidates,
reinforces, and decays the relevant long-term memory it affected.
_Avoid_: Import, sync, summarization

**Dreaming Phase**:
A distinct step in a dreaming cycle, such as crystallization prompting,
parse/apply, relation discovery, reinforcement and compaction prompting, or
audit recording. Provider-backed phases have editable prompts, while
deterministic phases are controlled by Hieronymus.
_Avoid_: Prompt, provider call, screen

**Dreaming Trigger**:
A configured condition that makes dreaming due, such as elapsed schedule time,
enough pending short-term memories, or an urgent pending-memory threshold.
Dreaming triggers request background dreaming rather than performing the cycle
inside the memory-ingestion workflow.
_Avoid_: Dreaming phase, provider check, manual run

**LLM Model Cache**:
A refreshable temporary cache of model lists discovered from configured LLM
provider profiles. It supports config UI suggestions and doctor checks, but is
not authoritative configuration. Model cache entries are stale after 24 hours.
_Avoid_: Dream config, provider profile, model selection

**Dreaming Backlog Escape**:
A scheduled dreaming rule that lets a small leftover short-term memory batch run
after several consecutive scheduled skips caused by the minimum-memory threshold.
_Avoid_: Manual run, urgent threshold, disabled threshold

**Affected Memory Set**:
The bounded group of short-term memories, concepts, concept facets, and crystals
that a dreaming cycle is allowed to inspect and change.
_Avoid_: Whole store, corpus, database

**Ambient Decay**:
A small decay applied to searched long-term memories that were considered during
dreaming but not used for linking, consolidation, reinforcement, or rule
creation. Ambient decay targets low-confidence unused crystals and reduces
strength before it reduces confidence.
_Avoid_: Supersession, deletion, rejection

**Rule Crystal**:
A crystal that records an explicit rule from an authoritative source and can be
used for deterministic validation in matching translation context. Active rule
crystals do not decay, but they can be explicitly superseded, archived, or
consolidated with other memories.
_Avoid_: Strict contract, termbase entry, suggestion

**Source Credibility**:
The trust assigned to the origin of a memory, such as rumor, source text,
expert explanation, user suggestion, or user rule. Standard credibility labels
are rumor, observation, source text, expert, user suggestion, and user rule.
_Avoid_: Confidence, importance, strictness

**Confidence**:
How likely a memory is to be true or correct, based on evidence consistency and
source credibility.
_Avoid_: Strength, relevance, frequency

**Confidence Penalty**:
A reduction in confidence applied when a memory is useful but has weaker
evidence, incomplete structure, malformed provider output, or other reliability
concerns.
_Avoid_: Decay, rejection, punishment

**Strength**:
How active or useful a memory is in recall, based on recency, reinforcement,
repeated use, and successful matching.
_Avoid_: Confidence, truth, source credibility

**Rule Intent**:
An explicit signal that a memory should be treated as a rule rather than an
ordinary observation, such as "User told me to always render this term this way."
_Avoid_: Confidence, relevance, suggestion

**Recall**:
The retrieval of relevant short-term memories and long-term crystals for the
current translation task. Recall returns a combined working set that identifies
whether each item came from short-term or long-term memory.
_Avoid_: Dreaming, validation, summarization

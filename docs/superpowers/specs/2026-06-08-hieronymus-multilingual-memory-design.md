# Hieronymus Multilingual Memory Design

## Purpose

Hieronymus should behave like a local memory system for literary translation:
small, durable memories accumulate over time, recall returns a useful working set
of those memories, and strict terminology stays deterministic only where a human
has explicitly made it strict.

This design defines the first release toward that model. It also records the
north-star direction: memory is multilingual and concept-centered, not locked to
one source-target pair.

## Current Context

The current implementation already has series registration, task sessions,
short-term memories, crystals, recall, dreaming, strict term validation, concept
proposals, generated agent skills, CLI commands, and MCP tools.

The main gaps are:

- Series currently carry one default source language and one default target
  language.
- MCP does not expose enough setup and management primitives for agents to work
  without CLI fallback.
- `hieronymus_read` and `hieronymus_learn` are MCP tools even though they are
  agent-judgment workflows.
- `hieronymus_memory_add` creates crystals directly, bypassing dreaming.
- Direct memory search loses soft origin/provenance information.
- Strict terminology is pair-shaped, while the desired model is multilingual
  concept memory with optional strict facets.

## Mental Model

### Series

A series is language-neutral. `only-sense-online` means the work itself, not
`ja->en` or `ja->ru`.

The series may contain memories, concepts, and strict contracts tagged with any
combination of languages. Some memories may be Japanese-only, some Russian-only,
some English and Russian, and some language-independent.

### Language Tags

Language tags describe where a memory, concept facet, or strict contract applies.
Examples:

- `ja`
- `en`
- `ru`
- `all`
- `ja,en`
- `en,ru`

Tags are not translation directions. They are facets of memory. A task session
can provide language tags to bias recall and validation, but a memory can exist
with partial language coverage.

### Concepts

A concept is a multilingual memory object. It can hold one or more language
facets and general notes.

Example:

- concept: Yun
- `ja`: `ユン`
- `en`: `Yun`
- `ru`: `Юн`
- `all`: main character, first-person narration, game-system context

Concept facets are advisory by default. They become strict only when a human
approves a specific facet as a contract.

### Strict Contracts

Strict validation is optional and explicit. Hieronymus should not treat every
concept as mandatory terminology.

A strict contract is an approved concept facet or rendering rule. Validation
checks only approved strict contracts that match the requested series and active
language tags/context.

This preserves the rule that fuzzy memory must never silently override strict
terminology, while also preventing ordinary multilingual memory from becoming
too rigid.

### Memories And Crystals

Short-term memories are session-scoped observations. Learned material enters
Hieronymus here first.

Crystals are long-term memory records created by dreaming. They should be atomic
but meaningful: one character fact, one rendering preference, one style lesson,
one world rule, one uncertainty, or one reusable translation observation.

Crystals carry language tags and optional soft origin hints. Origin hints are
memory-like, not archival citations. Good examples are:

- probably from volume 2
- learned from the story bible
- noticed during chapter 14 review

Hieronymus should not require precise source references for ordinary memories.
Strict contract approval may keep explicit rationale because it creates
mandatory behavior.

## MCP And Agent Skills

MCP should expose primitives. Agent skills should contain judgment.

### MCP Primitives

The first release should add or adjust MCP operations for:

- creating or updating a language-neutral series;
- listing series;
- starting and completing sessions with optional language tags;
- adding short-term memories with language tags and soft origin hints;
- recalling crystals by query with optional language tags;
- running dreaming;
- recording feedback;
- listing, proposing, approving, and rejecting concept facets or strict contract
  candidates;
- validating text only against approved strict contracts.

The MCP surface should allow an agent to initialize an empty store without using
the CLI.

### Read And Learn

Read and Learn are skills, not MCP judgment tools.

Read instructs the agent to inspect material for the current task. It should not
bulk-store the source. It may add a short-term memory only when the agent finds a
useful observation or the user asks it to remember something.

Learn instructs the agent to commit material into memory. It should split the
material into small blocks, infer or ask for useful language tags, add soft
origin hints, and write short-term memories. Learn must not create crystals.

Dreaming is the only ordinary path from learned material to crystals.

### Legacy Direct Memory Add

`hieronymus_memory_add` should stop creating crystals directly. The first
release should either deprecate it or redirect it into the short-term/session
workflow.

If compatibility requires keeping the tool name temporarily, its behavior should
be documented as short-term ingestion, not long-term crystal creation.

## Recall

Recall returns a flat ranked working set of crystals. It should not synthesize a
canonical summary or store reconstructed summaries.

The default limit should be around 10 to 15 results. Each result should expose
facets useful to agents:

- crystal id;
- title;
- text;
- crystal type;
- language tags;
- strength;
- confidence;
- soft origin hint;
- related crystal ids where available;
- rank, score, and reason.

The agent uses this working set as context and may group or summarize it in its
own reasoning.

## Dreaming

Dreaming processes completed sessions and creates long-term crystals or pending
concept/strict-contract candidates.

Dream output should prefer atomic meaningful memories. It may create links
between related crystals when useful. It may add or update concept facets. It
may propose strict contracts, but those remain pending until approved.

The deterministic fallback should follow the same data model. It should turn
short-term memory into small tagged crystals with soft origin hints and must not
bypass concept or strictness rules.

## Migration And Compatibility

Existing databases should continue to work.

Migration should convert existing pair-shaped data as follows:

- Existing series remain the same slug and title.
- Existing default source and target languages become language tags associated
  with existing sessions, terms, and crystals.
- Existing strict terms become concepts with one or more facets plus an approved
  strict contract for the old target rendering.
- Existing crystals keep their current series and language fields converted into
  tags.
- Existing memories get empty soft origin hints unless the old record already
  contains origin-like text.

CLI compatibility can remain through shortcuts. For example,
`init-series --source-language ja --target-language en` can create a
language-neutral series and initial language tags for compatibility, but the
underlying model should not be direction-first.

Documentation should stop presenting Read and Learn as MCP tools and describe
them as agent skills over primitive MCP operations.

## First Release Scope

Include:

- language-neutral series plus MCP `series_init` and `series_list`;
- language tags on sessions, short-term memories, crystals, recall results,
  concepts, and strict contract candidates;
- concept records with multilingual facets;
- optional strictness on approved concept facets;
- Read and Learn as generated agent skills, not MCP judgment tools;
- dreaming-only crystallization for learned material;
- deprecated or redirected direct memory add;
- flat ranked recall with 10 to 15 crystal facets by default;
- migration from current pair fields into language tags;
- docs and tests for the revised model.

Exclude from this release:

- source document registry;
- stored summary or reconstruction objects;
- complex graph traversal recall;
- broad automatic markdown crawling;
- full admin UI polish beyond the review operations needed for concepts and
  strict candidates.

## Testing Requirements

Tests should cover:

- one series containing `ja`, `en`, and `ru` tagged information;
- concepts with partial language coverage;
- the same concept having Japanese, English, and Russian facets;
- advisory concept facets not triggering validation;
- explicit strict contract facets triggering validation;
- validation filtered by active task language tags;
- Learn writing short-term memories only;
- direct memory add no longer creating crystals;
- dreaming converting completed short-term memories into tagged crystals;
- recall returning a flat 10 to 15 item result shape with facets;
- MCP tool listing matching the primitive boundary;
- migration preserving existing series, terms, crystals, and sessions.

## Open Implementation Notes

The implementation plan should decide the exact schema names and migration
sequence. The design intent is stable: tags and optional strict facets replace
direction-first terminology, while dreaming remains the ordinary path into
long-term crystals.

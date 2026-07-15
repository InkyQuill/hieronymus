# Hieronymus Product Vision

Status: Current product vision, amended by
[ADR 0007](0007-provider-catalog-and-workflow-assignments.md) for provider
catalog ownership. `provider.conf` now owns provider profiles and API keys;
`dream.conf` owns workflow assignments, prompts, trigger settings, and caps.

## Context

Hieronymus is a local-first translation memory system for long-form literary
translation. It serves coding and translation agents through MCP, local CLI
commands, and a terminal management interface. The original plans and specs
described a broader product direction than any single implementation task:
multilingual memory, concept-centered recall, autonomous dreaming, rule-crystal
validation, agent skill workflows, and a rich local TUI.

Execution plans are temporary. This ADR preserves the durable product vision so
old resolved plans can be removed without losing architectural intent.

## Decision

Hieronymus is a local-first memory application with five product surfaces:

1. **Primitive MCP tools** for agent integrations.
2. **Generated agent skills** for judgment-heavy Read, Learn, and Remember
   workflows.
3. **CLI commands** for local setup, debugging, import/export, validation,
   recall, dreaming, service management, and install/update flows.
4. **A React/OpenTUI terminal application** for configuration and memory
   administration.
5. **A background local service** for health, status, automation, and future
   daemon-backed command/adaptor paths.

Python remains the authoritative backend for storage, validation, domain logic,
dreaming, MCP behavior, settings, migrations, rule-crystal validation, and
service lifecycle. The TypeScript frontend is a local client. It renders UI,
tracks local interaction state, validates response shapes, and calls Python
through a typed JSON-RPC boundary. It must not write SQLite, parse human CLI
output, or duplicate domain mutation logic.

## Product Model

### Local-First Boundaries

Hieronymus stores application state on the local machine. Source code lives in
the project checkout; translation projects and runtime databases live elsewhere.
The app must not write tool source code into translation workspaces.

SQLite with FTS5 remains the primary storage engine. Join tables model graph
relationships first. A graph database should be reconsidered only if recall or
dreaming require deep multi-hop traversal as a primary feature.

### Series

A series is language-neutral. A slug such as `only-sense-online` identifies the
work itself, not a translation direction such as `ja->en` or `ja->ru`.

Series, sessions, short-term memories, crystals, concepts, and facets can carry
language tags such as `ja`, `en`, `ru`, or `all`. Tags describe where memory
applies; they are not translation directions. Legacy source/target language
fields may remain as compatibility inputs, but new logic treats language tags
as the canonical shape.

### Story Scopes And Semantic Tags

Story scopes are freeform work-position labels such as `book-5`,
`book-5/chapter-5`, `route-yun`, `episode-12`, or `story-bible`. They boost
recall relevance but normally do not hard-filter older memory. Supersession is
explicit.

Semantic tags are freeform meaning labels such as `talent`, `subskill`,
`faction`, `location`, `ability`, or `inventory-item`. They are not a fixed
taxonomy. They help disambiguate similarly named concepts and can be adjusted
by users or dreaming.

### English-First Memory

Ordinary memory prose is written primarily in English so recall remains
searchable across Japanese source material, Russian notes, and other
translation contexts. Non-English strings are preserved when they are the
remembered subject:

- concept facets and renderings;
- exact source forms;
- short quotations needed for disambiguation;
- semantic tags, story scopes, or metadata where the original string matters.

Dreaming prompts and agent skills should ask for English explanations by
default and must not translate away source forms or target renderings needed as
facets, rule-crystal renderings, or evidence.

### Concepts And Facets

Concepts are durable identity anchors for meaningful translation subjects:
characters, places, abilities, items, factions, titles, style rules, recurring
terms, and other stable entities.

Concepts do not decay like crystals. They may be:

- `candidate`: auto-created from weak or early evidence;
- `established`: promoted when evidence is consistent;
- `archived`: explicitly retired;
- `merged`: redirected into another concept.

Concept facets hold language-scoped or story-scoped concept information:
names, renderings, descriptions, notes, aliases, and canonical labels for a
language/context. Aliases are searchable facets, not a separate domain object.
Renderings become deterministic only through active rule crystals.

### Memories And Crystals

Short-term memories are compact observations attached to sessions or agent
workflows. They are normally one to six sentences, not raw source chunks.
Direct tool/API ingestion hard-rejects extremely oversized memories and warns
for large but acceptable input. Short-term memories remain useful in recall
while dreaming is disabled, misconfigured, delayed, or pending.

Crystals are long-term memory records created or maintained by dreaming or
explicit admin operations. They should be atomic but meaningful: one character
fact, rendering preference, style lesson, world rule, uncertainty, rule
crystal, or reusable translation observation. Most crystals are one or two
sentences and are written in English by default.

Crystals carry language tags, story scopes, semantic tags, source credibility,
strength, confidence, optional soft origin hints, concept links, and optional
malformed-output penalties. Soft origin hints are memory-like, not archival
citations.

Thought memories are low-confidence inferred crystals. They are recallable,
clearly marked as inferred, ranked below comparable source-backed memories, and
subject to ordinary decay. They can become stronger only if later source or
user evidence supports them.

### Rule Crystals

Hieronymus does not model deterministic terminology as a separate strict
termbase. It models enforceable rules as high-credibility active rule crystals
linked to concepts.

Rule crystals enforce concept-specific renderings, not raw string replacements.
If a surface form can refer to multiple concepts, validation uses concept
identity, semantic tags, nearby context, story scope, language tags, and agent
judgment. If the occurrence cannot be disambiguated reliably, validation warns
instead of blindly enforcing a raw string rule.

Active rule crystals are mandatory and do not decay while active. They may be
archived, superseded, or consolidated explicitly. Archiving is an immediate
validation safety valve. Replacement rules still enter as correction memories
and are reconciled by dreaming.

The admin interface must not expose a manual "promote to rule" action.
Corrections enter as high-credibility short-term memories with source
credibility such as `user_rule` and explicit `rule_intent`. Dreaming decides
when clean rule intent and reliable concept disambiguation justify an active
rule crystal. Malformed or ambiguous rule-like provider output must not create
active rules.

## Recall

Recall returns one flat ranked working set from short-term memories and
long-term crystals. It does not synthesize a canonical summary or store
reconstructed summaries.

Recall searches text, semantic tags, story scopes, concept labels, concept
facets, linked concepts, and relevant short-term memories. Results identify
their tier (`short_term` or `long_term`) and expose the fields agents need:
ids, title/kind, text, crystal type, linked concepts, language tags, story
scopes, semantic tags, source credibility, strength, confidence, soft origin,
rank, score, reason, and rule/thought markers.

Default recall boosts include text/tag matches, concept links, language-tag
matches, story-scope matches, active rule crystals for matching concepts,
source credibility, confidence, and strength. Story scopes boost relevance
rather than excluding older scoped memory unless an explicit debug/admin search
requests hard filtering.

## Dreaming

Dreaming is the ordinary path from learned material to durable long-term
memory. It processes completed short-term memories, creates and updates
concepts/facets/crystals, and maintains the relevant portion of the memory graph
it touched.

Dreaming is local, bounded, auditable, and multi-phase. It may run from
scheduled triggers, urgent backlog triggers, admin "Dream all", CLI/MCP
requests, or future service automation. Memory ingestion does not block on a
full dream cycle.

### Scheduling And Drain Behavior

Scheduled dreaming normally respects `min_pending_short_term_memories`. If the
schedule fires below that threshold, it records a `not_enough_memories` skip.
After enough consecutive skips, backlog escape permits the next scheduled run
to process the small leftover batch.

Default trigger shape:

- scheduled interval: 30 minutes;
- minimum pending short-term memories: 20;
- urgent maximum pending short-term memories: 200;
- maximum short-term memories per cycle: 50;
- backlog escape after five consecutive scheduled
  `not_enough_memories` skips.

Once scheduled, urgent, backlog-escape, manual, CLI, or MCP dreaming starts, it
drains pending short-term memories through successive capped cycles until none
remain. Each individual cycle still respects the per-cycle cap and bounded
affected-memory limits.

### Affected Memory Set

Each dream cycle operates on a bounded affected memory set. It starts with the
selected short-term memories, extracts candidate concepts, language tags, story
scopes, semantic tags, and keywords, then pulls nearby long-term memory by
linked concepts, concept facets, semantic tags, story scopes, FTS similarity,
recent activation, and reinforcement.

Dreaming may mutate only the affected memory set and newly created items. Hard
caps prevent unbounded provider calls or store-wide mutation.

### Phases

Provider-backed phases have separate workflow configuration, prompt text,
provider profile, model, input schema, output schema, audit records, and
mutation boundaries.

Required phases:

1. **Crystallization**: convert short-term memories into crystals, rule
   crystals, concepts, facets, links, semantic tags, story scopes, and thought
   memories.
2. **Parse and apply**: parse provider output best-effort, reject entries
   without content, apply confidence penalties for malformed optional fields,
   insert accepted records, and audit parse issues.
3. **Relation discovery**: run deterministic relation discovery using FTS,
   facets, tags, scopes, source ids, and links; optionally use an LLM-assisted
   pass for ambiguous relation work.
4. **Reinforcement and compaction**: inspect the affected-memory snapshot and
   decide reinforcement, compaction, supersession, concept rename/split/merge,
   and bounded decay candidates.
5. **Apply maintenance decisions**: validate and apply maintenance output
   within the affected set.
6. **Audit reporting**: record what was inspected, changed, skipped, recovered,
   rejected, and why.

Deterministic fallback may remain for CLI/debug/tests only. It must not
silently replace invalid configured provider workflows in scheduled, urgent, or
admin-triggered dreaming.

### Provider Configuration

Dreaming workflow config lives in `~/.config/hieronymus/dream.conf`. Provider
profiles and plaintext local API keys live in
`~/.config/hieronymus/provider.conf`. Provider model lists are cached separately in
`~/.config/hieronymus/llmcache.tmp`.

First-run defaults keep dreaming disabled until provider profiles are
configured. Crystallization points to the Anthropic workflow assignment.
LLM-assisted relation discovery is disabled by default and points to the Ollama
workflow assignment. Reinforcement/compaction points to the Ollama workflow
assignment.

Dreaming is disabled until required provider-backed workflows are enabled and
valid. Disabled optional workflows do not fail doctor checks for missing
provider/model/API settings, but invalid saved values may still warn.

API key values are persisted only in local provider config and must be redacted
from logs, bridge responses, terminal output, and doctor/config status.

### Dream Audit

Every dream cycle produces immutable audit records. Corrections append new
events rather than rewriting old records.

Audit records include short-term inputs, affected memory sets, searches,
concept/facet/tag/link changes, crystals created/reinforced/decayed/archived,
rule crystals created/superseded, thought memories, warnings, errors,
phase-by-phase provider/model/prompt metadata, redacted bounded raw provider
request/response payloads, parsed output, malformed-output penalties, and
maintenance decisions.

Dream audit is visible from the admin interface.

## MCP And Agent Skills

MCP exposes primitives. Agent skills contain judgment.

Primitive MCP operations include series creation/listing, sessions,
short-term-memory insertion, correction memory insertion, recall, dreaming,
feedback, concept/facet CRUD, concept-crystal links, rule-crystal listing,
rule-crystal archive, and validation against active rules.

Read, Learn, and Remember are generated agent skill workflows:

- **Read** inspects material for the current task and writes short-term memory
  only for useful observations or explicit remember requests.
- **Learn** splits material into compact observed facts with inferred tags,
  scopes, source credibility, and soft origin hints, then writes short-term
  memories. It does not create crystals directly.
- **Remember** preserves user corrections and rule intent in short-term memory
  so dreaming can convert clean rule intent into active rule crystals.

Legacy direct memory-add names may remain as compatibility wrappers, but their
behavior is short-term ingestion, not direct long-term crystal creation.

## Terminal Application Vision

The terminal UI is a React/OpenTUI app running on Bun. It is the first-class
interactive surface for local configuration and memory administration.

The frontend uses OpenTUI primitives (`box`, `scrollbox`, `input`, `select`,
`code`, `diff`) and `unicode-animations` for clear busy/loading feedback.
Python launches the built frontend bundle with Bun. The frontend calls Python
through JSON-RPC over stdio, currently by bridging back into the same Python
environment with `python -m hieronymus`.

### Config TUI

The config UI manages provider and dreaming configuration:

- `provider.conf` profiles for OpenAI, Anthropic, Gemini, and OpenAI-compatible local
  endpoints such as Ollama;
- plaintext local API key storage with redacted display;
- per-workflow provider/model selection in `dream.conf`;
- cached model suggestions from `llmcache.tmp`;
- provider and workflow test actions;
- dreaming trigger fields and caps;
- editable phase prompts;
- current short-term-memory and dreaming status;
- save, reload, validation, and unsaved draft handling.

Provider profile tests and workflow checks use the edited in-memory draft, not
only saved settings.

### Admin TUI

The admin UI manages memory state:

- views for crystals, lessons, concepts, proposals, short-term memories, dream
  runs, dream audits, and audit log;
- global stats, short-term status, dream status, and config status;
- table navigation, filtering, detail panes, command palette, and dialogs;
- crystal actions: add, edit, merge, split, supersede, reinforce, decay,
  deprecate, delete, provenance, and recall reasons;
- concept actions: create, update, rename, merge, archive, reinforce, decay,
  facet editing, canonical facet selection, and concept-crystal links;
- short-term-memory removal and admin correction-memory creation;
- manual Dream all and dream review;
- dream audit inspection, including malformed/recovered provider items.

Semantic corrections that change meaning, rendering, applicability, or rule
behavior should create correction memories rather than mutating long-term
memory directly.

## Install, Update, And Release

Managed installs live in `~/.local/share/hieronymus/app` and install console
commands through `uv tool install`.

The installer and update path must:

- check Python >= 3.12;
- check Bun >= 1.3;
- prompt to install or upgrade missing/outdated required tooling;
- install frontend dependencies with Bun;
- rebuild `frontend/dist/main.js`;
- only then install or reinstall the Python tool.

Semantic release builds must also run frontend dependency installation and
frontend build before `uv build`, because the Python wheel includes the built
frontend artifact.

## Migration And Compatibility

Existing databases continue to work. Migration converts pair-shaped and
strict-termbase data into the revised model:

- existing series keep slugs and titles;
- default source/target languages become language tags;
- strict terms become concepts, facets, and active rule crystals;
- strict proposals become candidate concepts, candidate facets, or candidate
  rule crystals depending on status and evidence;
- existing crystals keep series data and convert old tags to semantic tags
  where appropriate;
- old source/reference-like text becomes soft origin hints where available.

CLI shortcuts can remain for compatibility, but the underlying model must not
be direction-first.

## Non-Goals

- Do not rewrite the backend in TypeScript.
- Do not let the frontend write SQLite or config files directly.
- Do not store raw source documents as the main memory object.
- Do not create canonical summaries during recall.
- Do not let fuzzy recall override active rule crystals.
- Do not promote malformed or ambiguous rule-like output into active rules.
- Do not require a browser or Electron.
- Do not make a graph database a prerequisite for the first-release memory
  graph.

## Consequences

- The ADR set now contains both implementation decisions and durable product
  vision. Old execution specs/plans can be removed after their implemented and
  remaining parts are reflected here and in the current baseline documentation.
- Future work should update this ADR only when the product model changes. Task
  tracking belongs in `docs/roadmap.md` or issue/PR workflows.
- Tests should protect these boundaries: local-first storage, primitive MCP
  surface, rule-crystal precedence, combined recall, bounded dreaming, audit
  completeness, OpenTUI launch behavior, install/update build ordering, and
  secret redaction.

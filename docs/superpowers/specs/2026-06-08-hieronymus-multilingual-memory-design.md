# Hieronymus Multilingual Memory Design

## Purpose

Hieronymus should behave like a local memory system for literary translation:
small memories accumulate, recall returns a useful working set, and the system
keeps learning through dreaming with minimal human intervention.

This design defines the direction for the next refactor. Memory is multilingual,
concept-centered, and scope-aware without being locked to a single
source-target pair or a book/chapter structure.

## Current Context

The current implementation already has series registration, task sessions,
short-term memories, crystals, recall, dreaming, strict term validation, concept
proposals, generated agent skills, CLI commands, MCP tools, and admin views.

The main gaps are:

- Series currently carry one default source language and one default target
  language.
- Concepts are not first-class durable memory objects.
- Concept-like data is split between crystals and strict concept proposals.
- `hieronymus_read` and `hieronymus_learn` are MCP tools even though they are
  agent-judgment workflows.
- `hieronymus_memory_add` creates crystals directly, bypassing dreaming.
- Recall does not yet return one combined working set from short-term and
  long-term memory.
- Strict terminology is represented as separate strict-term objects, while the
  desired model is rule memory expressed as high-credibility rule crystals.
- Dreaming is currently one crystallization step, while the desired model is a
  multi-phase autonomous memory-maintenance cycle with full audit.

## Mental Model

### Series

A series is language-neutral. `only-sense-online` means the work itself, not
`ja->en` or `ja->ru`.

The series may contain memories, concepts, concept facets, semantic tags, story
scopes, and rule crystals tagged with any combination of languages. Some memory
may be Japanese-only, some Russian-only, some English and Russian, and some
language-independent.

### Language Tags

Language tags describe where a memory, concept facet, or rule applies.
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

### Memory Language

Hieronymus should store ordinary memory prose primarily in English so recall
stays searchable across Japanese source material, Russian notes, and other
translation contexts.

Japanese, Russian, and other non-English text should still be preserved when it
is the subject of memory:

- concept facets and renderings;
- exact source forms;
- short quotations needed for disambiguation;
- metadata, story scopes, or tags where the original text matters.

Dreaming prompts and agent skills must ask for English explanations by default.
They should not translate away source forms or target renderings that are
needed as concept facets, rule-crystal renderings, or evidence.

### Story Scopes

Story scopes are freeform work-position labels. They must not assume books and
chapters are the only possible structure.

Examples:

- `book-5`
- `book-5/chapter-5`
- `route-yun`
- `episode-12`
- `story-bible`
- `side-story-a`

Story scope can be attached to sessions, short-term memories, crystals,
concept facets, and concept-crystal links. Default recall boosts matching story
scopes but does not hard-filter non-matching memories. Older scoped memories may
still be relevant; supersession remains explicit.

### Semantic Tags

Semantic tags are freeform labels that describe meaning, such as `talent`,
`subskill`, `faction`, `location`, `ability`, or `inventory-item`.

Tags are not a fixed taxonomy. Dreaming may create and adjust them, and users
can edit them from the admin interface. Tags help distinguish concepts with the
same surface form, such as a `Cooking` talent and a `Cooking` subskill.

### Concepts

A concept is a first-class durable identity anchor. It represents one meaningful
translation subject and gathers facets, semantic tags, and related memories.
Concepts do not decay like crystals. They may be renamed, split, merged, or
archived through explicit memory operations.

Example:

- concept: Yun
- facets:
  - `ja`: `ユン`
  - `en`: `Yun`
  - `ru`: `Юн`
  - `all`: main character, first-person narration, game-system context
- semantic tags: `character`, `narrator`

Concept identity is usually series-wide. Story-specific or language-specific
variation belongs on concept facets, concept-crystal links, or crystals.

Concepts have a lifecycle:

- `candidate`: auto-created from weak or early evidence;
- `established`: auto-promoted when enough consistent evidence, facets, links,
  or source credibility accumulate;
- `archived`: explicitly retired;
- `merged`: redirected into another concept.

Dreaming may create, promote, rename, split, and consolidate concepts
autonomously within its bounded affected memory set. Previous labels should
remain searchable facets unless they are explicitly superseded as wrong.

### Concept Facets

A concept facet is a language-scoped or story-scoped piece of concept
information, such as a name, rendering, description, or note.

Suggested facet fields:

- language tags;
- story scopes;
- semantic tags;
- kind: `name`, `rendering`, `description`, `note`;
- text;
- whether it is the current canonical facet for that language/context.

Aliases are searchable facets, not a separate domain object. A rendering becomes
deterministically enforced only through a rule crystal with explicit rule intent.

### Rule Crystals

Hieronymus should not model deterministic validation as a separate strict
contract system. It should model rules as crystals with explicit rule intent and
authoritative source credibility.

Rule crystals enforce concept-specific renderings, not raw string replacements.
For example:

- concept: `Cooking Talent`
- semantic tags: `talent`, `skill-system`
- rule crystal: "Render this concept as `Готовка` in Russian."

If another concept has the same surface form, such as a `Cooking` subskill, it
must be disambiguated by concept identity, semantic tags, nearby context, story
scope, or agent judgment. If validation cannot disambiguate the occurrence, it
should warn instead of blindly enforcing a raw string rule.

Active rule crystals do not decay. They may be superseded, archived, or
consolidated explicitly. User rule intent should create active rule crystals
automatically during dreaming. Ambiguous suggestions remain advisory memories.
Malformed rule-like provider output must not create active rule crystals.
Promotion from advisory rule-like memory to an active rule crystal is performed
by later dreaming only when clean explicit rule intent and reliable concept
disambiguation are present.

The admin interface should not provide a manual rule-promotion action.
Corrections should enter the system as new high-credibility short-term memories,
such as user-rule correction memories, and dreaming decides how to supersede or
create active rule crystals.

Archiving an active rule crystal is allowed as an immediate validation safety
valve. It stops validation from using that rule right away and records an audit
event. Any replacement rule still enters as correction memory and is processed by
dreaming.

### Source Credibility, Confidence, And Strength

Source credibility describes the origin of a memory. Standard labels:

- `rumor`;
- `observation`;
- `source_text`;
- `expert`;
- `user_suggestion`;
- `user_rule`.

Confidence describes how likely a memory is to be true or correct, based on
source credibility and consistency with other evidence.

Strength describes how active or useful a memory is in recall, based on recency,
reinforcement, repeated use, and successful matching.

This distinction allows memories to be true but rarely relevant, frequently
encountered but suspicious, or both trusted and active.

### Memories And Crystals

Short-term memories are session-scoped distilled observations. They are normally
one to six sentences, not raw source chunks. Direct tool/API ingestion should
hard-reject extremely oversized memories and warn for large but acceptable
memories. Agent skills should split long input before writing short-term memory.
Short-term memory prose should be English by default, with non-English text
included only where it is the remembered form, rendering, quotation, or
metadata.

Short-term memories stay until dreaming processes them or an admin/user removes
them. They do not expire automatically just because dreaming is disabled,
misconfigured, or delayed.

Crystals are long-term memory records created by dreaming. They should be atomic
but meaningful: one character fact, one rendering preference, one style lesson,
one world rule, one uncertainty, one rule crystal, or one reusable translation
observation. Crystals are normally one or two sentences and should be written in
English by default.

Crystals carry language tags, story scopes, semantic tags, source credibility,
strength, confidence, and optional soft origin hints. Origin hints are
memory-like, not archival citations. Good examples:

- probably from volume 2;
- learned from the story bible;
- noticed during chapter 14 review.

Hieronymus should not require precise source references for ordinary memories.
Rule crystals should keep enough rationale and audit context to explain why they
are enforced.

### Thought Memories

Dreaming may create low-confidence thought memories when it notices a
potentially useful inference that did not come directly from source material or
user input.

Thought memories are long-term crystals, not short-term memories. They are
recallable by default, clearly marked as inferred, ranked below comparable
source-backed memories, and subject to ordinary decay. They can grow in
confidence over time if later source or user evidence supports them.

## MCP And Agent Skills

MCP should expose primitives. Agent skills should contain judgment.

### MCP Primitives

The first release should add or adjust MCP operations for:

- creating or updating a language-neutral series;
- listing series;
- starting and completing sessions with optional language tags and story scopes;
- adding short-term memories with language tags, story scopes, semantic tags,
  source credibility, rule intent, and soft origin hints;
- adding correction memories as ordinary short-term memories with correction
  kind/source metadata;
- recalling one combined ranked working set from short-term memories and
  long-term crystals;
- requesting or running dreaming;
- recording feedback;
- listing and editing concepts, concept facets, semantic tags, and
  concept-crystal links;
- listing rule crystals and validating text against active rule crystals.

The MCP surface should allow an agent to initialize an empty store without using
the CLI.

### Read, Learn, And Remember

Read, Learn, and Remember are skills, not MCP judgment tools.

Read instructs the agent to inspect material for the current task. It should not
bulk-store the source. It may add a short-term memory only when the agent finds a
useful observation or the user asks it to remember something.

Learn instructs the agent to commit material into memory. It should split the
material into small extracts, infer useful language tags, story scopes,
semantic tags, source credibility, and soft origin hints, then write short-term
memories. Learn must not create crystals directly.

Remember must preserve rule intent. If the user says to always render a concept
in a particular way, the skill should write a short-term memory such as "User
told me to always render ..." with source credibility `user_rule` and rule
intent. Dreaming then converts that into a rule crystal automatically.

Corrections are ordinary short-term memories with correction metadata, not a
separate queue. Admin-created corrections should include a correction kind,
source credibility, rule intent where applicable, linked concepts when known,
story scopes, language tags, semantic tags, and an admin-correction origin.

Direct admin edits are for metadata fixes and obvious typo cleanup. Semantic
corrections that change meaning, rendering, applicability, or rule behavior
should enter as correction memories so dreaming can reconcile them with the
affected memory set and produce audit.

Dreaming is the ordinary path from learned material to long-term crystals.

### Legacy Direct Memory Add

`hieronymus_memory_add` should stop creating crystals directly. The first
release should either deprecate it or redirect it into the short-term/session
workflow.

If compatibility requires keeping the tool name temporarily, its behavior should
be documented as short-term ingestion, not long-term crystal creation.

## Recall

Recall returns one flat ranked working set from both short-term memory and
long-term crystals. It should not synthesize a canonical summary or store
reconstructed summaries.

The query should search text, semantic tags, story scopes, concept labels,
concept facets, linked concepts, and relevant short-term memories. Results
should identify whether they came from short-term or long-term memory.

Default recall boosts:

- text and tag matches;
- linked concept matches;
- language tag matches;
- story scope matches;
- active rule crystals for matching concepts;
- source credibility;
- confidence and strength.

Story scope normally boosts relevance rather than filtering. Hard filtering is
reserved for explicit admin/debug/search operations.

The default limit should be around 10 to 15 results. Each result should expose
facets useful to agents:

- memory tier: `short_term` or `long_term`;
- id;
- title or kind;
- text;
- crystal type when long-term;
- linked concept ids and labels where available;
- language tags;
- story scopes;
- semantic tags;
- source credibility;
- strength;
- confidence;
- soft origin hint;
- rank, score, and reason;
- whether the item is a rule crystal or thought memory.

The agent uses this working set as context and may group or summarize it in its
own reasoning.

## Dreaming

Dreaming is an autonomous multi-phase cycle. It processes completed short-term
memories, creates and updates long-term memory, and maintains the relevant
portion of the memory system it touched.

Dreaming should run in the background when requested by triggers or manual
admin action. Memory ingestion should emit/request dreaming rather than blocking
on a full dream cycle.

Correction memories do not request immediate dreaming by themselves. They join
ordinary short-term memory and are processed when configured dreaming triggers
are met, or when an admin starts Dream all from the admin interface.

Scheduled dreaming normally respects `min_pending_short_term_memories`. If the
schedule fires and fewer pending memories exist, the service records a
`not_enough_memories` skip instead of running. To prevent small leftover batches
from staying in short-term memory indefinitely, a configured backlog escape
allows the next scheduled run to process the leftover batch after enough
consecutive `not_enough_memories` skips.

Default trigger behavior:

- `scheduled_interval_minutes`: 30;
- `min_pending_short_term_memories`: 20;
- `max_pending_short_term_memories`: 200, after which dreaming is urgent and
  should be requested as soon as possible;
- `max_short_term_memories_per_cycle`: 50;
- `not_enough_memories_cycle_threshold`: 5.

With a 30-minute schedule and a threshold of 5, the first five scheduled checks
that find too few memories are skipped. The sixth scheduled check, after about
three hours, temporarily disables the minimum-memory threshold for that run and
processes the small remaining batch.

### Affected Memory Set

Each dreaming cycle operates on a bounded affected memory set. It starts with
the selected completed short-term memories, extracts candidate concepts,
language tags, story scopes, semantic tags, and keywords, then pulls nearby
long-term memory by:

- linked concepts;
- matching concept facets;
- matching semantic tags;
- matching story scopes;
- keyword/FTS similarity;
- recent activation or reinforcement.

Dreaming may mutate only the affected memory set and newly created items. The
implementation should enforce hard caps, such as maximum short-term inputs,
maximum affected concepts, maximum related crystals per concept, and maximum
total long-term crystals considered.

Even when `max_pending_short_term_memories` makes dreaming urgent, each dreaming
cycle must process only a capped batch of short-term memories. Urgency schedules
work sooner; it does not permit an unbounded provider call.

When more pending short-term memories exist than the per-cycle cap allows,
batching is deterministic: process the oldest eligible completed-session
memories first while preserving session coherence where possible. If one
session exceeds the cap, split that session by memory creation order.

Once scheduled, urgent, backlog-escape, or manual dreaming starts, it drains all
pending short-term memories through successive capped cycles until no pending
short-term memories remain. Each individual cycle still respects
`max_short_term_memories_per_cycle`, and the final small batch is processed even
when fewer than `min_pending_short_term_memories` remain.

The admin interface exposes one manual dreaming action: Dream all. Dream all
uses the same drain-all behavior as a started scheduled or urgent dreaming run.

### Phases

Each phase has its own workflow, prompt, input schema, output schema, audit
record, retry/error behavior, and mutation boundary. Provider-backed phases may
use different configured providers/models. Dreaming config should use reusable
named provider profiles and let each workflow select a profile plus its own
model. This allows crystallization to use a stronger remote model while
relation discovery or reinforcement and compaction can use a cheaper or local
provider, such as an Ollama-backed endpoint.

Provider model lists are cached outside dreaming config in
`~/.config/hieronymus/llmcache.tmp`. The cache is refreshable and temporary:
config UI uses it for model suggestions, provider-profile tests update it, and
`hiero doctor` reads it to warn when selected workflow models are no longer
present in a provider's known model list. Doctor should try to refresh stale
model lists on start. Cache entries are stale after 24 hours. Failed refreshes
for unreachable providers are warnings, not doctor failures.

Doctor should fail dreaming config checks when `dream.conf` is invalid, a
workflow references a missing provider profile, a workflow model is not set, a
provider profile used by an enabled workflow cannot be reached during an active
check, a required API key is missing, the API key is rejected, or a selected
model is known not to exist when the provider can verify model availability.
Disabled optional workflows, such as LLM-assisted relation discovery, should not
cause doctor failures for missing provider/model/API settings, though doctor may
warn about invalid configured values.
Disabled optional workflow settings should still be saved in `dream.conf` so
they can be configured before being enabled.

First-run defaults should create provider profile stubs for Anthropic, OpenAI,
Gemini, and Ollama without plaintext API keys. Crystallization should point to
the Anthropic stub, LLM-assisted relation discovery should be disabled and point
to the Ollama stub, and reinforcement/compaction should point to the Ollama
stub. Workflows remain invalid until required models, endpoints, and credentials
are configured.

Dreaming is disabled until all required enabled provider-backed workflows are
valid. Deterministic fallback must not silently replace misconfigured dreaming
workflows in automatic or admin-triggered dreaming.
Disabled dreaming blocks conversion and maintenance only. Recall still searches
both short-term and long-term memory so newly captured observations remain
usable while dreaming configuration is incomplete.

Required first workflow:

- **Crystallization prompt**: send up to `max_short_term_memories_per_cycle`
  short-term memories as one provider prompt. The provider returns the crystals,
  rule crystals, concepts, concept facets, semantic tags, concept links, and
  thought memories it deems useful.
- **Parse and apply**: Hieronymus parses the provider output best-effort,
  rejects entries without required content, applies confidence penalties where
  fields are malformed, inserts accepted memory records, and records parse
  issues in the dream audit.
- **Relation discovery**: Hieronymus checks what changed, runs a deterministic
  relation-discovery pass using FTS, facets, semantic tags, story scopes,
  source ids, and existing links, then may use an optional LLM-assisted pass for
  ambiguous or intelligent linking. The result is a compact affected-memory
  snapshot. The LLM-assisted pass is disabled by default and may default to the
  reinforcement/compaction provider when enabled.
- **Reinforcement and compaction prompt**: send the affected-memory snapshot,
  created links, discovered relations, and relevant changed records to the
  configured reinforcement/compaction provider. The provider decides
  reinforcement, compaction, supersession, concept renames/splits/merges, and
  bounded decay candidates.
- **Apply maintenance decisions**: Hieronymus validates and applies the
  reinforcement/compaction output within the affected memory set.
- **Audit reporting**: record what was inspected, decided, changed, skipped,
  recovered, rejected, and why.

Dreaming may autonomously reinforce, consolidate, decay, rename, split, and
merge within its bounded affected memory set. It should preserve auditability
and avoid bringing the whole store into one cycle.

Deterministic fallback may remain as an explicit CLI/debug/test provider only.
It is not part of normal scheduled, urgent, or admin-triggered dreaming. When
used explicitly, it should follow the same data model and phase boundaries where
possible and must not bypass rule-intent, concept, source credibility, or
memory-size rules.

### Malformed Provider Output

Provider output may be partially malformed. Hieronymus should keep useful memory
instead of discarding an entire phase result whenever safe recovery is possible.

Content is required for crystals, rule crystals, concepts, and concept facets.
If content is absent or empty, the item is rejected and recorded in the dream
audit. If content exists but other fields are malformed, missing, or weakly
structured, the parser should recover best-effort values where possible and
apply a confidence penalty to the resulting memory.

Examples:

- a crystal with text but missing semantic tags can be kept with empty tags and
  lower confidence;
- a concept facet with text but malformed story scopes can be kept with no story
  scopes and lower confidence;
- a malformed rule-like item with explicit content can be kept as advisory or
  candidate memory with lower confidence, but it must not create an active rule
  crystal;
- an item with no usable content is rejected.

Recovered malformed items must be visible in the dream audit with the parse
problem and applied confidence penalty. Normal reinforcement, compaction, and
decay can later strengthen or weaken those memories.

### Default Provider Prompts

Hieronymus should ship editable default prompts for each provider-backed
dreaming phase. The implementation appends phase-specific input schemas, output
schemas, and strict format constraints automatically; users edit only the
guidance text.

All default prompts must make English the main language for ordinary memory
prose. Japanese, Russian, and other non-English strings should be preserved only
when they are source forms, renderings, quotations, concept facets, semantic
tags, story scopes, or metadata.

#### Crystallization Prompt

```text
Convert the provided short-term memories into compact long-term memory updates
for a literary translation memory system.

Write ordinary memory text in English. Preserve Japanese, Russian, or other
non-English strings only when they are exact source forms, target renderings,
quotations needed for disambiguation, concept facets, semantic tags, story
scopes, or metadata.

Create atomic crystals of one or two sentences. Do not store raw source chunks.
Create or update concepts when a memory refers to a durable identity such as a
character, place, ability, item, faction, title, style rule, or recurring term.
Use concept facets for names, renderings, descriptions, and notes. Use semantic
tags to disambiguate similarly named concepts, such as talent, subskill,
location, faction, or ability.

If a short-term memory contains explicit rule intent, especially user wording
such as "always render" or "must translate as", create an active rule crystal
linked to the relevant concept. Rule crystals enforce concept-specific
renderings, not raw string replacements. If the concept is ambiguous, create the
best disambiguating concept/facet/tag structure available and record the
ambiguity.

If you infer a potentially useful idea that is not directly stated by the
source or user, create it only as a low-confidence thought memory. Mark it as
inferred. Never create a rule crystal from an inference.

Prefer small, searchable English trivia over broad summaries. Keep source
credibility, language tags, story scopes, semantic tags, and soft origin hints
when they are present or can be inferred safely.
```

#### Reinforcement And Compaction Prompt

```text
Inspect the affected-memory snapshot produced after crystallization, parsing,
database insertion, and relation discovery.

Write ordinary analysis labels and proposed memory text in English. Preserve
non-English strings only as exact forms, renderings, quotations, facets, tags,
story scopes, or metadata.

Use the snapshot of created links, discovered relations, changed concepts,
changed facets, changed crystals, rule crystals, thought memories, searched but
unused memories, and parse warnings. Decide which memories should be
reinforced, compacted, combined, superseded, renamed, split, merged, linked,
unlinked, archived, or decayed.

Use story scopes and semantic tags as relevance signals, not as hard filters.
Older scoped memories can still be relevant. Supersession must be explicit.

When a surface form could refer to several concepts, prefer concept identity,
semantic tags, nearby context, language tags, and story scopes over raw string
matching. If disambiguation is not reliable, record the ambiguity instead of
forcing a link.

Active rule crystals do not decay, but they may be superseded or consolidated
explicitly. Malformed rule-like items that were recovered as advisory memories
must not be promoted unless clean explicit rule intent and reliable concept
disambiguation are present in the affected snapshot.

Thought memories remain low-confidence unless later source-backed or user-backed
evidence in the affected snapshot supports them.

Return structured maintenance decisions only for the affected memory set. Do not
invent unrelated store-wide changes.
```

#### LLM-Assisted Relation Discovery Prompt

```text
Review the changed memories and deterministic relation candidates.

Write ordinary labels and explanations in English. Preserve Japanese, Russian,
or other non-English strings only as exact source forms, renderings, quotations,
concept facets, semantic tags, story scopes, or metadata.

Use the provided candidate links, concept facets, semantic tags, story scopes,
source ids, and text matches to identify likely concept links, duplicate
concepts, same-name ambiguities, missing semantic tags, and useful relation
types. Prefer existing concepts when identity is clear. Create new relation
suggestions only inside the affected memory set.

Do not promote malformed rule-like memories into active rule crystals. Do not
make store-wide changes. If a relation is uncertain, return it as uncertain with
the reason instead of forcing it.
```

### Dream Audit

Every dreaming cycle must produce an immutable dream audit. Corrections append
new audit events rather than rewriting prior records.

The audit should include:

- short-term inputs processed;
- affected memory set;
- searches performed;
- concepts, facets, semantic tags, and links changed;
- crystals created, reinforced, consolidated, decayed, superseded, or archived;
- rule crystals created or superseded;
- thought memories created;
- warnings and errors;
- phase-by-phase provider, model, prompt version, input, output, and decision
  metadata;
- redacted bounded raw provider request/response payloads when available.

Dream audits must be visible from the admin interface.

## Migration And Compatibility

Existing databases should continue to work.

Migration should convert existing pair-shaped data as follows:

- Existing series remain the same slug and title.
- Existing default source and target languages become language tags associated
  with existing sessions, memories, crystals, concepts, and rule crystals.
- Existing strict terms become concepts with facets plus active rule crystals.
- Existing strict concept proposals become candidate concepts, candidate
  facets, or candidate rule crystals depending on status and evidence.
- Existing crystals keep their current series and language fields converted into
  language tags, and existing tag data becomes semantic tags where appropriate.
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
- language tags and story scopes on sessions, short-term memories, crystals,
  recall results, concepts, concept facets, rule crystals, and thought memories;
- semantic tags on concepts, facets, crystals, and short-term memories;
- concept records with multilingual facets and concept-crystal links;
- rule crystals replacing separate strict-term objects as deterministic
  validation inputs;
- source credibility, confidence, and strength in recall and dreaming;
- Read, Learn, and Remember as generated agent skills, not MCP judgment tools;
- dreaming-only crystallization for learned material;
- provider-backed dreaming workflows and prompts;
- immutable dream audit visible from admin;
- deprecated or redirected direct memory add;
- combined short-term/long-term recall with 10 to 15 results by default;
- migration from current pair fields and strict terms into the revised model;
- docs and tests for the revised model.

Exclude from this release:

- source document registry;
- stored summary or reconstruction objects;
- graph database migration;
- broad automatic markdown crawling;
- complex graph traversal recall beyond concept links and bounded affected
  memory expansion.

SQLite with join tables remains sufficient for the first release. Concept links
are many-to-many: one crystal may link to several concepts, and one concept may
link to many crystals. A graph database should be reconsidered only if recall or
dreaming require deep multi-hop traversal as a primary feature.

## Testing Requirements

Tests should cover:

- one series containing `ja`, `en`, and `ru` tagged information;
- freeform story scopes that are not book/chapter shaped;
- semantic tags disambiguating same-name concepts;
- concepts with partial language coverage;
- the same concept having Japanese, English, and Russian facets;
- concept links where one crystal links to several concepts;
- rule crystals enforcing concept-specific renderings;
- high-confidence advisory crystals not triggering validation unless they have
  rule intent;
- validation warning when a surface form cannot be disambiguated to one concept;
- active rule crystals not decaying;
- explicit supersession of rule crystals;
- Learn writing short-term memories only;
- Remember preserving user rule intent in short-term memory;
- correction memories being ordinary short-term memories with correction
  metadata;
- direct memory add no longer creating crystals;
- hard rejection and warnings for oversized short-term memories;
- best-effort recovery for malformed provider output with required content and
  confidence penalties;
- recall returning a combined ranked short-term/long-term result shape;
- thought memories being recallable, low-confidence, and inferred;
- dreaming converting completed short-term memories through phase workflows;
- dreaming using a bounded affected memory set;
- ambient decay targeting low-confidence searched-but-unused crystals;
- dream audit recording phase decisions and redacted provider payloads;
- MCP tool listing matching the primitive boundary;
- migration preserving existing series, terms, crystals, and sessions.

## Open Implementation Notes

The implementation plan should decide exact schema names and migration sequence.
The design intent is stable: concepts are first-class durable identity anchors,
rules are high-credibility rule crystals, recall combines short-term and
long-term memory, and dreaming is an autonomous bounded multi-phase maintenance
cycle.

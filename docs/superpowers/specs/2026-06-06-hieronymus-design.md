# Hieronymus — Translation Memory MCP Design Spec

**Date:** 2026-06-06
**Status:** Draft for review

## Purpose

Build a project-local translation memory system for long-form book translation. The system must keep strict terminology stable across a series while also giving agents a searchable fuzzy memory for contextual decisions, plot facts, voice notes, and unresolved questions.

This system is separate from `agentmemory`. `agentmemory` remains coding-oriented. Translation memory is scoped by series, not by coding session.

The project is named **Hieronymus**, after Saint Jerome of Stridon, one of the best-known Bible translators and the creator of the canonical Latin Vulgate translation.

## Goals

- Prevent drift in approved names, game terms, UI strings, item names, skills, locations, and recurring phrases.
- Let translators and reviewers retrieve relevant context without loading every memory file into prompt context.
- Preserve provenance for decisions: chapter references, user decisions, official/fandom evidence, reviewer notes.
- Support pending proposals from agents without letting them silently become rules.
- Allow future background synthesis ("dreaming") that turns many fuzzy memories into compact memory crystals and proposed glossary entries.

## Non-Goals

- Do not replace the existing `memories/*.md` story bible immediately.
- Do not use `agentmemory` as the translation database.
- Do not auto-approve agent-proposed translations.
- Do not require embeddings in the MVP.
- Do not make volume-level overrides the default; series-level decisions should win unless an explicit override exists.

## Storage

The implementation code must live outside the translation workspace:

```text
~/Development/hieronymus/
├── .git/
├── cmd/ or src/
├── migrations/
├── docs/
└── tests/
```

The translation workspace contains books, generated EPUBs, and optional runtime databases only. It must not contain the tool's source code.

Use separate SQLite databases per series:

```text
.translation-memory/
├── registry.sqlite
└── series/
    ├── death-march.sqlite
    └── only-sense-online.sqlite
```

`registry.sqlite` stores known series and paths. Each series database owns that series' strict termbase, fuzzy memories, crystals, evidence, and indexes. This keeps Death March and OSO isolated and makes a single series database easy to back up, inspect, or migrate.

The data root is configurable. For this translation workspace, the expected data root is:

```text
/home/inky/Yandex.Disk/Translation/.translation-memory/
```

Optional exports can be generated for human-readable review:

```text
.translation-memory/
└── exports/
    ├── death-march/
    │   ├── terminology.md
    │   └── characters.md
    └── only-sense-online/
        ├── terminology.md
        └── characters.md
```

## Data Model

### Series

Stored in `registry.sqlite`. Records each translation project and the path to its per-series database.

```sql
series(
  id integer primary key,
  slug text unique not null,
  title text not null,
  source_language text not null,
  target_language text not null,
  database_path text not null,
  created_at text not null,
  updated_at text not null
)
```

Examples: `death-march`, `only-sense-online`.

### Terms

Strict terminology entries. Approved entries are contractual.

```sql
terms(
  id integer primary key,
  override_of_term_id integer references terms(id),
  category text not null,
  source_text text not null,
  canonical_translation text not null,
  status text not null,
  scope text not null,
  volume text,
  confidence real not null default 1.0,
  notes text not null default '',
  created_at text not null,
  updated_at text not null
)
```

Categories must be domain-neutral. A termbase should work for LitRPG, romance, mystery, cookbooks, historical fiction, and any other book type. Categories describe the linguistic role, not a specific series' mechanics.

Core categories:

- `person_name` — people, avatars, aliases used as names.
- `place_name` — cities, dungeons, countries, shops, landmarks.
- `organization_name` — guilds, companies, kingdoms, teams.
- `work_title` — book titles, song titles, quest titles, in-world media.
- `object_name` — named items, weapons, tools, vehicles, artifacts.
- `creature_name` — monsters, species, animals, familiars.
- `plant_name` — herbs, crops, flowers, medicinal plants.
- `substance_name` — minerals, potions, materials, chemicals, ingredients.
- `food_name` — dishes, drinks, cooking terms when translation must be stable.
- `ability_name` — any named capability: skill, spell, technique, Sense, blessing, curse, talent.
- `system_term` — stats, ranks, UI concepts, game/system mechanics, classifications.
- `interface_text` — exact UI labels, system messages, status screen strings.
- `title_or_honorific` — noble titles, epithets, honorifics, role names.
- `cultural_concept` — idioms, archetypes, religious/cultural terms.
- `recurring_phrase` — repeated formulas, catchphrases, announcements.
- `style_convention` — strict formatting or rendering decision.
- `other` — temporary fallback; should be refined during review.

Series-specific concepts are represented with tags or notes, not hard-coded categories. For example, OSO `Sense` and Death March `Skill` both use `ability_name`; OSO `ATK` and Death March `ATK/DEF` use `system_term`; a non-LitRPG herb name uses `plant_name` or `substance_name`.

Statuses:

- `pending` — proposed by an agent or reviewer.
- `approved` — must be followed by translators and reviewers.
- `deprecated` — old approved form retained for audit.
- `rejected` — explicitly wrong form.
- `conflict` — needs human decision.

Scopes:

- `series` — applies to every volume in the series.
- `volume` — applies only to one volume.
- `chapter` — local one-off, rarely used.

### Term Tags

Tags add series-specific nuance without changing the global category taxonomy.

```sql
term_tags(
  term_id integer not null references terms(id),
  tag text not null,
  primary key(term_id, tag)
)
```

Examples:

- OSO: `ability_name` with tags `sense`, `player-build`, `status-screen`.
- Death March: `ability_name` with tags `skill`, `unique-skill`, `magic`.
- Non-LitRPG herbal term: `plant_name` with tags `medicinal`, `poisonous`, `culinary`.
- Book title: `work_title` with tags `in-world-book`, `chapter-heading`, `quest-title`.

### Term Aliases

Stores variants and forbidden forms.

```sql
term_aliases(
  id integer primary key,
  term_id integer not null references terms(id),
  language text not null,
  text text not null,
  kind text not null,
  case_sensitive integer not null default 1
)
```

Alias kinds:

- `source_variant`
- `approved_variant`
- `forbidden_variant`
- `search_alias`

Example: `Attack Increase` can be a forbidden English variant for OSO `ATK Up`; `Gantz` can be a forbidden variant for `Ganz` if that decision is approved.

### Evidence

Records why a term decision exists.

```sql
term_evidence(
  id integer primary key,
  term_id integer not null references terms(id),
  source_type text not null,
  source_ref text not null,
  quote text not null default '',
  url text not null default '',
  notes text not null default '',
  created_at text not null
)
```

Evidence types:

- `chapter`
- `official`
- `fandom`
- `user_decision`
- `reviewer_note`
- `existing_translation`

Official/fandom checks are only required for names, titles, named locations, named items, and other proper nouns where an established translation may exist.

### Fuzzy Memories

Stores searchable contextual memory. These entries are advisory, not contractual.

```sql
memories(
  id integer primary key,
  kind text not null,
  text text not null,
  importance integer not null default 3,
  status text not null default 'active',
  source_ref text not null default '',
  created_at text not null,
  updated_at text not null
)
```

Memory kinds:

- `decision`
- `plot_fact`
- `style_note`
- `voice_note`
- `character_note`
- `worldbuilding`
- `translation_rationale`
- `unresolved_question`
- `reviewer_observation`

### Links

Connects fuzzy memories to strict terms.

```sql
memory_links(
  memory_id integer not null references memories(id),
  term_id integer not null references terms(id),
  primary key(memory_id, term_id)
)
```

### Crystals

Background-synthesized compact facts.

```sql
crystals(
  id integer primary key,
  kind text not null,
  text text not null,
  confidence real not null,
  status text not null default 'active',
  created_at text not null,
  updated_at text not null
)
```

```sql
crystal_sources(
  crystal_id integer not null references crystals(id),
  memory_id integer not null references memories(id),
  primary key(crystal_id, memory_id)
)
```

Crystals are advisory until they produce an approved term or explicit user decision.

## Search

Use SQLite FTS5 for MVP search:

```sql
terms_fts(source_text, canonical_translation, notes)
memories_fts(text)
crystals_fts(text)
```

Later, optional embeddings can be added without changing the core contract:

```sql
embeddings(
  id integer primary key,
  owner_type text not null,
  owner_id integer not null,
  model text not null,
  vector_blob blob not null,
  created_at text not null
)
```

The design borrows three retrieval lessons from existing memory systems:

- Keep source memory verbatim and scoped instead of flattening everything into one corpus.
- Combine lexical search, semantic search, recency, importance, and explicit links rather than trusting one retrieval mode.
- Track embedding model metadata so entries can be re-indexed when the model/provider changes.

## MCP API

### Strict Termbase

`termbase.lookup(series, query, category?, tags?)`

Returns approved and pending terms matching source text, translation, aliases, or notes.

`termbase.contract(series, volume?, raw_text)`

Returns the short glossary contract for a chapter. It includes approved terms whose source forms or aliases appear in the raw text, plus their forbidden variants.

`termbase.validate(series, volume?, raw_text?, translated_text)`

Checks translated text for forbidden variants and inconsistent approved terms. If raw text is provided, it can also verify that source terms present in the raw chapter are rendered with their approved canonical translations. Returns structured findings with severity, expected translation, observed text, and related term id.

`termbase.propose(series, category, source_text, canonical_translation, tags?, evidence?, notes?)`

Creates a `pending` term. Agents may call this for new names, terms, item names, herbs, titles, recurring phrases, or formatting conventions.

`termbase.approve(term_id)`

Promotes a pending term to `approved`.

`termbase.reject(term_id, reason)`

Marks a pending term as rejected.

`termbase.add_alias(term_id, kind, text, language, case_sensitive?)`

Adds source variants, approved variants, forbidden variants, or search aliases.

### Fuzzy Memory

`memory.search(series, query, kind?, limit?)`

Searches memories and crystals for relevant context.

`memory.add(series, kind, text, source_ref?, importance?)`

Adds a contextual memory. This does not create a strict term.

`memory.link(memory_id, term_id)`

Connects a memory to a term.

### Dreaming

`dream.extract_crystals(series, limit?)`

Synthesizes compact crystals from fuzzy memories. MVP can be manual or disabled; later this can use an external LLM API.

`dream.propose_terms(series)`

Finds repeated unresolved terminology in memories and proposes pending termbase entries. It must not approve them.

## Translation Workflow Integration

### Before Translation

The orchestrator calls `termbase.contract(series, volume, raw_text)` and gives the Translator a compact "Chapter Glossary Contract":

```text
Required terms:
- 攻撃力上昇 -> ATK Up
  Forbidden: Attack Increase, Attack Power Increase
- ガンツ -> Ganz
  Forbidden: Gantz

Relevant memories:
- Claude uses theatrical, artisan-like language.
- OSO system UI uses bracketed Sense names: 【Sense Name】.
```

The Translator must treat approved termbase entries as mandatory.

### After Translation

The orchestrator calls `termbase.validate(series, volume, raw_text, translated_text)` before Accuracy Review. Validation failures are sent back to the Translator as concrete fixes.

Accuracy Reviewer still checks meaning, completeness, and context, but strict terminology drift should be caught mechanically first.

### New Discoveries

Translator and reviewers can propose new entries:

- New person/avatar: create pending `person_name`.
- New ability/spell/skill/Sense: create pending `ability_name`.
- New UI/stat/mechanic term: create pending `system_term` or `interface_text`.
- New herb/material/ingredient: create pending `plant_name`, `substance_name`, or `food_name`.
- New in-world title or book/song/quest heading: create pending `work_title`.
- Unclear name spelling: create pending entry with `conflict` status or add unresolved fuzzy memory.

Proper nouns should include official/fandom evidence when available, but final approval remains a user or lead-agent decision.

## Series and Volume Precedence

Lookup order:

1. Explicit volume/chapter overrides whose `override_of_term_id` points at a series-level term.
2. Series-level approved terms.
3. Volume-level approved terms that do not conflict with series-level approved terms.
4. Chapter-level approved terms that do not conflict with series-level approved terms.
5. Pending/conflict terms for reviewer attention.
6. Fuzzy memories and crystals.

Series-level terms normally win. A lower-scope term can only override a series-level term if `override_of_term_id` is set and evidence explains why.

## Export Compatibility

The current `memories/*.md` files remain useful for human reading and broad story context. The MCP should eventually export strict terms back into markdown:

- `memories/terminology.md`
- `memories/characters.md`
- `memories/locations.md`
- `memories/known-issues.md`

Markdown exports are generated artifacts, not the source of truth for strict terms.

## MVP Implementation Scope

MVP should include:

1. SQLite schema and migrations.
2. Seed command for `death-march` and `only-sense-online`.
3. CLI or MCP tools:
   - `lookup`
   - `contract`
   - `validate`
   - `propose`
   - `approve`
   - `memory search`
   - `memory add`
4. FTS5 search for terms and memories.
5. Prompt updates so Translator and Accuracy Reviewer use the Chapter Glossary Contract.
6. Initial OSO seed entries for known drift examples:
   - `攻撃力上昇 -> ATK Up`
   - `ガンツ -> Ganz` or whichever spelling is explicitly approved.
7. Tool repository initialized separately at `~/Development/hieronymus`.

Dreaming, embeddings, external API synthesis, and automatic markdown exports are later phases.

## Testing

- Unit tests for schema migrations.
- Unit tests for lookup precedence.
- Unit tests for forbidden variant detection.
- Unit tests for contract generation from raw text.
- Regression fixture for OSO `ATK Up` vs `Attack Increase`.
- Regression fixture for OSO `Ganz` vs `Gantz`, after the approved spelling is decided.

## Open Decisions

- Whether the approved OSO spelling is `Ganz` or `Gantz`.
- Whether `.translation-memory/registry.sqlite` and per-series databases should be committed or treated as generated local state.
- Whether generated markdown exports should overwrite existing `memories/*.md` or write to `.translation-memory/exports/` only.
- Which runtime should implement the MCP server: Python, TypeScript, or Rust.
- Whether runtime databases should live beside translation projects by default or under a global user data directory.

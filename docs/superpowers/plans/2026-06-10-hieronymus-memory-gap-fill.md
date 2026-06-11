# Hieronymus Memory Gap Fill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining implementation gaps between `docs/superpowers/specs/2026-06-08-hieronymus-multilingual-memory-design.md`, `CONTEXT.md`, and the current codebase, while preserving deliberate compatibility wrappers only where they help tests and users transition.

**Architecture:** Keep SQLite + FTS5 as the primary store. Make concepts durable first-class identity anchors; store language tags, story scopes, semantic tags, facets, rule crystals, and short-term memory metadata as typed relational rows instead of opaque pair-oriented fields. Recall must combine short-term and long-term candidates across text, tags, scopes, concepts, facets, and links. Dreaming remains automatic and bounded, with complete audit trails and phase-specific workflows.

**Tech Stack:** Python 3.12+, SQLite FTS5, pytest, ruff, MCP stdio server, existing Hieronymus CLI/admin modules.

---

## Current Gap Summary

The previous migration implemented most of the dream configuration, dream scheduling, provider profile cache, audit scaffolding, rule-crystal compatibility, and TUI direction. The remaining gaps are mostly in the memory model boundary:

- Series, sessions, short-term memories, and crystals still carry compatibility language-pair fields as primary shape.
- Language tags, story scopes, and semantic tags are not consistently first-class across series, sessions, short-term memories, facets, crystals, and recall responses.
- Concepts exist, but their lifecycle is still `vague/solid` compatibility instead of `candidate/established/archived/merged`.
- Concept facets are too small: they lack canonical flags, story scopes, semantic tags, robust CRUD, rename history, and merge/split behavior.
- Rule validation still behaves like raw string termbase matching instead of concept-specific active rule-crystal validation with ambiguity warnings.
- Recall does not search or return the full enriched memory surface required by the spec.
- MCP still exposes judgment-heavy `hieronymus_read` and `hieronymus_learn` tools. The target model is primitive MCP operations plus agent skills for Read/Learn/Remember behavior.
- Migration of old strict terms, strict proposals, pair fields, and legacy tags into the new memory graph is incomplete.
- Dream audit exists, but it does not yet capture every affected-set, prompt, parse, penalty, decision, and skipped-action detail needed for debugging learning behavior.

## Implementation Tasks

### 1. Add Typed Metadata Tables and Model Fields

- [x] Open `src/hieronymus/migrations/global.sql` and locate the existing `series`, `task_sessions`, `short_term_memories`, `crystals`, `concepts`, `concept_facets`, `crystal_story_scopes`, and `crystal_semantic_tags` definitions.

- [x] Add typed side tables for language tags, story scopes, and semantic tags where the spec requires them:

  ```sql
  create table if not exists series_language_tags (
      series_id integer not null references series(id) on delete cascade,
      language_tag text not null,
      created_at text not null default (datetime('now')),
      primary key (series_id, language_tag)
  );

  create table if not exists task_session_language_tags (
      session_id integer not null references task_sessions(id) on delete cascade,
      language_tag text not null,
      primary key (session_id, language_tag)
  );

  create table if not exists task_session_story_scopes (
      session_id integer not null references task_sessions(id) on delete cascade,
      story_scope text not null,
      primary key (session_id, story_scope)
  );

  create table if not exists task_session_semantic_tags (
      session_id integer not null references task_sessions(id) on delete cascade,
      semantic_tag text not null,
      primary key (session_id, semantic_tag)
  );

  create table if not exists short_term_memory_language_tags (
      memory_id integer not null references short_term_memories(id) on delete cascade,
      language_tag text not null,
      primary key (memory_id, language_tag)
  );

  create table if not exists short_term_memory_story_scopes (
      memory_id integer not null references short_term_memories(id) on delete cascade,
      story_scope text not null,
      primary key (memory_id, story_scope)
  );

  create table if not exists short_term_memory_semantic_tags (
      memory_id integer not null references short_term_memories(id) on delete cascade,
      semantic_tag text not null,
      primary key (memory_id, semantic_tag)
  );

  create table if not exists crystal_language_tags (
      crystal_id integer not null references crystals(id) on delete cascade,
      language_tag text not null,
      primary key (crystal_id, language_tag)
  );

  create table if not exists concept_facet_story_scopes (
      facet_id integer not null references concept_facets(id) on delete cascade,
      story_scope text not null,
      primary key (facet_id, story_scope)
  );

  create table if not exists concept_facet_semantic_tags (
      facet_id integer not null references concept_facets(id) on delete cascade,
      semantic_tag text not null,
      primary key (facet_id, semantic_tag)
  );
  ```

- [x] Add compatibility-safe columns to existing tables through schema bootstrap logic:

  ```sql
  -- Add with ALTER TABLE in the Python schema upgrader when missing.
  concept_facets.is_canonical integer not null default 0
  concept_facets.superseded_at text
  concepts.merged_into_concept_id integer references concepts(id)
  short_term_memories.source_credibility text
  short_term_memories.rule_intent text
  short_term_memories.soft_origin text
  crystals.soft_origin text
  crystals.is_inferred integer not null default 0
  ```

- [x] If there is no central idempotent column upgrader, add one in `src/hieronymus/db.py`:

  ```python
  def ensure_column(conn: sqlite3.Connection, table: str, column: str, definition: str) -> None:
      rows = conn.execute(f"pragma table_info({table})").fetchall()
      if column not in {row["name"] for row in rows}:
          conn.execute(f"alter table {table} add column {column} {definition}")
  ```

- [x] Keep existing pair columns (`default_source_language`, `default_target_language`, `source_language`, `target_language`) as compatibility fields for now; do not use them as the canonical source of truth in new code.

- [x] Update `src/hieronymus/memory_models.py` records:

  ```python
  @dataclass(frozen=True)
  class ShortTermMemoryRecord:
      ...
      language_tags: tuple[str, ...] = ()
      story_scopes: tuple[str, ...] = ()
      semantic_tags: tuple[str, ...] = ()
      source_credibility: str = "observation"
      rule_intent: str = ""
      soft_origin: str = ""

  @dataclass(frozen=True)
  class CrystalRecord:
      ...
      language_tags: tuple[str, ...] = ()
      story_scopes: tuple[str, ...] = ()
      semantic_tags: tuple[str, ...] = ()
      soft_origin: str = ""
      is_inferred: bool = False

  @dataclass(frozen=True)
  class ConceptFacetRecord:
      ...
      language_tags: tuple[str, ...] = ()
      story_scopes: tuple[str, ...] = ()
      semantic_tags: tuple[str, ...] = ()
      is_canonical: bool = False
  ```

- [x] Add `tests/test_memory_schema_metadata.py`.

- [x] In `tests/test_memory_schema_metadata.py`, create a fresh temporary global database and assert all new tables exist.

- [x] In `tests/test_memory_schema_metadata.py`, create a database with the previous schema shape, run schema initialization, and assert the compatibility-safe columns are added without dropping existing rows.

- [x] Run:

  ```bash
  uv run pytest tests/test_memory_schema_metadata.py
  ```

### 2. Make Series Language-Neutral With Language Tags

- [x] Open `src/hieronymus/registry.py` and identify `Series` model creation, lookup, and update paths.

- [x] Extend the `Series` record returned by the registry with `language_tags: tuple[str, ...]`.

- [x] Add helper functions in `registry.py`:

  ```python
  def _normalize_language_tags(tags: Iterable[str]) -> tuple[str, ...]:
      return tuple(sorted({tag.strip().lower() for tag in tags if tag.strip()}))

  def _compat_language_tags(source_language: str, target_language: str) -> tuple[str, ...]:
      return _normalize_language_tags(tag for tag in (source_language, target_language) if tag)
  ```

- [x] Update `create_series` so explicit `language_tags` become canonical and legacy `source_language` / `target_language` only seed tags when no explicit tags are provided.

- [x] Add `list_series()` if missing, returning all series with language tags.

- [x] Add a registry method to update language tags without changing pair compatibility fields:

  ```python
  def set_series_language_tags(self, series_id: int, language_tags: Iterable[str]) -> None:
      ...
  ```

- [x] Update `src/hieronymus/mcp_server.py` with primitive series tools:

  - `hieronymus_series_create`
  - `hieronymus_series_list`
  - `hieronymus_series_set_language_tags`

- [x] Keep existing `hieronymus_series_init` as a wrapper if tests or docs already depend on it; make it call the new primitive method.

- [x] Add `tests/test_series_language_tags.py`.

- [x] Test that a series can be created with `language_tags=["ja", "en", "ru"]` and no translation direction.

- [x] Test that a legacy call with source/target language fields stores those values in `series_language_tags`.

- [x] Test that `hieronymus_series_list` returns language tags and does not require source/target direction fields.

- [x] Run:

  ```bash
  uv run pytest tests/test_series_language_tags.py
  ```

### 3. Store Session and Short-Term Memory Metadata as First-Class Rows

- [x] Open `src/hieronymus/workspace.py`, `src/hieronymus/memory.py`, and `src/hieronymus/mcp_server.py`.

- [x] Extend `TranslationContext` with canonical metadata fields:

  ```python
  language_tags: tuple[str, ...] = ()
  story_scopes: tuple[str, ...] = ()
  semantic_tags: tuple[str, ...] = ()
  ```

- [x] Keep `source_language`, `target_language`, `volume`, `chapter`, and `tags` as compatibility inputs. Normalize them into canonical fields at the boundary:

  - Source and target languages seed `language_tags`.
  - Existing freeform `tags` seed `semantic_tags`.
  - Existing `volume` and `chapter` become story scopes only when callers explicitly supply them. Use stable freeform strings such as `volume:<value>` and `chapter:<value>` so the app remains structure-neutral.

- [x] Update session creation to write:

  - `task_session_language_tags`
  - `task_session_story_scopes`
  - `task_session_semantic_tags`

- [x] Update short-term memory creation to accept and persist:

  - `language_tags`
  - `story_scopes`
  - `semantic_tags`
  - `source_credibility`
  - `rule_intent`
  - `soft_origin`

- [x] Store compatibility metadata in `metadata_json` only as a mirror for older code paths, not as the primary read path.

- [x] Add a helper in `memory.py`:

  ```python
  def add_short_term_memory(
      self,
      *,
      session_id: int | None,
      content: str,
      language_tags: Iterable[str] = (),
      story_scopes: Iterable[str] = (),
      semantic_tags: Iterable[str] = (),
      source_credibility: str = "observation",
      rule_intent: str = "",
      soft_origin: str = "",
  ) -> ShortTermMemoryRecord:
      ...
  ```

- [x] Update `hieronymus_short_term_add` in `mcp_server.py` to expose those fields.

- [x] Add a compatibility wrapper for existing tests that call the old short-term add function with `metadata`.

- [x] Add `tests/test_short_term_metadata.py`.

- [x] Test that a user correction is stored as short-term memory with `source_credibility="user_rule"` or `source_credibility="user_suggestion"` and `rule_intent` set.

- [x] Test that short-term memory rows can be searched by keyword and returned with typed metadata.

- [x] Test that deleting a short-term memory removes its side-table metadata through cascade.

- [x] Run:

  ```bash
  uv run pytest tests/test_short_term_metadata.py
  ```

### 4. Implement Full Concept and Facet Lifecycle

- [x] Open `src/hieronymus/concepts.py` and `src/hieronymus/memory_models.py`.

- [x] Replace new-code lifecycle constants with the spec vocabulary:

  ```python
  CONCEPT_CANDIDATE = "candidate"
  CONCEPT_ESTABLISHED = "established"
  CONCEPT_ARCHIVED = "archived"
  CONCEPT_MERGED = "merged"
  ```

- [x] Add a compatibility mapper so old `vague` reads as `candidate` and old `solid` reads as `established`.

- [x] Extend `ConceptStore` with primitive operations:

  ```python
  def create_concept(..., status: str = CONCEPT_CANDIDATE, semantic_tags: Iterable[str] = ()) -> ConceptRecord: ...
  def list_concepts(..., status: str | None = None, semantic_tag: str | None = None) -> list[ConceptRecord]: ...
  def add_facet(..., is_canonical: bool = False, story_scopes: Iterable[str] = (), semantic_tags: Iterable[str] = ()) -> ConceptFacetRecord: ...
  def list_facets(concept_id: int) -> list[ConceptFacetRecord]: ...
  def set_canonical_facet(concept_id: int, facet_id: int) -> None: ...
  def rename_concept(concept_id: int, new_label: str, *, source_crystal_id: int | None = None) -> ConceptRecord: ...
  def archive_concept(concept_id: int, reason: str) -> None: ...
  def merge_concepts(source_concept_id: int, target_concept_id: int, reason: str) -> None: ...
  def set_semantic_tags(concept_id: int, tags: Iterable[str]) -> None: ...
  ```

- [x] Make `rename_concept` preserve the previous label as a searchable facet unless it is already present.

- [x] Make `merge_concepts` relink:

  - `crystal_concepts`
  - `concept_facets`
  - `concept_semantic_tags`
  - active rule-crystal links

- [x] When a concept is merged, set `status="merged"` and `merged_into_concept_id=<target>`.

- [x] Do not physically delete concepts from ordinary workflows. Use archive or merge.

- [x] Add `tests/test_concept_lifecycle.py`.

- [x] Test candidate-to-established reinforcement based on confidence and linked evidence count.

- [x] Test rename keeps old label searchable as a facet.

- [x] Test one crystal linked to several concepts: `Yun`, `Sense`, `Enchant`.

- [x] Test same visible name on two concepts differentiated by semantic tags such as `talent` and `subskill`.

- [x] Test merged concepts preserve crystal links and facet searchability.

- [x] Run:

  ```bash
  uv run pytest tests/test_concept_lifecycle.py
  ```

### 5. Make Concept Facets Multilingual, Scoped, and Searchable

- [x] Extend concept facet persistence to support:

  - multiple language tags per facet
  - zero or more story scopes
  - zero or more semantic tags
  - `kind` values from the spec: `name`, `rendering`, `description`, `note`
  - canonical facet marker

- [x] Add normalization helpers in `concepts.py`:

  ```python
  VALID_FACET_KINDS = {"name", "rendering", "description", "note"}
  ```

- [x] Reject empty facet content at the store boundary.

- [x] Accept malformed LLM facet metadata with confidence penalty in dream parsing, but never accept missing content.

- [x] Update any FTS indexing code so facet values and concept labels are searchable.

- [x] Add `tests/test_concept_facets_multilingual.py`.

- [x] Test Japanese, English, and Russian facets under one concept:

  - `ja:name` or `ja:note`
  - `en:name`
  - `ru:rendering`

- [x] Test a partial concept with only one language remains valid.

- [x] Test story scope boosts a scoped facet during recall but does not filter out unscoped results.

- [x] Test semantic tag search finds the correct concept even when the text query is ambiguous.

- [x] Run:

  ```bash
  uv run pytest tests/test_concept_facets_multilingual.py
  ```

### 6. Convert Rule Validation From Raw Termbase Matching to Concept-Specific Active Rules

- [x] Open `src/hieronymus/termbase.py`, `src/hieronymus/rule_crystals.py` if present, and the rule validation tests.

- [x] Introduce a rule-crystal view model:

  ```python
  @dataclass(frozen=True)
  class ActiveRuleCrystal:
      crystal_id: int
      concept_ids: tuple[int, ...]
      source_forms: tuple[str, ...]
      required_renderings: tuple[str, ...]
      forbidden_renderings: tuple[str, ...]
      language_tags: tuple[str, ...]
      story_scopes: tuple[str, ...]
      semantic_tags: tuple[str, ...]
      confidence: float
      strength: float
  ```

- [x] Build active rules from crystals where:

  - `crystal_type == "rule"`
  - status is active
  - the crystal is linked to at least one concept
  - confidence and strength meet the deterministic validation threshold

- [x] Keep legacy strict terms only as a source for migrated active rule crystals, not as the canonical validator.

- [x] Update validation to resolve source occurrences using:

  - explicit concept links where available
  - concept facets
  - language tags
  - story scopes
  - semantic tags

- [x] If a surface form maps to multiple active concepts and the context cannot disambiguate, emit an ambiguity warning finding instead of enforcing one rendering.

- [x] Active rule crystals do not decay in decay jobs. They can be superseded, archived, or consolidated.

- [x] Add `tests/test_concept_specific_rule_validation.py`.

- [x] Test the Cooking progression:

  - old rule: Cooking Talent -> `Кулинария`
  - superseding rule: Cooking talent -> `Готовка`
  - new subskill concept: Preparation/Cooking subskill -> `Приготовление`

- [x] Assert validation applies the talent rule only when the context resolves to the talent concept.

- [x] Assert validation warns when `Cooking` is ambiguous and context has no concept, semantic tag, or story signal.

- [x] Assert high-confidence advisory crystals do not behave as active rules.

- [x] Run:

  ```bash
  uv run pytest tests/test_concept_specific_rule_validation.py
  ```

### 7. Enrich Recall Across Short-Term and Long-Term Memory

- [x] Open `src/hieronymus/recall.py` and its tests.

- [x] Extend recall result models with the spec-required fields:

  ```python
  tier: Literal["short_term", "long_term"]
  id: int
  title: str
  kind: str
  text: str
  crystal_type: str | None
  concept_ids: tuple[int, ...]
  concept_labels: tuple[str, ...]
  language_tags: tuple[str, ...]
  story_scopes: tuple[str, ...]
  semantic_tags: tuple[str, ...]
  source_credibility: str
  confidence: float
  strength: float
  soft_origin: str
  is_rule: bool
  is_thought: bool
  score: float
  rank_reason: str
  ```

- [x] Search long-term memory candidates through:

  - crystal FTS text
  - crystal semantic tags
  - crystal story scopes
  - concept labels
  - concept facets
  - concept semantic tags
  - crystal-concept links

- [x] Search short-term memory candidates through:

  - short-term FTS text
  - short-term language tags
  - short-term story scopes
  - short-term semantic tags
  - source credibility
  - rule intent

- [x] Ranking requirements:

  - story scopes boost but do not filter
  - semantic tags boost but do not filter
  - exact concept/facet matches boost
  - active rule crystals boost when relevant
  - low confidence inferred thoughts remain recallable but rank below established evidence

- [x] Preserve activation tracking for long-term recall.

- [x] Add activation records for short-term hits only if the existing activation model can distinguish tiers. Otherwise skip activation for short-term and document why in code.

- [x] Update `hieronymus_recall` MCP output to return the enriched fields.

- [x] Add `tests/test_recall_enriched_memory.py`.

- [x] Test query `"Yun Enchant"` returns one short-term and one long-term hit with tier markers.

- [x] Test concept facets make `"Enchant"` find a crystal whose prose does not contain that exact word but is linked to the Enchant concept.

- [x] Test story scope `book:5/chapter:5` boosts but does not remove a relevant `book:1` result.

- [x] Test low-confidence thought memory is returned with `is_thought=true`.

- [x] Run:

  ```bash
  uv run pytest tests/test_recall_enriched_memory.py
  ```

### 8. Add Primitive MCP Concept, Facet, Tag, and Rule Tools

- [ ] Open `src/hieronymus/mcp_server.py`.

- [x] Add concept primitives:

  - `hieronymus_concept_list`
  - `hieronymus_concept_get`
  - `hieronymus_concept_create`
  - `hieronymus_concept_update`
  - `hieronymus_concept_archive`
  - `hieronymus_concept_merge`
  - `hieronymus_concept_rename`

- [x] Add facet primitives:

  - `hieronymus_concept_facet_add`
  - `hieronymus_concept_facet_update`
  - `hieronymus_concept_facet_list`
  - `hieronymus_concept_facet_set_canonical`

- [x] Add tag and link primitives:

  - `hieronymus_concept_semantic_tags_set`
  - `hieronymus_crystal_link_concept`
  - `hieronymus_crystal_story_scopes_set`
  - `hieronymus_crystal_semantic_tags_set`

- [x] Add rule-crystal primitives:

  - `hieronymus_rule_crystals_list`
  - `hieronymus_rule_crystal_archive`
  - `hieronymus_rule_crystal_validate`

- [x] Keep `hieronymus_termbase_contract`, `hieronymus_concept_proposals_list`, and strict proposal approval tools as compatibility wrappers only if tests still need them.

- [x] Mark compatibility wrappers in tool descriptions with:

  ```text
  Compatibility wrapper. New workflows should use concept, facet, short-term memory, and rule-crystal primitives.
  ```

- [x] Do not expose a manual "promote to rule" admin/MCP action. A user correction must be stored as short-term memory and crystallized later by dreaming.

- [x] Add `tests/test_mcp_memory_primitives.py`.

- [x] Test every new primitive appears in the MCP tool list.

- [x] Test concept create -> facet add -> crystal link -> recall works through MCP handlers.

- [x] Test user correction MCP path creates short-term memory and does not create a rule crystal immediately.

- [x] Run:

  ```bash
  uv run pytest tests/test_mcp_memory_primitives.py
  ```

### 9. Move Read/Learn/Remember Judgment to Agent Skills While Preserving Compatibility

- [x] Locate current MCP implementations for `hieronymus_read` and `hieronymus_learn` in `src/hieronymus/mcp_server.py`.

- [x] Add or update docs under `docs/skills/` or the repository’s existing skill-doc location for:

  - Read skill: user/agent summarizes source text into small short-term extracts.
  - Learn skill: user/agent records observed facts, source credibility, language tags, story scopes, semantic tags.
  - Remember skill: user/agent records corrections as short-term memories, using `"User told me to ..."` phrasing for high-credibility user rules.

- [x] Make those skill docs explicit that MCP tools are storage/retrieval primitives, not judgment engines.

- [x] Remove `hieronymus_read` and `hieronymus_learn` from the MCP tool surface. Compatibility is not required for these judgment-heavy wrappers.

- [x] Update old read/learn tests and docs to use agent skill workflow plus `hieronymus_short_term_add`.

- [x] Add `tests/test_mcp_read_learn_compatibility.py`.

- [x] Test `hieronymus_read` and `hieronymus_learn` are no longer exposed as MCP tools.

- [x] Test `hieronymus_short_term_add` rejects huge chunks according to short-term memory size rules and accepts warning-sized chunks.

- [x] Run:

  ```bash
  uv run pytest tests/test_mcp_read_learn_compatibility.py
  ```

### 10. Implement Legacy Memory Graph Migration

- [x] Add `src/hieronymus/memory_migration.py`.

- [x] Implement an idempotent migrator:

  ```python
  class MemoryGraphMigrator:
      def __init__(self, db: Database) -> None: ...

      def run(self) -> MemoryGraphMigrationReport:
          ...
  ```

- [x] Migration rules:

  - series default source/target languages -> `series_language_tags`
  - task session source/target languages -> `task_session_language_tags`
  - task session volume/chapter -> `task_session_story_scopes`
  - task session tags -> `task_session_semantic_tags`
  - crystal source/target languages -> `crystal_language_tags`
  - crystal `tags_json` -> `crystal_semantic_tags`
  - existing strict terms -> concept + facets + active rule crystal linked to that concept
  - strict concept proposals -> candidate concepts/facets/rules according to proposal state
  - old vague/solid concepts -> candidate/established lifecycle values
  - old source references where available -> `soft_origin`

- [x] Add a migration ledger table so repeated runs do not duplicate generated concepts, facets, or rule crystals:

  ```sql
  create table if not exists memory_graph_migration_ledger (
      source_table text not null,
      source_id text not null,
      target_table text not null,
      target_id integer not null,
      created_at text not null default (datetime('now')),
      primary key (source_table, source_id, target_table)
  );
  ```

- [x] Add CLI/admin entry point if a migration command already exists. Otherwise expose the migrator from doctor and schema startup only as a dry report, not an automatic destructive operation.

- [x] Add `tests/test_memory_graph_migration.py`.

- [x] Seed an old-style strict term and assert migration creates:

  - concept
  - source facet
  - target rendering facet
  - active rule crystal
  - crystal-concept link

- [x] Seed an old crystal with `source_language`, `target_language`, and `tags_json`; assert those values become language tags and semantic tags.

- [x] Seed an old strict concept proposal; assert it becomes a candidate concept and facets.

- [x] Run the migrator twice and assert row counts do not grow on the second run.

- [x] Run:

  ```bash
  uv run pytest tests/test_memory_graph_migration.py
  ```

### 11. Complete Dream Parsing Penalties and Thought Memory Behavior

- [x] Open `src/hieronymus/dreaming.py`, `src/hieronymus/dream_parser.py` if present, and current dream tests.

- [x] Ensure all phase prompts require English memory prose by default:

  - Japanese/Russian/etc. may appear only as terms, names, renderings, quotes, or metadata.
  - Long-term crystals must be 1-2 sentences.
  - Short-term memories must be 1-6 sentences.

- [x] Split phase prompt constants if still combined:

  - crystallization
  - concept discovery
  - rule discovery
  - consolidation/compaction
  - decay/reinforcement review

- [x] In dream response parsing, enforce:

  - missing crystal/rule/concept/facet content is hard reject
  - malformed optional fields are best-effort parsed
  - malformed accepted entries receive confidence penalty
  - provider-generated inferred additions become `thought` crystals with low confidence and `is_inferred=true`

- [x] Add a `DreamParseWarning` model if missing:

  ```python
  @dataclass(frozen=True)
  class DreamParseWarning:
      entry_path: str
      code: str
      message: str
      confidence_penalty: float
  ```

- [x] Store parse warnings in dream audit details.

- [x] Add `tests/test_dream_parse_penalties.py`.

- [x] Test malformed optional facet metadata is accepted with lower confidence.

- [x] Test missing content is rejected.

- [x] Test provider-suggested thoughts become low-confidence inferred thought crystals.

- [x] Test active rule crystals created from user-rule short-term memory have higher credibility than thoughts.

- [x] Run:

  ```bash
  uv run pytest tests/test_dream_parse_penalties.py
  ```

### 12. Bound Dream Affected Sets and Complete Audit Records

- [x] Open dream service code in `src/hieronymus/dreaming.py` and audit store code.

- [x] Define bounded affected-set config defaults if not already present:

  ```python
  max_changed_crystals_per_cycle = 200
  max_related_concepts_per_cycle = 80
  max_related_crystals_per_concept = 20
  max_total_affected_crystals = 500
  ```

- [x] Ensure each dream cycle processes at most `max_short_term_memories_per_cycle` short-term memories per LLM crystallization prompt.

- [x] Ensure scheduled, urgent, and manual `Dream all` drain short-term memory through successive capped batches until no eligible memories remain.

- [x] Manual `Dream all` must process the final small batch even when it is below the minimum short-term count.

- [x] Scheduled dreaming must respect the minimum threshold until the configured stale-cycle override fires; default is the sixth `NOT_ENOUGH_MEMORIES` check after five skipped cycles.

- [x] Urgent dreaming starts when short-term count reaches the max cap and drains all batches.

- [x] Complete audit records must include:

  - trigger type
  - threshold state
  - selected short-term memory ids
  - phase name
  - prompt version
  - provider profile
  - model
  - request summary
  - response summary
  - parse warnings
  - accepted entries
  - rejected entries
  - confidence penalties
  - created crystals
  - created concepts
  - created facets
  - created links
  - superseded crystals
  - reinforced crystals
  - decayed crystals
  - searched related candidates
  - affected memory set
  - skipped candidates and reasons

- [x] Add `tests/test_dream_bounded_audit.py`.

- [x] Test scheduled threshold, stale-cycle override, urgent cap, and manual final-batch behavior.

- [x] Test affected set stays within configured caps.

- [x] Test audit lookup returns all phase entries and parse warning records for admin display.

- [x] Run:

  ```bash
  uv run pytest tests/test_dream_bounded_audit.py
  ```

### 13. Update Admin Contracts for Memory, Concepts, and Audits

- [x] Open current admin/TUI modules under `src/hieronymus/admin*`, `src/hieronymus/tui*`, or the equivalent package paths.

- [x] Add backend commands/view models for:

  - header status with logo metadata
  - crystal list with decay/reinforce controls
  - concept list/detail with CRUD, decay/reinforce where applicable, rename, merge, archive
  - facet list/editor for each concept
  - short-term memory list with remove action
  - current short-term status pane on all pages
  - dream process status: `IDLE` or `WORKING` with current phase/progress
  - dream audit list/detail
  - config editor for named provider profiles, workflow model selection, prompts, thresholds, and model-cache warnings

- [x] Do not add a manual promote-to-rule action.

- [x] Add admin command for user correction that creates short-term memory only.

- [x] Add admin command for `Dream all`, enabled even for final small batches.

- [x] For scheduled/urgent dreaming, admin must display drain progress until short-term memory is empty.

- [x] Add keyboard navigation contracts in tests if the TUI already has testable reducers/store state.

- [x] Add `tests/test_admin_memory_contracts.py`.

- [x] Test audit list/detail can display a dream cycle with multiple phases.

- [x] Test manual correction creates short-term memory and not a rule crystal.

- [x] Test concept/facet admin commands call the primitive stores.

- [x] Run:

  ```bash
  uv run pytest tests/test_admin_memory_contracts.py
  ```

### 14. Update Documentation and CONTEXT Cross-References

- [x] Update `docs/memory-dreaming.md` to describe:

  - language-neutral series
  - concepts as durable identity anchors
  - facets
  - semantic tags
  - story scopes as boosts
  - thought memories
  - malformed dream output penalties
  - affected memory sets
  - audit contents

- [x] Update `docs/usage.md` so current examples use primitive storage/recall tools and agent skill workflows instead of treating `hieronymus_read` / `hieronymus_learn` as the preferred interface.

- [x] Update `docs/translation-workspace-integration.md` with the new correction workflow:

  ```text
  Store "User told me to ..." as short-term memory with user_rule credibility. Dreaming converts it into a rule crystal when the next cycle runs.
  ```

- [x] Update `CONTEXT.md` only if implementation reveals a terminology mismatch. Keep existing decisions intact:

  - concept is first-class
  - facets hold forms/renderings/notes
  - active rule crystals do not decay
  - story scopes boost, not filter
  - corrections enter short-term memory

- [x] Add or update examples showing a concept with:

  - English canonical name
  - Japanese source form
  - Russian rendering
  - semantic tag `talent`
  - story scope `book:5/chapter:5`

- [x] Run documentation checks available in the repo. If no docs checker exists, run:

  ```bash
  rg -n "TODO|TBD|strict termbase|promote to rule|source_language.*target_language" docs CONTEXT.md
  ```

- [x] Review every match and either update stale wording or leave it only where explicitly marked as compatibility.

### 15. Final Full-Repo Verification

- [x] Run focused tests added in this plan:

  ```bash
  uv run pytest \
    tests/test_memory_schema_metadata.py \
    tests/test_series_language_tags.py \
    tests/test_short_term_metadata.py \
    tests/test_concept_lifecycle.py \
    tests/test_concept_facets_multilingual.py \
    tests/test_concept_specific_rule_validation.py \
    tests/test_recall_enriched_memory.py \
    tests/test_mcp_memory_primitives.py \
    tests/test_mcp_read_learn_compatibility.py \
    tests/test_memory_graph_migration.py \
    tests/test_dream_parse_penalties.py \
    tests/test_dream_bounded_audit.py \
    tests/test_admin_memory_contracts.py
  ```

- [x] Run the full test suite:

  ```bash
  uv run pytest
  ```

- [x] Run lint:

  ```bash
  uv run ruff check .
  ```

- [x] Run format check:

  ```bash
  uv run ruff format --check .
  ```

- [x] Inspect changed files:

  ```bash
  git diff --stat
  git diff -- docs/superpowers/specs/2026-06-08-hieronymus-multilingual-memory-design.md CONTEXT.md docs src tests
  ```

- [x] Confirm no unintentional write occurred outside `/home/inky/Development/hieronymus`.

- [x] Confirm compatibility wrappers are clearly marked and that new docs recommend primitive MCP tools plus agent skills.

- [x] Commit the completed implementation with a message such as:

  ```bash
  git add docs src tests CONTEXT.md
  git commit -m "feat: complete multilingual memory graph gaps"
  ```

## Design Guardrails for Implementers

- Concepts are durable identity anchors. They do not ordinary-decay. Archive, merge, split, or supersede them explicitly.
- Concept facets are not second-class aliases. They are scoped facts/forms/renderings/notes attached to a concept.
- Semantic tags are freeform and LLM-managed/user-editable. Do not hard-code domain-specific tags such as `book`, `talent`, or `subskill` into schema rules.
- Story scopes are freeform relevance signals. They boost recall and validation ranking but do not filter by themselves.
- Active rule crystals are deterministic validation material. They do not decay while active, but can be superseded, archived, or consolidated.
- User corrections do not mutate long-term truth immediately. They become high-credibility short-term memories and dreaming crystallizes them.
- Thought memories are allowed, but they start with low confidence, are marked inferred, and grow or decay through normal evidence.
- English is the default prose language for memory content. Other languages are preserved as terms, renderings, quotes, facets, or metadata.
- Malformed LLM output is best-effort only when content exists. Missing content is rejected.
- Recall must search both short-term and long-term memory and clearly mark which tier each result came from.

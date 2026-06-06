# Hieronymus Memory Housekeeping and Dreaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild Hieronymus storage around one global SQLite brain and implement task-scoped short-term memory, long-term crystals, weighted recall, feedback events, cycle-based decay, and deterministic dreaming hooks that can later call an external LLM.

**Architecture:** Replace the MVP per-series database layout with a single global database under the configured data root. Keep strict terminology deterministic and explicit, while fuzzy memory becomes a scored advisory layer. Dreaming consumes completed task workspaces and emits validated crystals plus strict concept/rendering proposals; proposals do not become mandatory termbase entries automatically.

**Tech Stack:** Python 3.12+, uv, SQLite/FTS5, pytest, ruff, Click CLI, existing MCP server style.

---

## Ground Rules

- Rebuild the schema in place. There is no runtime-memory migration requirement because no production memories exist yet.
- Store all project data in one global SQLite file, `hieronymus.sqlite`, under `HIERONYMUS_DATA_ROOT` or `~/.hieronymus`.
- Series, language pairs, volumes, chapters, and domains are scope fields inside the global store, not separate database files.
- Strict terminology remains deterministic. Dreaming may create strict concept/rendering proposals, but only explicit acceptance can make them active contract entries.
- Long-term crystals are advisory by default. Lessons may be injected into prompts and later soft validation, but they must not create hard validation failures.
- Recall creates short-term activations but does not reinforce strength or confidence by itself. Recall protects the crystal from decay during the next dream cycle.
- Explicit user feedback applies immediately. Passive workflow evidence is applied during dreaming.
- External LLM integration is represented by a provider protocol plus deterministic test provider in this plan. Add Gemini or another provider after the local contract is stable.

## Target File Structure

```text
src/hieronymus/
├── cli.py
├── config.py
├── concepts.py
├── crystals.py
├── db.py
├── dreaming.py
├── mcp_server.py
├── memory.py
├── memory_models.py
├── migrations/
│   └── global.sql
├── recall.py
├── registry.py
├── scoring.py
├── termbase.py
└── workspace.py
tests/
├── test_concepts.py
├── test_crystals.py
├── test_dreaming.py
├── test_global_registry.py
├── test_recall.py
├── test_scoring.py
└── test_workspace.py
```

Existing MVP test files should be edited rather than preserved as historical compatibility tests. The feature target is the rebuilt structure, not the old per-series persistence shape.

## Task 1: Rebuild Config and Global Schema

**Files:**
- Modify: `src/hieronymus/config.py`
- Modify: `src/hieronymus/db.py`
- Replace: `src/hieronymus/migrations/registry.sql`
- Replace: `src/hieronymus/migrations/series.sql`
- Create: `src/hieronymus/migrations/global.sql`
- Modify: `tests/test_config.py`
- Create: `tests/test_global_registry.py`

- [ ] **Step 1: Write config test for the global database path**

Add or update `tests/test_config.py`:

```python
from pathlib import Path

from hieronymus.config import HieronymusConfig, load_config


def test_config_exposes_single_global_database(tmp_path: Path):
    config = HieronymusConfig(data_root=tmp_path)

    assert config.database_path == tmp_path / "hieronymus.sqlite"


def test_load_config_uses_explicit_data_root(tmp_path: Path):
    config = load_config(str(tmp_path))

    assert config.data_root == tmp_path
    assert config.database_path == tmp_path / "hieronymus.sqlite"
```

- [ ] **Step 2: Change `HieronymusConfig`**

Replace `registry_path` and `series_dir` use with a global path. Keep no compatibility alias unless an updated test still needs it.

```python
@dataclass(frozen=True)
class HieronymusConfig:
    data_root: Path

    @property
    def database_path(self) -> Path:
        return self.data_root / "hieronymus.sqlite"
```

- [ ] **Step 3: Replace migrations with `global.sql`**

Create `src/hieronymus/migrations/global.sql`:

```sql
create table if not exists series (
  id integer primary key,
  slug text unique not null,
  title text not null,
  default_source_language text not null,
  default_target_language text not null,
  created_at text not null,
  updated_at text not null
);

create table if not exists task_sessions (
  id integer primary key,
  series_slug text not null,
  source_language text not null,
  target_language text not null,
  task_type text not null,
  volume text not null default '',
  chapter text not null default '',
  status text not null,
  cycle_id integer,
  created_at text not null,
  completed_at text,
  foreign key(series_slug) references series(slug)
);

create table if not exists short_term_memories (
  id integer primary key,
  session_id integer not null references task_sessions(id) on delete cascade,
  source_role text not null,
  kind text not null,
  text text not null,
  source_ref text not null default '',
  metadata_json text not null default '{}',
  created_at text not null,
  archived_at text
);

create virtual table if not exists short_term_memories_fts using fts5(
  text,
  content='short_term_memories',
  content_rowid='id'
);

create table if not exists crystals (
  id integer primary key,
  crystal_type text not null,
  text text not null,
  title text not null default '',
  scope_type text not null,
  scope_key text not null default '',
  series_slug text not null default '',
  source_language text not null default '',
  target_language text not null default '',
  tags_json text not null default '[]',
  strength real not null,
  confidence real not null,
  status text not null,
  created_cycle integer not null default 0,
  last_activated_cycle integer,
  last_reinforced_cycle integer,
  created_at text not null,
  updated_at text not null
);

create virtual table if not exists crystals_fts using fts5(
  title,
  text,
  content='crystals',
  content_rowid='id'
);

create table if not exists crystal_sources (
  crystal_id integer not null references crystals(id) on delete cascade,
  short_term_memory_id integer not null references short_term_memories(id),
  primary key(crystal_id, short_term_memory_id)
);

create table if not exists crystal_links (
  source_crystal_id integer not null references crystals(id) on delete cascade,
  target_crystal_id integer not null references crystals(id) on delete cascade,
  link_type text not null,
  primary key(source_crystal_id, target_crystal_id, link_type)
);

create table if not exists crystal_activations (
  id integer primary key,
  crystal_id integer not null references crystals(id),
  session_id integer not null references task_sessions(id) on delete cascade,
  recall_query text not null,
  rank integer not null,
  score real not null,
  reason text not null default '',
  cycle_id integer,
  created_at text not null
);

create table if not exists memory_events (
  id integer primary key,
  crystal_id integer references crystals(id),
  session_id integer references task_sessions(id),
  event_type text not null,
  source_role text not null,
  evidence text not null default '',
  strength_delta real not null default 0,
  confidence_delta real not null default 0,
  applied integer not null default 0,
  cycle_id integer,
  created_at text not null
);

create table if not exists dream_runs (
  id integer primary key,
  cycle_id integer not null unique,
  status text not null,
  provider text not null,
  input_count integer not null default 0,
  created_crystal_count integer not null default 0,
  proposal_count integer not null default 0,
  error text not null default '',
  created_at text not null,
  completed_at text
);

create table if not exists strict_concept_proposals (
  id integer primary key,
  dream_run_id integer references dream_runs(id),
  series_slug text not null default '',
  source_language text not null,
  target_language text not null,
  concept_text text not null,
  source_form text not null,
  canonical_rendering text not null,
  approved_variants_json text not null default '[]',
  forbidden_variants_json text not null default '[]',
  rationale text not null default '',
  status text not null,
  created_at text not null,
  updated_at text not null
);
```

Delete or empty the old migration files only after updating all code paths to call `global.sql`.

- [ ] **Step 4: Update migration helper call sites**

`db.apply_migration()` can stay unchanged. Callers must request `global.sql`.

- [ ] **Step 5: Run targeted tests**

Run: `uv run pytest tests/test_config.py -v`

Expected: config tests pass and no code references `registry_path` or `series_dir`.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/config.py src/hieronymus/db.py src/hieronymus/migrations tests/test_config.py
git commit -m "feat: rebuild storage around global database"
```

## Task 2: Rebuild the Registry on the Global Store

**Files:**
- Modify: `src/hieronymus/registry.py`
- Modify: `src/hieronymus/models.py`
- Modify: `tests/test_registry.py`
- Use: `tests/test_global_registry.py`

- [ ] **Step 1: Write global registry tests**

`tests/test_global_registry.py`:

```python
from hieronymus.registry import Registry


def test_create_series_records_scope_in_global_database(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    assert series.slug == "only-sense-online"
    assert series.source_language == "ja"
    assert series.target_language == "en"
    assert config.database_path.exists()
    assert registry.get_series("only-sense-online").title == "Only Sense Online"


def test_series_slug_is_unique_and_upsert_updates_title(config):
    registry = Registry(config)
    registry.create_series(
        slug="death-march",
        title="Death March",
        source_language="ja",
        target_language="en",
    )
    series = registry.create_series(
        slug="death-march",
        title="Death March to the Parallel World Rhapsody",
        source_language="ja",
        target_language="en",
    )

    assert series.title == "Death March to the Parallel World Rhapsody"
    assert len(registry.list_series()) == 1
```

- [ ] **Step 2: Update `Series`**

Remove `database_path` from `Series`. Services should receive `HieronymusConfig` plus explicit scope instead of a per-series path.

```python
@dataclass(frozen=True)
class Series:
    slug: str
    title: str
    source_language: str
    target_language: str
```

- [ ] **Step 3: Implement global registry**

`Registry.__init__()` should initialize `global.sql` on `config.database_path`. `create_series()`, `get_series()`, and `list_series()` should read/write the global `series` table only.

- [ ] **Step 4: Run registry tests**

Run: `uv run pytest tests/test_global_registry.py tests/test_registry.py -v`

Expected: all registry tests pass after old per-series path assertions are removed.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/registry.py src/hieronymus/models.py tests/test_registry.py tests/test_global_registry.py
git commit -m "feat: store series registry in global database"
```

## Task 3: Add Memory Domain Models

**Files:**
- Create: `src/hieronymus/memory_models.py`
- Modify: `src/hieronymus/models.py`
- Create: `tests/test_memory_models.py`

- [ ] **Step 1: Add model tests**

`tests/test_memory_models.py`:

```python
from hieronymus.memory_models import TranslationContext


def test_translation_context_defaults_to_series_scope():
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )

    assert context.scope_key == "series:only-sense-online"
```

- [ ] **Step 2: Implement domain dataclasses**

`src/hieronymus/memory_models.py`:

```python
from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class TranslationContext:
    series_slug: str
    source_language: str
    target_language: str
    task_type: str
    volume: str = ""
    chapter: str = ""
    tags: tuple[str, ...] = ()

    @property
    def scope_key(self) -> str:
        return f"series:{self.series_slug}"


@dataclass(frozen=True)
class TaskSessionRecord:
    id: int
    context: TranslationContext
    status: str
    cycle_id: int | None


@dataclass(frozen=True)
class ShortTermMemoryRecord:
    id: int
    session_id: int
    source_role: str
    kind: str
    text: str
    source_ref: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class CrystalRecord:
    id: int
    crystal_type: str
    text: str
    title: str
    scope_type: str
    scope_key: str
    series_slug: str
    source_language: str
    target_language: str
    strength: float
    confidence: float
    status: str


@dataclass(frozen=True)
class RecallResult:
    crystal: CrystalRecord
    rank: int
    score: float
    reason: str
```

- [ ] **Step 3: Run model tests**

Run: `uv run pytest tests/test_memory_models.py -v`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/hieronymus/memory_models.py tests/test_memory_models.py
git commit -m "feat: add memory domain models"
```

## Task 4: Implement Task Sessions and Short-Term Memory

**Files:**
- Create: `src/hieronymus/workspace.py`
- Create: `tests/test_workspace.py`

- [ ] **Step 1: Write workspace tests**

`tests/test_workspace.py`:

```python
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def test_workspace_records_short_term_memory(config):
    Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    workspace = WorkspaceStore(config)
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        task_type="translation",
        volume="01",
        chapter="002",
    )

    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mundane",
        kind="translation",
        text="攻撃力上昇 was translated as ATK Up in this chapter.",
        source_ref="chapter-002",
    )

    memories = workspace.list_short_term_memories(session.id)
    assert memory_id == memories[0].id
    assert memories[0].source_role == "mundane"
```

- [ ] **Step 2: Implement `WorkspaceStore`**

Implement:

```python
class WorkspaceStore:
    def __init__(self, config: HieronymusConfig) -> None: ...
    def start_session(self, context: TranslationContext) -> TaskSessionRecord: ...
    def complete_session(self, session_id: int) -> None: ...
    def add_short_term_memory(
        self,
        session_id: int,
        *,
        source_role: str,
        kind: str,
        text: str,
        source_ref: str = "",
        metadata: dict[str, object] | None = None,
    ) -> int: ...
    def list_short_term_memories(self, session_id: int) -> list[ShortTermMemoryRecord]: ...
```

Use `json.dumps(..., ensure_ascii=False, sort_keys=True)` for metadata. Insert into `short_term_memories_fts` for every short-term memory row.

- [ ] **Step 3: Validate roles and states**

Allowed roles: `mundane`, `mentor`, `user`, `system`.

Allowed session states: `active`, `completed`, `dreamed`.

Raise `ValueError` for empty text, unknown role, or unknown status.

- [ ] **Step 4: Run workspace tests**

Run: `uv run pytest tests/test_workspace.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/workspace.py tests/test_workspace.py
git commit -m "feat: record task-scoped short-term memory"
```

## Task 5: Implement Long-Term Crystal Storage

**Files:**
- Create: `src/hieronymus/crystals.py`
- Create: `tests/test_crystals.py`

- [ ] **Step 1: Write crystal tests**

`tests/test_crystals.py`:

```python
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry


def test_add_and_search_series_crystal(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    store = CrystalStore(config)
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="If a Japanese cultural term may be unfamiliar, consider defining it in narration or a footnote.",
        title="Define unfamiliar cultural terms",
        strength=0.55,
        confidence=0.70,
    )

    results = store.search(context, "cultural term", limit=5)

    assert results[0].id == crystal_id
    assert results[0].crystal_type == "lesson"
```

- [ ] **Step 2: Implement `CrystalStore`**

Implement:

```python
class CrystalStore:
    def __init__(self, config: HieronymusConfig) -> None: ...
    def add_crystal(
        self,
        context: TranslationContext,
        *,
        crystal_type: str,
        text: str,
        title: str = "",
        strength: float = 0.5,
        confidence: float = 0.5,
        status: str = "active",
        source_memory_ids: list[int] | None = None,
    ) -> int: ...
    def get(self, crystal_id: int) -> CrystalRecord: ...
    def search(self, context: TranslationContext, query: str, *, limit: int = 10) -> list[CrystalRecord]: ...
```

Allowed crystal types: `lesson`, `concept`, `erudition`.

Allowed statuses: `active`, `candidate`, `archived`, `rejected`.

Clamp `strength` and `confidence` into `0.0 <= value <= 1.0`.

- [ ] **Step 3: Implement scoped search**

Search must include:

- exact series scope matches,
- global active lessons where `scope_type = 'global'`,
- source/target language matches or empty language fields.

Order by a weighted score:

```text
score = bm25_score_component + (strength * 0.35) + (confidence * 0.20) + scope_bonus
```

Use `bm25(crystals_fts)` for FTS ranking and invert it into a positive component in Python or SQL. Keep the formula in one helper so tests can pin behavior.

- [ ] **Step 4: Run crystal tests**

Run: `uv run pytest tests/test_crystals.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/crystals.py tests/test_crystals.py
git commit -m "feat: store scored long-term crystals"
```

## Task 6: Implement Weighted Recall and Activation Traces

**Files:**
- Create: `src/hieronymus/recall.py`
- Create: `tests/test_recall.py`

- [ ] **Step 1: Write recall tests**

`tests/test_recall.py`:

```python
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def test_recall_records_activation_without_reinforcing(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    crystal_id = crystals.add_crystal(
        context,
        crystal_type="concept",
        text="Gantz is a martial arts user.",
        strength=0.6,
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "Gantz fighting style", limit=10)
    after = crystals.get(crystal_id)

    assert results[0].crystal.id == crystal_id
    assert after.strength == 0.6
    assert after.confidence == 0.8
```

- [ ] **Step 2: Implement `RecallService`**

```python
class RecallService:
    def __init__(self, config: HieronymusConfig) -> None: ...
    def recall(
        self,
        session_id: int,
        context: TranslationContext,
        query: str,
        *,
        limit: int = 10,
    ) -> list[RecallResult]: ...
```

For each returned crystal, insert one row into `crystal_activations` with the rank, weighted score, query, and current cycle id if known.

- [ ] **Step 3: Add activation short-term traces**

After inserting activations, add one `short_term_memories` row per recalled crystal with:

```text
source_role = system
kind = recalled_crystal
text = crystal.text
metadata_json = {"crystal_id": <id>, "rank": <rank>, "score": <score>}
```

This makes recall visible in the task workspace while preserving the rule that recall is not reinforcement.

- [ ] **Step 4: Run recall tests**

Run: `uv run pytest tests/test_recall.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/recall.py tests/test_recall.py
git commit -m "feat: recall crystals into task workspace"
```

## Task 7: Implement Feedback Events and Immediate User Scoring

**Files:**
- Create: `src/hieronymus/scoring.py`
- Create: `tests/test_scoring.py`

- [ ] **Step 1: Write scoring tests**

`tests/test_scoring.py`:

```python
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.scoring import FeedbackStore


def test_user_confirmation_immediately_reinforces_crystal(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="review",
    )
    crystals = CrystalStore(config)
    crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Prefer concise game-like system messages.",
        strength=0.4,
        confidence=0.5,
    )

    FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type="confirmed_by_user",
        source_role="user",
        evidence="User explicitly approved the lesson.",
    )

    updated = crystals.get(crystal_id)
    assert updated.strength > 0.4
    assert updated.confidence > 0.5
```

- [ ] **Step 2: Implement feedback deltas**

In `scoring.py`, define:

```python
IMMEDIATE_EVENT_DELTAS = {
    "confirmed_by_user": (0.15, 0.20),
    "contradicted_by_user": (-0.20, -0.25),
    "deleted_by_user": (-0.50, -0.35),
}

PASSIVE_EVENT_DELTAS = {
    "cited": (0.03, 0.02),
    "used_in_translation": (0.05, 0.02),
    "passed_review": (0.07, 0.05),
    "caused_correction": (-0.10, -0.12),
    "superseded": (-0.12, -0.05),
}
```

`FeedbackStore.record()` should insert every event. For immediate event types, apply the score delta inside the same transaction and set `applied = 1`.

- [ ] **Step 3: Add deletion semantics**

For `deleted_by_user`, reduce scores and set `status = 'archived'` when strength drops below `0.05`. Do not physically delete the crystal; the TUI can later display provenance and archived state.

- [ ] **Step 4: Run scoring tests**

Run: `uv run pytest tests/test_scoring.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/scoring.py tests/test_scoring.py
git commit -m "feat: reinforce crystals through feedback events"
```

## Task 8: Implement Dream Provider Contract and Deterministic Dreaming

**Files:**
- Create: `src/hieronymus/dreaming.py`
- Create: `tests/test_dreaming.py`

- [ ] **Step 1: Write dreaming tests**

`tests/test_dreaming.py`:

```python
from hieronymus.crystals import CrystalStore
from hieronymus.dreaming import DeterministicDreamProvider, DreamService
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def test_dreaming_crystallizes_completed_short_term_memory(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="User prefers defining obscure Japanese cultural terms in narration or footnotes.",
    )
    workspace.complete_session(session.id)

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()
    crystals = CrystalStore(config).search(context, "obscure Japanese cultural terms")

    assert run.status == "completed"
    assert crystals[0].crystal_type == "lesson"
```

- [ ] **Step 2: Define provider dataclasses**

In `dreaming.py`:

```python
@dataclass(frozen=True)
class DreamCrystalCandidate:
    crystal_type: str
    title: str
    text: str
    strength: float
    confidence: float
    source_memory_ids: list[int]


@dataclass(frozen=True)
class DreamConceptProposal:
    series_slug: str
    source_language: str
    target_language: str
    concept_text: str
    source_form: str
    canonical_rendering: str
    approved_variants: list[str]
    forbidden_variants: list[str]
    rationale: str


@dataclass(frozen=True)
class DreamOutput:
    crystals: list[DreamCrystalCandidate]
    concept_proposals: list[DreamConceptProposal]
```

- [ ] **Step 3: Define provider protocol**

```python
class DreamProvider(Protocol):
    name: str

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput: ...
```

- [ ] **Step 4: Implement deterministic provider**

`DeterministicDreamProvider` should be simple and testable:

- user-role memories become lesson candidates,
- mentor-role memories become erudition candidates,
- mundane memories become concept candidates,
- each candidate text is normalized to 1-3 sentences by taking the first three sentence-like chunks.

This is not the final intelligence. It establishes the persistence and validation contract for a later Gemini-backed provider.

- [ ] **Step 5: Implement `DreamService.run_cycle()`**

Behavior:

1. Create the next integer cycle id as `max(cycle_id) + 1` from `dream_runs`.
2. Load completed sessions with `cycle_id is null`.
3. Group short-term memories by session/context.
4. Call the provider for each group.
5. Validate candidate type, non-empty text, score ranges, and source ids.
6. Insert crystals and `crystal_sources`.
7. Insert strict concept proposals with `status = 'pending'`.
8. Mark sessions as `dreamed` and set their `cycle_id`.
9. Complete the dream run with counts.

- [ ] **Step 6: Run dreaming tests**

Run: `uv run pytest tests/test_dreaming.py -v`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/dreaming.py tests/test_dreaming.py
git commit -m "feat: crystallize short-term memory during dream cycles"
```

## Task 9: Implement Cycle-Based Decay and Passive Reinforcement

**Files:**
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/scoring.py`
- Modify: `tests/test_dreaming.py`
- Modify: `tests/test_scoring.py`

- [ ] **Step 1: Add decay tests**

Add to `tests/test_dreaming.py`:

```python
def test_dreaming_decays_inactive_crystals_but_not_recalled_crystals(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug="oso",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )
    crystals = CrystalStore(config)
    inactive_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Inactive lesson.",
        strength=0.50,
        confidence=0.90,
    )
    recalled_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Recall-protected lesson.",
        strength=0.50,
        confidence=0.90,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    RecallService(config).recall(session.id, context, "Recall-protected", limit=1)
    workspace.complete_session(session.id)

    DreamService(config, DeterministicDreamProvider()).run_cycle()

    assert crystals.get(inactive_id).strength < 0.50
    assert crystals.get(recalled_id).strength == 0.50
```

- [ ] **Step 2: Apply passive events during dreaming**

At the end of each dream cycle:

- load `memory_events` where `applied = 0`,
- apply `PASSIVE_EVENT_DELTAS`,
- set `applied = 1` and `cycle_id = current_cycle_id`.

- [ ] **Step 3: Apply decay after passive events**

Decay only active/candidate crystals not protected by:

- `last_activated_cycle = current_cycle_id`,
- `last_reinforced_cycle = current_cycle_id`,
- positive event applied in the current cycle.

Use these initial constants:

```python
STRENGTH_DECAY_PER_CYCLE = 0.03
CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW = 0.20
CONFIDENCE_DECAY_PER_CYCLE = 0.01
```

Reduce confidence only when strength is already below `0.20`.

- [ ] **Step 4: Run decay and scoring tests**

Run: `uv run pytest tests/test_dreaming.py tests/test_scoring.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/scoring.py tests/test_dreaming.py tests/test_scoring.py
git commit -m "feat: apply cycle-based memory reinforcement and decay"
```

## Task 10: Add Strict Concept Proposal Management

**Files:**
- Create: `src/hieronymus/concepts.py`
- Create: `tests/test_concepts.py`
- Modify: `src/hieronymus/dreaming.py`

- [ ] **Step 1: Write concept proposal tests**

`tests/test_concepts.py`:

```python
from hieronymus.concepts import ConceptProposalStore
from hieronymus.registry import Registry


def test_concept_proposals_are_series_and_language_pair_scoped(config):
    Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    store = ConceptProposalStore(config)

    proposal_id = store.create(
        dream_run_id=None,
        series_slug="oso",
        source_language="ja",
        target_language="en",
        concept_text="Yun's attack buff sense",
        source_form="攻撃力上昇",
        canonical_rendering="ATK Up",
        approved_variants=["Attack Up"],
        forbidden_variants=["Attack Increase"],
        rationale="Existing translation evidence uses ATK Up.",
    )
    proposal = store.get(proposal_id)

    assert proposal.series_slug == "oso"
    assert proposal.source_language == "ja"
    assert proposal.target_language == "en"
    assert proposal.status == "pending"
```

- [ ] **Step 2: Implement proposal model and store**

Add a dataclass in `concepts.py`:

```python
@dataclass(frozen=True)
class StrictConceptProposal:
    id: int
    series_slug: str
    source_language: str
    target_language: str
    concept_text: str
    source_form: str
    canonical_rendering: str
    approved_variants: list[str]
    forbidden_variants: list[str]
    rationale: str
    status: str
```

Implement `create()`, `get()`, `list_pending()`, `approve()`, and `reject()`.

- [ ] **Step 3: Keep proposal acceptance explicit**

`approve()` should only change proposal status to `approved`. Do not insert hard termbase entries in this task. The later Management TUI plan can wire proposal approval into strict concept/rendering creation with a visible user action.

- [ ] **Step 4: Wire dreaming to proposal store**

`DreamService.run_cycle()` should insert every valid `DreamConceptProposal` through `ConceptProposalStore.create()`.

- [ ] **Step 5: Run concept tests**

Run: `uv run pytest tests/test_concepts.py tests/test_dreaming.py -v`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/concepts.py src/hieronymus/dreaming.py tests/test_concepts.py tests/test_dreaming.py
git commit -m "feat: store strict concept proposals from dreams"
```

## Task 11: Rebuild Existing Memory and Termbase APIs on the Global Store

**Files:**
- Modify: `src/hieronymus/memory.py`
- Modify: `src/hieronymus/termbase.py`
- Modify: `tests/test_memory_search.py`
- Modify: `tests/test_termbase_contract.py`
- Modify: `tests/test_termbase_validate.py`

- [ ] **Step 1: Update fuzzy memory API to crystals**

`MemoryStore` should become a compatibility facade over `CrystalStore`:

```python
class MemoryStore:
    def __init__(self, config: HieronymusConfig, context: TranslationContext) -> None:
        self.config = config
        self.context = context

    def add(self, *, kind: str, text: str, source_ref: str = "", importance: int = 3) -> int:
        strength = min(max(importance / 5, 0.0), 1.0)
        return CrystalStore(self.config).add_crystal(
            self.context,
            crystal_type="erudition",
            text=text,
            title=kind,
            strength=strength,
            confidence=0.5,
        )
```

Update tests to construct `MemoryStore(config, context)` rather than the old per-series path constructor.

- [ ] **Step 2: Rebuild termbase tables inside `global.sql`**

Add deterministic strict term tables to `global.sql`:

```sql
create table if not exists strict_terms (
  id integer primary key,
  series_slug text not null references series(slug),
  source_language text not null,
  target_language text not null,
  category text not null,
  source_text text not null,
  canonical_translation text not null,
  status text not null,
  notes text not null default '',
  created_at text not null,
  updated_at text not null
);

create table if not exists strict_term_tags (
  term_id integer not null references strict_terms(id) on delete cascade,
  tag text not null,
  primary key(term_id, tag)
);

create table if not exists strict_term_aliases (
  id integer primary key,
  term_id integer not null references strict_terms(id) on delete cascade,
  language text not null,
  text text not null,
  kind text not null,
  case_sensitive integer not null default 1
);

create virtual table if not exists strict_terms_fts using fts5(
  source_text,
  canonical_translation,
  notes,
  content='strict_terms',
  content_rowid='id'
);
```

- [ ] **Step 3: Rebuild `Termbase` constructor**

Change `Termbase` to accept global config and context:

```python
class Termbase:
    def __init__(self, config: HieronymusConfig, context: TranslationContext) -> None:
        self.config = config
        self.context = context
```

Every termbase query must filter by `series_slug`, `source_language`, and `target_language`.

- [ ] **Step 4: Preserve deterministic validation behavior**

The existing public methods remain:

- `propose()`,
- `approve()`,
- `add_alias()`,
- `contract(raw_text)`,
- `validate(raw_text, translated_text)`.

They now read/write `strict_terms`, `strict_term_tags`, and `strict_term_aliases` in the global database.

- [ ] **Step 5: Run legacy API tests**

Run:

```bash
uv run pytest tests/test_memory_search.py tests/test_termbase_contract.py tests/test_termbase_validate.py -v
```

Expected: PASS after tests use `TranslationContext` and the global config constructor.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/memory.py src/hieronymus/termbase.py src/hieronymus/migrations/global.sql tests/test_memory_search.py tests/test_termbase_contract.py tests/test_termbase_validate.py
git commit -m "feat: rebuild memory and termbase APIs on global store"
```

## Task 12: Add CLI Commands for Sessions, Recall, Feedback, and Dreaming

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli.py`

- [ ] **Step 1: Add context construction helper**

In `cli.py`, create one helper used by all memory commands:

```python
def _context(
    *,
    series_slug: str,
    source_language: str,
    target_language: str,
    task_type: str,
    volume: str,
    chapter: str,
) -> TranslationContext:
    return TranslationContext(
        series_slug=series_slug,
        source_language=source_language,
        target_language=target_language,
        task_type=task_type,
        volume=volume,
        chapter=chapter,
    )
```

- [ ] **Step 2: Add CLI commands**

Add commands:

```text
session-start SERIES_SLUG --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
session-complete SESSION_ID
remember-short SESSION_ID --role user --kind correction --text "..."
recall SESSION_ID --series SERIES_SLUG --query "..." --source-language ja --target-language en
feedback CRYSTAL_ID --event confirmed_by_user --role user --evidence "..."
dream --provider deterministic
```

The `recall` command has one `--target-language` option.

- [ ] **Step 3: Add CLI tests for JSON shape**

Test at least:

- `session-start` returns `{"session_id": <int>}`,
- `remember-short` returns `{"memory_id": <int>}`,
- `dream` returns `{"cycle_id": <int>, "status": "completed"}`.

- [ ] **Step 4: Run CLI tests**

Run: `uv run pytest tests/test_cli.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/cli.py tests/test_cli.py
git commit -m "feat: add memory dreaming CLI commands"
```

## Task 13: Add MCP Tools for Agent Workflows

**Files:**
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Add MCP tool wrappers**

Expose these tools:

```text
hieronymus_session_start
hieronymus_session_complete
hieronymus_short_term_add
hieronymus_recall
hieronymus_feedback
hieronymus_dream
hieronymus_concept_proposals_list
```

Each tool should call the same services as CLI commands.

- [ ] **Step 2: Keep termbase tools scoped**

Update existing termbase MCP tools to accept or derive:

- `series_slug`,
- `source_language`,
- `target_language`,
- optional `volume`,
- optional `chapter`.

They should instantiate `Termbase(config, context)`.

- [ ] **Step 3: Add MCP smoke tests**

Update `tests/test_mcp_server.py` to verify every new tool wrapper is importable and registered in the same structure used by the current MCP server tests.

- [ ] **Step 4: Run MCP tests**

Run: `uv run pytest tests/test_mcp_server.py -v`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/mcp_server.py tests/test_mcp_server.py
git commit -m "feat: expose memory dreaming MCP tools"
```

## Task 14: Documentation and Examples

**Files:**
- Create or modify: `docs/memory-dreaming.md`
- Modify: `docs/usage.md` if it exists

- [ ] **Step 1: Document the mental model**

`docs/memory-dreaming.md` must cover:

- global store,
- task sessions as the activation surface,
- short-term source roles,
- crystals as long-term memory,
- lessons/concepts/erudition,
- recall versus reinforcement,
- dream cycles,
- strict concept proposals,
- why decay is cycle-based.

- [ ] **Step 2: Add CLI workflow example**

Include this example:

```bash
hieronymus init-series oso --title "Only Sense Online" --source-language ja --target-language en
hieronymus session-start oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
hieronymus remember-short 1 --role user --kind correction --text "Define obscure Japanese cultural terms when the average English reader may not know them."
hieronymus session-complete 1
hieronymus dream --provider deterministic
hieronymus recall 2 --series oso --source-language ja --target-language en --task-type translation --query "cultural terms and footnotes"
```

- [ ] **Step 3: State strictness boundary**

Document this exact rule:

```text
Crystals are advisory. Strict term/rendering proposals are not mandatory until an explicit user or management workflow accepts them into the deterministic termbase.
```

- [ ] **Step 4: Commit**

```bash
git add docs/memory-dreaming.md docs/usage.md
git commit -m "docs: explain memory dreaming workflow"
```

## Task 15: Final Verification

**Files:**
- Modify only if verification finds issues.

- [ ] **Step 1: Search for old per-series database assumptions**

Run:

```bash
rg "registry_path|series_dir|series\\.sqlite|MemoryStore\\(series\\.database_path|Termbase\\(series\\.database_path" src tests
```

Expected: no source or test references to removed per-series runtime layout. Review docs separately and keep any MVP per-series references clearly marked as replaced by the global store.

- [ ] **Step 2: Run tests**

Run: `uv run pytest`

Expected: all tests pass.

- [ ] **Step 3: Run lint**

Run: `uv run ruff check .`

Expected: no violations.

- [ ] **Step 4: Run format check**

Run: `uv run ruff format --check .`

Expected: no formatting changes needed.

- [ ] **Step 5: Commit verification fixes**

If fixes were required:

```bash
git add .
git commit -m "test: verify memory dreaming rebuild"
```

If no fixes were required, do not create an empty commit.

## Self-Review

Spec coverage:

- Global storage: Tasks 1 and 2 replace per-series persistence with one `hieronymus.sqlite`.
- No migration: The plan explicitly rebuilds schema and updates tests/code to the new structure.
- Long-term memory: Task 5 implements crystals with strength, confidence, type, status, and scope.
- Short-term memory: Task 4 implements task-scoped workspace entries with source roles.
- Recall: Task 6 implements weighted recall and activation traces without reinforcement.
- Feedback: Task 7 implements immediate user feedback and passive event recording.
- Dreaming: Task 8 implements dream cycles, provider protocol, deterministic provider, provenance, and output validation.
- Decay: Task 9 implements cycle-based strength-first decay and recall protection.
- Lessons/concepts/erudition: Tasks 5 and 8 model all three crystal types.
- Strict proposals: Task 10 stores scoped concept/rendering proposals without making them mandatory.
- Agent integration surface: Tasks 12 and 13 expose CLI and MCP workflows for later skills/hooks.
- Existing MVP behavior: Task 11 rebuilds memory and termbase APIs on the global store so strict validation remains deterministic.

Quality checks:

- Every implementation task has targeted tests and a commit point.
- The plan avoids external network/API requirements by using a provider protocol plus deterministic provider.
- Score formulas and decay constants are concrete enough for implementation and tests.
- The final verification commands match project `AGENTS.md`.

Known sequencing risk:

- Task 11 is the broadest step because it rewires existing termbase and memory APIs after the new storage model is in place. If implementation becomes too large for one worker, split it into two commits: memory facade first, strict termbase second.

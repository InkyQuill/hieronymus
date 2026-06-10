# Hieronymus Multilingual Memory Design Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or superpowers:executing-plans
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the old strict proposal memory model with a learning multilingual
memory system built from short-term memories, durable concepts, concept facets, rule
crystals, thought crystals, confidence-aware recall, and autonomous dreaming.

**Architecture:** Keep SQLite + FTS5 as the storage engine and model concepts as
first-class durable identity anchors with many-to-many links to crystals. Dreaming is
a provider-backed workflow runner with separate crystallization, relation discovery,
and reinforcement/compaction phases, while recall remains deterministic and combines
short-term and long-term keyword matches.

**Tech Stack:** Python 3.12+, SQLite FTS5, TOML via `tomllib`/`tomli_w`, pytest, uv,
existing MCP stdio server, existing CLI and TUI bridge contracts.

---

## Scope

This plan implements the backend/domain side of
`docs/superpowers/specs/2026-06-08-hieronymus-multilingual-memory-design.md` and the
shared terminology in `CONTEXT.md`. It includes the admin/config JSON contracts needed
by the future Ink React TUI, but it does not implement the React/Ink screens from
`docs/superpowers/specs/2026-06-08-hieronymus-ink-react-tui-migration.md`.

The existing Textual TUI may keep working through compatibility views while the Ink
migration is planned separately.

## File Structure

Create:

- `src/hieronymus/dream_config.py` - plaintext `dream.conf` models, parser,
  validation, redaction, and save/load helpers.
- `src/hieronymus/llm_cache.py` - `llmcache.tmp` model cache, stale checks, refresh
  result models, and JSON/TOML-safe serialization.
- `src/hieronymus/dream_workflows.py` - workflow names, default prompts, prompt
  assembly, provider workflow resolution, and workflow validation.
- `src/hieronymus/concept_models.py` - dataclasses for concepts, facets, tags, links,
  and rename records.
- `src/hieronymus/short_memory.py` - short-term memory validation and shared insert
  helpers.
- `src/hieronymus/dream_audit.py` - immutable dream audit append/read helpers with
  redaction.
- `tests/test_dream_config.py`
- `tests/test_llm_cache.py`
- `tests/test_concept_store.py`
- `tests/test_short_memory.py`
- `tests/test_combined_recall.py`
- `tests/test_dream_workflows.py`
- `tests/test_dream_audit.py`

Modify:

- `src/hieronymus/config.py` - add `dream_config_path` and `llm_cache_path`
  properties under `~/.config/hieronymus`.
- `src/hieronymus/migrations/global.sql` - add concepts, facets, tags, story scopes,
  richer crystals, workflow runs, and audit tables while preserving old tables for
  migration compatibility.
- `src/hieronymus/memory_models.py` - add explicit recall source, short-term status,
  concept, facet, and crystal metadata fields.
- `src/hieronymus/workspace.py` - route all short-term inserts through validation.
- `src/hieronymus/crystals.py` - support crystal kinds `lesson`, `rule`, `thought`,
  `observation`, `concept_note`; source credibility; story scopes; semantic tags;
  rule intent; supersession; and active-rule no-decay behavior.
- `src/hieronymus/concepts.py` - replace proposal-only store with first-class concept
  store while keeping compatibility methods for old callers during migration.
- `src/hieronymus/recall.py` - combine long-term and short-term keyword recall and
  record activations only for long-term results.
- `src/hieronymus/dream_providers.py` - resolve named provider profiles from
  `dream.conf`, use plaintext API keys, support Ollama/openai-compatible profiles,
  and fetch/cache model suggestions.
- `src/hieronymus/dreaming.py` - implement true drain cycles, phase workflow calls,
  best-effort parsing, confidence penalties, relation discovery, compaction, and
  audit logging.
- `src/hieronymus/dream_autostart.py` - replace old thresholds with schedule/min/max
  urgent/backlog escape logic.
- `src/hieronymus/doctor.py` - validate `dream.conf`, provider connectivity, model
  cache freshness, referenced profiles, API key failures, and disabled dreaming state.
- `src/hieronymus/admin.py` - expose concepts, facets, crystals, short-term status,
  dream status, dream audits, and Dream all action through existing admin store.
- `src/hieronymus/tui_bridge/admin_api.py` - expose the new admin data shape for Ink.
- `src/hieronymus/tui_bridge/config_api.py` - expose provider profiles, workflow
  assignments, prompt editors, test results, and cached model suggestions.
- `src/hieronymus/mcp_server.py` - update primitive memory tools such as
  `hieronymus_short_term_add`, `hieronymus_recall`, `hieronymus_feedback`, and
  `hieronymus_dream`; do not treat old read/learn workflow tools as current
  MCP surface.
- `src/hieronymus/cli.py` - update `remember-short`, `recall`, `dream`, config JSON,
  and doctor-facing output.
- Existing tests covering the touched modules.

## Task 1: Add Config Paths And Dream Config Models

**Files:**

- Modify: `src/hieronymus/config.py`
- Create: `src/hieronymus/dream_config.py`
- Test: `tests/test_dream_config.py`

- [x] **Step 1: Write failing config path tests**

```python
from pathlib import Path

from hieronymus.config import HieronymusConfig


def test_dream_config_paths_live_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.dream_config_path == config.config_root / "dream.conf"
    assert config.llm_cache_path == config.config_root / "llmcache.tmp"
```

- [x] **Step 2: Run the focused failing test**

Run: `uv run pytest tests/test_dream_config.py::test_dream_config_paths_live_under_config_root -v`

Expected: FAIL with `AttributeError` for `dream_config_path`.

- [x] **Step 3: Add config path properties**

In `src/hieronymus/config.py`, add these properties after `settings_path`:

```python
    @property
    def dream_config_path(self) -> Path:
        return self.config_root / "dream.conf"

    @property
    def llm_cache_path(self) -> Path:
        return self.config_root / "llmcache.tmp"
```

- [x] **Step 4: Add dream config defaults and plaintext secret test**

Append to `tests/test_dream_config.py`:

```python
import tomllib

from hieronymus.dream_config import (
    DreamConfigError,
    ProviderProfile,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
    save_dream_config,
)


def test_default_dream_config_matches_memory_spec(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    dream_config = load_dream_config(config)

    assert dream_config.enabled is False
    assert dream_config.schedule_interval_minutes == 30
    assert dream_config.min_pending_short_term_memories == 20
    assert dream_config.max_pending_short_term_memories == 200
    assert dream_config.max_short_term_memories_per_cycle == 50
    assert dream_config.not_enough_memories_cycle_threshold == 5
    assert dream_config.workflows["crystallization"].provider == "anthropic"
    assert dream_config.workflows["relation_discovery"].enabled is False
    assert dream_config.workflows["reinforcement_compaction"].provider == "ollama"


def test_save_dream_config_writes_plaintext_api_key_and_redacts_json(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_provider(
        "anthropic",
        ProviderProfile(
            type="anthropic",
            endpoint="https://api.anthropic.com",
            api_key="secret-value",
            timeout_seconds=30.0,
        ),
    )

    save_dream_config(config, dream_config)

    raw = config.dream_config_path.read_text(encoding="utf-8")
    assert "secret-value" in raw
    payload = tomllib.loads(raw)
    assert payload["providers"]["anthropic"]["api_key"] == "secret-value"
    redacted = redacted_dream_config_payload(dream_config)
    assert redacted["providers"]["anthropic"]["api_key"] == "***"


def test_load_dream_config_rejects_invalid_threshold_order(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        "[dreaming]\n"
        "enabled = true\n"
        "schedule_interval_minutes = 30\n"
        "min_pending_short_term_memories = 20\n"
        "max_pending_short_term_memories = 10\n"
        "max_short_term_memories_per_cycle = 50\n"
        "not_enough_memories_cycle_threshold = 5\n",
        encoding="utf-8",
    )

    with pytest.raises(DreamConfigError, match="max_pending_short_term_memories"):
        load_dream_config(config)


def test_workflow_references_existing_provider(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_workflow(
        "crystallization",
        WorkflowProfile(provider="missing", model="model", enabled=True),
    )

    with pytest.raises(DreamConfigError, match="referenced provider profile is missing"):
        save_dream_config(config, dream_config)
```

- [x] **Step 5: Create dream config implementation**

Create `src/hieronymus/dream_config.py`:

```python
from __future__ import annotations

import tomllib
from dataclasses import dataclass, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig


class DreamConfigError(ValueError):
    """Raised when dream.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ProviderProfile:
    type: str
    endpoint: str = ""
    api_key: str = ""
    timeout_seconds: float = 30.0

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        return {
            "type": self.type,
            "endpoint": self.endpoint,
            "api_key": "***" if redact and self.api_key else self.api_key,
            "timeout_seconds": self.timeout_seconds,
        }


@dataclass(frozen=True)
class WorkflowProfile:
    provider: str
    model: str
    enabled: bool = True

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "model": self.model,
            "enabled": self.enabled,
        }


@dataclass(frozen=True)
class DreamConfig:
    enabled: bool
    schedule_interval_minutes: int
    min_pending_short_term_memories: int
    max_pending_short_term_memories: int
    max_short_term_memories_per_cycle: int
    not_enough_memories_cycle_threshold: int
    general_prompt: str
    providers: dict[str, ProviderProfile]
    workflows: dict[str, WorkflowProfile]

    def with_provider(self, name: str, provider: ProviderProfile) -> DreamConfig:
        return replace(self, providers={**self.providers, name: provider})

    def with_workflow(self, name: str, workflow: WorkflowProfile) -> DreamConfig:
        return replace(self, workflows={**self.workflows, name: workflow})

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        return {
            "dreaming": {
                "enabled": self.enabled,
                "schedule_interval_minutes": self.schedule_interval_minutes,
                "min_pending_short_term_memories": self.min_pending_short_term_memories,
                "max_pending_short_term_memories": self.max_pending_short_term_memories,
                "max_short_term_memories_per_cycle": self.max_short_term_memories_per_cycle,
                "not_enough_memories_cycle_threshold": (
                    self.not_enough_memories_cycle_threshold
                ),
                "general_prompt": self.general_prompt,
            },
            "providers": {
                name: provider.to_payload(redact=redact)
                for name, provider in self.providers.items()
            },
            "workflows": {
                name: workflow.to_payload() for name, workflow in self.workflows.items()
            },
        }


def default_dream_config() -> DreamConfig:
    return DreamConfig(
        enabled=False,
        schedule_interval_minutes=30,
        min_pending_short_term_memories=20,
        max_pending_short_term_memories=200,
        max_short_term_memories_per_cycle=50,
        not_enough_memories_cycle_threshold=5,
        general_prompt=(
            "Use English as the primary searchable memory language. Preserve Japanese "
            "and Russian only as names, translations, quoted evidence, or metadata."
        ),
        providers={
            "anthropic": ProviderProfile(type="anthropic", endpoint="https://api.anthropic.com"),
            "openai": ProviderProfile(type="openai", endpoint="https://api.openai.com/v1"),
            "gemini": ProviderProfile(type="gemini", endpoint="https://generativelanguage.googleapis.com"),
            "ollama": ProviderProfile(type="ollama", endpoint="http://localhost:11434"),
        },
        workflows={
            "crystallization": WorkflowProfile(
                provider="anthropic",
                model="claude-sonnet-4-6",
                enabled=True,
            ),
            "relation_discovery": WorkflowProfile(
                provider="ollama",
                model="gemma4-e3b",
                enabled=False,
            ),
            "reinforcement_compaction": WorkflowProfile(
                provider="ollama",
                model="gemma4-e3b",
                enabled=True,
            ),
        },
    )


def load_dream_config(config: HieronymusConfig) -> DreamConfig:
    if not config.dream_config_path.exists():
        return validate_dream_config(default_dream_config())
    try:
        payload = tomllib.loads(config.dream_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise DreamConfigError(f"dream.conf invalid: {error}") from error
    return validate_dream_config(_dream_config_from_payload(payload))


def save_dream_config(config: HieronymusConfig, dream_config: DreamConfig) -> None:
    dream_config = validate_dream_config(dream_config)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.dream_config_path, tomli_w.dumps(dream_config.to_payload()))


def redacted_dream_config_payload(dream_config: DreamConfig) -> dict[str, object]:
    return dream_config.to_payload(redact=True)


def validate_dream_config(dream_config: DreamConfig) -> DreamConfig:
    _positive("schedule_interval_minutes", dream_config.schedule_interval_minutes)
    _positive("min_pending_short_term_memories", dream_config.min_pending_short_term_memories)
    _positive("max_pending_short_term_memories", dream_config.max_pending_short_term_memories)
    _positive("max_short_term_memories_per_cycle", dream_config.max_short_term_memories_per_cycle)
    _positive(
        "not_enough_memories_cycle_threshold",
        dream_config.not_enough_memories_cycle_threshold,
    )
    if (
        dream_config.max_pending_short_term_memories
        < dream_config.min_pending_short_term_memories
    ):
        raise DreamConfigError(
            "max_pending_short_term_memories must be greater than or equal to "
            "min_pending_short_term_memories"
        )
    if (
        dream_config.max_short_term_memories_per_cycle
        > dream_config.max_pending_short_term_memories
    ):
        raise DreamConfigError(
            "max_short_term_memories_per_cycle must be less than or equal to "
            "max_pending_short_term_memories"
        )
    for name, provider in dream_config.providers.items():
        if provider.type not in {"openai", "anthropic", "gemini", "ollama"}:
            raise DreamConfigError(f"unsupported provider type for {name}: {provider.type}")
        if provider.timeout_seconds <= 0:
            raise DreamConfigError(f"providers.{name}.timeout_seconds must be greater than 0")
    for workflow_name, workflow in dream_config.workflows.items():
        if workflow.enabled and workflow.provider not in dream_config.providers:
            raise DreamConfigError(
                f"referenced provider profile is missing: {workflow_name}.{workflow.provider}"
            )
        if workflow.enabled and not workflow.model:
            raise DreamConfigError(f"model not set for workflow: {workflow_name}")
    return dream_config


def _dream_config_from_payload(payload: dict[str, Any]) -> DreamConfig:
    defaults = default_dream_config()
    dreaming = _table(payload.get("dreaming"), "dreaming")
    providers_payload = _table(payload.get("providers"), "providers")
    workflows_payload = _table(payload.get("workflows"), "workflows")
    providers = dict(defaults.providers)
    for name, raw_provider in providers_payload.items():
        provider = _table(raw_provider, f"providers.{name}")
        providers[name] = ProviderProfile(
            type=str(provider.get("type", providers.get(name, ProviderProfile("openai")).type)),
            endpoint=str(provider.get("endpoint", "")),
            api_key=str(provider.get("api_key", "")),
            timeout_seconds=float(provider.get("timeout_seconds", 30.0)),
        )
    workflows = dict(defaults.workflows)
    for name, raw_workflow in workflows_payload.items():
        workflow = _table(raw_workflow, f"workflows.{name}")
        current = workflows.get(name, WorkflowProfile(provider="", model="", enabled=False))
        workflows[name] = WorkflowProfile(
            provider=str(workflow.get("provider", current.provider)),
            model=str(workflow.get("model", current.model)),
            enabled=bool(workflow.get("enabled", current.enabled)),
        )
    return DreamConfig(
        enabled=bool(dreaming.get("enabled", defaults.enabled)),
        schedule_interval_minutes=int(
            dreaming.get("schedule_interval_minutes", defaults.schedule_interval_minutes)
        ),
        min_pending_short_term_memories=int(
            dreaming.get(
                "min_pending_short_term_memories",
                defaults.min_pending_short_term_memories,
            )
        ),
        max_pending_short_term_memories=int(
            dreaming.get(
                "max_pending_short_term_memories",
                defaults.max_pending_short_term_memories,
            )
        ),
        max_short_term_memories_per_cycle=int(
            dreaming.get(
                "max_short_term_memories_per_cycle",
                defaults.max_short_term_memories_per_cycle,
            )
        ),
        not_enough_memories_cycle_threshold=int(
            dreaming.get(
                "not_enough_memories_cycle_threshold",
                defaults.not_enough_memories_cycle_threshold,
            )
        ),
        general_prompt=str(dreaming.get("general_prompt", defaults.general_prompt)),
        providers=providers,
        workflows=workflows,
    )


def _table(value: object, name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise DreamConfigError(f"{name} must be a table")
    return value


def _positive(name: str, value: int) -> None:
    if type(value) is not int:
        raise DreamConfigError(f"{name} must be an integer")
    if value < 1:
        raise DreamConfigError(f"{name} must be at least 1")
```

- [x] **Step 6: Run focused tests**

Run: `uv run pytest tests/test_dream_config.py -v`

Expected: PASS.

- [x] **Step 7: Commit**

```bash
git add src/hieronymus/config.py src/hieronymus/dream_config.py tests/test_dream_config.py
git commit -m "feat: add dream config model"
```

## Task 2: Add LLM Model Cache

**Files:**

- Create: `src/hieronymus/llm_cache.py`
- Modify: `src/hieronymus/dream_providers.py`
- Modify: `src/hieronymus/doctor.py`
- Test: `tests/test_llm_cache.py`
- Test: `tests/test_doctor.py`

- [x] **Step 1: Write model cache tests**

```python
from datetime import UTC, datetime, timedelta
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    load_model_cache,
    save_model_cache,
)


def test_model_cache_round_trips_provider_models(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    cache = CachedModels(
        providers={
            "anthropic": ModelCacheEntry(
                provider="anthropic",
                models=("claude-sonnet-4-6", "claude-haiku-4-5"),
                fetched_at="2026-06-09T10:00:00+00:00",
                error="",
            )
        }
    )

    save_model_cache(config, cache)
    loaded = load_model_cache(config)

    assert loaded.providers["anthropic"].models == (
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
    )


def test_cache_entry_is_stale_after_24_hours() -> None:
    entry = ModelCacheEntry(
        provider="openai",
        models=("gpt-4.1-mini",),
        fetched_at="2026-06-08T09:59:59+00:00",
        error="",
    )

    assert entry.is_stale(datetime(2026, 6, 9, 10, 0, 0, tzinfo=UTC)) is True
    assert entry.is_stale(datetime(2026, 6, 9, 9, 59, 0, tzinfo=UTC)) is False
```

- [x] **Step 2: Create cache implementation**

Create `src/hieronymus/llm_cache.py`:

```python
from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

CACHE_TTL = timedelta(hours=24)


@dataclass(frozen=True)
class ModelCacheEntry:
    provider: str
    models: tuple[str, ...]
    fetched_at: str
    error: str = ""

    def is_stale(self, now: datetime | None = None) -> bool:
        now = now or datetime.now(UTC)
        fetched = datetime.fromisoformat(self.fetched_at)
        return now - fetched >= CACHE_TTL

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": list(self.models),
            "fetched_at": self.fetched_at,
            "error": self.error,
        }


@dataclass(frozen=True)
class CachedModels:
    providers: dict[str, ModelCacheEntry] = field(default_factory=dict)

    def with_entry(self, entry: ModelCacheEntry) -> CachedModels:
        return CachedModels(providers={**self.providers, entry.provider: entry})

    def to_payload(self) -> dict[str, object]:
        return {
            "providers": {
                name: entry.to_payload() for name, entry in self.providers.items()
            }
        }


def load_model_cache(config: HieronymusConfig) -> CachedModels:
    if not config.llm_cache_path.exists():
        return CachedModels()
    payload = json.loads(config.llm_cache_path.read_text(encoding="utf-8"))
    providers = {}
    for name, raw_entry in payload.get("providers", {}).items():
        providers[name] = ModelCacheEntry(
            provider=str(raw_entry.get("provider", name)),
            models=tuple(str(model) for model in raw_entry.get("models", [])),
            fetched_at=str(raw_entry.get("fetched_at", "")),
            error=str(raw_entry.get("error", "")),
        )
    return CachedModels(providers=providers)


def save_model_cache(config: HieronymusConfig, cache: CachedModels) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(
        config.llm_cache_path,
        json.dumps(cache.to_payload(), ensure_ascii=False, indent=2, sort_keys=True),
    )
```

- [x] **Step 3: Run model cache tests**

Run: `uv run pytest tests/test_llm_cache.py -v`

Expected: PASS.

- [x] **Step 4: Wire provider model suggestions through cache**

In `src/hieronymus/dream_providers.py`, keep existing HTTP fetching methods but change
`ProviderRegistry.list_model_suggestions` so it:

```python
from datetime import UTC, datetime

from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    load_model_cache,
    save_model_cache,
)


def _cached_or_fetch_models(
    config: HieronymusConfig,
    provider_name: str,
    fetch: Callable[[], ModelSuggestionResult],
) -> ModelSuggestionResult:
    cache = load_model_cache(config)
    cached = cache.providers.get(provider_name)
    if cached is not None and not cached.is_stale():
        return ModelSuggestionResult(
            provider=provider_name,
            models=list(cached.models),
            source="llmcache.tmp",
            error=cached.error,
        )
    result = fetch()
    updated = cache.with_entry(
        ModelCacheEntry(
            provider=provider_name,
            models=tuple(result.models),
            fetched_at=datetime.now(UTC).isoformat(),
            error=result.error,
        )
    )
    save_model_cache(config, updated)
    return result
```

Use this helper inside `list_model_suggestions` for OpenAI, Gemini, Anthropic, and
Ollama. For Anthropic, cache default hints with `source="defaults"` because there is
no stable public models endpoint in the existing implementation.

- [x] **Step 5: Add doctor warning for stale or failed cache refresh**

In `tests/test_doctor.py`, add:

```python
def test_doctor_warns_when_llm_cache_refresh_failed(config: HieronymusConfig) -> None:
    from hieronymus.doctor import DoctorIssue, doctor_issues
    from hieronymus.llm_cache import CachedModels, ModelCacheEntry, save_model_cache

    save_model_cache(
        config,
        CachedModels(
            providers={
                "anthropic": ModelCacheEntry(
                    provider="anthropic",
                    models=(),
                    fetched_at="2026-06-08T00:00:00+00:00",
                    error="network error",
                )
            }
        ),
    )

    issues = doctor_issues(config)

    assert DoctorIssue(
        severity="warning",
        code="llm_model_cache_refresh_failed",
        message="Model cache refresh failed for provider profile: anthropic",
    ) in issues
```

Then update `doctor_issues` to append that warning for cache entries with `error`.

- [x] **Step 6: Run focused tests**

Run: `uv run pytest tests/test_llm_cache.py tests/test_doctor.py -v`

Expected: PASS.

- [x] **Step 7: Commit**

```bash
git add src/hieronymus/llm_cache.py src/hieronymus/dream_providers.py src/hieronymus/doctor.py tests/test_llm_cache.py tests/test_doctor.py
git commit -m "feat: cache provider model suggestions"
```

## Task 3: Migrate Schema For Concepts, Facets, Tags, Scopes, And Audits

**Files:**

- Modify: `src/hieronymus/migrations/global.sql`
- Create: `src/hieronymus/concept_models.py`
- Modify: `src/hieronymus/memory_models.py`
- Test: `tests/test_memory_models.py`
- Test: `tests/test_concept_store.py`

- [x] **Step 1: Write schema smoke test**

Create `tests/test_concept_store.py`:

```python
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect


def test_memory_design_tables_exist(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        table_names = {
            row["name"]
            for row in conn.execute(
                "select name from sqlite_master where type in ('table', 'view')"
            )
        }

    assert {
        "concepts",
        "concept_facets",
        "concept_semantic_tags",
        "concept_renames",
        "crystal_concepts",
        "crystal_story_scopes",
        "crystal_semantic_tags",
        "dream_audit_entries",
        "dream_phase_runs",
    }.issubset(table_names)
```

- [x] **Step 2: Add schema tables**

Append to `src/hieronymus/migrations/global.sql`:

```sql
create table if not exists concepts (
  id integer primary key,
  canonical_name text not null,
  description text not null default '',
  status text not null default 'vague',
  confidence real not null default 0.2,
  created_at text not null,
  updated_at text not null
);

create virtual table if not exists concepts_fts using fts5(
  canonical_name,
  description,
  content='concepts',
  content_rowid='id'
);

create table if not exists concept_facets (
  id integer primary key,
  concept_id integer not null references concepts(id) on delete cascade,
  language text not null default '',
  facet_type text not null,
  value text not null,
  source_crystal_id integer references crystals(id) on delete set null,
  confidence real not null default 0.2,
  created_at text not null,
  updated_at text not null
);

create table if not exists concept_semantic_tags (
  concept_id integer not null references concepts(id) on delete cascade,
  tag text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(concept_id, tag)
);

create table if not exists concept_renames (
  id integer primary key,
  concept_id integer not null references concepts(id) on delete cascade,
  old_name text not null,
  new_name text not null,
  reason text not null default '',
  dream_run_id integer references dream_runs(id) on delete set null,
  created_at text not null
);

create table if not exists crystal_concepts (
  crystal_id integer not null references crystals(id) on delete cascade,
  concept_id integer not null references concepts(id) on delete cascade,
  link_type text not null default 'mentions',
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, concept_id, link_type)
);

create table if not exists crystal_story_scopes (
  crystal_id integer not null references crystals(id) on delete cascade,
  scope text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, scope)
);

create table if not exists crystal_semantic_tags (
  crystal_id integer not null references crystals(id) on delete cascade,
  tag text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, tag)
);

create table if not exists dream_phase_runs (
  id integer primary key,
  dream_run_id integer not null references dream_runs(id) on delete cascade,
  phase text not null,
  provider_profile text not null,
  provider_type text not null,
  model text not null,
  status text not null,
  input_count integer not null default 0,
  output_count integer not null default 0,
  error text not null default '',
  prompt_hash text not null default '',
  created_at text not null,
  completed_at text
);

create table if not exists dream_audit_entries (
  id integer primary key,
  dream_run_id integer not null references dream_runs(id) on delete cascade,
  phase_run_id integer references dream_phase_runs(id) on delete set null,
  event_type text not null,
  severity text not null default 'info',
  summary text not null,
  payload_json text not null default '{}',
  created_at text not null
);
```

Use `alter table` guarded by `try/except sqlite3.OperationalError` in `db.py` only if
SQLite migrations in this project do not re-run changed `global.sql` against existing
databases. Preserve the old `strict_terms` and `strict_concept_proposals` tables.

- [x] **Step 3: Add model dataclasses**

Create `src/hieronymus/concept_models.py`:

```python
from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ConceptRecord:
    id: int
    canonical_name: str
    description: str
    status: str
    confidence: float
    tags: tuple[str, ...] = ()


@dataclass(frozen=True)
class ConceptFacetRecord:
    id: int
    concept_id: int
    language: str
    facet_type: str
    value: str
    confidence: float
    source_crystal_id: int | None = None


@dataclass(frozen=True)
class ConceptLinkRecord:
    crystal_id: int
    concept_id: int
    link_type: str
    confidence: float


@dataclass(frozen=True)
class ConceptCandidate:
    name: str
    description: str = ""
    tags: tuple[str, ...] = field(default_factory=tuple)
    confidence: float = 0.2
```

Modify `src/hieronymus/memory_models.py` so `CrystalRecord` and `RecallResult` become:

```python
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
    source_credibility: str = "observation"
    rule_intent: str = ""
    malformed_penalty: float = 0.0
    supersedes_crystal_id: int | None = None
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    concept_ids: tuple[int, ...] = ()


@dataclass(frozen=True)
class RecallResult:
    source: str
    rank: int
    score: float
    reason: str
    crystal: CrystalRecord | None = None
    short_term_memory: ShortTermMemoryRecord | None = None
```

- [x] **Step 4: Run schema/model tests**

Run: `uv run pytest tests/test_memory_models.py tests/test_concept_store.py -v`

Expected: PASS for existing tests plus the new schema smoke test.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/migrations/global.sql src/hieronymus/concept_models.py src/hieronymus/memory_models.py tests/test_memory_models.py tests/test_concept_store.py
git commit -m "feat: add multilingual memory schema"
```

## Task 4: Implement Short-Term Memory Validation

**Files:**

- Create: `src/hieronymus/short_memory.py`
- Modify: `src/hieronymus/workspace.py`
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/cli.py`
- Test: `tests/test_short_memory.py`
- Test: `tests/test_mcp_server.py`

- [x] **Step 1: Write validation tests**

```python
import pytest

from hieronymus.short_memory import ShortMemoryValidation, validate_short_memory_text


def test_short_memory_accepts_one_to_six_sentences() -> None:
    result = validate_short_memory_text("Yun uses Enchant. The term should stay in English.")

    assert result == ShortMemoryValidation(ok=True, warning="", sentence_count=2)


def test_short_memory_warns_for_large_but_allowed_memory() -> None:
    text = " ".join(f"Sentence {index}." for index in range(1, 10))

    result = validate_short_memory_text(text)

    assert result.ok is True
    assert result.warning == "short-term memory is large; prefer 1-6 sentences"
    assert result.sentence_count == 9


def test_short_memory_rejects_huge_memory() -> None:
    text = " ".join(f"Sentence {index}." for index in range(1, 41))

    with pytest.raises(ValueError, match="short-term memory is too large"):
        validate_short_memory_text(text)
```

- [x] **Step 2: Create validation helper**

Create `src/hieronymus/short_memory.py`:

```python
from __future__ import annotations

import re
from dataclasses import dataclass

PREFERRED_SENTENCE_MAX = 6
HARD_SENTENCE_MAX = 30


@dataclass(frozen=True)
class ShortMemoryValidation:
    ok: bool
    warning: str
    sentence_count: int


def validate_short_memory_text(text: str) -> ShortMemoryValidation:
    stripped = text.strip()
    if not stripped:
        raise ValueError("short-term memory text must not be empty")
    sentence_count = _sentence_count(stripped)
    if sentence_count > HARD_SENTENCE_MAX:
        raise ValueError("short-term memory is too large")
    warning = ""
    if sentence_count > PREFERRED_SENTENCE_MAX:
        warning = "short-term memory is large; prefer 1-6 sentences"
    return ShortMemoryValidation(ok=True, warning=warning, sentence_count=sentence_count)


def _sentence_count(text: str) -> int:
    matches = re.findall(r"[^.!?。！？]+[.!?。！？]+", text)
    if matches:
        return len(matches)
    return 1
```

- [x] **Step 3: Route workspace inserts through validation**

In `src/hieronymus/workspace.py`, import `validate_short_memory_text`. In
`WorkspaceStore.add_short_term_memory`, call:

```python
validation = validate_short_memory_text(text)
metadata = dict(metadata or {})
if validation.warning:
    metadata["validation_warning"] = validation.warning
metadata["sentence_count"] = validation.sentence_count
```

Serialize that merged metadata into `metadata_json`.

- [x] **Step 4: Add correction memory MCP test**

In `tests/test_mcp_server.py`, add:

```python
def test_mcp_feedback_adds_correction_short_term_memory(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    series = Registry(load_config()).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    from hieronymus import mcp_server

    started = mcp_server.hieronymus_session_start(series.slug, "translate")
    result = mcp_server.hieronymus_feedback(
        started["session_id"],
        "User told me to remember that Cooking Talent is translated as Кулинария.",
    )

    memories = WorkspaceStore(load_config()).list_short_term_memories(started["session_id"])
    assert result == {"memory_id": 1}
    assert memories[0].kind == "correction"
    assert memories[0].text.startswith("User told me to remember")
```

Update `hieronymus_feedback` so user corrections are stored as short-term memory with
`kind="correction"` and no immediate dream request.

- [x] **Step 5: Run focused tests**

Run: `uv run pytest tests/test_short_memory.py tests/test_mcp_server.py -v`

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add src/hieronymus/short_memory.py src/hieronymus/workspace.py src/hieronymus/mcp_server.py src/hieronymus/cli.py tests/test_short_memory.py tests/test_mcp_server.py
git commit -m "feat: validate short-term memories"
```

## Task 5: Implement Concept Store And Crystal Links

**Files:**

- Modify: `src/hieronymus/concepts.py`
- Modify: `src/hieronymus/crystals.py`
- Test: `tests/test_concept_store.py`
- Test: `tests/test_crystals.py`

- [x] **Step 1: Write concept store tests**

Append to `tests/test_concept_store.py`:

```python
from hieronymus.concepts import ConceptStore


def test_concept_store_creates_vague_then_solid_concept(config: HieronymusConfig) -> None:
    store = ConceptStore(config)

    concept_id = store.create_or_reinforce(
        "Yun",
        description="Main character identity anchor.",
        tags=("character",),
        confidence_delta=0.25,
    )
    store.create_or_reinforce(
        "Yun",
        description="Main character identity anchor.",
        tags=("character", "sense-user"),
        confidence_delta=0.8,
    )

    concept = store.get(concept_id)
    assert concept.canonical_name == "Yun"
    assert concept.status == "solid"
    assert concept.tags == ("character", "sense-user")


def test_concept_store_links_crystal_to_multiple_concepts(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    yun_id = store.create_or_reinforce("Yun", confidence_delta=0.8)
    sense_id = store.create_or_reinforce("Sense", confidence_delta=0.8)
    enchant_id = store.create_or_reinforce("Enchant", confidence_delta=0.8)

    crystal_id = 42
    store.link_crystal(crystal_id, yun_id, link_type="mentions", confidence=0.9)
    store.link_crystal(crystal_id, sense_id, link_type="mentions", confidence=0.9)
    store.link_crystal(crystal_id, enchant_id, link_type="mentions", confidence=0.9)

    assert store.concept_ids_for_crystal(crystal_id) == (yun_id, sense_id, enchant_id)
```

- [x] **Step 2: Implement concept store**

Add `ConceptStore` to `src/hieronymus/concepts.py` while keeping
`ConceptProposalStore` available:

```python
class ConceptStore:
    SOLID_CONFIDENCE = 0.75

    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def create_or_reinforce(
        self,
        canonical_name: str,
        *,
        description: str = "",
        tags: tuple[str, ...] = (),
        confidence_delta: float = 0.2,
    ) -> int:
        name = canonical_name.strip()
        if not name:
            raise ValueError("concept canonical_name must not be empty")
        now = _now()
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select id, confidence from concepts where canonical_name = ?",
                (name,),
            ).fetchone()
            if row is None:
                confidence = min(1.0, max(0.0, confidence_delta))
                status = "solid" if confidence >= self.SOLID_CONFIDENCE else "vague"
                cursor = conn.execute(
                    """
                    insert into concepts(
                      canonical_name, description, status, confidence, created_at, updated_at
                    )
                    values (?, ?, ?, ?, ?, ?)
                    """,
                    (name, description, status, confidence, now, now),
                )
                concept_id = int(cursor.lastrowid)
                conn.execute(
                    "insert into concepts_fts(rowid, canonical_name, description) values (?, ?, ?)",
                    (concept_id, name, description),
                )
            else:
                concept_id = int(row["id"])
                confidence = min(1.0, float(row["confidence"]) + confidence_delta)
                status = "solid" if confidence >= self.SOLID_CONFIDENCE else "vague"
                conn.execute(
                    """
                    update concepts
                    set description = case when ? != '' then ? else description end,
                        confidence = ?,
                        status = ?,
                        updated_at = ?
                    where id = ?
                    """,
                    (description, description, confidence, status, now, concept_id),
                )
            for tag in sorted({tag.strip() for tag in tags if tag.strip()}):
                conn.execute(
                    """
                    insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                    values (?, ?, ?, ?)
                    on conflict(concept_id, tag) do update set
                      confidence = max(confidence, excluded.confidence)
                    """,
                    (concept_id, tag, confidence, now),
                )
            conn.commit()
            return concept_id

    def get(self, concept_id: int) -> ConceptRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from concepts where id = ?", (concept_id,)).fetchone()
            if row is None:
                raise KeyError(f"unknown concept: {concept_id}")
            tags = tuple(
                tag_row["tag"]
                for tag_row in conn.execute(
                    "select tag from concept_semantic_tags where concept_id = ? order by tag",
                    (concept_id,),
                )
            )
        return ConceptRecord(
            id=int(row["id"]),
            canonical_name=row["canonical_name"],
            description=row["description"],
            status=row["status"],
            confidence=float(row["confidence"]),
            tags=tags,
        )

    def link_crystal(
        self,
        crystal_id: int,
        concept_id: int,
        *,
        link_type: str,
        confidence: float,
    ) -> None:
        now = _now()
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                insert into crystal_concepts(
                  crystal_id, concept_id, link_type, confidence, created_at
                )
                values (?, ?, ?, ?, ?)
                on conflict(crystal_id, concept_id, link_type) do update set
                  confidence = max(confidence, excluded.confidence)
                """,
                (crystal_id, concept_id, link_type, confidence, now),
            )
            conn.commit()

    def concept_ids_for_crystal(self, crystal_id: int) -> tuple[int, ...]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select concept_id
                from crystal_concepts
                where crystal_id = ?
                order by concept_id
                """,
                (crystal_id,),
            ).fetchall()
        return tuple(int(row["concept_id"]) for row in rows)
```

- [x] **Step 3: Extend CrystalStore writes**

In `src/hieronymus/crystals.py`, expand allowed crystal types:

```python
_ALLOWED_CRYSTAL_TYPES = frozenset(
    {"lesson", "rule", "thought", "observation", "concept_note", "concept", "erudition"}
)
```

Add optional keyword parameters to `add_crystal`:

```python
source_credibility: str = "observation",
rule_intent: str = "",
malformed_penalty: float = 0.0,
supersedes_crystal_id: int | None = None,
story_scopes: tuple[str, ...] = (),
semantic_tags: tuple[str, ...] = (),
concept_ids: tuple[int, ...] = (),
```

After inserting the crystal row, insert rows into `crystal_story_scopes`,
`crystal_semantic_tags`, and `crystal_concepts`. Keep `tags_json` populated with
`semantic_tags` for old readers.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_concept_store.py tests/test_crystals.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/concepts.py src/hieronymus/crystals.py tests/test_concept_store.py tests/test_crystals.py
git commit -m "feat: add first-class concept links"
```

## Task 6: Implement Combined Recall

**Files:**

- Modify: `src/hieronymus/recall.py`
- Modify: `src/hieronymus/crystals.py`
- Modify: `src/hieronymus/mcp_server.py`
- Test: `tests/test_combined_recall.py`
- Test: `tests/test_recall.py`
- Test: `tests/test_mcp_server.py`

- [x] **Step 1: Write combined recall tests**

Create `tests/test_combined_recall.py`:

```python
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language="ja",
        target_language="ru",
        task_type="translate",
        volume="5",
        chapter="5",
        tags=("Book 5 Chapter 5",),
    )


def test_recall_returns_short_term_and_long_term_marked_by_source(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        "user",
        "note",
        "User told me Cooking Talent is now translated as Готовка.",
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Кулинария.",
        confidence=0.95,
        strength=0.8,
        source_credibility="user_rule",
        semantic_tags=("cooking", "talent"),
    )

    results = RecallService(config).recall(session.id, context, "Cooking Talent", limit=5)

    assert {result.source for result in results} == {"short_term", "long_term"}
    assert any(
        result.short_term_memory and "Готовка" in result.short_term_memory.text
        for result in results
    )
    assert any(result.crystal and result.crystal.crystal_type == "rule" for result in results)


def test_story_scope_boosts_but_does_not_filter(config: HieronymusConfig) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    store = CrystalStore(config)
    scoped = store.add_crystal(
        context,
        crystal_type="observation",
        text="Enchant appears in Book 5 Chapter 5.",
        story_scopes=("Book 5 Chapter 5",),
        strength=0.4,
        confidence=0.7,
    )
    unscoped = store.add_crystal(
        context,
        crystal_type="observation",
        text="Enchant appears in Book 1.",
        story_scopes=("Book 1",),
        strength=0.4,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)
    crystal_ids = [result.crystal.id for result in results if result.crystal]

    assert crystal_ids[0] == scoped
    assert unscoped in crystal_ids
```

- [x] **Step 2: Update RecallService**

Replace `RecallService.recall` with logic that:

```python
long_term = self.crystals.search_scored(context, query, limit=limit)
short_term = self._search_short_term(session_id, query, limit=limit)
combined = self._rank_combined(long_term, short_term, context)
```

Add `_search_short_term`:

```python
def _search_short_term(
    self,
    session_id: int,
    query: str,
    *,
    limit: int,
) -> list[tuple[ShortTermMemoryRecord, float]]:
    with connect(self.config.database_path) as conn:
        rows = conn.execute(
            """
            select stm.*
            from short_term_memories_fts fts
            join short_term_memories stm on stm.id = fts.rowid
            where short_term_memories_fts match ?
              and stm.session_id = ?
              and stm.archived_at is null
            order by bm25(short_term_memories_fts)
            limit ?
            """,
            (query, session_id, limit),
        ).fetchall()
    return [(_short_memory_from_row(row), 0.65) for row in rows]
```

Change `recall` to return `RecallResult(source="long_term", crystal=...)` and
`RecallResult(source="short_term", short_term_memory=...)`. Do not create a new
`recalled_crystal` short-term memory for each recall result.

- [x] **Step 3: Keep long-term activation records**

When recording activations, insert only results with `source == "long_term"` and a
non-null `crystal`. Use the rank from the combined list so audit shows the actual
rank seen by the caller.

- [x] **Step 4: Update MCP recall payload**

In `hieronymus_recall`, return:

```python
{
    "source": result.source,
    "rank": result.rank,
    "score": result.score,
    "reason": result.reason,
    "crystal": _crystal_payload(result.crystal) if result.crystal else None,
    "short_term_memory": (
        _short_term_payload(result.short_term_memory)
        if result.short_term_memory
        else None
    ),
}
```

- [x] **Step 5: Run focused tests**

Run: `uv run pytest tests/test_combined_recall.py tests/test_recall.py tests/test_mcp_server.py -v`

Expected: PASS after updating old recall tests that expected `recalled_crystal` traces.

- [x] **Step 6: Commit**

```bash
git add src/hieronymus/recall.py src/hieronymus/crystals.py src/hieronymus/mcp_server.py tests/test_combined_recall.py tests/test_recall.py tests/test_mcp_server.py
git commit -m "feat: combine short and long recall"
```

## Task 7: Add Dream Workflow Prompts And Provider Resolution

**Files:**

- Create: `src/hieronymus/dream_workflows.py`
- Modify: `src/hieronymus/dream_providers.py`
- Test: `tests/test_dream_workflows.py`
- Test: `tests/test_dream_providers.py`

- [x] **Step 1: Write prompt and workflow tests**

```python
from hieronymus.dream_config import default_dream_config
from hieronymus.dream_workflows import (
    CRYSTALLIZATION_PHASE,
    RELATION_DISCOVERY_PHASE,
    REINFORCEMENT_COMPACTION_PHASE,
    build_phase_prompt,
    resolve_enabled_workflows,
)


def test_default_workflows_have_separate_prompts() -> None:
    dream_config = default_dream_config()

    crystallization = build_phase_prompt(
        dream_config,
        CRYSTALLIZATION_PHASE,
        "Input memories: []",
    )
    compaction = build_phase_prompt(
        dream_config,
        REINFORCEMENT_COMPACTION_PHASE,
        "Affected snapshot: {}",
    )

    assert "Convert short-term memories" in crystallization
    assert "reinforce, combine, supersede, or decay" in compaction
    assert crystallization != compaction


def test_disabled_relation_discovery_is_not_resolved_by_default() -> None:
    workflows = resolve_enabled_workflows(default_dream_config())

    assert RELATION_DISCOVERY_PHASE not in workflows
    assert CRYSTALLIZATION_PHASE in workflows
    assert REINFORCEMENT_COMPACTION_PHASE in workflows
```

- [x] **Step 2: Create workflow module**

Create `src/hieronymus/dream_workflows.py`:

```python
from __future__ import annotations

from dataclasses import dataclass

from hieronymus.dream_config import DreamConfig, WorkflowProfile

CRYSTALLIZATION_PHASE = "crystallization"
RELATION_DISCOVERY_PHASE = "relation_discovery"
REINFORCEMENT_COMPACTION_PHASE = "reinforcement_compaction"


@dataclass(frozen=True)
class ResolvedWorkflow:
    phase: str
    provider: str
    model: str
    prompt: str


PHASE_PROMPTS = {
    CRYSTALLIZATION_PHASE: (
        "Convert short-term memories into concise long-term memory candidates. "
        "Create crystals of 1-2 English sentences, rule crystals for user rules, "
        "durable concepts, concept facets, semantic tags, story scopes, and links. "
        "Short-term correction memories that say 'User told me to' should become "
        "rule crystals when they express a translation rule. Return JSON."
    ),
    RELATION_DISCOVERY_PHASE: (
        "Inspect the affected memory set and propose additional concept links, "
        "semantic tags, story scopes, and rename candidates. Return only JSON "
        "relations that are supported by the provided snapshot."
    ),
    REINFORCEMENT_COMPACTION_PHASE: (
        "Review changed crystals, concepts, links, and recall misses. Decide what "
        "to reinforce, combine, supersede, or decay. Active rule crystals do not "
        "decay; they may be superseded or combined. Return JSON maintenance actions."
    ),
}


def resolve_enabled_workflows(dream_config: DreamConfig) -> dict[str, WorkflowProfile]:
    return {
        name: workflow
        for name, workflow in dream_config.workflows.items()
        if workflow.enabled
    }


def build_phase_prompt(
    dream_config: DreamConfig,
    phase: str,
    phase_input: str,
) -> str:
    if phase not in PHASE_PROMPTS:
        raise ValueError(f"unknown dream workflow phase: {phase}")
    return "\n\n".join(
        (
            dream_config.general_prompt,
            PHASE_PROMPTS[phase],
            "Format constraints are mandatory: return a single JSON object.",
            phase_input,
        )
    )
```

- [x] **Step 3: Resolve provider profiles from dream.conf**

In `src/hieronymus/dream_providers.py`, add:

```python
def resolve_profile_provider(
    config: HieronymusConfig,
    profile_name: str,
    *,
    model: str,
    transport: HTTPTransport | None = None,
) -> DreamProvider:
    dream_config = load_dream_config(config)
    if profile_name not in dream_config.providers:
        raise ValueError(f"referenced provider profile is missing: {profile_name}")
    profile = dream_config.providers[profile_name]
    if profile.type != "ollama" and not profile.api_key:
        raise ValueError(f"API key missing for provider profile: {profile_name}")
    return _provider_from_profile(profile_name, profile, model, transport=transport)
```

Implement `_provider_from_profile` by adapting existing OpenAI, Gemini, and Anthropic
provider classes to accept an explicit API key string instead of `api_key_env`.
Add Ollama as OpenAI-compatible chat completions against `/api/chat` or the existing
OpenAI-compatible path when endpoint ends with `/v1`.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_dream_workflows.py tests/test_dream_providers.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/dream_workflows.py src/hieronymus/dream_providers.py tests/test_dream_workflows.py tests/test_dream_providers.py
git commit -m "feat: add dream workflow prompts"
```

## Task 8: Implement Dream Audit

**Files:**

- Create: `src/hieronymus/dream_audit.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_dream_audit.py`
- Test: `tests/test_admin_store.py`

- [x] **Step 1: Write audit tests**

```python
from hieronymus.dream_audit import DreamAuditStore


def test_dream_audit_redacts_api_keys(config: HieronymusConfig) -> None:
    store = DreamAuditStore(config)

    entry_id = store.append(
        dream_run_id=1,
        phase_run_id=None,
        event_type="provider_request",
        severity="info",
        summary="Sent crystallization request",
        payload={
            "headers": {"x-api-key": "secret-value"},
            "body": {"model": "claude-sonnet-4-6"},
        },
    )

    entry = store.list_for_run(1)[0]
    assert entry.id == entry_id
    assert entry.payload["headers"]["x-api-key"] == "***"
    assert entry.payload["body"]["model"] == "claude-sonnet-4-6"
```

- [x] **Step 2: Create audit store**

Create `src/hieronymus/dream_audit.py`:

```python
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

SECRET_KEYS = {"api_key", "x-api-key", "authorization", "anthropic-version"}


@dataclass(frozen=True)
class DreamAuditEntry:
    id: int
    dream_run_id: int
    phase_run_id: int | None
    event_type: str
    severity: str
    summary: str
    payload: dict[str, object]
    created_at: str


class DreamAuditStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def append(
        self,
        *,
        dream_run_id: int,
        phase_run_id: int | None,
        event_type: str,
        severity: str,
        summary: str,
        payload: dict[str, object],
    ) -> int:
        now = datetime.now(UTC).isoformat()
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                insert into dream_audit_entries(
                  dream_run_id, phase_run_id, event_type, severity, summary,
                  payload_json, created_at
                )
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    dream_run_id,
                    phase_run_id,
                    event_type,
                    severity,
                    summary,
                    json.dumps(_redact(payload), ensure_ascii=False, sort_keys=True),
                    now,
                ),
            )
            conn.commit()
            return int(cursor.lastrowid)

    def list_for_run(self, dream_run_id: int) -> list[DreamAuditEntry]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_audit_entries
                where dream_run_id = ?
                order by id
                """,
                (dream_run_id,),
            ).fetchall()
        return [
            DreamAuditEntry(
                id=int(row["id"]),
                dream_run_id=int(row["dream_run_id"]),
                phase_run_id=row["phase_run_id"],
                event_type=row["event_type"],
                severity=row["severity"],
                summary=row["summary"],
                payload=json.loads(row["payload_json"]),
                created_at=row["created_at"],
            )
            for row in rows
        ]


def _redact(value: Any) -> Any:
    if isinstance(value, dict):
        redacted = {}
        for key, child in value.items():
            if str(key).lower() in SECRET_KEYS:
                redacted[key] = "***"
            else:
                redacted[key] = _redact(child)
        return redacted
    if isinstance(value, list):
        return [_redact(child) for child in value]
    return value
```

- [x] **Step 3: Expose audit rows in AdminStore**

In `src/hieronymus/admin.py`, add `dream_audits` to the admin view list and implement
`_list_dream_audit_entries` using `dream_audit_entries`. Each row label should be
`"{event_type}: {summary}"`, kind should be `"dream audit"`, and detail body should
show redacted JSON payload.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_dream_audit.py tests/test_admin_store.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/dream_audit.py src/hieronymus/admin.py tests/test_dream_audit.py tests/test_admin_store.py
git commit -m "feat: add dream audit log"
```

## Task 9: Implement True Drain Dreaming Pipeline

**Files:**

- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/dream_autostart.py`
- Modify: `src/hieronymus/workspace.py`
- Test: `tests/test_dreaming.py`
- Test: `tests/test_dream_autostart.py`

- [x] **Step 1: Write drain behavior tests**

Add to `tests/test_dreaming.py`:

```python
def test_manual_dreaming_drains_small_batch_even_below_minimum(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "correction", "User told me Sense is Сенс.")
    workspace.complete_session(session.id)

    run = DreamService(config, DeterministicDreamProvider()).run_all(owner="admin")

    assert run.status == "completed"
    with connect(config.database_path) as conn:
        pending = conn.execute(
            "select count(*) from short_term_memories where archived_at is null"
        ).fetchone()[0]
    assert pending == 0


def test_scheduled_dreaming_drains_all_pending_in_capped_cycles(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    for index in range(55):
        workspace.add_short_term_memory(
            session.id,
            "user",
            "note",
            f"Memory {index} for dreaming.",
        )
    workspace.complete_session(session.id)

    run = DreamService(
        config,
        DeterministicDreamProvider(),
        max_short_term_memories_per_cycle=20,
    ).run_all(owner="scheduler")

    assert run.status == "completed"
    with connect(config.database_path) as conn:
        phase_batches = conn.execute(
            "select count(*) from dream_phase_runs where phase = 'crystallization'"
        ).fetchone()[0]
        pending = conn.execute(
            "select count(*) from short_term_memories where archived_at is null"
        ).fetchone()[0]
    assert phase_batches == 3
    assert pending == 0
```

- [x] **Step 2: Add `DreamService.run_all`**

In `src/hieronymus/dreaming.py`, add:

```python
def run_all(
    self,
    *,
    owner: str = "manual",
    skip_when_locked: bool = False,
    wait: bool = False,
    ignore_minimum: bool = True,
) -> DreamRunRecord:
    with dream_cycle_lock(self.config, owner=owner, wait=wait):
        run = self._start_parent_run(owner)
        while True:
            batch = self._load_pending_short_term_batch(
                limit=self.max_short_term_memories_per_cycle
            )
            if not batch:
                return self._complete_parent_run(run)
            self._run_crystallization_phase(run, batch)
            affected = self._affected_memory_snapshot(run)
            self._run_relation_discovery_phase(run, affected)
            self._run_reinforcement_compaction_phase(run, affected)
            self._archive_processed_short_term(batch, run.cycle_id)
```

Keep `run_cycle` as a compatibility wrapper around one batch for existing tests during
the transition. New scheduler and admin calls must use `run_all`.

- [x] **Step 3: Implement scheduled threshold and backlog escape**

In `src/hieronymus/dream_autostart.py`, replace old threshold fields with
`DreamConfig` fields. Scheduled checks should:

```python
if pending_count >= dream_config.max_pending_short_term_memories:
    return DreamRequest(reason="urgent", ignore_minimum=True)
if pending_count >= dream_config.min_pending_short_term_memories:
    return DreamRequest(reason="scheduled", ignore_minimum=False)
if skipped_checks >= dream_config.not_enough_memories_cycle_threshold:
    return DreamRequest(reason="backlog_escape", ignore_minimum=True)
return DreamRequest(reason="not_enough_memories", ignore_minimum=False)
```

Persist skipped check count in a small table or an audit event so the sixth scheduled
check after five `not_enough_memories` checks drains leftovers.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_dreaming.py tests/test_dream_autostart.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/dream_autostart.py src/hieronymus/workspace.py tests/test_dreaming.py tests/test_dream_autostart.py
git commit -m "feat: drain pending memories during dreaming"
```

## Task 10: Apply Dream Outputs With Confidence Penalties

**Files:**

- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/crystals.py`
- Modify: `src/hieronymus/concepts.py`
- Test: `tests/test_dreaming.py`
- Test: `tests/test_concept_store.py`

- [x] **Step 1: Write malformed output test**

Add to `tests/test_dreaming.py`:

```python
def test_dreaming_best_effort_parses_malformed_entries_with_confidence_penalty(
    config: HieronymusConfig,
) -> None:
    class MalformedProvider:
        name = "malformed"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {
                        "body": "Cooking Talent should be translated as Готовка.",
                        "kind": "rule",
                        "source_credibility": "user_rule",
                    }
                ],
                "concepts": [{"label": "Cooking", "tags": ["talent"]}],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        "user",
        "correction",
        "User told me Cooking Talent should be translated as Готовка.",
    )
    workspace.complete_session(session.id)

    DreamService(config, MalformedProvider()).run_all(owner="test")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals").fetchone()
        concept = conn.execute("select * from concepts").fetchone()
    assert crystal["text"] == "Cooking Talent should be translated as Готовка."
    assert crystal["confidence"] < 0.9
    assert concept["canonical_name"] == "Cooking"
```

- [x] **Step 2: Add normalized output parser**

In `src/hieronymus/dreaming.py`, add parsing helpers:

```python
MALFORMED_CONFIDENCE_PENALTY = 0.2


def _text_from_candidate(raw: dict[str, object]) -> tuple[str, float]:
    if isinstance(raw.get("content"), str) and raw["content"].strip():
        return raw["content"].strip(), 0.0
    if isinstance(raw.get("text"), str) and raw["text"].strip():
        return raw["text"].strip(), 0.0
    if isinstance(raw.get("body"), str) and raw["body"].strip():
        return raw["body"].strip(), MALFORMED_CONFIDENCE_PENALTY
    raise ValueError("dream candidate content is required")


def _kind_from_candidate(raw: dict[str, object]) -> tuple[str, float]:
    kind = raw.get("crystal_type", raw.get("type", raw.get("kind", "observation")))
    if not isinstance(kind, str):
        return "observation", MALFORMED_CONFIDENCE_PENALTY
    if kind == "rule_crystal":
        return "rule", MALFORMED_CONFIDENCE_PENALTY
    return kind, 0.0
```

Use these helpers for crystals, rule crystals, concepts, facets, semantic tags, story
scopes, links, and thoughts. Reject only entries that have no recoverable content.

- [x] **Step 3: Apply source credibility and rule intent**

Map source credibility to initial confidence:

```python
SOURCE_CREDIBILITY_CONFIDENCE = {
    "rumor": 0.15,
    "observation": 0.35,
    "source_text": 0.7,
    "expert": 0.85,
    "user_suggestion": 0.8,
    "user_rule": 0.95,
    "thought": 0.2,
}
```

Apply `malformed_penalty` by subtracting it from initial confidence and clamping to
`0.05..1.0`.

- [x] **Step 4: Implement rule supersession actions**

When compaction output contains:

```json
{
  "supersede": [
    {
      "old_crystal_id": 1,
      "new_crystal_id": 2,
      "reason": "Cooking Talent rule replaced by more specific Cooking/Chef rules"
    }
  ]
}
```

Set old crystal status to `superseded`, set `supersedes_crystal_id` on the new
crystal when present, and append a `memory_events` row with `event_type="supersede"`.

- [x] **Step 5: Run focused tests**

Run: `uv run pytest tests/test_dreaming.py tests/test_concept_store.py -v`

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/crystals.py src/hieronymus/concepts.py tests/test_dreaming.py tests/test_concept_store.py
git commit -m "feat: apply dream outputs with confidence penalties"
```

## Task 11: Implement Reinforcement, Decay, And Active Rule Behavior

**Files:**

- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/crystals.py`
- Modify: `src/hieronymus/scoring.py`
- Test: `tests/test_dreaming.py`
- Test: `tests/test_scoring.py`

- [x] **Step 1: Write rule no-decay test**

Add to `tests/test_dreaming.py`:

```python
def test_active_rule_crystals_do_not_decay(config: HieronymusConfig) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Готовка.",
        confidence=0.95,
        strength=0.6,
        status="active",
    )

    DreamService(config, DeterministicDreamProvider()).decay_candidates(
        crystal_ids=(crystal_id,),
        reason="ambient low confidence decay",
    )

    crystal = CrystalStore(config).get(crystal_id)
    assert crystal.strength == 0.6
    assert crystal.status == "active"
```

- [x] **Step 2: Add decay candidate selection**

In `src/hieronymus/dreaming.py`, implement:

```python
def select_ambient_decay_candidates(
    self,
    *,
    recalled_crystal_ids: tuple[int, ...],
    linked_crystal_ids: tuple[int, ...],
    limit: int = 5,
) -> tuple[int, ...]:
    unused = tuple(
        crystal_id
        for crystal_id in recalled_crystal_ids
        if crystal_id not in set(linked_crystal_ids)
    )
    return self.crystals.low_confidence_first(unused, limit=limit)
```

In `CrystalStore.low_confidence_first`, exclude `crystal_type='rule' and status='active'`.

- [x] **Step 3: Add compaction maintenance actions**

Handle compaction output keys:

```python
{
  "reinforce": [{"crystal_id": 1, "strength_delta": 0.1, "confidence_delta": 0.05}],
  "decay": [{"crystal_id": 2, "strength_delta": -0.05, "confidence_delta": -0.02}],
  "combine": [{"source_crystal_ids": [3, 4], "content": "Combined memory text."}],
  "supersede": [{"old_crystal_id": 5, "new_crystal_id": 6, "reason": "New rule is specific."}]
}
```

Clamp strength and confidence to `0.0..1.0`. If confidence reaches `0.0`, set status
to `archived` unless the crystal is an active rule.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_dreaming.py tests/test_scoring.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/crystals.py src/hieronymus/scoring.py tests/test_dreaming.py tests/test_scoring.py
git commit -m "feat: add autonomous memory maintenance"
```

## Task 12: Replace Strict Contract Surface With Rule Crystal Validation

**Files:**

- Modify: `src/hieronymus/termbase.py`
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/admin.py`
- Test: `tests/test_termbase_contract.py`
- Test: `tests/test_termbase_validate.py`
- Test: `tests/test_mcp_server.py`

- [x] **Step 1: Write rule crystal validation test**

In `tests/test_termbase_validate.py`, add:

```python
def test_rule_crystal_validation_reports_forbidden_old_rendering(config: HieronymusConfig) -> None:
    context = _context(config)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Готовка, not Кулинария.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        semantic_tags=("translation-rule", "cooking"),
    )

    findings = Termbase(config, context).validate(
        source_text="Cooking Talent",
        translated_text="Кулинария",
    )

    assert findings[0].expected == "Готовка"
    assert findings[0].observed == "Кулинария"
```

- [x] **Step 2: Keep old tool names but source data from rule crystals**

In `hieronymus_termbase_contract`, include active rule crystals whose text matches the
source text by FTS/tag search. Return the existing payload shape for compatibility:

```python
{
    "id": crystal.id,
    "category": "rule",
    "source_text": extracted_source,
    "canonical_translation": extracted_canonical,
    "forbidden_variants": extracted_forbidden,
    "tags": list(crystal.semantic_tags),
    "notes": crystal.text,
}
```

Use deterministic parsing only for rules in the form
`"<source> is translated as <target>"` and
`"<source> is translated as <target>, not <forbidden>"`. Rules that cannot be parsed
stay recallable but do not produce contract validation findings.

- [x] **Step 3: Keep strict tables as import compatibility only**

Leave `strict_terms` and `strict_concept_proposals` in the schema. Admin approval of
old strict proposals should now create a high-confidence rule crystal with
`source_credibility="user_rule"` instead of inserting a new strict term.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_termbase_contract.py tests/test_termbase_validate.py tests/test_mcp_server.py -v`

Expected: PASS after updating old assertions from `strict_terms` rows to rule crystal
rows where needed.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/termbase.py src/hieronymus/mcp_server.py src/hieronymus/admin.py tests/test_termbase_contract.py tests/test_termbase_validate.py tests/test_mcp_server.py
git commit -m "feat: validate translations from rule crystals"
```

## Task 13: Update Admin And Config Bridge Contracts

**Files:**

- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/admin_models.py`
- Modify: `src/hieronymus/tui_bridge/admin_api.py`
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Test: `tests/test_admin_store.py`
- Test: `tests/test_tui_bridge_admin.py`
- Test: `tests/test_tui_bridge_config.py`

- [x] **Step 1: Write admin status contract test**

In `tests/test_tui_bridge_admin.py`, add:

```python
def test_admin_bridge_exposes_memory_and_dream_status(config: HieronymusConfig) -> None:
    api = AdminBridge(config)

    payload = api.dashboard()

    assert "short_term_status" in payload
    assert "dream_status" in payload
    assert payload["dream_status"]["state"] in {"IDLE", "WORKING", "DISABLED"}
    assert "concepts" in payload["views"]
    assert "dream_audits" in payload["views"]
```

- [x] **Step 2: Add admin dashboard fields**

Extend admin summary payloads with:

```python
{
    "short_term_status": {
        "pending_count": pending_count,
        "min_pending_short_term_memories": dream_config.min_pending_short_term_memories,
        "max_pending_short_term_memories": dream_config.max_pending_short_term_memories,
        "urgent": pending_count >= dream_config.max_pending_short_term_memories,
    },
    "dream_status": {
        "state": "DISABLED" if not dream_config.enabled else current_state,
        "current_phase": current_phase,
        "progress": progress,
    },
}
```

- [x] **Step 3: Add Dream all guard**

Admin Dream all action should call `DreamService.run_all(owner="admin",
ignore_minimum=True)`. The UI should allow Dream all only when pending short-term
memory count is greater than zero. Scheduled and urgent dreaming use service logic
rather than admin guards.

- [x] **Step 4: Add config API provider/workflow payload**

In `ConfigBridge`, expose:

```python
{
    "dreaming": redacted_dream_config_payload(dream_config)["dreaming"],
    "providers": redacted_dream_config_payload(dream_config)["providers"],
    "workflows": redacted_dream_config_payload(dream_config)["workflows"],
    "model_cache": load_model_cache(config).to_payload(),
}
```

Provider test actions must update `llmcache.tmp` when model fetch succeeds.

- [x] **Step 5: Run focused tests**

Run: `uv run pytest tests/test_admin_store.py tests/test_tui_bridge_admin.py tests/test_tui_bridge_config.py -v`

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add src/hieronymus/admin.py src/hieronymus/admin_models.py src/hieronymus/tui_bridge/admin_api.py src/hieronymus/tui_bridge/config_api.py tests/test_admin_store.py tests/test_tui_bridge_admin.py tests/test_tui_bridge_config.py
git commit -m "feat: expose memory admin contracts"
```

## Task 14: Update MCP And CLI Workflow Behavior

**Files:**

- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `docs/usage.md`
- Modify: `docs/memory-dreaming.md`
- Test: `tests/test_mcp_server.py`
- Test: `tests/test_cli.py`

- [x] **Step 1: Write MCP tool list update test**

Superseded note: this dated plan originally kept the judgment-heavy
`hieronymus_read` and `hieronymus_learn` tools. The current memory model uses
primitive storage/recall tools plus agent skill workflows instead; do not
reintroduce those tools from this historical checklist.

Update `tests/test_mcp_server.py::test_mcp_server_registers_expected_tool_names` so
the set remains stable unless a new tool is needed. The current primitive tool
surface is:

```python
{
    "hieronymus_termbase_contract",
    "hieronymus_termbase_validate",
    "hieronymus_termbase_propose",
    "hieronymus_termbase_approve",
    "hieronymus_memory_search",
    "hieronymus_memory_add",
    "hieronymus_session_start",
    "hieronymus_session_complete",
    "hieronymus_short_term_add",
    "hieronymus_recall",
    "hieronymus_feedback",
    "hieronymus_dream",
    "hieronymus_concept_proposals_list",
}
```

`hieronymus_concept_proposals_list` becomes a compatibility view over vague concepts
and recent dream audit proposals.

- [x] **Step 2: Update `hieronymus_memory_add` behavior**

Keep the tool name but route user memory additions to short-term memory by default:

```python
def hieronymus_memory_add(...):
    session = _ensure_default_session(...)
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction" if kind in {"rule", "correction"} else "note",
        text=text,
        source_ref=source_ref,
        metadata={"legacy_kind": kind, "importance": importance},
    )
    return {"memory_id": memory_id, "storage": "short_term"}
```

Do not request dreaming immediately.

- [x] **Step 3: Update `hiero dream`**

The CLI `dream` command should run true drain:

```python
run = DreamService(config, provider).run_all(owner="cli", ignore_minimum=True)
click.echo(f"Dream run {run.cycle_id}: {run.status}")
```

Scheduled service paths should call `run_all(ignore_minimum=False)` and rely on
`DreamAutostart` to decide whether a run should start.

- [x] **Step 4: Update docs**

In `docs/memory-dreaming.md`, document:

```markdown
- Short-term memories stay pending until dreaming processes them or a user removes them.
- Manual `hiero dream` drains all pending short-term memories, including a final small batch.
- Scheduled dreaming respects the minimum threshold unless the urgent cap or backlog
  escape rule fires.
- Corrections are stored as short-term memories and become rule crystals through dreaming.
```

- [x] **Step 5: Run focused tests**

Run: `uv run pytest tests/test_mcp_server.py tests/test_cli.py -v`

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add src/hieronymus/mcp_server.py src/hieronymus/cli.py docs/usage.md docs/memory-dreaming.md tests/test_mcp_server.py tests/test_cli.py
git commit -m "feat: route memory tools through learning workflow"
```

## Task 15: Update Doctor Checks For Dreaming Readiness

**Files:**

- Modify: `src/hieronymus/doctor.py`
- Modify: `src/hieronymus/cli.py`
- Test: `tests/test_doctor.py`
- Test: `tests/test_cli.py`

- [x] **Step 1: Write doctor readiness tests**

In `tests/test_doctor.py`, add parameterized checks:

```python
@pytest.mark.parametrize(
    ("raw_config", "code", "severity"),
    [
        ("[dreaming]\nenabled = true\n[workflows.crystallization]\nprovider='missing'\nmodel='x'\nenabled=true\n", "dream_provider_profile_missing", "error"),
        ("[dreaming]\nenabled = true\n[workflows.crystallization]\nprovider='anthropic'\nmodel=''\nenabled=true\n", "dream_model_not_set", "error"),
        ("[dreaming]\nenabled = true\n[providers.anthropic]\ntype='anthropic'\napi_key=''\n[workflows.crystallization]\nprovider='anthropic'\nmodel='x'\nenabled=true\n", "dream_api_key_missing", "error"),
    ],
)
def test_doctor_reports_dream_conf_readiness_errors(
    config: HieronymusConfig,
    raw_config: str,
    code: str,
    severity: str,
) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text(raw_config, encoding="utf-8")

    issues = doctor_issues(config)

    assert any(issue.code == code and issue.severity == severity for issue in issues)
```

- [x] **Step 2: Implement readiness checks**

Doctor must produce:

```python
DoctorIssue("error", "dream_conf_invalid", "dream.conf invalid")
DoctorIssue("error", "dream_provider_profile_missing", "Referenced provider profile is missing")
DoctorIssue("error", "dream_model_not_set", "Model not set for workflow")
DoctorIssue("error", "dream_api_key_missing", "API key missing for provider profile")
DoctorIssue("error", "dream_api_key_rejected", "API key rejected with 403")
DoctorIssue("warning", "dream_model_missing", "Configured model was not found in provider cache")
DoctorIssue("warning", "dream_provider_unreachable", "Provider in use cannot be reached")
```

If `dream_config.enabled is False`, doctor should report a non-failing warning:

```python
DoctorIssue("warning", "dreaming_disabled", "Dreaming is disabled")
```

Disabled optional workflows must not fail doctor.

- [x] **Step 3: Run focused tests**

Run: `uv run pytest tests/test_doctor.py tests/test_cli.py -v`

Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add src/hieronymus/doctor.py src/hieronymus/cli.py tests/test_doctor.py tests/test_cli.py
git commit -m "feat: validate dream readiness in doctor"
```

## Task 16: Compatibility Migration And Cleanup

**Files:**

- Modify: `src/hieronymus/memory.py`
- Modify: `src/hieronymus/concepts.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `docs/agent-workflows.md`
- Test: `tests/test_memory_search.py`
- Test: `tests/test_admin_actions.py`
- Test: `tests/test_concepts.py`

- [x] **Step 1: Write compatibility tests**

In `tests/test_memory_search.py`, add:

```python
def test_legacy_memory_add_is_searchable_as_short_term_then_long_term(config: HieronymusConfig) -> None:
    memory = MemoryStore(config)

    memory_id = memory.add(
        series_slug="only-sense-online",
        kind="translation_rationale",
        text="Use Yun for ユン.",
        source_ref="chapter-1",
        importance=4,
    )

    assert memory_id == 1
    results = memory.search("only-sense-online", "Yun")
    assert results[0]["text"] == "Use Yun for ユン."
```

- [x] **Step 2: Preserve old APIs with new storage semantics**

`MemoryStore.add` should create a default task session if needed and add a
short-term memory with metadata:

```python
{
    "legacy_kind": kind,
    "importance": importance,
    "storage_semantics": "short_term_until_dreamed",
}
```

`MemoryStore.search` should call `RecallService` if an active/default session exists;
otherwise it should run deterministic FTS over short-term and long-term tables and
return the old dict shape.

- [x] **Step 3: Keep old concept proposal APIs as compatibility**

`ConceptProposalStore.list_pending` should return rows from
`strict_concept_proposals` plus vague concepts with facet suggestions. The returned
dataclass shape remains compatible with tests that still inspect `concept_text`.

- [x] **Step 4: Run focused tests**

Run: `uv run pytest tests/test_memory_search.py tests/test_admin_actions.py tests/test_concepts.py -v`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/hieronymus/memory.py src/hieronymus/concepts.py src/hieronymus/admin.py docs/agent-workflows.md tests/test_memory_search.py tests/test_admin_actions.py tests/test_concepts.py
git commit -m "chore: keep legacy memory APIs compatible"
```

## Task 17: Full Verification And Spec Alignment

**Files:**

- Modify: `docs/memory-dreaming.md`
- Modify: `docs/usage.md`
- Modify: `docs/translation-workspace-integration.md`
- Test: whole repository

- [x] **Step 1: Run the full pytest suite**

Run: `uv run pytest`

Expected: PASS.

- [x] **Step 2: Run ruff check**

Run: `uv run ruff check .`

Expected: PASS.

- [x] **Step 3: Run ruff format check**

Run: `uv run ruff format --check .`

Expected: PASS.

- [x] **Step 4: Run docs whitespace check**

Run:
`git diff --check -- CONTEXT.md docs/adr docs/superpowers/specs docs/superpowers/plans docs/memory-dreaming.md docs/usage.md docs/translation-workspace-integration.md`

Expected: no output and exit code 0.

- [x] **Step 5: Confirm spec coverage**

Run:
`rg -n "strict contract|api_key_env|new_short_term_memory_threshold|max_cycles_per_autostart|deterministic provider" docs src tests`

Expected: only compatibility notes and tests that explicitly describe old behavior.

Result: remaining hits are legacy `settings.toml`/config TUI compatibility
implementation and tests, plus this plan's own command text. The primary docs now
describe `dream.conf`, named provider profiles/workflows, rule crystals, and
short-term memory compatibility wrappers.

- [x] **Step 6: Commit**

```bash
git add docs/memory-dreaming.md docs/usage.md docs/translation-workspace-integration.md
git commit -m "docs: describe multilingual memory workflow"
```

## Self-Review

Spec coverage:

- Config thresholds, provider profiles, workflow model assignments, prompt editors,
  plaintext `dream.conf`, and `llmcache.tmp` are covered by Tasks 1, 2, 7, 13, and 15.
- Concepts as durable first-class anchors, facets, freeform semantic tags, concept
  rename support, and many-to-many crystal links are covered by Tasks 3 and 5.
- Story scopes that boost recall without filtering are covered by Tasks 5 and 6.
- Short-term memory size constraints and correction memories are covered by Task 4.
- Combined short-term and long-term recall is covered by Task 6.
- Multi-step dreaming, true drain behavior, min/max/scheduled/backlog escape triggers,
  best-effort parsing, confidence penalties, thoughts, source credibility, autonomous
  compaction, reinforcement, decay, and active rule no-decay behavior are covered by
  Tasks 7 through 11.
- Rule crystals replacing strict contracts are covered by Task 12.
- Complete dream audits and admin visibility are covered by Tasks 8 and 13.
- MCP/CLI compatibility and docs are covered by Tasks 14 through 17.

Type consistency:

- `DreamConfig`, `ProviderProfile`, and `WorkflowProfile` are introduced in Task 1 and
  reused by workflow, provider, admin, config bridge, and doctor tasks.
- `ConceptRecord`, `ConceptFacetRecord`, and `ConceptStore` are introduced before
  crystal/concept linking tasks.
- `RecallResult.source`, `RecallResult.crystal`, and
  `RecallResult.short_term_memory` are introduced before MCP payload changes.
- Dreaming phases use the same phase constants from `dream_workflows.py` across
  provider resolution, audit, and phase run tables.

Execution risk:

- This is a large domain migration. Use a fresh worktree for execution.
- Keep old tables and MCP tool names until the Ink React TUI migration lands.
- Commit after every task so regressions can be bisected cleanly.

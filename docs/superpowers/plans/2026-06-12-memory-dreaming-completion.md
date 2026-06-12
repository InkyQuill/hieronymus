# Memory And Dreaming Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the next Memory/Dreaming product slice by moving ingestion policy into `ingest.conf`, retiring `settings.toml` provider configuration, and locking dream provider/audit behavior with tests.

**Architecture:** Keep configuration local-first and file-based. `dream.conf` owns dreaming provider profiles, workflows, prompts, thresholds, and plaintext API keys; `ingest.conf` owns ingestion limits; `settings.toml` is removed rather than migrated. Existing domain services remain the mutation boundary: `WorkspaceStore` writes short-term memory, `IngestionService` performs Read/Learn splitting, `DreamService` performs dreaming, and `ConfigBridge` exposes config payloads to OpenTUI.

**Tech Stack:** Python 3.12, TOML via `tomllib`/`tomli_w`, SQLite, pytest, React/OpenTUI on Bun.

---

## Current Code Map

- `src/hieronymus/config.py`: path registry for data-root files. It now exposes
  `dream_config_path`, `ingest_config_path`, and `release_config_path`; the
  legacy `settings_path` has been removed.
- `src/hieronymus/ingest_config.py`: `ingest.conf` model for short-memory
  warning/rejection thresholds and Learn block splitting limits.
- `src/hieronymus/short_memory.py`: validates sentence and symbol limits through
  `ShortMemoryLimits`.
- `src/hieronymus/workspace.py`: `WorkspaceStore.add_short_term_memory()` loads
  `ingest.conf` short-memory limits and stores validation metadata.
- `src/hieronymus/agent_ingestion.py`: `IngestionService.learn()` loads
  `ingest.conf` Learn limits for block splitting.
- `src/hieronymus/dream_config.py`: canonical `dream.conf` model for provider
  profiles, workflows, prompts, thresholds, and plaintext local API keys.
- `src/hieronymus/dream_providers.py`: provider runtime, status, checks, and
  model suggestions resolve through `dream.conf` profiles.
- `src/hieronymus/secrets.py`: redacts configured provider API keys from
  `DreamConfig`.
- `src/hieronymus/tui_bridge/config_api.py`: config TUI bridge edits
  `dream.conf`, `ingest.conf`, and `release.conf` drafts, redacting provider
  API keys in payloads.
- `src/hieronymus/tui_bridge/server.py` and `src/hieronymus/tui_bridge/protocol.py`:
  error redaction uses a `DreamConfig` redaction context.
- `src/hieronymus/dreaming.py`: dream run errors are redacted with the
  service's loaded `DreamConfig`; audit helpers record provider
  request/response, parse warnings, affected memory sets, and maintenance phase
  summaries.
- Tests updated/added: `tests/test_ingest_config.py`,
  `tests/test_short_memory.py`, `tests/test_workspace.py`,
  `tests/test_agent_ingestion.py`, `tests/test_dream_providers.py`,
  `tests/test_tui_bridge_config.py`, `tests/test_cli_service.py`,
  `tests/test_tui_bridge_protocol.py`, `tests/test_dreaming.py`, and
  `tests/test_dream_bounded_audit.py`. The legacy `tests/test_settings.py` was
  removed with `src/hieronymus/settings.py`.

---

### Task 1: Add `ingest.conf` Model

**Files:**
- Modify: `src/hieronymus/config.py`
- Create: `src/hieronymus/ingest_config.py`
- Test: `tests/test_ingest_config.py`

- [ ] **Step 1: Write failing path/default/round-trip tests**

Add `tests/test_ingest_config.py`:

```python
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.ingest_config import (
    IngestConfig,
    IngestConfigError,
    LearnLimits,
    ShortMemoryLimits,
    load_ingest_config,
    save_ingest_config,
)


def test_ingest_config_path_lives_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.ingest_config_path == config.config_root / "ingest.conf"


def test_default_ingest_config_preserves_current_behavior(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    ingest_config = load_ingest_config(config)

    assert ingest_config.short_memory.warning_sentence_count == 6
    assert ingest_config.short_memory.rejection_sentence_count == 30
    assert ingest_config.short_memory.warning_symbol_count == 0
    assert ingest_config.short_memory.rejection_symbol_count == 0
    assert ingest_config.learn.max_block_chars == 1200


def test_save_ingest_config_round_trips_limits(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    ingest_config = IngestConfig(
        short_memory=ShortMemoryLimits(
            warning_sentence_count=5,
            rejection_sentence_count=12,
            warning_symbol_count=1000,
            rejection_symbol_count=3000,
        ),
        learn=LearnLimits(max_block_chars=900),
    )

    save_ingest_config(config, ingest_config)

    assert load_ingest_config(config) == ingest_config
    raw = config.ingest_config_path.read_text(encoding="utf-8")
    assert "warning_symbol_count = 1000" in raw
    assert "max_block_chars = 900" in raw


def test_load_ingest_config_rejects_invalid_threshold_order(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.ingest_config_path.write_text(
        "[short_memory]\n"
        "warning_sentence_count = 30\n"
        "rejection_sentence_count = 6\n",
        encoding="utf-8",
    )

    with pytest.raises(IngestConfigError, match="rejection_sentence_count"):
        load_ingest_config(config)
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
uv run pytest tests/test_ingest_config.py -q
```

Expected: FAIL because `ingest_config_path` and `hieronymus.ingest_config` do not exist.

- [ ] **Step 3: Implement `ingest_config_path`**

In `src/hieronymus/config.py`, add:

```python
    @property
    def ingest_config_path(self) -> Path:
        return self.config_root / "ingest.conf"
```

- [ ] **Step 4: Implement `src/hieronymus/ingest_config.py`**

Create:

```python
from __future__ import annotations

import tomllib
from dataclasses import dataclass, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig


class IngestConfigError(ValueError):
    """Raised when ingest.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ShortMemoryLimits:
    warning_sentence_count: int = 6
    rejection_sentence_count: int = 30
    warning_symbol_count: int = 0
    rejection_symbol_count: int = 0

    def to_payload(self) -> dict[str, int]:
        return {
            "warning_sentence_count": self.warning_sentence_count,
            "rejection_sentence_count": self.rejection_sentence_count,
            "warning_symbol_count": self.warning_symbol_count,
            "rejection_symbol_count": self.rejection_symbol_count,
        }


@dataclass(frozen=True)
class LearnLimits:
    max_block_chars: int = 1200

    def to_payload(self) -> dict[str, int]:
        return {"max_block_chars": self.max_block_chars}


@dataclass(frozen=True)
class IngestConfig:
    short_memory: ShortMemoryLimits = ShortMemoryLimits()
    learn: LearnLimits = LearnLimits()

    def to_payload(self) -> dict[str, object]:
        return {
            "short_memory": self.short_memory.to_payload(),
            "learn": self.learn.to_payload(),
        }

    def with_short_memory(self, limits: ShortMemoryLimits) -> IngestConfig:
        return replace(self, short_memory=limits)

    def with_learn(self, limits: LearnLimits) -> IngestConfig:
        return replace(self, learn=limits)


def default_ingest_config() -> IngestConfig:
    return IngestConfig()


def load_ingest_config(config: HieronymusConfig) -> IngestConfig:
    if not config.ingest_config_path.exists():
        return validate_ingest_config(default_ingest_config())
    try:
        payload = tomllib.loads(config.ingest_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise IngestConfigError(f"ingest.conf is not valid TOML: {error}") from error
    return validate_ingest_config(_ingest_config_from_payload(payload))


def save_ingest_config(config: HieronymusConfig, ingest_config: IngestConfig) -> None:
    ingest_config = validate_ingest_config(ingest_config)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.ingest_config_path, tomli_w.dumps(ingest_config.to_payload()))


def validate_ingest_config(ingest_config: IngestConfig) -> IngestConfig:
    _validate_int_model("short_memory", ingest_config.short_memory)
    _validate_int_model("learn", ingest_config.learn)
    _require_minimum("short_memory.warning_sentence_count", ingest_config.short_memory.warning_sentence_count, 1)
    _require_minimum("short_memory.rejection_sentence_count", ingest_config.short_memory.rejection_sentence_count, 1)
    _require_minimum("short_memory.warning_symbol_count", ingest_config.short_memory.warning_symbol_count, 0)
    _require_minimum("short_memory.rejection_symbol_count", ingest_config.short_memory.rejection_symbol_count, 0)
    _require_minimum("learn.max_block_chars", ingest_config.learn.max_block_chars, 1)
    if ingest_config.short_memory.rejection_sentence_count < ingest_config.short_memory.warning_sentence_count:
        raise IngestConfigError(
            "short_memory.rejection_sentence_count must be greater than or equal to "
            "short_memory.warning_sentence_count"
        )
    if (
        ingest_config.short_memory.warning_symbol_count
        and ingest_config.short_memory.rejection_symbol_count
        and ingest_config.short_memory.rejection_symbol_count
        < ingest_config.short_memory.warning_symbol_count
    ):
        raise IngestConfigError(
            "short_memory.rejection_symbol_count must be greater than or equal to "
            "short_memory.warning_symbol_count"
        )
    return ingest_config


def _ingest_config_from_payload(payload: dict[str, Any]) -> IngestConfig:
    _validate_unknown_keys(payload, allowed=frozenset({"short_memory", "learn"}), prefix=None)
    defaults = default_ingest_config()
    short_memory_payload = _dict_payload(payload.get("short_memory"), "short_memory")
    learn_payload = _dict_payload(payload.get("learn"), "learn")
    _validate_unknown_keys(short_memory_payload, allowed=_field_names(ShortMemoryLimits), prefix="short_memory")
    _validate_unknown_keys(learn_payload, allowed=_field_names(LearnLimits), prefix="learn")
    return IngestConfig(
        short_memory=replace(defaults.short_memory, **short_memory_payload),
        learn=replace(defaults.learn, **learn_payload),
    )


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise IngestConfigError(f"{field_name} must be a table")
    return value


def _field_names(model: type[object]) -> frozenset[str]:
    return frozenset(field.name for field in fields(model))


def _validate_unknown_keys(payload: dict[str, Any], *, allowed: frozenset[str], prefix: str | None) -> None:
    for key in payload:
        if key not in allowed:
            setting = key if prefix is None else f"{prefix}.{key}"
            raise IngestConfigError(f"unknown ingest config setting: {setting}")


def _validate_int_model(prefix: str, model: object) -> None:
    for field in fields(model):
        value = getattr(model, field.name)
        if type(value) is not int:
            raise IngestConfigError(f"{prefix}.{field.name} must be an integer")


def _require_minimum(field_name: str, value: int, minimum: int) -> None:
    if value < minimum:
        raise IngestConfigError(f"{field_name} must be at least {minimum}")
```

- [ ] **Step 5: Run tests and format**

Run:

```bash
uv run pytest tests/test_ingest_config.py -q
uv run ruff format src/hieronymus/ingest_config.py tests/test_ingest_config.py
uv run ruff check src/hieronymus/ingest_config.py tests/test_ingest_config.py
```

Expected: tests pass, ruff passes.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/config.py src/hieronymus/ingest_config.py tests/test_ingest_config.py
git commit -m "feat: add ingest configuration file"
```

---

### Task 2: Wire `ingest.conf` Into Short-Term Memory And Learn Splitting

**Files:**
- Modify: `src/hieronymus/short_memory.py`
- Modify: `src/hieronymus/workspace.py`
- Modify: `src/hieronymus/agent_ingestion.py`
- Test: `tests/test_short_memory.py`
- Test: `tests/test_workspace.py`
- Test: `tests/test_agent_ingestion.py`

- [ ] **Step 1: Write failing short-memory policy tests**

In `tests/test_short_memory.py`, add:

```python
from hieronymus.ingest_config import ShortMemoryLimits


def test_short_memory_warns_when_symbol_warning_limit_is_configured() -> None:
    validation = validate_short_memory_text(
        "abcdefghijk",
        limits=ShortMemoryLimits(warning_symbol_count=10, rejection_symbol_count=0),
    )

    assert validation.warning == "short-term memory is large; prefer <= 10 symbols"
    assert validation.symbol_count == 11


def test_short_memory_rejects_when_symbol_rejection_limit_is_configured() -> None:
    with pytest.raises(ValueError, match="short-term memory exceeds 10 symbols"):
        validate_short_memory_text(
            "abcdefghijk",
            limits=ShortMemoryLimits(warning_symbol_count=0, rejection_symbol_count=10),
        )
```

- [ ] **Step 2: Write failing workspace metadata test**

In `tests/test_workspace.py`, add:

```python
from dataclasses import replace

from hieronymus.ingest_config import default_ingest_config, save_ingest_config


def test_short_term_memory_uses_configured_symbol_warning_limit(config: HieronymusConfig) -> None:
    save_ingest_config(
        config,
        default_ingest_config().with_short_memory(
            replace(
                default_ingest_config().short_memory,
                warning_symbol_count=10,
            )
        ),
    )
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    store.add_short_term_memory(session.id, "user", "note", "abcdefghijk")

    memory = store.list_short_term_memories(session.id)[0]
    assert memory.metadata["symbol_count"] == 11
    assert memory.metadata["validation_warning"] == "short-term memory is large; prefer <= 10 symbols"
```

- [ ] **Step 3: Write failing Learn split config test**

In `tests/test_agent_ingestion.py`, add:

```python
from dataclasses import replace

from hieronymus.ingest_config import default_ingest_config, save_ingest_config


def test_learn_uses_configured_block_character_limit(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_ingest_config(
        config,
        default_ingest_config().with_learn(
            replace(default_ingest_config().learn, max_block_chars=40)
        ),
    )
    session_id = _session(config)

    result = IngestionService(config).learn(
        session_id=session_id,
        text=" ".join(f"word{index}" for index in range(20)),
        source_role="mentor",
    )

    assert result.block_count > 1
    assert all(
        len(memory.text) <= 40
        for memory in WorkspaceStore(config).list_short_term_memories(session_id)
    )
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
uv run pytest tests/test_short_memory.py tests/test_workspace.py::test_short_term_memory_uses_configured_symbol_warning_limit tests/test_agent_ingestion.py::test_learn_uses_configured_block_character_limit -q
```

Expected: FAIL because `validate_short_memory_text()` does not accept limits and services do not load `ingest.conf`.

- [ ] **Step 5: Implement validation policy**

In `src/hieronymus/short_memory.py`, change the dataclass and function:

```python
from hieronymus.ingest_config import ShortMemoryLimits


@dataclass(frozen=True)
class ShortMemoryValidation:
    ok: bool
    warning: str
    sentence_count: int
    symbol_count: int


def validate_short_memory_text(
    text: str,
    *,
    limits: ShortMemoryLimits | None = None,
) -> ShortMemoryValidation:
    active_limits = limits or ShortMemoryLimits()
    stripped = text.strip()
    if not stripped:
        raise ValueError("short-term memory text must not be empty")

    sentence_count = _count_sentences(stripped)
    symbol_count = len(stripped)
    if sentence_count > active_limits.rejection_sentence_count:
        raise ValueError("short-term memory is too large")
    if (
        active_limits.rejection_symbol_count
        and symbol_count > active_limits.rejection_symbol_count
    ):
        raise ValueError(
            f"short-term memory exceeds {active_limits.rejection_symbol_count} symbols"
        )

    warnings: list[str] = []
    if sentence_count > active_limits.warning_sentence_count:
        warnings.append(
            f"short-term memory is large; prefer 1-{active_limits.warning_sentence_count} sentences"
        )
    if active_limits.warning_symbol_count and symbol_count > active_limits.warning_symbol_count:
        warnings.append(
            f"short-term memory is large; prefer <= {active_limits.warning_symbol_count} symbols"
        )

    return ShortMemoryValidation(
        ok=True,
        warning="; ".join(warnings),
        sentence_count=sentence_count,
        symbol_count=symbol_count,
    )
```

- [ ] **Step 6: Load ingest config in WorkspaceStore**

In `src/hieronymus/workspace.py`, import `load_ingest_config` and change:

```python
validation = validate_short_memory_text(
    text,
    limits=load_ingest_config(self.config).short_memory,
)
```

Also strip caller-controlled `symbol_count`:

```python
if key not in {"sentence_count", "symbol_count", "validation_warning"}
```

and store:

```python
memory_metadata["symbol_count"] = validation.symbol_count
```

- [ ] **Step 7: Load ingest config in IngestionService.learn**

In `src/hieronymus/agent_ingestion.py`, import `load_ingest_config` and change:

```python
limits = load_ingest_config(self.config).learn
blocks = split_learning_blocks(text, max_chars=limits.max_block_chars)
```

- [ ] **Step 8: Run targeted tests**

```bash
uv run pytest tests/test_short_memory.py tests/test_workspace.py::test_short_term_memory_uses_configured_symbol_warning_limit tests/test_agent_ingestion.py -q
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/hieronymus/short_memory.py src/hieronymus/workspace.py src/hieronymus/agent_ingestion.py tests/test_short_memory.py tests/test_workspace.py tests/test_agent_ingestion.py
git commit -m "feat: apply ingest limits to memory ingestion"
```

---

### Task 3: Expose `ingest.conf` Through Config CLI And ConfigBridge

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `frontend/src/rpc/schema.ts`
- Modify: `frontend/src/rpc/schema.test.ts`
- Test: `tests/test_cli_service.py`
- Test: `tests/test_tui_bridge_config.py`

- [ ] **Step 1: Write failing CLI JSON test**

In `tests/test_cli_service.py`, extend `test_config_json_returns_real_settings_and_paths` or add:

```python
def test_config_json_returns_ingest_config_path_and_defaults(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"

    result = CliRunner().invoke(
        main,
        ["--data-root", str(data_root), "config", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["ingest_config_path"] == str(data_root / "ingest.conf")
    assert payload["ingest"]["short_memory"]["warning_sentence_count"] == 6
    assert payload["ingest"]["learn"]["max_block_chars"] == 1200
```

- [ ] **Step 2: Write failing bridge test**

In `tests/test_tui_bridge_config.py`, add:

```python
def test_config_bootstrap_exposes_ingest_config_defaults(tmp_path: Path) -> None:
    payload = ConfigBridge(_config(tmp_path)).bootstrap({})

    assert payload["config_paths"]["ingest_config_path"].endswith("ingest.conf")
    assert payload["ingest"]["short_memory"]["warning_sentence_count"] == 6
    assert payload["ingest"]["short_memory"]["rejection_sentence_count"] == 30
    assert payload["ingest"]["learn"]["max_block_chars"] == 1200
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
uv run pytest tests/test_cli_service.py::test_config_json_returns_ingest_config_path_and_defaults tests/test_tui_bridge_config.py::test_config_bootstrap_exposes_ingest_config_defaults -q
```

Expected: FAIL because payloads do not include ingest config yet.

- [ ] **Step 4: Add payloads**

In `src/hieronymus/cli.py`, import `load_ingest_config` and include:

```python
ingest_config = load_ingest_config(config)
...
"ingest_config_path": str(config.ingest_config_path),
"ingest": ingest_config.to_payload(),
```

In `src/hieronymus/tui_bridge/config_api.py`, import `load_ingest_config`, load it in `_payload()`, and include:

```python
"ingest_config_path": str(self.config.ingest_config_path),
...
"ingest": load_ingest_config(self.config).to_payload(),
```

- [ ] **Step 5: Update frontend schema**

In `frontend/src/rpc/schema.ts`, add to `ConfigBootstrapSchema`:

```typescript
ingest: z
  .object({
    short_memory: z.record(z.number()),
    learn: z.record(z.number()),
  })
  .passthrough()
  .default({ short_memory: {}, learn: {} }),
```

In `frontend/src/rpc/schema.test.ts`, assert default parsing for missing `ingest` remains safe:

```typescript
expect(payload.ingest).toEqual({ short_memory: {}, learn: {} });
```

- [ ] **Step 6: Run targeted tests**

```bash
uv run pytest tests/test_cli_service.py tests/test_tui_bridge_config.py -q
bun --cwd frontend test src/rpc/schema.test.ts
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/cli.py src/hieronymus/tui_bridge/config_api.py frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts tests/test_cli_service.py tests/test_tui_bridge_config.py
git commit -m "feat: expose ingest configuration"
```

---

### Task 4: Move Provider Runtime To `dream.conf`

**Files:**
- Modify: `src/hieronymus/dream_providers.py`
- Modify: `src/hieronymus/dream_config.py`
- Modify: `src/hieronymus/secrets.py`
- Test: `tests/test_dream_providers.py`
- Test: `tests/test_tui_bridge_protocol.py`

- [ ] **Step 1: Write failing provider status/check tests using `dream.conf`**

In `tests/test_dream_providers.py`, add tests that do not import or save settings:

```python
def test_provider_status_uses_dream_config_profiles(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="",
                timeout_seconds=12.5,
            ),
        ),
    )

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["configured"] is False
    assert openai["error"] == "API key missing for provider profile"
    assert openai["base_url"] == "https://llm.example.test/v1"
    assert "api_key_env" not in openai
```

Add:

```python
def test_provider_check_uses_plaintext_profile_key(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "openai",
            ProviderProfile(
                type="openai",
                endpoint="https://llm.example.test/v1",
                api_key="secret-test-key",
                timeout_seconds=12.5,
            ),
        ),
    )
    transport = FakeTransport(HTTPResponse(status=200, body=json.dumps({"id": "ok"})), [])

    result = ProviderRegistry(transport=transport).check(config, "openai")

    assert result.ok is True
    assert transport.requests[0]["headers"]["Authorization"] == "Bearer secret-test-key"
    assert transport.requests[0]["timeout"] == 12.5
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
uv run pytest tests/test_dream_providers.py::test_provider_status_uses_dream_config_profiles tests/test_dream_providers.py::test_provider_check_uses_plaintext_profile_key -q
```

Expected: FAIL because `ProviderRegistry.status_payload()` and `.check()` still use `settings.toml`.

- [ ] **Step 3: Convert ProviderRegistry public methods to dream profiles**

In `src/hieronymus/dream_providers.py`:

- Keep `ProviderSettings` only if needed internally for HTTP provider classes, but import it from a new internal location or define a private equivalent in this file during Task 6.
- Change `status_payload(config)` to read `load_dream_config(config)` and map provider names to `ProviderProfile`.
- Return `api_key_present` from `bool(profile.api_key.strip())`, not environment variables.
- Change `check(config, name)` to use `dream_config.providers[name]` and `check_profile(config, name, profile)`.
- Change `list_model_suggestions(config, name)` to use `dream_config.providers[name]` and `list_profile_model_suggestions(config, name, profile)`.

The status item for configured remote profiles should have this shape:

```python
{
    "name": metadata.name,
    "display_name": metadata.display_name,
    "configured": configured,
    "model": _model_for_profile(dream_config, metadata.name),
    "api_key_present": bool(profile.api_key.strip()),
    "base_url": profile.endpoint,
    "timeout_seconds": profile.timeout_seconds,
    "error": error,
}
```

- [ ] **Step 4: Update secret redaction**

In `src/hieronymus/secrets.py`, replace settings-based redaction with dream-config values:

```python
from hieronymus.dream_config import DreamConfig


def configured_secret_values(dream_config: DreamConfig) -> set[str]:
    return {
        provider.api_key
        for provider in dream_config.providers.values()
        if len(provider.api_key.strip()) >= 4
    }


def redact_configured_secret_values(text: str, dream_config: DreamConfig) -> str:
    redacted = text
    for value in sorted(configured_secret_values(dream_config), key=len, reverse=True):
        redacted = redacted.replace(value, "[redacted]")
    return redacted
```

- [ ] **Step 5: Update protocol redaction callers**

In `src/hieronymus/tui_bridge/server.py` and `src/hieronymus/tui_bridge/protocol.py`, replace settings arguments with a redaction config or precomputed redaction callback. Keep the external JSON error shape unchanged.

Minimum target behavior for tests:

```python
response = dispatch(config, {"id": "1", "method": "missing", "params": {}})
assert "secret-test-key" not in repr(response)
```

- [ ] **Step 6: Run provider/protocol tests**

```bash
uv run pytest tests/test_dream_providers.py tests/test_tui_bridge_protocol.py -q
```

Expected: PASS after tests are updated away from `settings.toml`.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/dream_providers.py src/hieronymus/dream_config.py src/hieronymus/secrets.py src/hieronymus/tui_bridge/server.py src/hieronymus/tui_bridge/protocol.py tests/test_dream_providers.py tests/test_tui_bridge_protocol.py
git commit -m "feat: use dream config for provider runtime"
```

---

### Task 5: Make ConfigBridge Edit `dream.conf` And `ingest.conf`, Not `settings.toml`

**Files:**
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `src/hieronymus/tui_bridge/config_state.py`
- Modify: `frontend/src/config/ConfigForm.tsx`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Modify: `frontend/src/rpc/schema.ts`
- Test: `tests/test_tui_bridge_config.py`
- Test: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Write failing bridge save test for dream profile**

In `tests/test_tui_bridge_config.py`, add:

```python
def test_config_save_persists_provider_profile_to_dream_conf(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key": "plain-secret",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "12.5",
            },
            "dreaming": {
                "enabled": "yes",
                "min_pending_short_term_memories": "20",
                "max_pending_short_term_memories": "200",
                "max_short_term_memories_per_cycle": "50",
            },
            "ingest": {
                "warning_sentence_count": "6",
                "rejection_sentence_count": "30",
                "max_block_chars": "1200",
            },
        }
    )["draft"]

    payload = bridge.save({"selected_provider": "openai", "draft": draft})

    dream_config = load_dream_config(config)
    assert payload["validation"]["ok"] is True
    assert dream_config.providers["openai"].api_key == "plain-secret"
    assert dream_config.providers["openai"].endpoint == "https://llm.example.test/v1"
    assert dream_config.workflows["crystallization"].provider == "openai"
    assert not config.settings_path.exists()
```

- [ ] **Step 2: Run test to verify it fails**

```bash
uv run pytest tests/test_tui_bridge_config.py::test_config_save_persists_provider_profile_to_dream_conf -q
```

Expected: FAIL because ConfigBridge still saves provider draft via `save_settings()`.

- [ ] **Step 3: Redesign bridge draft shape**

In `ConfigBridge._payload()`, keep the existing top-level `dreaming`, `providers`, and `workflows` sections, but make `draft` represent real config files:

```python
"draft": {
    "dream": redacted_dream_config_payload(dream_config),
    "ingest": ingest_config.to_payload(),
    "release": _release_draft(release_config),
}
```

Keep `provider_choices` as UI metadata.

- [ ] **Step 4: Replace provider form fields**

The provider form should use:

```python
{
    "model": selected_workflow.model,
    "api_key": profile.api_key,
    "api_path": profile.endpoint,
    "timeout_seconds": str(profile.timeout_seconds),
}
```

Do not expose `api_key_env`.

- [ ] **Step 5: Save dream and ingest configs**

On `config.save`, call:

```python
save_dream_config(self.config, next_dream_config)
save_ingest_config(self.config, next_ingest_config)
save_release_config(self.config, release_config)
```

Do not call `save_settings()`.

- [ ] **Step 6: Update frontend fields**

In `frontend/src/config/ConfigForm.tsx`, replace `API Key Env` with:

```typescript
{
  label: "API Key",
  key: "provider.api_key",
  placeholder: "stored in dream.conf",
  type: "text",
}
```

Add ingest fields:

```typescript
{
  label: "Memory Warn Sentences",
  key: "ingest.warning_sentence_count",
  placeholder: "e.g. 6",
  type: "text",
}
{
  label: "Memory Reject Sentences",
  key: "ingest.rejection_sentence_count",
  placeholder: "e.g. 30",
  type: "text",
}
{
  label: "Learn Block Characters",
  key: "ingest.max_block_chars",
  placeholder: "e.g. 1200",
  type: "text",
}
```

- [ ] **Step 7: Run bridge/frontend tests**

```bash
uv run pytest tests/test_tui_bridge_config.py -q
bun --cwd frontend test src/config/ConfigScreen.test.tsx src/rpc/schema.test.ts
bun run --cwd frontend typecheck
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py src/hieronymus/tui_bridge/config_state.py frontend/src/config/ConfigForm.tsx frontend/src/config/ConfigScreen.tsx frontend/src/rpc/schema.ts tests/test_tui_bridge_config.py frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: edit dream and ingest config from tui"
```

---

### Task 6: Delete `settings.toml` Model And Compatibility Surface

**Files:**
- Delete: `src/hieronymus/settings.py`
- Delete: `tests/test_settings.py`
- Modify: `src/hieronymus/config.py`
- Modify: all imports found by `rg "hieronymus.settings|settings_path|settings.toml"`
- Test: affected tests from the grep list

- [ ] **Step 1: List remaining settings callers**

Run:

```bash
rg -n "hieronymus.settings|load_settings|save_settings|ProviderSettings|DreamingSettings|settings_path|settings.toml" src tests docs
```

Expected before work: matches exist in provider tests, CLI tests, config bridge, doctor, protocol, and docs.

- [ ] **Step 2: Remove `settings_path` from config**

In `src/hieronymus/config.py`, delete:

```python
    @property
    def settings_path(self) -> Path:
        return self.config_root / "settings.toml"
```

- [ ] **Step 3: Delete settings module and tests**

```bash
git rm src/hieronymus/settings.py tests/test_settings.py
```

- [ ] **Step 4: Replace remaining imports**

For each remaining import:

- `ProviderSettings` used only to feed HTTP providers should be replaced with a local internal dataclass in `src/hieronymus/dream_providers.py`, named `ProviderRuntimeSettings`.
- `DreamingSettings` usage in tests should be replaced with `DreamConfig` updates.
- `load_settings()`/`save_settings()` in tests should be replaced with `load_dream_config()`/`save_dream_config()` or removed.
- JSON payload tests should assert `dream_config_path` and `ingest_config_path`, not `settings_path`.

Use this command until it returns no matches:

```bash
rg -n "hieronymus.settings|load_settings|save_settings|ProviderSettings|DreamingSettings|settings_path|settings.toml" src tests docs
```

- [ ] **Step 5: Update docs**

In `docs/roadmap.md`, move these Memory/Dreaming bullets from Remaining to Completed:

```markdown
- Keep `dream.conf` as the canonical configuration file for dreaming providers,
  workflows, prompts, thresholds, caps, and plaintext local API keys.
- Remove the older `settings.toml` provider model instead of migrating it. The
  project is pre-release, so compatibility migrations are unnecessary.
- Add `ingest.conf` as a global data-root configuration file for ingestion
  policy. It should cover direct short-term memory warning/rejection thresholds
  and Learn-style block splitting limits.
```

Only move the bullets whose code is complete.

- [ ] **Step 6: Run broad tests**

```bash
uv run pytest tests/test_config.py tests/test_cli_service.py tests/test_dream_providers.py tests/test_tui_bridge_config.py tests/test_tui_bridge_protocol.py -q
uv run ruff check .
uv run ruff format --check .
```

Expected: PASS and no `settings.toml` references in `src`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: remove legacy settings config"
```

---

### Task 7: Lock Deterministic Fallback And Dream Audit Coverage

**Files:**
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/dream_providers.py`
- Test: `tests/test_dreaming.py`
- Test: `tests/test_dream_bounded_audit.py`
- Test: `tests/test_dream_providers.py`

- [ ] **Step 1: Write failing invalid workflow provider test**

In `tests/test_dreaming.py`, add:

```python
def test_dreaming_does_not_fall_back_to_deterministic_for_invalid_workflow(
    config: HieronymusConfig,
) -> None:
    save_dream_config(
        config,
        default_dream_config().with_workflow(
            "crystallization",
            WorkflowProfile(provider="missing", model="model", enabled=True),
        ),
    )
    session = WorkspaceStore(config).start_session(_context())
    WorkspaceStore(config).add_short_term_memory(
        session.id,
        "user",
        "note",
        "A memory that should not silently dream.",
    )
    WorkspaceStore(config).complete_session(session.id)

    with pytest.raises(DreamConfigError, match="referenced provider profile is missing"):
        resolve_profile_provider(config, "crystallization")
```

If this already passes because validation catches it, keep the test as regression coverage and do not change production code.

- [ ] **Step 2: Write audit completeness test**

In `tests/test_dream_bounded_audit.py`, add assertions to the existing provider-backed cycle test or add:

```python
def test_provider_backed_dream_audit_contains_required_payload_sections(
    config: HieronymusConfig,
) -> None:
    run = _run_provider_backed_cycle_with_parse_warning(config)
    entries = DreamAuditStore(config).list_for_run(run.id)
    payloads = {entry.event_type: entry.payload for entry in entries}

    assert {"provider_request", "provider_response", "parse_warnings", "phase_completed"} <= set(payloads)
    assert "request_summary" in payloads["provider_request"]
    assert "selected_short_term_memory_ids" in payloads["provider_request"]
    assert "response_summary" in payloads["provider_response"]
    assert "parse_warnings" in payloads["provider_response"]
    assert "warnings" in payloads["parse_warnings"]
    assert "affected_memory_set" in payloads["phase_completed"]
```

Use existing helpers in `tests/test_dream_bounded_audit.py` rather than creating a new provider fixture if one already exists.

- [ ] **Step 3: Run tests to verify current state**

```bash
uv run pytest tests/test_dreaming.py::test_dreaming_does_not_fall_back_to_deterministic_for_invalid_workflow tests/test_dream_bounded_audit.py::test_provider_backed_dream_audit_contains_required_payload_sections -q
```

Expected: Either PASS immediately, or FAIL with a concrete missing payload key.

- [ ] **Step 4: Implement only missing audit fields**

If a payload key is missing, update the existing helper in `src/hieronymus/dreaming.py`:

- `_audit_provider_request()`
- `_audit_provider_response()`
- `_audit_phase_completed()`
- `_audit_parse_warnings()`

Add only the missing keys asserted by the test. Do not create a second audit pathway.

- [ ] **Step 5: Run dreaming audit tests**

```bash
uv run pytest tests/test_dreaming.py tests/test_dream_bounded_audit.py tests/test_dream_parse_penalties.py -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/dream_providers.py tests/test_dreaming.py tests/test_dream_bounded_audit.py tests/test_dream_providers.py
git commit -m "test: lock dreaming fallback and audit payloads"
```

---

### Task 8: Final Verification And Roadmap Update

**Files:**
- Modify: `docs/roadmap.md`
- Modify: `docs/usage.md`
- Optional modify: `docs/memory-dreaming.md`

- [ ] **Step 1: Search for obsolete config language**

Run:

```bash
rg -n "settings.toml|api_key_env|environment variable|env var|HIERONYMUS_.*KEY" README.md docs src tests
```

Expected: no source references to `settings.toml`; docs may mention environment variables only when explicitly contrasting old behavior or host-agent config.

- [ ] **Step 2: Update usage docs**

In `docs/usage.md`, ensure the config section says:

```markdown
The config interface edits local plaintext config files:
`dream.conf` for dreaming providers/workflows/prompts, `ingest.conf` for memory
ingestion limits, and `release.conf` for update channel selection. Provider API
keys are local plaintext fields inside `dream.conf`; JSON output, logs, provider
checks, doctor output, and dream audit payloads redact those values.
```

- [ ] **Step 3: Update roadmap Memory/Dreaming status**

In `docs/roadmap.md`, move completed bullets out of Memory/Dreaming Remaining work. Leave these bullets remaining until fully covered by tests:

```markdown
- Add provider-backed dreaming smoke coverage that exercises multi-phase
  provider payloads through crystallization and maintenance paths.
- Verify dream audit records include provider request and response payloads,
  affected memory set summaries, parse warnings, and maintenance decisions.
```

Move them only if Task 7 produced real coverage for both crystallization and maintenance.

- [ ] **Step 4: Full verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend format
bun run --cwd frontend typecheck
bun --cwd frontend test
bun run --cwd frontend build
git diff --check
```

Expected: all commands pass. Existing React `act(...)` and OpenTUI `TerminalConsoleCache` warnings may still appear in Bun tests unless separately addressed by the OpenTUI polish work.

- [ ] **Step 5: Commit docs and final cleanup**

```bash
git add docs/roadmap.md docs/usage.md docs/memory-dreaming.md
git commit -m "docs: update memory dreaming completion status"
```

---

## Self-Review

Spec coverage:

- `dream.conf` remains canonical: Tasks 4-6 move provider runtime and config editing to `dream.conf`.
- `settings.toml` removed without migration: Task 6 deletes the model and all references.
- `ingest.conf` added: Tasks 1-3 create the file and expose it.
- Current default behavior preserved: Task 1 defaults keep 6 sentence warning, 30 sentence rejection, 1200 Learn block chars, and disabled symbol thresholds unless configured.
- Explicit symbol thresholds: Task 2 adds warning/rejection symbol checks and metadata.
- Deterministic fallback explicit: Task 7 adds regression coverage for invalid workflow handling.
- Provider-backed dreaming smoke/audit coverage: Task 7 locks required audit payloads and only adds missing production fields.

Placeholder scan:

- No placeholder markers or open-ended “add tests” steps remain.
- Steps include exact files, test names, commands, and expected outcomes.

Type consistency:

- `IngestConfig`, `ShortMemoryLimits`, `LearnLimits`, `load_ingest_config()`, and `save_ingest_config()` are introduced in Task 1 and reused consistently.
- `warning_symbol_count` and `rejection_symbol_count` use `0` as disabled to preserve current behavior.
- `api_key` replaces `api_key_env` only after provider runtime is moved to `dream.conf`.

# Provider Catalog Workflow Assignments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split LLM provider endpoint profiles into global `provider.conf` while keeping `dream.conf` focused on workflow provider/model assignments with deterministic defaults.

**Architecture:** Add a new provider catalog model beside existing config modules, then refactor dream/runtime code to resolve workflow provider+model through the catalog. Migrate old `dream.conf.providers` into `provider.conf` with fail-closed collision handling, then update the config bridge/TUI to edit provider profiles separately from workflow assignments.

**Tech Stack:** Python 3.12, TOML via `tomllib`/`tomli_w`, Click CLI, React/OpenTUI frontend, Bun tests, pytest, ruff.

---

## File Structure

- Create `src/hieronymus/provider_config.py`: typed provider catalog model, load/save/validate, migration helpers for old dream provider payloads.
- Modify `src/hieronymus/config.py`: add `provider_config_path`.
- Modify `src/hieronymus/dream_config.py`: remove provider ownership from `DreamConfig`, add explicit workflow assignment semantics with fallback resolution support.
- Modify `src/hieronymus/dream_providers.py`: resolve `WorkflowProfile.provider` through `ProviderCatalog`; keep provider runtime checks and model suggestions profile-based.
- Modify `src/hieronymus/dream_workflows.py`, `src/hieronymus/dream_autostart.py`, and any direct workflow provider consumers to use resolved workflow runtime profiles.
- Modify `src/hieronymus/tui_bridge/config_api.py`: expose two groups, `provider_catalog` and `workflows`, and route secret edits to `provider.conf`.
- Modify `frontend/src/rpc/schema.ts`, `frontend/src/config/ConfigScreen.tsx`, and `frontend/src/config/ConfigForm.tsx`: render provider catalog fields and workflow assignment fields separately.
- Add/modify tests in `tests/test_provider_config.py`, `tests/test_dream_config.py`, `tests/test_dream_providers.py`, `tests/test_tui_bridge_config.py`, and `frontend/src/config/ConfigScreen.test.tsx`.
- Update `docs/usage.md` and `docs/memory-dreaming.md` after behavior is implemented.

## Task 1: Add Provider Catalog Config Model

**Files:**
- Create: `src/hieronymus/provider_config.py`
- Modify: `src/hieronymus/config.py`
- Test: `tests/test_provider_config.py`

- [ ] **Step 1: Write failing tests for provider.conf load/save/defaults**

Create `tests/test_provider_config.py` with:

```python
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderCatalogError,
    ProviderDefaults,
    ProviderProfile,
    load_provider_catalog,
    save_provider_catalog,
    validate_provider_catalog,
)


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def test_load_provider_catalog_defaults_when_missing(tmp_path: Path) -> None:
    catalog = load_provider_catalog(_config(tmp_path))

    assert catalog.providers == {}
    assert catalog.defaults == ProviderDefaults(provider="", model="")


def test_save_and_load_provider_catalog_round_trips_profiles_and_defaults(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "deepseek-api": ProviderProfile(
                    name="Deepseek",
                    type="openai",
                    url="https://api.deepseek.com",
                    key="raw-secret",
                    timeout_seconds=45.0,
                ),
                "local-ollama": ProviderProfile(
                    name="Ollama",
                    type="openai",
                    url="http://127.0.0.1:6000/v1",
                    key="",
                    timeout_seconds=30.0,
                ),
            },
            defaults=ProviderDefaults(
                provider="deepseek-api",
                model="deepseek-v4-flash",
            ),
        ),
    )

    assert load_provider_catalog(config) == ProviderCatalog(
        providers={
            "deepseek-api": ProviderProfile(
                name="Deepseek",
                type="openai",
                url="https://api.deepseek.com",
                key="raw-secret",
                timeout_seconds=45.0,
            ),
            "local-ollama": ProviderProfile(
                name="Ollama",
                type="openai",
                url="http://127.0.0.1:6000/v1",
                key="",
                timeout_seconds=30.0,
            ),
        },
        defaults=ProviderDefaults(
            provider="deepseek-api",
            model="deepseek-v4-flash",
        ),
    )


def test_provider_catalog_validates_default_provider_exists() -> None:
    with pytest.raises(ProviderCatalogError, match="default provider is missing"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={},
                defaults=ProviderDefaults(provider="deepseek-api", model="deepseek-v4-flash"),
            ),
        )


def test_provider_catalog_rejects_unknown_provider_type() -> None:
    with pytest.raises(ProviderCatalogError, match="unsupported provider type"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={
                    "bad": ProviderProfile(
                        name="Bad",
                        type="made-up",
                        url="https://example.test",
                    )
                },
                defaults=ProviderDefaults(),
            ),
        )
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_provider_config.py -q
```

Expected: import failure for `hieronymus.provider_config`.

- [ ] **Step 3: Add provider_config_path**

In `src/hieronymus/config.py`, add:

```python
    @property
    def provider_config_path(self) -> Path:
        return self.config_root / "provider.conf"
```

- [ ] **Step 4: Implement provider catalog model**

Create `src/hieronymus/provider_config.py`:

```python
from __future__ import annotations

import math
import tomllib
from dataclasses import dataclass, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

SUPPORTED_PROVIDER_TYPES = frozenset({"openai", "google", "anthropic", "ollama"})


class ProviderCatalogError(ValueError):
    """Raised when provider.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ProviderProfile:
    name: str
    type: str
    url: str
    key: str = ""
    timeout_seconds: float = 30.0

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        return {
            "name": self.name,
            "type": self.type,
            "url": self.url,
            "key": "***" if redact and self.key else self.key,
            "timeout_seconds": self.timeout_seconds,
        }


@dataclass(frozen=True)
class ProviderDefaults:
    provider: str = ""
    model: str = ""

    def to_payload(self) -> dict[str, object]:
        return {"provider": self.provider, "model": self.model}


@dataclass(frozen=True)
class ProviderCatalog:
    providers: dict[str, ProviderProfile]
    defaults: ProviderDefaults = ProviderDefaults()

    def with_provider(self, profile_id: str, profile: ProviderProfile) -> ProviderCatalog:
        return replace(self, providers={**self.providers, profile_id: profile})

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        payload = {
            profile_id: profile.to_payload(redact=redact)
            for profile_id, profile in self.providers.items()
        }
        payload["defaults"] = self.defaults.to_payload()
        return payload


def default_provider_catalog() -> ProviderCatalog:
    return ProviderCatalog(providers={}, defaults=ProviderDefaults())


def load_provider_catalog(config: HieronymusConfig) -> ProviderCatalog:
    if not config.provider_config_path.exists():
        return validate_provider_catalog(default_provider_catalog())
    try:
        payload = tomllib.loads(config.provider_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise ProviderCatalogError(f"provider.conf is not valid TOML: {error}") from error
    return validate_provider_catalog(_provider_catalog_from_payload(payload))


def save_provider_catalog(config: HieronymusConfig, catalog: ProviderCatalog) -> None:
    catalog = validate_provider_catalog(catalog)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.provider_config_path, tomli_w.dumps(catalog.to_payload()))


def redacted_provider_catalog_payload(catalog: ProviderCatalog) -> dict[str, object]:
    return catalog.to_payload(redact=True)


def validate_provider_catalog(catalog: ProviderCatalog) -> ProviderCatalog:
    if type(catalog.providers) is not dict:
        raise ProviderCatalogError("providers must be a mapping")
    if not isinstance(catalog.defaults, ProviderDefaults):
        raise ProviderCatalogError("defaults has invalid profile type")
    for profile_id, profile in catalog.providers.items():
        _validate_profile_id(profile_id)
        _validate_provider_profile(profile_id, profile)
    if catalog.defaults.provider and catalog.defaults.provider not in catalog.providers:
        raise ProviderCatalogError(
            f"default provider is missing: {catalog.defaults.provider}",
        )
    if catalog.defaults.model:
        _require_exact_str("defaults.model", catalog.defaults.model)
    return catalog


def _provider_catalog_from_payload(payload: dict[str, Any]) -> ProviderCatalog:
    providers: dict[str, ProviderProfile] = {}
    defaults = ProviderDefaults()
    for table_name, raw_table in payload.items():
        table = _dict_payload(raw_table, table_name)
        if table_name == "defaults":
            _validate_unknown_keys(table, allowed=frozenset({"provider", "model"}), prefix="defaults")
            defaults = ProviderDefaults(
                provider=str(table.get("provider", "")),
                model=str(table.get("model", "")),
            )
            continue
        _validate_unknown_keys(
            table,
            allowed=frozenset({"name", "type", "url", "key", "timeout_seconds"}),
            prefix=table_name,
        )
        if "name" not in table:
            table["name"] = table_name
        if "type" not in table:
            raise ProviderCatalogError(f"{table_name}.type is required")
        if "url" not in table:
            raise ProviderCatalogError(f"{table_name}.url is required")
        providers[table_name] = ProviderProfile(
            name=str(table["name"]),
            type=str(table["type"]),
            url=str(table["url"]),
            key=str(table.get("key", "")),
            timeout_seconds=_coerce_positive_float(
                f"{table_name}.timeout_seconds",
                table.get("timeout_seconds", 30.0),
            ),
        )
    return ProviderCatalog(providers=providers, defaults=defaults)


def _validate_profile_id(profile_id: object) -> None:
    if type(profile_id) is not str or not profile_id:
        raise ProviderCatalogError("provider profile id must be a non-empty string")
    if profile_id == "defaults":
        raise ProviderCatalogError("defaults is reserved for provider catalog defaults")


def _validate_provider_profile(profile_id: str, profile: ProviderProfile) -> None:
    if not isinstance(profile, ProviderProfile):
        raise ProviderCatalogError(f"{profile_id} has invalid profile type")
    _require_exact_str(f"{profile_id}.name", profile.name)
    _require_exact_str(f"{profile_id}.type", profile.type)
    _require_exact_str(f"{profile_id}.url", profile.url)
    _require_exact_str(f"{profile_id}.key", profile.key)
    _require_positive_float(f"{profile_id}.timeout_seconds", profile.timeout_seconds)
    if profile.type not in SUPPORTED_PROVIDER_TYPES:
        raise ProviderCatalogError(f"unsupported provider type for {profile_id}: {profile.type}")


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if type(value) is not dict:
        raise ProviderCatalogError(f"{field_name} must be a table")
    return value


def _validate_unknown_keys(
    payload: dict[str, Any],
    *,
    allowed: frozenset[str],
    prefix: str,
) -> None:
    for key in payload:
        if key not in allowed:
            raise ProviderCatalogError(f"unknown provider config setting: {prefix}.{key}")


def _require_exact_str(field_name: str, value: object) -> None:
    if type(value) is not str:
        raise ProviderCatalogError(f"{field_name} must be a string")


def _coerce_positive_float(field_name: str, value: object) -> float:
    if type(value) not in (int, float):
        raise ProviderCatalogError(f"{field_name} must be a number")
    value = float(value)
    if not math.isfinite(value) or value <= 0:
        raise ProviderCatalogError(f"{field_name} must be finite and greater than 0")
    return value


def _require_positive_float(field_name: str, value: object) -> None:
    _coerce_positive_float(field_name, value)
```

- [ ] **Step 5: Run tests**

Run:

```bash
uv run pytest tests/test_provider_config.py -q
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/config.py src/hieronymus/provider_config.py tests/test_provider_config.py
git commit -m "feat: add provider catalog config"
```

## Task 2: Add Migration From dream.conf.providers to provider.conf

**Files:**
- Modify: `src/hieronymus/provider_config.py`
- Modify: `src/hieronymus/dream_config.py`
- Test: `tests/test_provider_config.py`
- Test: `tests/test_dream_config.py`

- [ ] **Step 1: Write failing migration tests**

Append to `tests/test_provider_config.py`:

```python
from hieronymus.provider_config import migrate_dream_provider_payload


def test_migrate_dream_provider_payload_preserves_secret_and_endpoint() -> None:
    catalog = migrate_dream_provider_payload(
        {
            "openai": {
                "type": "openai",
                "endpoint": "https://api.deepseek.com",
                "api_key": "secret",
                "timeout_seconds": 12,
            }
        },
        existing=ProviderCatalog(providers={}, defaults=ProviderDefaults()),
    )

    assert catalog.providers["openai"] == ProviderProfile(
        name="Openai",
        type="openai",
        url="https://api.deepseek.com",
        key="secret",
        timeout_seconds=12.0,
    )


def test_migrate_dream_provider_payload_rejects_profile_collision() -> None:
    existing = ProviderCatalog(
        providers={
            "openai": ProviderProfile(
                name="Existing",
                type="openai",
                url="https://api.openai.com/v1",
            )
        },
        defaults=ProviderDefaults(),
    )

    with pytest.raises(ProviderCatalogError, match="would overwrite provider profile"):
        migrate_dream_provider_payload(
            {
                "openai": {
                    "type": "openai",
                    "endpoint": "https://api.deepseek.com",
                    "api_key": "secret",
                }
            },
            existing=existing,
        )
```

Add to `tests/test_dream_config.py`:

```python
def test_load_dream_config_ignores_deprecated_providers_after_migration(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        """
[providers.openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"

[workflows.crystallization]
provider = "openai"
model = "deepseek-v4-flash"
enabled = true
""",
        encoding="utf-8",
    )

    dream_config = load_dream_config(config)

    assert dream_config.workflows["crystallization"].provider == "openai"
    assert "openai" not in dream_config.to_payload()
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_provider_config.py tests/test_dream_config.py -q
```

Expected: missing `migrate_dream_provider_payload` and dream config still exposes providers.

- [ ] **Step 3: Implement migration helper**

Add to `src/hieronymus/provider_config.py`:

```python
def migrate_dream_provider_payload(
    providers_payload: dict[str, object],
    *,
    existing: ProviderCatalog,
) -> ProviderCatalog:
    next_catalog = existing
    for profile_id, raw_profile in providers_payload.items():
        _validate_profile_id(profile_id)
        table = _dict_payload(raw_profile, f"providers.{profile_id}")
        profile = ProviderProfile(
            name=str(table.get("name", profile_id.replace("_", " ").title())),
            type=str(table["type"]),
            url=str(table.get("endpoint", table.get("url", ""))),
            key=str(table.get("api_key", table.get("key", ""))),
            timeout_seconds=_coerce_positive_float(
                f"providers.{profile_id}.timeout_seconds",
                table.get("timeout_seconds", 30.0),
            ),
        )
        existing_profile = next_catalog.providers.get(profile_id)
        if existing_profile is not None and existing_profile != profile:
            raise ProviderCatalogError(
                f"dream.conf migration would overwrite provider profile: {profile_id}",
            )
        next_catalog = next_catalog.with_provider(profile_id, profile)
    return validate_provider_catalog(next_catalog)
```

- [ ] **Step 4: Update dream config payload shape**

In `src/hieronymus/dream_config.py`, remove `providers` from `DreamConfig` and `to_payload()`. Keep parser support for old payloads by accepting the `providers` top-level key but not storing it. Change `validate_dream_config()` to validate workflows without requiring providers:

```python
@dataclass(frozen=True)
class DreamConfig:
    enabled: bool
    schedule_interval_minutes: int
    min_pending_short_term_memories: int
    max_pending_short_term_memories: int
    max_short_term_memories_per_cycle: int
    not_enough_memories_cycle_threshold: int
    max_changed_crystals_per_cycle: int
    max_related_concepts_per_cycle: int
    max_related_crystals_per_concept: int
    max_total_affected_crystals: int
    general_prompt: str
    workflows: dict[str, WorkflowProfile]
```

Update `_dream_config_from_payload()`:

```python
_validate_unknown_keys(
    payload,
    allowed=frozenset({"dreaming", "providers", "workflows"}),
    prefix=None,
)
```

Do not add parsed providers to `DreamConfig`.

Update `_validate_workflow_profile()` signature:

```python
def _validate_workflow_profile(name: str, workflow: WorkflowProfile) -> None:
    prefix = f"workflows.{name}"
    _require_exact_str(f"{prefix}.provider", workflow.provider)
    _require_exact_str(f"{prefix}.model", workflow.model)
    _require_exact_bool(f"{prefix}.enabled", workflow.enabled)
```

Runtime provider existence is validated later against `provider.conf`.

- [ ] **Step 5: Run tests**

Run:

```bash
uv run pytest tests/test_provider_config.py tests/test_dream_config.py -q
```

Expected: all tests pass after updating existing test expectations that referenced `dream_config.providers`.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/provider_config.py src/hieronymus/dream_config.py tests/test_provider_config.py tests/test_dream_config.py
git commit -m "feat: split dream config provider ownership"
```

## Task 3: Resolve Workflow Runtime Profiles Through provider.conf

**Files:**
- Modify: `src/hieronymus/dream_providers.py`
- Modify: `src/hieronymus/dream_workflows.py`
- Modify: `src/hieronymus/dream_autostart.py`
- Test: `tests/test_dream_providers.py`
- Test: `tests/test_dream_workflows.py`

- [ ] **Step 1: Write failing provider resolution tests**

Add to `tests/test_dream_providers.py`:

```python
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderDefaults,
    ProviderProfile as CatalogProviderProfile,
    save_provider_catalog,
)


def test_provider_registry_checks_workflow_against_provider_catalog(config) -> None:
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "deepseek-api": CatalogProviderProfile(
                    name="Deepseek",
                    type="openai",
                    url="https://api.deepseek.com",
                    key="secret",
                )
            },
            defaults=ProviderDefaults(),
        ),
    )
    save_dream_config(
        config,
        default_dream_config().with_workflow(
            "crystallization",
            WorkflowProfile(
                provider="deepseek-api",
                model="deepseek-v4-flash",
                enabled=True,
            ),
        ),
    )
    registry = ProviderRegistry(transport=FakeTransport(status=200, body='{"data":[]}'))

    result = registry.check(config, "deepseek-api", model="deepseek-v4-flash")

    assert result.ok is True
    assert result.name == "deepseek-api"
```

Add a missing-profile test:

```python
def test_provider_registry_fails_for_unknown_catalog_profile(config) -> None:
    result = ProviderRegistry().check(config, "missing-profile", model="gpt-test")

    assert result.ok is False
    assert result.error == "provider profile missing: missing-profile"
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_dream_providers.py -q
```

Expected: registry still assumes provider name is protocol metadata or old dream profile.

- [ ] **Step 3: Add catalog-to-runtime conversion**

In `src/hieronymus/dream_providers.py`, import catalog model and add:

```python
from hieronymus.provider_config import (
    ProviderCatalogError,
    ProviderProfile as CatalogProviderProfile,
    load_provider_catalog,
)


def _catalog_profile_to_runtime(profile: CatalogProviderProfile) -> ProviderProfile:
    profile_type = "gemini" if profile.type == "google" else profile.type
    return ProviderProfile(
        type=profile_type,
        endpoint=profile.url,
        api_key=profile.key,
        timeout_seconds=profile.timeout_seconds,
    )
```

Update `ProviderRegistry.check()` to look up the named profile in `provider.conf`:

```python
def check(self, config: HieronymusConfig, name: str, *, model: str = "") -> ProviderCheckResult:
    if name == "deterministic":
        return ProviderCheckResult(name="deterministic", ok=True, model="")
    try:
        catalog = load_provider_catalog(config)
    except ProviderCatalogError as error:
        return ProviderCheckResult(name=name, ok=False, model=model, error=str(error))
    profile = catalog.providers.get(name)
    if profile is None:
        return ProviderCheckResult(
            name=name,
            ok=False,
            model=model,
            error=f"provider profile missing: {name}",
        )
    return self.check_profile(
        config,
        name,
        _catalog_profile_to_runtime(profile),
        model=model,
    )
```

- [ ] **Step 4: Add workflow effective resolution helper**

In `src/hieronymus/dream_workflows.py`, add:

```python
from dataclasses import replace

from hieronymus.provider_config import ProviderCatalog, ProviderCatalogError


def resolve_effective_workflow(
    dream_config: DreamConfig,
    provider_catalog: ProviderCatalog,
    workflow_name: str,
) -> WorkflowProfile:
    workflow = dream_config.workflows.get(
        workflow_name,
        WorkflowProfile(provider="", model="", enabled=False),
    )
    provider = workflow.provider or provider_catalog.defaults.provider
    model = workflow.model or provider_catalog.defaults.model
    effective = replace(workflow, provider=provider, model=model)
    if effective.enabled and not effective.provider:
        raise ProviderCatalogError(f"workflow provider is not configured: {workflow_name}")
    if effective.enabled and effective.provider not in provider_catalog.providers:
        raise ProviderCatalogError(
            f"workflow provider profile is missing: {workflow_name}.{effective.provider}",
        )
    if effective.enabled and not effective.model:
        raise ProviderCatalogError(f"workflow model is not configured: {workflow_name}")
    return effective
```

- [ ] **Step 5: Update dream/autostart consumers**

Where code reads `dream_config.workflows["crystallization"]`, load provider catalog and call `resolve_effective_workflow()`. Use the resolved workflow provider id and model when calling `ProviderRegistry.check()` or resolving a `DreamProvider`.

Example replacement pattern:

```python
provider_catalog = load_provider_catalog(config)
workflow = resolve_effective_workflow(
    dream_config,
    provider_catalog,
    "crystallization",
)
```

- [ ] **Step 6: Run provider/workflow tests**

Run:

```bash
uv run pytest tests/test_dream_providers.py tests/test_dream_workflows.py tests/test_dream_autostart.py -q
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/dream_providers.py src/hieronymus/dream_workflows.py src/hieronymus/dream_autostart.py tests/test_dream_providers.py tests/test_dream_workflows.py tests/test_dream_autostart.py
git commit -m "feat: resolve workflows through provider catalog"
```

## Task 4: Update Config Bridge Contract

**Files:**
- Modify: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `src/hieronymus/cli.py`
- Test: `tests/test_tui_bridge_config.py`
- Test: `tests/test_cli.py`

- [ ] **Step 1: Write failing bridge tests**

Add to `tests/test_tui_bridge_config.py`:

```python
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderDefaults,
    ProviderProfile as CatalogProviderProfile,
    load_provider_catalog,
    save_provider_catalog,
)


def test_config_bootstrap_exposes_provider_catalog_and_workflow_assignments(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "deepseek-api": CatalogProviderProfile(
                    name="Deepseek",
                    type="openai",
                    url="https://api.deepseek.com",
                    key="secret",
                )
            },
            defaults=ProviderDefaults(
                provider="deepseek-api",
                model="deepseek-v4-flash",
            ),
        ),
    )

    payload = ConfigBridge(config).bootstrap({})

    assert payload["config_paths"]["provider_config_path"].endswith("provider.conf")
    assert payload["provider_catalog"]["profiles"]["deepseek-api"]["key"] == "***"
    assert payload["provider_catalog"]["defaults"] == {
        "provider": "deepseek-api",
        "model": "deepseek-v4-flash",
    }
    assert "provider_catalog" in {section["id"] for section in payload["form_schema"]["sections"]}


def test_config_save_updates_provider_conf_not_dream_conf(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)

    payload = bridge.save(
        {
            "provider_catalog": {
                "profiles": {
                    "deepseek-api": {
                        "name": "Deepseek",
                        "type": "openai",
                        "url": "https://api.deepseek.com",
                        "key": "raw-secret",
                        "timeout_seconds": "30",
                    }
                },
                "defaults": {
                    "provider": "deepseek-api",
                    "model": "deepseek-v4-flash",
                },
            },
            "workflows": {
                "crystallization": {
                    "provider": "deepseek-api",
                    "model": "deepseek-v4-flash",
                    "enabled": "yes",
                }
            },
        }
    )

    assert payload["validation"]["ok"] is True
    assert load_provider_catalog(config).providers["deepseek-api"].key == "raw-secret"
    assert "providers" not in config.dream_config_path.read_text(encoding="utf-8")
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py -q
```

Expected: missing `provider_catalog` payload and form schema section.

- [ ] **Step 3: Extend form schema**

In `_form_schema()` in `src/hieronymus/tui_bridge/config_api.py`, add sections/groups:

```python
{
    "id": "provider_catalog",
    "label": "Providers",
    "description": "provider.conf",
}
```

Add fields with keys:

```python
provider_catalog.defaults.provider
provider_catalog.defaults.model
provider_catalog.profile.name
provider_catalog.profile.type
provider_catalog.profile.url
provider_catalog.profile.key
provider_catalog.profile.timeout_seconds
workflows.crystallization.provider
workflows.crystallization.model
workflows.crystallization.enabled
workflows.reinforcement_compaction.provider
workflows.reinforcement_compaction.model
workflows.reinforcement_compaction.enabled
workflows.relation_discovery.provider
workflows.relation_discovery.model
workflows.relation_discovery.enabled
```

Keep field generation Python-owned so the frontend does not invent config structure.

- [ ] **Step 4: Add provider catalog payload methods**

In `ConfigBridge._payload()`, load `ProviderCatalog` and include:

```python
"provider_catalog": {
    "profiles": redacted_provider_catalog_payload(provider_catalog),
    "defaults": provider_catalog.defaults.to_payload(),
},
```

Also add:

```python
"provider_config_path": str(self.config.provider_config_path),
```

to `config_paths`.

- [ ] **Step 5: Route provider form updates to provider.conf**

Implement bridge helpers:

```python
def _provider_catalog_from_params(
    self,
    params: dict[str, object],
) -> tuple[ProviderCatalog, str]:
    try:
        catalog = load_provider_catalog(self.config)
    except ProviderCatalogError as error:
        catalog = default_provider_catalog()
        load_error = str(error)
    else:
        load_error = ""
    raw_catalog = params.get("provider_catalog")
    if type(raw_catalog) is dict:
        catalog = _catalog_from_bridge_payload(catalog, raw_catalog, self._pending_api_keys)
    return catalog, load_error
```

Call `save_provider_catalog()` from `save()` and never write provider keys into `dream.conf`.

- [ ] **Step 6: Run bridge tests**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py tests/test_cli.py -q
```

Expected: all pass after updating expected payloads.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py src/hieronymus/cli.py tests/test_tui_bridge_config.py tests/test_cli.py
git commit -m "feat: expose provider catalog in config bridge"
```

## Task 5: Update React/OpenTUI Config Editor

**Files:**
- Modify: `frontend/src/rpc/schema.ts`
- Modify: `frontend/src/config/ConfigScreen.tsx`
- Modify: `frontend/src/config/ConfigForm.tsx`
- Test: `frontend/src/config/ConfigScreen.test.tsx`
- Test: `frontend/src/rpc/schema.test.ts`

- [ ] **Step 1: Write failing frontend tests**

Add to `frontend/src/config/ConfigScreen.test.tsx`:

```tsx
it("renders provider catalog separately from workflow assignments", async () => {
  const { render, waitForFrame } = setupTest();

  await render(<ConfigScreen initial={payloadWithProviderCatalog()} client={undefined} />);

  const output = await waitForFrame((frame) =>
    frame.includes("Providers") && frame.includes("Workflows"),
  );
  expect(output).toContain("Providers");
  expect(output).toContain("provider.conf");
  expect(output).toContain("deepseek-api");
  expect(output).toContain("Workflows");
  expect(output).toContain("Crystallization Provider: deepseek-api");
  expect(output).not.toContain("Provider/API");
})
```

Add helper:

```tsx
function payloadWithProviderCatalog(): ConfigBootstrap {
  return {
    ...payload(),
    config_paths: {
      ...payload().config_paths,
      provider_config_path: "/tmp/provider.conf",
    },
    provider_catalog: {
      profiles: {
        "deepseek-api": {
          name: "Deepseek",
          type: "openai",
          url: "https://api.deepseek.com",
          key: "***",
          timeout_seconds: 30,
        },
      },
      defaults: {
        provider: "deepseek-api",
        model: "deepseek-v4-flash",
      },
    },
  };
}
```

- [ ] **Step 2: Run frontend tests to verify failure**

Run:

```bash
cd frontend
bun test src/config/ConfigScreen.test.tsx -t "provider catalog separately"
```

Expected: schema parse or render failure.

- [ ] **Step 3: Extend runtime schema**

In `frontend/src/rpc/schema.ts`, add:

```ts
const ProviderCatalogSchema = z.object({
  profiles: z.record(
    z.object({
      name: z.string(),
      type: z.string(),
      url: z.string(),
      key: z.string().default(""),
      timeout_seconds: z.union([z.number(), z.string()]).default(30),
    }).passthrough(),
  ),
  defaults: z.object({
    provider: z.string().default(""),
    model: z.string().default(""),
  }).passthrough(),
}).passthrough();
```

Add to `ConfigBootstrapSchema`:

```ts
provider_catalog: ProviderCatalogSchema.default({
  profiles: {},
  defaults: { provider: "", model: "" },
}),
```

- [ ] **Step 4: Render provider catalog and workflow groups from backend schema**

Keep `ConfigForm` generic. Update `ConfigScreen` mapping so field keys under `provider_catalog.` and `workflows.` read/write new local form maps:

```ts
type ConfigFormValues = {
  providerCatalog: Record<string, string>;
  workflows: Record<string, string>;
  dreaming: Record<string, string>;
  ingest: Record<string, string>;
  release: Record<string, string>;
};
```

Route `provider_catalog.*` keys to `providerCatalog` and `workflows.*` keys to `workflows`. Submit payload:

```ts
params: {
  provider_catalog: draftValues.providerCatalog,
  workflows: draftValues.workflows,
  dreaming: draftValues.dreaming,
  ingest: draftValues.ingest,
  release: draftValues.release,
}
```

- [ ] **Step 5: Run frontend tests**

Run:

```bash
cd frontend
bun test src/config/ConfigScreen.test.tsx src/rpc/schema.test.ts
bun run typecheck
```

Expected: tests and typecheck pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/rpc/schema.ts frontend/src/rpc/schema.test.ts frontend/src/config/ConfigScreen.tsx frontend/src/config/ConfigForm.tsx frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: edit provider catalog in config tui"
```

## Task 6: Update Doctor, Docs, and End-to-End Verification

**Files:**
- Modify: `src/hieronymus/doctor.py`
- Modify: `docs/usage.md`
- Modify: `docs/memory-dreaming.md`
- Test: `tests/test_doctor.py`
- Test: `tests/test_cli_opentui.py`

- [ ] **Step 1: Write failing doctor test**

Add to `tests/test_doctor.py`:

```python
def test_doctor_reports_missing_provider_catalog_profile(config: HieronymusConfig) -> None:
    save_dream_config(
        config,
        default_dream_config().with_workflow(
            "crystallization",
            WorkflowProfile(
                provider="missing-profile",
                model="deepseek-v4-flash",
                enabled=True,
            ),
        ),
    )

    report = Doctor(config).run()

    assert any(
        finding.message == "workflow provider profile is missing: crystallization.missing-profile"
        for finding in report.errors
    )
```

- [ ] **Step 2: Run doctor test to verify failure**

Run:

```bash
uv run pytest tests/test_doctor.py::test_doctor_reports_missing_provider_catalog_profile -q
```

Expected: finding is absent.

- [ ] **Step 3: Add doctor provider catalog checks**

In `src/hieronymus/doctor.py`, load provider catalog and validate each enabled workflow through `resolve_effective_workflow()`. Add errors for `ProviderCatalogError` messages.

Use exact message propagation:

```python
try:
    provider_catalog = load_provider_catalog(self.config)
    dream_config = load_dream_config(self.config)
    for workflow_name in dream_config.workflows:
        resolve_effective_workflow(dream_config, provider_catalog, workflow_name)
except ProviderCatalogError as error:
    errors.append(DoctorFinding(str(error)))
```

- [ ] **Step 4: Update docs**

In `docs/usage.md`, replace examples showing `[providers.*]` in `dream.conf` with:

```toml
# provider.conf
[deepseek-api]
name = "Deepseek"
type = "openai"
url = "https://api.deepseek.com"
key = "..."

[defaults]
provider = "deepseek-api"
model = "deepseek-v4-flash"

# dream.conf
[workflows.crystallization]
provider = "deepseek-api"
model = "deepseek-v4-flash"
enabled = true
```

In `docs/memory-dreaming.md`, state that dreaming phases consume workflow assignments from `dream.conf` and provider endpoint profiles from `provider.conf`.

- [ ] **Step 5: Run final verification**

Run:

```bash
cd frontend
bun test
bun run typecheck
bun run build
cd ..
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected:

- frontend tests pass;
- TypeScript typecheck passes;
- frontend bundle builds;
- `uv run pytest` reports all Python tests passing;
- ruff check and format check pass.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/doctor.py docs/usage.md docs/memory-dreaming.md tests/test_doctor.py tests/test_cli_opentui.py
git commit -m "docs: document provider catalog workflow assignments"
```

## Self-Review

Spec coverage:

- ADR requires global `provider.conf`: Task 1 implements model and config path.
- ADR requires defaults provider+model: Task 1 validates defaults and Task 3 resolves them.
- ADR requires `dream.conf` workflow provider+model assignments: Task 2 removes provider ownership and keeps workflows.
- ADR requires deterministic fallback/fail-closed behavior: Task 3 implements `resolve_effective_workflow`.
- ADR requires TUI separation: Task 4 exposes bridge contract and Task 5 renders it.
- ADR requires migration: Task 2 adds migration helper and tests.
- ADR requires redaction and secret preservation: Task 4 routes provider key edits to `provider.conf`.
- ADR requires docs: Task 6 updates user-facing docs.

Placeholder scan: no placeholder steps remain; every task includes exact files, commands, and expected results.

Type consistency: this plan consistently uses `ProviderCatalog`, `ProviderDefaults`, catalog `ProviderProfile`, dream `WorkflowProfile`, `provider.conf`, and `dream.conf`.

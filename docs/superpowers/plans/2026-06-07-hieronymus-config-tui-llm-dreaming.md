# Hieronymus Config TUI and LLM Dreaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the config placeholder with a real config TUI, persisted settings, provider health checks, OpenAI/Gemini/Anthropic dreaming providers, and cycle-based automatic dreaming controls.

**Architecture:** Add a typed settings layer over `settings.toml`, then build a provider registry that can validate config, check provider health, and create `DreamProvider` instances. Keep CLI, MCP, service status, doctor, admin, and the Textual config TUI as thin clients over the settings/provider/dreaming facades.

**Tech Stack:** Python 3.12, Click, Textual/Rich, TOML via `tomllib`/`tomli-w`, stdlib `urllib` HTTP transport, SQLite, pytest, ruff.

---

## File Structure

- Create `src/hieronymus/settings.py`: dataclasses, defaults, validation, atomic `settings.toml` load/save, JSON-safe payloads.
- Modify `src/hieronymus/config.py`: add `settings_path` property.
- Create `src/hieronymus/dream_providers.py`: provider metadata, registry, config validation, provider checks, REST transport, LLM response parsing, provider factory.
- Modify `src/hieronymus/dreaming.py`: expose provider name in `DreamRunRecord`, keep provider output validation central.
- Create `src/hieronymus/dream_autostart.py`: pending short-term memory counts, interval/threshold decisions, autostart run state.
- Modify `src/hieronymus/cli.py`: launch config TUI, emit config JSON, use configured dream provider, add `hiero dream --json`.
- Modify `src/hieronymus/admin.py`: run manual dreaming through configured active provider.
- Modify `src/hieronymus/mcp_server.py`: use configured provider by default and accept any enabled registered provider.
- Modify `src/hieronymus/service_http.py`: report provider and dreaming automation status instead of `providers: []`.
- Modify `src/hieronymus/doctor.py`: validate settings and active provider health.
- Create `src/hieronymus/tui/config_app.py`, `src/hieronymus/tui/config_screens.py`: Textual config UI.
- Modify `src/hieronymus/tui/styles.tcss`: shared config TUI styling.
- Create tests: `tests/test_settings.py`, `tests/test_dream_providers.py`, `tests/test_dream_autostart.py`, `tests/test_config_tui.py`.
- Modify tests: `tests/test_cli_service.py`, `tests/test_dreaming.py`, `tests/test_admin_store.py`, `tests/test_mcp_server.py`, `tests/test_service_http.py`, `tests/test_doctor.py`, docs tests.
- Modify docs: `README.md`, `docs/usage.md`, `docs/memory-dreaming.md`, `docs/service-toolkit.md`.

## Ground Rules

- API key values are never written to `settings.toml`, JSON output, logs, failed dream run errors, or TUI detail panes.
- OpenAI, Gemini, and Anthropic are all real provider adapters in this plan. Deterministic remains the default and offline fallback.
- Network behavior is tested with injected transports. CI must not need real API keys.
- Strict terminology remains review-gated. LLM dream output only creates crystals and strict concept proposals through the existing validation path.
- No production-facing text should say config TUI, provider checks, or external LLM dreaming are deferred.

## Task 1: Persisted Settings Model

**Files:**
- Create: `src/hieronymus/settings.py`
- Modify: `src/hieronymus/config.py`
- Create: `tests/test_settings.py`

- [ ] **Step 1: Add failing settings tests**

Create `tests/test_settings.py`:

```python
from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.settings import (
    DreamingSettings,
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)


def test_load_settings_returns_defaults_when_file_is_missing(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    settings = load_settings(config)

    assert config.settings_path == config.config_root / "settings.toml"
    assert settings.dreaming.active_provider == "deterministic"
    assert settings.dreaming.autostart_enabled is False
    assert settings.dreaming.min_interval_minutes == 30
    assert settings.dreaming.new_short_term_memory_threshold == 25
    assert settings.dreaming.max_cycles_per_autostart == 1
    assert settings.providers["deterministic"].enabled is True
    assert settings.providers["openai"].model == "gpt-4.1-mini"
    assert settings.providers["openai"].base_url == "https://api.openai.com/v1"
    assert settings.providers["openai"].api_key_env == "OPENAI_API_KEY"
    assert settings.providers["gemini"].model == "gemini-2.5-flash"
    assert settings.providers["gemini"].api_key_env == "GEMINI_API_KEY"
    assert settings.providers["anthropic"].model == "claude-3-5-haiku-latest"
    assert settings.providers["anthropic"].api_key_env == "ANTHROPIC_API_KEY"


def test_save_settings_writes_toml_without_secret_values(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_KEY",
            base_url="https://llm.example.test/v1",
        ),
    )

    save_settings(config, settings)

    raw = config.settings_path.read_text(encoding="utf-8")
    assert "secret" not in raw.lower()
    assert "HIERONYMUS_OPENAI_KEY" in raw
    payload = tomllib.loads(raw)
    assert payload["providers"]["openai"]["enabled"] is True
    assert payload["providers"]["openai"]["base_url"] == "https://llm.example.test/v1"


def test_load_settings_rejects_malformed_toml(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text("[dreaming\n", encoding="utf-8")

    with pytest.raises(SettingsError, match="settings.toml is not valid TOML"):
        load_settings(config)


def test_load_settings_rejects_invalid_active_provider(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text(
        "[dreaming]\nactive_provider = 'missing'\n",
        encoding="utf-8",
    )

    with pytest.raises(SettingsError, match="active provider is not configured: missing"):
        load_settings(config)


def test_load_settings_rejects_non_positive_dreaming_values(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text(
        "[dreaming]\nmin_interval_minutes = 0\nnew_short_term_memory_threshold = 25\nmax_cycles_per_autostart = 1\n",
        encoding="utf-8",
    )

    with pytest.raises(SettingsError, match="min_interval_minutes must be at least 1"):
        load_settings(config)


def test_settings_to_json_masks_key_source_only(tmp_path: Path) -> None:
    settings = load_settings(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    payload = settings.to_json_dict()

    assert payload["dreaming"]["active_provider"] == "deterministic"
    assert payload["providers"]["openai"]["api_key_env"] == "OPENAI_API_KEY"
    assert "api_key" not in payload["providers"]["openai"]
```

- [ ] **Step 2: Run settings tests to verify failure**

Run:

```bash
uv run pytest tests/test_settings.py -v
```

Expected: FAIL because `hieronymus.settings` and `HieronymusConfig.settings_path` do not exist.

- [ ] **Step 3: Add `settings_path`**

Modify `src/hieronymus/config.py`:

```python
    @property
    def settings_path(self) -> Path:
        return self.config_root / "settings.toml"
```

- [ ] **Step 4: Implement settings model**

Create `src/hieronymus/settings.py` with these public names and behavior:

```python
from __future__ import annotations

import tomllib
from dataclasses import dataclass, replace

import tomli_w

from hieronymus.config import HieronymusConfig


class SettingsError(ValueError):
    pass


@dataclass(frozen=True)
class DreamingSettings:
    active_provider: str = "deterministic"
    autostart_enabled: bool = False
    min_interval_minutes: int = 30
    new_short_term_memory_threshold: int = 25
    max_cycles_per_autostart: int = 1

    def to_json_dict(self) -> dict[str, object]:
        return {
            "active_provider": self.active_provider,
            "autostart_enabled": self.autostart_enabled,
            "min_interval_minutes": self.min_interval_minutes,
            "new_short_term_memory_threshold": self.new_short_term_memory_threshold,
            "max_cycles_per_autostart": self.max_cycles_per_autostart,
        }


@dataclass(frozen=True)
class ProviderSettings:
    enabled: bool = False
    model: str = ""
    api_key_env: str = ""
    base_url: str = ""
    timeout_seconds: float = 30.0

    def to_json_dict(self) -> dict[str, object]:
        payload: dict[str, object] = {
            "enabled": self.enabled,
            "model": self.model,
            "api_key_env": self.api_key_env,
            "timeout_seconds": self.timeout_seconds,
        }
        if self.base_url:
            payload["base_url"] = self.base_url
        return payload


@dataclass(frozen=True)
class HieronymusSettings:
    dreaming: DreamingSettings
    providers: dict[str, ProviderSettings]

    def with_provider(self, name: str, provider: ProviderSettings) -> HieronymusSettings:
        providers = dict(self.providers)
        providers[name] = provider
        return replace(self, providers=providers)

    def with_dreaming(self, dreaming: DreamingSettings) -> HieronymusSettings:
        return replace(self, dreaming=dreaming)

    def to_json_dict(self) -> dict[str, object]:
        return {
            "dreaming": self.dreaming.to_json_dict(),
            "providers": {
                name: provider.to_json_dict()
                for name, provider in sorted(self.providers.items())
            },
        }
```

Implement helper functions:

```python
def default_settings() -> HieronymusSettings:
    return HieronymusSettings(
        dreaming=DreamingSettings(),
        providers={
            "deterministic": ProviderSettings(enabled=True),
            "openai": ProviderSettings(
                enabled=False,
                model="gpt-4.1-mini",
                api_key_env="OPENAI_API_KEY",
                base_url="https://api.openai.com/v1",
            ),
            "gemini": ProviderSettings(
                enabled=False,
                model="gemini-2.5-flash",
                api_key_env="GEMINI_API_KEY",
            ),
            "anthropic": ProviderSettings(
                enabled=False,
                model="claude-3-5-haiku-latest",
                api_key_env="ANTHROPIC_API_KEY",
            ),
        },
    )
```

`load_settings(config)` must return defaults when the file is absent, parse TOML when present, merge with defaults so omitted providers keep default fields, and raise `SettingsError` for parse or validation failures. `save_settings(config, settings)` must call validation first, create `config.config_root`, write TOML to `settings.toml.tmp-<pid>`, and atomically replace `settings.toml`.

Validation rules:

```python
def validate_settings(settings: HieronymusSettings) -> None:
    if settings.dreaming.active_provider not in settings.providers:
        raise SettingsError(f"active provider is not configured: {settings.dreaming.active_provider}")
    if settings.dreaming.min_interval_minutes < 1:
        raise SettingsError("min_interval_minutes must be at least 1")
    if settings.dreaming.new_short_term_memory_threshold < 1:
        raise SettingsError("new_short_term_memory_threshold must be at least 1")
    if settings.dreaming.max_cycles_per_autostart < 1:
        raise SettingsError("max_cycles_per_autostart must be at least 1")
    for name, provider in settings.providers.items():
        if provider.enabled and name != "deterministic" and not provider.model.strip():
            raise SettingsError(f"{name} model must not be empty when provider is enabled")
        if provider.enabled and name != "deterministic" and not provider.api_key_env.strip():
            raise SettingsError(f"{name} api_key_env must not be empty when provider is enabled")
```

Use `tomli_w.dumps()` to serialize dictionaries. Do not add an `api_key` field anywhere.

- [ ] **Step 5: Run settings tests**

Run:

```bash
uv run pytest tests/test_settings.py -v
```

Expected: PASS.

- [ ] **Step 6: Commit settings model**

```bash
git add src/hieronymus/config.py src/hieronymus/settings.py tests/test_settings.py
git commit -m "feat: add persisted settings model"
```

## Task 2: Provider Registry, Status, and Health Checks

**Files:**
- Create: `src/hieronymus/dream_providers.py`
- Create: `tests/test_dream_providers.py`

- [ ] **Step 1: Add failing provider registry and check tests**

Create `tests/test_dream_providers.py`:

```python
from __future__ import annotations

import json
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import (
    HTTPResponse,
    ProviderCheckResult,
    ProviderRegistry,
    resolve_provider,
)
from hieronymus.settings import ProviderSettings, load_settings, save_settings


@dataclass
class FakeTransport:
    response: HTTPResponse
    requests: list[dict[str, object]]

    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse:
        self.requests.append(
            {"url": url, "headers": headers, "payload": payload, "timeout": timeout}
        )
        return self.response


def test_registry_lists_real_providers() -> None:
    registry = ProviderRegistry()

    assert [provider.name for provider in registry.list()] == [
        "deterministic",
        "openai",
        "gemini",
        "anthropic",
    ]


def test_provider_status_marks_missing_env_for_enabled_provider(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="MISSING_OPENAI_KEY",
            base_url="https://api.openai.com/v1",
        ),
    )
    save_settings(config, settings)
    monkeypatch.delenv("MISSING_OPENAI_KEY", raising=False)

    statuses = ProviderRegistry().status_payload(config)

    openai = next(item for item in statuses if item["name"] == "openai")
    assert openai["enabled"] is True
    assert openai["configured"] is False
    assert openai["error"] == "missing environment variable: MISSING_OPENAI_KEY"


def test_deterministic_check_passes_without_network(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    result = ProviderRegistry().check(config, "deterministic")

    assert result == ProviderCheckResult(
        name="deterministic",
        ok=True,
        model="",
        error="",
        latency_ms=None,
    )


def test_openai_check_uses_temporary_key_without_saving_secret(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://llm.example.test/v1",
        ),
    )
    save_settings(config, settings)
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"id": "ok"})),
        [],
    )

    result = ProviderRegistry(transport=transport).check(
        config,
        "openai",
        temporary_api_key="secret-test-key",
    )

    assert result.ok is True
    assert transport.requests[0]["url"] == "https://llm.example.test/v1/chat/completions"
    assert transport.requests[0]["headers"]["Authorization"] == "Bearer secret-test-key"
    assert "secret-test-key" not in config.settings_path.read_text(encoding="utf-8")


def test_resolve_provider_rejects_disabled_provider(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    try:
        resolve_provider(config, "openai")
    except ValueError as exc:
        assert str(exc) == "dream provider is disabled: openai"
    else:
        raise AssertionError("disabled provider should fail")
```

- [ ] **Step 2: Run provider tests to verify failure**

Run:

```bash
uv run pytest tests/test_dream_providers.py -v
```

Expected: FAIL because `hieronymus.dream_providers` does not exist.

- [ ] **Step 3: Implement provider metadata and HTTP transport**

Create `src/hieronymus/dream_providers.py` with:

```python
from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.dreaming import DeterministicDreamProvider, DreamProvider
from hieronymus.settings import ProviderSettings, load_settings


@dataclass(frozen=True)
class HTTPResponse:
    status: int
    body: str


class HTTPTransport(Protocol):
    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse: ...


class UrllibTransport:
    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        request = urllib.request.Request(url, data=body, method="POST")
        request.add_header("Content-Type", "application/json")
        for key, value in headers.items():
            request.add_header(key, value)
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return HTTPResponse(
                    status=int(response.status),
                    body=response.read().decode("utf-8"),
                )
        except urllib.error.HTTPError as exc:
            return HTTPResponse(status=int(exc.code), body=exc.read().decode("utf-8"))
```

Add dataclasses:

```python
@dataclass(frozen=True)
class ProviderMetadata:
    name: str
    display_name: str
    requires_api_key: bool
    supports_base_url: bool


@dataclass(frozen=True)
class ProviderCheckResult:
    name: str
    ok: bool
    model: str
    error: str = ""
    latency_ms: int | None = None

    def to_json_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "ok": self.ok,
            "model": self.model,
            "error": self.error,
            "latency_ms": self.latency_ms,
        }
```

- [ ] **Step 4: Implement registry status and checks**

In `src/hieronymus/dream_providers.py`, implement:

```python
class ProviderRegistry:
    def __init__(self, transport: HTTPTransport | None = None) -> None:
        self.transport = transport if transport is not None else UrllibTransport()
        self._providers = (
            ProviderMetadata("deterministic", "Deterministic", False, False),
            ProviderMetadata("openai", "OpenAI compatible", True, True),
            ProviderMetadata("gemini", "Gemini", True, False),
            ProviderMetadata("anthropic", "Anthropic", True, False),
        )

    def list(self) -> list[ProviderMetadata]:
        return list(self._providers)

    def metadata(self, name: str) -> ProviderMetadata:
        for provider in self._providers:
            if provider.name == name:
                return provider
        raise ValueError(f"unsupported dream provider: {name}")

    def status_payload(self, config: HieronymusConfig) -> list[dict[str, object]]:
        settings = load_settings(config)
        rows = []
        for metadata in self._providers:
            provider = settings.providers[metadata.name]
            configured, error = _configured_status(metadata.name, provider)
            rows.append(
                {
                    "name": metadata.name,
                    "display_name": metadata.display_name,
                    "enabled": provider.enabled,
                    "configured": configured,
                    "model": provider.model,
                    "api_key_env": provider.api_key_env,
                    "base_url": provider.base_url,
                    "error": error,
                }
            )
        return rows

    def check(
        self,
        config: HieronymusConfig,
        name: str,
        *,
        temporary_api_key: str | None = None,
    ) -> ProviderCheckResult:
        settings = load_settings(config)
        provider = settings.providers.get(name)
        if provider is None:
            raise ValueError(f"unsupported dream provider: {name}")
        if name == "deterministic":
            return ProviderCheckResult(name="deterministic", ok=True, model="")
        key = temporary_api_key or os.environ.get(provider.api_key_env, "")
        if not key:
            return ProviderCheckResult(
                name=name,
                ok=False,
                model=provider.model,
                error=f"missing environment variable: {provider.api_key_env}",
            )
        started = time.monotonic()
        response = self._check_remote(name, provider, key)
        latency_ms = round((time.monotonic() - started) * 1000)
        if 200 <= response.status < 300:
            return ProviderCheckResult(name=name, ok=True, model=provider.model, latency_ms=latency_ms)
        return ProviderCheckResult(
            name=name,
            ok=False,
            model=provider.model,
            error=f"provider returned HTTP {response.status}",
            latency_ms=latency_ms,
        )
```

`_check_remote()` must send minimal non-dreaming requests:

- OpenAI: `POST {base_url}/chat/completions` with `Authorization: Bearer <key>`, model, one user message `"Reply with ok."`, `max_tokens: 1`.
- Gemini: `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key=<key>` with one user text part `"Reply with ok."`.
- Anthropic: `POST https://api.anthropic.com/v1/messages` with `x-api-key`, `anthropic-version: 2023-06-01`, model, `max_tokens: 1`, one user message.

Add helper:

```python
def _configured_status(name: str, provider: ProviderSettings) -> tuple[bool, str]:
    if name == "deterministic":
        return True, ""
    if not provider.model.strip():
        return False, "model is empty"
    if not provider.api_key_env.strip():
        return False, "api_key_env is empty"
    if provider.enabled and not os.environ.get(provider.api_key_env):
        return False, f"missing environment variable: {provider.api_key_env}"
    return True, ""
```

- [ ] **Step 5: Add provider factory stub**

Still in `dream_providers.py`, add the factory boundary used by later tasks:

```python
def resolve_provider(
    config: HieronymusConfig,
    name: str | None = None,
    *,
    transport: HTTPTransport | None = None,
) -> DreamProvider:
    settings = load_settings(config)
    provider_name = settings.dreaming.active_provider if name is None else name
    provider_settings = settings.providers.get(provider_name)
    if provider_settings is None:
        raise ValueError(f"unsupported dream provider: {provider_name}")
    if not provider_settings.enabled:
        raise ValueError(f"dream provider is disabled: {provider_name}")
    if provider_name == "deterministic":
        return DeterministicDreamProvider()
    raise ValueError(f"dream provider is not implemented: {provider_name}")
```

The OpenAI/Gemini/Anthropic factory branches are completed in Task 3. The exact error above is temporary only inside this task and must be removed by Task 3.

- [ ] **Step 6: Run provider tests**

Run:

```bash
uv run pytest tests/test_dream_providers.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit provider registry**

```bash
git add src/hieronymus/dream_providers.py tests/test_dream_providers.py
git commit -m "feat: add dream provider registry"
```

## Task 3: OpenAI, Gemini, and Anthropic Dream Providers

**Files:**
- Modify: `src/hieronymus/dream_providers.py`
- Modify: `tests/test_dream_providers.py`

- [ ] **Step 1: Add failing LLM provider parsing tests**

Append to `tests/test_dream_providers.py`:

```python
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.settings import DreamingSettings


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="en",
        task_type="translation",
    )


def _memory() -> ShortTermMemoryRecord:
    return ShortTermMemoryRecord(
        id=7,
        session_id=3,
        source_role="user",
        kind="style",
        text="Use compact UI labels for inventory skill names.",
        source_ref="chapter 1",
        metadata={},
    )


def _llm_payload() -> dict[str, object]:
    return {
        "crystals": [
            {
                "crystal_type": "lesson",
                "title": "Compact UI Labels",
                "text": "Use compact UI labels for inventory skill names.",
                "strength": 0.7,
                "confidence": 0.8,
                "source_memory_ids": [7],
            }
        ],
        "concept_proposals": [
            {
                "series_slug": "only-sense-online",
                "source_language": "ja",
                "target_language": "en",
                "concept_text": "Sense",
                "source_form": "センス",
                "canonical_rendering": "Sense",
                "approved_variants": ["Sense"],
                "forbidden_variants": ["Senses"],
                "rationale": "Existing series terminology.",
            }
        ],
    }


def test_openai_provider_crystallizes_structured_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://api.openai.test/v1",
        ),
    ).with_dreaming(DreamingSettings(active_provider="openai"))
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps(
                {"choices": [{"message": {"content": json.dumps(_llm_payload())}}]}
            ),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].title == "Compact UI Labels"
    assert output.concept_proposals[0].source_form == "センス"
    assert transport.requests[0]["url"] == "https://api.openai.test/v1/chat/completions"


def test_gemini_provider_crystallizes_structured_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "gemini",
        ProviderSettings(enabled=True, model="gemini-2.5-flash", api_key_env="GEMINI_API_KEY"),
    ).with_dreaming(DreamingSettings(active_provider="gemini"))
    save_settings(config, settings)
    monkeypatch.setenv("GEMINI_API_KEY", "secret-gemini")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps(
                {"candidates": [{"content": {"parts": [{"text": json.dumps(_llm_payload())}]}}]}
            ),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].source_memory_ids == [7]
    assert "generateContent?key=secret-gemini" in transport.requests[0]["url"]


def test_anthropic_provider_crystallizes_structured_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "anthropic",
        ProviderSettings(
            enabled=True,
            model="claude-3-5-haiku-latest",
            api_key_env="ANTHROPIC_API_KEY",
        ),
    ).with_dreaming(DreamingSettings(active_provider="anthropic"))
    save_settings(config, settings)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "secret-anthropic")
    transport = FakeTransport(
        HTTPResponse(
            status=200,
            body=json.dumps({"content": [{"type": "text", "text": json.dumps(_llm_payload())}]}),
        ),
        [],
    )

    provider = resolve_provider(config, transport=transport)
    output = provider.crystallize(_context(), [_memory()])

    assert output.crystals[0].confidence == 0.8
    assert transport.requests[0]["headers"]["x-api-key"] == "secret-anthropic"


def test_llm_provider_rejects_invalid_json_response(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="OPENAI_API_KEY",
            base_url="https://api.openai.test/v1",
        ),
    ).with_dreaming(DreamingSettings(active_provider="openai"))
    save_settings(config, settings)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = FakeTransport(
        HTTPResponse(status=200, body=json.dumps({"choices": [{"message": {"content": "nope"}}]})),
        [],
    )

    provider = resolve_provider(config, transport=transport)

    try:
        provider.crystallize(_context(), [_memory()])
    except ValueError as exc:
        assert str(exc) == "openai response did not contain valid dream JSON"
    else:
        raise AssertionError("invalid JSON should fail")
```

- [ ] **Step 2: Run LLM provider tests to verify failure**

Run:

```bash
uv run pytest tests/test_dream_providers.py -v
```

Expected: FAIL because `resolve_provider()` still raises `dream provider is not implemented`.

- [ ] **Step 3: Implement shared LLM provider base**

In `src/hieronymus/dream_providers.py`, import the dream output dataclasses and memory/context types:

```python
from hieronymus.dreaming import (
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
)
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
```

Add:

```python
def _dream_prompt(context: TranslationContext, memories: list[ShortTermMemoryRecord]) -> str:
    memory_payload = [
        {
            "id": memory.id,
            "source_role": memory.source_role,
            "kind": memory.kind,
            "text": memory.text,
            "source_ref": memory.source_ref,
        }
        for memory in memories
    ]
    return json.dumps(
        {
            "instruction": (
                "Return only JSON with keys crystals and concept_proposals. "
                "Use only provided source memory ids. Do not add markdown."
            ),
            "context": {
                "series_slug": context.series_slug,
                "source_language": context.source_language,
                "target_language": context.target_language,
                "task_type": context.task_type,
                "volume": context.volume,
                "chapter": context.chapter,
            },
            "memories": memory_payload,
            "schema": {
                "crystals": [
                    {
                        "crystal_type": "lesson|concept|erudition",
                        "title": "string",
                        "text": "string",
                        "strength": 0.7,
                        "confidence": 0.8,
                        "source_memory_ids": [1],
                    }
                ],
                "concept_proposals": [
                    {
                        "series_slug": context.series_slug,
                        "source_language": context.source_language,
                        "target_language": context.target_language,
                        "concept_text": "string",
                        "source_form": "string",
                        "canonical_rendering": "string",
                        "approved_variants": ["string"],
                        "forbidden_variants": ["string"],
                        "rationale": "string",
                    }
                ],
            },
        },
        ensure_ascii=False,
    )
```

Add `_parse_dream_json(provider_name, raw_text)` that `json.loads()` the raw text, requires dict keys
`crystals` and `concept_proposals` as lists, and maps dictionaries to `DreamCrystalCandidate` and
`DreamConceptProposal`. On JSON decode failure, raise:

```python
raise ValueError(f"{provider_name} response did not contain valid dream JSON") from error
```

On missing/malformed keys, raise:

```python
raise ValueError(f"{provider_name} response did not match dream schema")
```

- [ ] **Step 4: Implement OpenAI, Gemini, and Anthropic providers**

Add classes:

```python
class OpenAIDreamProvider:
    name = "openai"

    def __init__(self, settings: ProviderSettings, api_key: str, transport: HTTPTransport) -> None:
        self.settings = settings
        self.api_key = api_key
        self.transport = transport

    def crystallize(self, context: TranslationContext, memories: list[ShortTermMemoryRecord]) -> DreamOutput:
        response = self.transport.post_json(
            f"{self.settings.base_url.rstrip('/')}/chat/completions",
            headers={"Authorization": f"Bearer {self.api_key}"},
            payload={
                "model": self.settings.model,
                "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
                "temperature": 0.1,
                "response_format": {"type": "json_object"},
            },
            timeout=self.settings.timeout_seconds,
        )
        if not 200 <= response.status < 300:
            raise ValueError(f"openai returned HTTP {response.status}")
        payload = json.loads(response.body)
        text = payload["choices"][0]["message"]["content"]
        return _parse_dream_json("openai", str(text))
```

Implement `GeminiDreamProvider` similarly with URL
`https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}` and
payload:

```python
{
    "contents": [{"role": "user", "parts": [{"text": _dream_prompt(context, memories)}]}],
    "generationConfig": {"temperature": 0.1, "responseMimeType": "application/json"},
}
```

Extract text from:

```python
payload["candidates"][0]["content"]["parts"][0]["text"]
```

Implement `AnthropicDreamProvider` with URL `https://api.anthropic.com/v1/messages`, headers
`x-api-key` and `anthropic-version`, payload:

```python
{
    "model": self.settings.model,
    "max_tokens": 2000,
    "temperature": 0.1,
    "messages": [{"role": "user", "content": _dream_prompt(context, memories)}],
}
```

Extract text from:

```python
payload["content"][0]["text"]
```

- [ ] **Step 5: Complete provider factory**

Replace the Task 2 provider-not-implemented branch in `resolve_provider()`:

```python
    key = os.environ.get(provider_settings.api_key_env, "")
    if not key:
        raise ValueError(
            f"missing environment variable for {provider_name}: {provider_settings.api_key_env}"
        )
    active_transport = transport if transport is not None else UrllibTransport()
    if provider_name == "openai":
        return OpenAIDreamProvider(provider_settings, key, active_transport)
    if provider_name == "gemini":
        return GeminiDreamProvider(provider_settings, key, active_transport)
    if provider_name == "anthropic":
        return AnthropicDreamProvider(provider_settings, key, active_transport)
    raise ValueError(f"unsupported dream provider: {provider_name}")
```

- [ ] **Step 6: Run provider tests**

Run:

```bash
uv run pytest tests/test_dream_providers.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit LLM providers**

```bash
git add src/hieronymus/dream_providers.py tests/test_dream_providers.py
git commit -m "feat: add llm dream providers"
```

## Task 4: Wire Configured Providers Into CLI, Admin, and MCP Dreaming

**Files:**
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `tests/test_cli_service.py`
- Modify: `tests/test_dreaming.py`
- Modify: `tests/test_admin_store.py`
- Modify: `tests/test_mcp_server.py`

- [ ] **Step 1: Add failing CLI dream tests**

Append to `tests/test_cli_service.py`:

```python
def test_dream_json_uses_configured_active_provider(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    settings = load_settings(config).with_dreaming(
        DreamingSettings(active_provider="deterministic")
    )
    save_settings(config, settings)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(data_root), "dream", "--json"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["provider"] == "deterministic"
    assert payload["status"] == "completed"
    assert payload["input_count"] == 0
    assert payload["created_crystal_count"] == 0
    assert payload["proposal_count"] == 0


def test_dream_rejects_disabled_provider(tmp_path: Path) -> None:
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(tmp_path / "hieronymus"), "dream", "--provider", "openai"])

    assert result.exit_code != 0
    assert "dream provider is disabled: openai" in result.output
```

Add imports in `tests/test_cli_service.py`:

```python
from hieronymus.settings import DreamingSettings, load_settings, save_settings
```

- [ ] **Step 2: Add failing MCP provider tests**

Modify `tests/test_mcp_server.py::test_mcp_dream_rejects_unsupported_provider` to expect the registry
error for unknown names, and add:

```python
def test_mcp_dream_uses_configured_provider(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    save_settings(
        config,
        load_settings(config).with_dreaming(DreamingSettings(active_provider="deterministic")),
    )

    from hieronymus import mcp_server

    dreamed = mcp_server.hieronymus_dream()

    assert dreamed["provider"] == "deterministic"
    assert dreamed["status"] == "completed"
```

Add imports:

```python
from hieronymus.settings import DreamingSettings, load_settings, save_settings
```

- [ ] **Step 3: Run focused tests to verify failure**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_dream_json_uses_configured_active_provider tests/test_cli_service.py::test_dream_rejects_disabled_provider tests/test_mcp_server.py::test_mcp_dream_uses_configured_provider -v
```

Expected: FAIL because CLI and MCP still hard-code deterministic behavior and `hiero dream --json` does not exist.

- [ ] **Step 4: Add provider field to dream records**

Modify `src/hieronymus/dreaming.py`:

```python
@dataclass(frozen=True)
class DreamRunRecord:
    id: int
    cycle_id: int
    status: str
    provider: str = ""
    input_count: int = 0
    created_crystal_count: int = 0
    proposal_count: int = 0
    error: str = ""
```

When returning completed records in `DreamService.run_cycle()`, include:

```python
provider=self.provider.name,
```

When constructing failed record data is needed by callers, query `dream_runs` by id or include provider in JSON from the known provider name. Existing tests that instantiate `DreamRunRecord` must pass because the new field has a default.

- [ ] **Step 5: Wire CLI dream command**

Modify imports in `src/hieronymus/cli.py`:

```python
from hieronymus.dream_providers import resolve_provider
from hieronymus.settings import SettingsError
```

Remove the `DeterministicDreamProvider` import. Replace the dream command with:

```python
@main.command("dream")
@click.option("--provider", default=None)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def dream(ctx: click.Context, provider: str | None, json_output: bool) -> None:
    try:
        dream_provider = resolve_provider(ctx.obj["config"], provider)
        run = DreamService(ctx.obj["config"], dream_provider).run_cycle()
    except (KeyError, ValueError, SettingsError) as error:
        _raise_click_error(error)
    payload = {
        "cycle_id": run.cycle_id,
        "status": run.status,
        "provider": run.provider,
        "input_count": run.input_count,
        "created_crystal_count": run.created_crystal_count,
        "proposal_count": run.proposal_count,
        "error": run.error,
    }
    if json_output:
        click.echo(render_json(payload))
        return
    click.echo(json.dumps(payload, ensure_ascii=False))
```

- [ ] **Step 6: Wire admin manual dreaming**

Modify `src/hieronymus/admin.py` imports:

```python
from hieronymus.dream_providers import resolve_provider
```

Remove `DeterministicDreamProvider` from the import. Change `run_manual_dreaming()`:

```python
    def run_manual_dreaming(self) -> DreamRunRecord:
        run = DreamService(self.config, resolve_provider(self.config)).run_cycle()
        self._audit(
            "run_manual_dreaming",
            "dream_run",
            run.id,
            note=f"Manual dream run {run.cycle_id} with provider {run.provider}",
        )
        return run
```

- [ ] **Step 7: Wire MCP dream tool**

Modify `src/hieronymus/mcp_server.py` imports:

```python
from hieronymus.dream_providers import resolve_provider
```

Remove `DeterministicDreamProvider`. Replace `hieronymus_dream()`:

```python
@server.tool()
def hieronymus_dream(provider: str | None = None) -> dict[str, int | str]:
    """Run a dream cycle over completed sessions."""
    config = _load_validated_config()
    run = DreamService(config, resolve_provider(config, provider)).run_cycle()
    return {
        "cycle_id": run.cycle_id,
        "status": run.status,
        "provider": run.provider,
        "input_count": run.input_count,
        "created_crystal_count": run.created_crystal_count,
        "proposal_count": run.proposal_count,
    }
```

- [ ] **Step 8: Run focused dream wiring tests**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_dream_json_uses_configured_active_provider tests/test_cli_service.py::test_dream_rejects_disabled_provider tests/test_mcp_server.py::test_mcp_dream_uses_configured_provider tests/test_admin_store.py::test_admin_runs_manual_dreaming_and_reviews_outputs -v
```

Expected: PASS.

- [ ] **Step 9: Commit provider wiring**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/cli.py src/hieronymus/admin.py src/hieronymus/mcp_server.py tests/test_cli_service.py tests/test_mcp_server.py tests/test_admin_store.py
git commit -m "feat: use configured dream providers"
```

## Task 5: Dreaming Autostart Policy and Service/Doctor Status

**Files:**
- Create: `src/hieronymus/dream_autostart.py`
- Modify: `src/hieronymus/service_http.py`
- Modify: `src/hieronymus/service_daemon.py`
- Modify: `src/hieronymus/doctor.py`
- Create: `tests/test_dream_autostart.py`
- Modify: `tests/test_service_http.py`
- Modify: `tests/test_doctor.py`

- [ ] **Step 1: Add failing autostart policy tests**

Create `tests/test_dream_autostart.py`:

```python
from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.dream_autostart import (
    AutostartState,
    DreamAutostart,
    load_autostart_state,
    save_autostart_state,
)
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.settings import DreamingSettings, load_settings, save_settings
from hieronymus.workspace import WorkspaceStore


def _seed_completed_session(config: HieronymusConfig, memory_count: int) -> None:
    Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(
        TranslationContext(
            series_slug="only-sense-online",
            source_language="ja",
            target_language="en",
            task_type="translation",
        )
    )
    for index in range(memory_count):
        workspace.add_short_term_memory(
            session_id=session.id,
            source_role="user",
            kind="style",
            text=f"Memory {index}",
        )
    workspace.complete_session(session.id)


def test_autostart_counts_pending_short_term_memories_only(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    _seed_completed_session(config, 3)

    status = DreamAutostart(config).status()

    assert status["pending_short_term_memories"] == 3
    assert status["pending_completed_sessions"] == 1


def test_volume_trigger_runs_when_threshold_is_reached(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_settings(
        config,
        load_settings(config).with_dreaming(
            DreamingSettings(
                active_provider="deterministic",
                autostart_enabled=True,
                min_interval_minutes=30,
                new_short_term_memory_threshold=2,
                max_cycles_per_autostart=1,
            )
        ),
    )
    _seed_completed_session(config, 2)

    result = DreamAutostart(config).run_due(now=datetime(2026, 6, 7, tzinfo=UTC))

    assert result["ran"] is True
    assert result["reason"] == "threshold"
    assert result["cycles"] == 1


def test_interval_trigger_requires_pending_memory_and_elapsed_minutes(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_settings(
        config,
        load_settings(config).with_dreaming(
            DreamingSettings(
                active_provider="deterministic",
                autostart_enabled=True,
                min_interval_minutes=30,
                new_short_term_memory_threshold=99,
                max_cycles_per_autostart=1,
            )
        ),
    )
    save_autostart_state(
        config,
        AutostartState(last_started_at=datetime(2026, 6, 7, 12, 0, tzinfo=UTC), last_error=""),
    )
    _seed_completed_session(config, 1)

    result = DreamAutostart(config).run_due(now=datetime(2026, 6, 7, 12, 31, tzinfo=UTC))

    assert result["ran"] is True
    assert result["reason"] == "interval"


def test_autostart_state_round_trips(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    state = AutostartState(
        last_started_at=datetime(2026, 6, 7, 12, 0, tzinfo=UTC),
        last_error="",
    )

    save_autostart_state(config, state)

    assert load_autostart_state(config) == state
```

- [ ] **Step 2: Run autostart tests to verify failure**

Run:

```bash
uv run pytest tests/test_dream_autostart.py -v
```

Expected: FAIL because `hieronymus.dream_autostart` does not exist.

- [ ] **Step 3: Implement autostart state and policy**

Create `src/hieronymus/dream_autostart.py`:

```python
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import DreamService
from hieronymus.settings import load_settings


@dataclass(frozen=True)
class AutostartState:
    last_started_at: datetime | None = None
    last_error: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return {
            "last_started_at": self.last_started_at.isoformat() if self.last_started_at else None,
            "last_error": self.last_error,
        }
```

Use `config.config_root / "dream-autostart.json"` for the state file. Implement
`load_autostart_state()` and `save_autostart_state()` with JSON and atomic replace.

Implement `DreamAutostart.status()`:

```python
    def status(self) -> dict[str, object]:
        settings = load_settings(self.config)
        pending_sessions, pending_memories = self._pending_counts()
        state = load_autostart_state(self.config)
        return {
            "enabled": settings.dreaming.autostart_enabled,
            "active_provider": settings.dreaming.active_provider,
            "min_interval_minutes": settings.dreaming.min_interval_minutes,
            "new_short_term_memory_threshold": settings.dreaming.new_short_term_memory_threshold,
            "max_cycles_per_autostart": settings.dreaming.max_cycles_per_autostart,
            "pending_completed_sessions": pending_sessions,
            "pending_short_term_memories": pending_memories,
            "last_started_at": state.last_started_at.isoformat() if state.last_started_at else None,
            "last_error": state.last_error,
        }
```

`_pending_counts()` must count:

```sql
select count(distinct task_sessions.id) as sessions,
       count(short_term_memories.id) as memories
from task_sessions
join short_term_memories on short_term_memories.session_id = task_sessions.id
where task_sessions.status = 'completed'
  and task_sessions.cycle_id is null
  and short_term_memories.archived_at is null
```

Implement `run_due(now=None)`:

- Return `{"ran": False, "reason": "disabled", "cycles": 0}` when disabled.
- Return `{"ran": False, "reason": "no-pending-memory", "cycles": 0}` when pending memory count is 0.
- Run for reason `"threshold"` when pending memory count is at least the configured threshold.
- Otherwise run for reason `"interval"` when no last start exists or elapsed minutes is at least `min_interval_minutes`.
- Run at most `max_cycles_per_autostart` cycles, stopping early when no pending memories remain.
- Save state with `last_started_at=now` and empty `last_error` on success.
- On exception, save `last_error=str(exc)` and re-raise.

- [ ] **Step 4: Add service and doctor tests**

Modify `tests/test_service_http.py::test_status_endpoint_returns_paths_and_pid` expectations:

```python
    assert [provider["name"] for provider in payload["providers"]] == [
        "deterministic",
        "openai",
        "gemini",
        "anthropic",
    ]
    assert payload["dreaming"]["enabled"] is False
    assert payload["dreaming"]["active_provider"] == "deterministic"
    assert payload["housekeeping"]["pending"] is False
```

Append to `tests/test_doctor.py` or `tests/test_cli_service.py` if doctor tests live there:

```python
def test_doctor_reports_missing_active_provider_env(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_settings(
        config,
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="MISSING_OPENAI_KEY",
                base_url="https://api.openai.com/v1",
            ),
        )
        .with_dreaming(DreamingSettings(active_provider="openai")),
    )

    report = Doctor(config).run()

    assert any(
        finding.code == "provider-env-missing"
        and "MISSING_OPENAI_KEY" in finding.message
        for finding in report["errors"]
    )
```

Add imports as needed:

```python
from hieronymus.settings import DreamingSettings, ProviderSettings, load_settings, save_settings
```

- [ ] **Step 5: Wire service status**

Modify `src/hieronymus/service_http.py` imports:

```python
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_providers import ProviderRegistry
```

Replace:

```python
"providers": [],
"housekeeping": {"last_cycle": None, "pending": False},
```

with:

```python
"providers": ProviderRegistry().status_payload(config),
"dreaming": DreamAutostart(config).status(),
"housekeeping": {
    "last_cycle": None,
    "pending": DreamAutostart(config).status()["pending_short_term_memories"] > 0,
},
```

Store `dreaming_status = DreamAutostart(config).status()` once to avoid computing it twice.

- [ ] **Step 6: Run autostart from daemon startup**

Modify `src/hieronymus/service_daemon.py` after `write_server_state(config, state)`:

```python
    from hieronymus.dream_autostart import DreamAutostart

    try:
        DreamAutostart(config).run_due()
    except Exception:
        pass
```

This makes startup opportunistically run due cycles while preserving daemon availability. The captured error is persisted by `DreamAutostart.run_due()` and surfaced in status/doctor.

- [ ] **Step 7: Wire doctor provider checks**

Modify `src/hieronymus/doctor.py` imports:

```python
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.settings import SettingsError, load_settings
```

Call `_check_settings_and_providers(report)` from `Doctor.run()`. Implement:

```python
    def _check_settings_and_providers(self, report: DoctorReport) -> None:
        try:
            settings = load_settings(self.config)
        except SettingsError as error:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="settings-invalid",
                    message=str(error),
                )
            )
            return
        active_name = settings.dreaming.active_provider
        active = settings.providers[active_name]
        if not active.enabled:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="active-provider-disabled",
                    message=f"Active dream provider is disabled: {active_name}",
                )
            )
            return
        if active_name != "deterministic" and active.api_key_env:
            import os

            if not os.environ.get(active.api_key_env):
                report["errors"].append(
                    DoctorFinding(
                        level="error",
                        code="provider-env-missing",
                        message=f"{active_name} requires environment variable {active.api_key_env}",
                    )
                )
                return
        report["autofixed"].append(
            DoctorFinding(
                level="info",
                code="provider-configured",
                message=f"Active dream provider is configured: {active_name}",
            )
        )
```

- [ ] **Step 8: Run autostart/service/doctor tests**

Run:

```bash
uv run pytest tests/test_dream_autostart.py tests/test_service_http.py tests/test_doctor.py tests/test_cli_service.py::test_doctor_json_has_expected_sections -v
```

Expected: PASS.

- [ ] **Step 9: Commit autostart and status**

```bash
git add src/hieronymus/dream_autostart.py src/hieronymus/service_http.py src/hieronymus/service_daemon.py src/hieronymus/doctor.py tests/test_dream_autostart.py tests/test_service_http.py tests/test_doctor.py tests/test_cli_service.py
git commit -m "feat: add dream autostart status"
```

## Task 6: Config CLI and Textual Config TUI

**Files:**
- Create: `src/hieronymus/tui/config_app.py`
- Create: `src/hieronymus/tui/config_screens.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/tui/styles.tcss`
- Create: `tests/test_config_tui.py`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Replace config placeholder tests**

Modify `tests/test_cli_service.py::test_config_json_returns_paths_and_tui_placeholder` into:

```python
def test_config_json_returns_real_settings_and_paths(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    runner = CliRunner()

    result = runner.invoke(
        main,
        ["--data-root", str(data_root), "config", "--json"],
    )

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload["config_root"] == str(data_root)
    assert payload["database_path"] == str(data_root / "hieronymus.sqlite")
    assert payload["settings_path"] == str(data_root / "settings.toml")
    assert payload["tui"] == "available"
    assert payload["settings"]["dreaming"]["active_provider"] == "deterministic"
    assert payload["providers"][0]["name"] == "deterministic"
```

Add:

```python
def test_config_launch_invokes_textual_app(monkeypatch, tmp_path: Path) -> None:
    launched: dict[str, object] = {}

    class FakeApp:
        def __init__(self, config):
            launched["settings_path"] = str(config.settings_path)

        def run(self):
            launched["ran"] = True

    monkeypatch.setattr("hieronymus.cli.HieronymusConfigApp", FakeApp)
    runner = CliRunner()

    result = runner.invoke(main, ["--data-root", str(tmp_path / "hieronymus"), "config"])

    assert result.exit_code == 0
    assert launched == {
        "settings_path": str(tmp_path / "hieronymus" / "settings.toml"),
        "ran": True,
    }
```

- [ ] **Step 2: Add failing Textual config TUI tests**

Create `tests/test_config_tui.py`:

```python
from __future__ import annotations

from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui.config_app import HieronymusConfigApp


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


async def test_config_tui_mounts_provider_rows(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        table = app.screen.query_one("#config-table")
        labels = [str(row[0]) for row in table.rows.values()]
        assert "deterministic" in labels
        assert "openai" in labels
        assert "gemini" in labels
        assert "anthropic" in labels


async def test_config_tui_can_save_active_provider(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        await pilot.press("s")

    assert load_settings(config).dreaming.active_provider == "openai"
```

- [ ] **Step 3: Run config CLI/TUI tests to verify failure**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_config_json_returns_real_settings_and_paths tests/test_cli_service.py::test_config_launch_invokes_textual_app tests/test_config_tui.py -v
```

Expected: FAIL because the config TUI app does not exist and CLI still prints the placeholder.

- [ ] **Step 4: Implement config app shell**

Create `src/hieronymus/tui/config_app.py`:

```python
from __future__ import annotations

from textual.app import App

from hieronymus.config import HieronymusConfig
from hieronymus.tui.config_screens import ConfigScreen


class HieronymusConfigApp(App[None]):
    TITLE = "Hieronymus Config"
    CSS_PATH = "styles.tcss"
    COMMAND_PALETTE_BINDING = "ctrl+shift+p"

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config

    def on_mount(self) -> None:
        self.push_screen(ConfigScreen(self.config))
```

- [ ] **Step 5: Implement config screen**

Create `src/hieronymus/tui/config_screens.py`:

```python
from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import Screen
from textual.widgets import DataTable, Footer, Static

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.settings import DreamingSettings, load_settings, save_settings


class ConfigScreen(Screen[None]):
    BINDINGS = [
        Binding("1", "set_active('deterministic')", "Deterministic"),
        Binding("2", "set_active('openai')", "OpenAI"),
        Binding("3", "set_active('gemini')", "Gemini"),
        Binding("4", "set_active('anthropic')", "Anthropic"),
        Binding("s", "save", "Save"),
        Binding("r", "reload", "Reload"),
        Binding("c", "check_selected", "Check"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, config: HieronymusConfig) -> None:
        super().__init__()
        self.config = config
        self.settings = load_settings(config)

    def compose(self) -> ComposeResult:
        yield Static("Providers", id="config-title")
        with Horizontal(id="workspace"):
            yield DataTable(id="config-table")
            yield Static("", id="config-detail")
        yield Footer()
```

In `on_mount()`, add columns `provider`, `active`, `enabled`, `model`, `key env`, `configured`, `error`, then call `_refresh()`. `_refresh()` must fill rows from `ProviderRegistry().status_payload(self.config)` and set detail text containing `settings_path`, `database_path`, autostart settings, and selected provider help.

Implement actions:

```python
    def action_set_active(self, name: str) -> None:
        provider = self.settings.providers[name]
        if not provider.enabled:
            provider = replace(provider, enabled=True)
            self.settings = self.settings.with_provider(name, provider)
        self.settings = self.settings.with_dreaming(
            DreamingSettings(
                active_provider=name,
                autostart_enabled=self.settings.dreaming.autostart_enabled,
                min_interval_minutes=self.settings.dreaming.min_interval_minutes,
                new_short_term_memory_threshold=self.settings.dreaming.new_short_term_memory_threshold,
                max_cycles_per_autostart=self.settings.dreaming.max_cycles_per_autostart,
            )
        )
        self._refresh()

    def action_save(self) -> None:
        save_settings(self.config, self.settings)
        self._refresh()

    def action_reload(self) -> None:
        self.settings = load_settings(self.config)
        self._refresh()
```

Import `replace` from `dataclasses`. `action_check_selected()` should call `ProviderRegistry().check()` for the selected provider and show the result in `#config-detail`; it does not save temporary secrets.

- [ ] **Step 6: Wire CLI config command**

Modify `src/hieronymus/cli.py` config imports:

```python
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_providers import ProviderRegistry, resolve_provider
from hieronymus.settings import SettingsError, load_settings
from hieronymus.tui.config_app import HieronymusConfigApp
```

Replace `config_command()` with:

```python
@main.command("config")
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def config_command(ctx: click.Context, json_output: bool) -> None:
    config = ctx.obj["config"]
    if json_output:
        try:
            settings = load_settings(config)
        except SettingsError as error:
            raise click.ClickException(str(error)) from error
        payload = {
            "config_root": str(config.config_root),
            "database_path": str(config.database_path),
            "settings_path": str(config.settings_path),
            "tui": "available",
            "settings": settings.to_json_dict(),
            "providers": ProviderRegistry().status_payload(config),
            "dreaming": DreamAutostart(config).status(),
        }
        click.echo(render_json(payload))
        return

    HieronymusConfigApp(config).run()
```

Update help text from `Show config paths` to `Open the configuration TUI`.

- [ ] **Step 7: Run config CLI/TUI tests**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_config_json_returns_real_settings_and_paths tests/test_cli_service.py::test_config_launch_invokes_textual_app tests/test_config_tui.py -v
```

Expected: PASS.

- [ ] **Step 8: Commit config TUI**

```bash
git add src/hieronymus/cli.py src/hieronymus/tui/config_app.py src/hieronymus/tui/config_screens.py src/hieronymus/tui/styles.tcss tests/test_cli_service.py tests/test_config_tui.py
git commit -m "feat: add configuration tui"
```

## Task 7: Documentation and Placeholder Cleanup

**Files:**
- Modify: `README.md`
- Modify: `docs/usage.md`
- Modify: `docs/memory-dreaming.md`
- Modify: `docs/service-toolkit.md`
- Modify: `tests/test_cli_service.py`

- [ ] **Step 1: Add docs placeholder and provider docs tests**

Append to `tests/test_cli_service.py`:

```python
def test_docs_describe_real_config_tui_and_llm_providers() -> None:
    combined = "\n".join(
        Path(path).read_text(encoding="utf-8")
        for path in [
            "README.md",
            "docs/usage.md",
            "docs/memory-dreaming.md",
            "docs/service-toolkit.md",
        ]
    )

    forbidden = [
        "not-available-in-this-pass",
        "config TUI is separate work",
        "only provider implemented now is the deterministic provider",
        "External LLM providers are a later extension",
        "external LLM providers are deferred",
    ]
    for phrase in forbidden:
        assert phrase not in combined

    assert "hiero config" in combined
    assert "OPENAI_API_KEY" in combined
    assert "GEMINI_API_KEY" in combined
    assert "ANTHROPIC_API_KEY" in combined
    assert "API key values are not stored" in combined
    assert "new_short_term_memory_threshold" in combined
```

- [ ] **Step 2: Run docs test to verify failure**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_docs_describe_real_config_tui_and_llm_providers -v
```

Expected: FAIL because docs still contain deterministic-only and config placeholder text.

- [ ] **Step 3: Update README**

Add a configuration section near the install/usage commands:

````markdown
## Configuration

Open the configuration TUI:

```bash
hiero config
```

Machine-readable status:

```bash
hiero config --json
```

Hieronymus stores non-secret settings in `~/.config/hieronymus/settings.toml`. API key values are not
stored. Provider entries store environment variable names, then runtime reads the secret from the
environment.

Supported dream providers:

- `deterministic`: offline fallback and test provider.
- `openai`: OpenAI and OpenAI-compatible endpoints. Default key env: `OPENAI_API_KEY`.
- `gemini`: Gemini API. Default key env: `GEMINI_API_KEY`.
- `anthropic`: Anthropic Messages API. Default key env: `ANTHROPIC_API_KEY`.

Dreaming automation is controlled by:

- `autostart_enabled`
- `min_interval_minutes`
- `new_short_term_memory_threshold`
- `max_cycles_per_autostart`
````

- [ ] **Step 4: Update usage and memory dreaming docs**

In `docs/usage.md`, replace old `hiero config` path-only text with the README configuration section plus examples:

```bash
export OPENAI_API_KEY=...
hiero config
hiero dream --provider openai --json
```

In `docs/memory-dreaming.md`, replace deterministic-only wording with:

```markdown
Dream cycles use the configured active provider by default. `deterministic` is the offline fallback.
OpenAI, Gemini, and Anthropic providers produce structured JSON that is validated before outputs are
applied. Invalid provider output records a failed dream run and leaves completed sessions pending.
```

Document automatic triggers:

```markdown
Automatic dreaming is cycle-based. It can run once every `min_interval_minutes` when completed
sessions have pending short-term memories, or as soon as `new_short_term_memory_threshold` pending
short-term memories exists. The threshold counts short-term memories, not long-term crystals or
remembered strict terminology.
```

- [ ] **Step 5: Update service toolkit docs**

In `docs/service-toolkit.md`, replace `hiero config` and provider status notes with:

```markdown
- `hiero config` opens the configuration TUI for providers, dreaming automation, service status, paths,
  and diagnostics.
- `hiero config --json` reports settings, provider status, and dreaming automation state.
- `hiero doctor --json` checks settings parseability, active provider enablement, provider key env
  configuration, store health, and service health.
```

- [ ] **Step 6: Run docs test**

Run:

```bash
uv run pytest tests/test_cli_service.py::test_docs_describe_real_config_tui_and_llm_providers -v
```

Expected: PASS.

- [ ] **Step 7: Commit docs cleanup**

```bash
git add README.md docs/usage.md docs/memory-dreaming.md docs/service-toolkit.md tests/test_cli_service.py
git commit -m "docs: document config tui and llm providers"
```

## Task 8: Final Verification and Integration Review

**Files:**
- Modify: verification state only

- [ ] **Step 1: Run focused integration tests**

Run:

```bash
uv run pytest tests/test_settings.py tests/test_dream_providers.py tests/test_dream_autostart.py tests/test_config_tui.py tests/test_cli_service.py tests/test_mcp_server.py tests/test_service_http.py tests/test_doctor.py tests/test_admin_store.py -v
```

Expected: PASS.

- [ ] **Step 2: Run full project verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected:

- all tests pass;
- Ruff reports no lint failures;
- format check reports all files formatted.

- [ ] **Step 3: Search for forbidden placeholder and deterministic-only claims**

Run:

```bash
rg -n "not-available-in-this-pass|unavailable in this pass|config TUI is separate work|only provider implemented now is the deterministic provider|External LLM providers" src tests docs README.md
```

Expected: no output.

- [ ] **Step 4: Check final git status**

Run:

```bash
git status --short
```

Expected: only unrelated pre-existing untracked files are present, or no output.

- [ ] **Step 5: Commit verification fixes if needed**

If Step 1, 2, or 3 reveals code/doc issues, fix them in the relevant module and commit:

```bash
git add <fixed-files>
git commit -m "fix: complete config provider integration"
```

If no fixes are needed, no commit is required.

## Plan Self-Review

- Spec coverage: persisted settings, secret handling, provider registry, OpenAI/Gemini/Anthropic dream providers, provider checks, configured CLI/admin/MCP dreaming, cycle-based autostart, service status, doctor, config TUI, docs cleanup, and final verification are covered.
- Placeholder scan: the plan intentionally includes a final forbidden-text search and has no product-facing incomplete-provider escape path.
- Type consistency: `HieronymusSettings`, `DreamingSettings`, `ProviderSettings`, `ProviderRegistry`, `ProviderCheckResult`, `resolve_provider`, `DreamAutostart`, and `HieronymusConfigApp` are introduced before use in later tasks.

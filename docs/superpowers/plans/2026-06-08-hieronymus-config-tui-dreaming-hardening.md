# Hieronymus Config TUI and Dreaming Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `hiero config` a complete saved/draft provider and dreaming automation editor, and serialize all manual and automatic dream cycles per data root.

**Architecture:** Keep persisted settings as the source of truth, but add a small TUI draft layer that can validate and check edited settings before save. Add a dream-cycle lock with both in-process and `fcntl` file locking, then route CLI, MCP, admin TUI, and daemon autostart dream runs through `DreamService.run_cycle()` so every entry point shares one guard. Secret redaction is centralized and applied before settings/status JSON, TUI detail text, doctor output, and dream run error records can expose values.

**Tech Stack:** Python 3.12, Textual 0.86, Click, SQLite, stdlib `fcntl`/`threading`, TOML via `tomllib`/`tomli-w`, pytest, ruff.

---

## File Structure

- Create `src/hieronymus/secrets.py`: find configured API key environment names, report key presence, and redact known environment values from strings.
- Modify `src/hieronymus/settings.py`: expose public `validate_settings(settings)` wrapper over existing validation for in-memory TUI save validation.
- Modify `src/hieronymus/dream_providers.py`: allow provider status and checks to use an in-memory `HieronymusSettings`, include `api_key_present`, and avoid reading or returning raw key values.
- Create `src/hieronymus/tui/config_state.py`: pure helpers for editing provider/dreaming fields, parsing numeric values, tracking unsaved changes, and returning save validation errors.
- Modify `src/hieronymus/tui/config_screens.py`: replace read-only provider view with an editable draft UI, saved/draft/check/error detail text, save/reload behavior, and in-memory provider checks.
- Modify `src/hieronymus/tui/styles.tcss`: add form layout styles for config inputs and status messages.
- Create `tests/test_config_tui.py` coverage for provider field edits, automation edits, save, reload, validation failures, and no secret values in detail text.
- Create `src/hieronymus/dream_locks.py`: per-data-root dream lock, active lock state JSON, conservative stale-state cleanup, and `DreamCycleAlreadyRunning`.
- Modify `src/hieronymus/dreaming.py`: acquire/release the dream lock around cycle work, support explicit wait, record skipped autostart runs, sanitize stored errors, and expose skipped/active statuses.
- Modify `src/hieronymus/dream_autostart.py`: add skipped state fields, detect active cycles, skip scheduled runs cleanly, and expose lock state through status.
- Modify `src/hieronymus/cli.py`: add `hiero dream --wait`, return clear already-running errors, and keep `hiero config --json` secret-safe.
- Modify `src/hieronymus/mcp_server.py`: add `wait: bool = False` to `hieronymus_dream`.
- Modify `src/hieronymus/admin.py`: keep manual dreaming non-waiting and return/raise the shared already-running error.
- Modify `src/hieronymus/service_http.py`: include active/skipped dream-cycle fields in status payload.
- Modify `src/hieronymus/doctor.py`: continue reporting env var names only, and use the same secret redaction helper for defensive output.
- Create `tests/test_dream_locks.py`: lock acquisition/release, second acquisition failure, exception release, active state, and stale-state behavior.
- Modify `tests/test_dreaming.py`: lock release after provider exceptions and sanitized failed-run errors.
- Modify `tests/test_dream_autostart.py`: autostart skips when a cycle is active and records status.
- Modify `tests/test_cli.py`, `tests/test_mcp_server.py`, `tests/test_admin_store.py`, `tests/test_service_http.py`, `tests/test_doctor.py`: shared guard and secret-safety integration coverage.
- Modify `docs/memory-dreaming.md` and `docs/usage.md`: document editable config fields, `--wait`, lock behavior, and conservative stale-state handling.

## Ground Rules

- Never store or print raw API key values. Only show `api_key_env` and a boolean/key-present marker.
- Provider checks must accept an edited `HieronymusSettings` object so the config TUI can check drafts without saving.
- Manual dream runs default to fail-fast when a cycle is active. Only `--wait` in CLI and `wait=True` in MCP block for the active run.
- Autostart never waits. If due while a cycle is active, it records a skipped run/state and returns `{"ran": False, "reason": "cycle-active", "cycles": 0}`.
- Stale state files are informational; the OS lock is authoritative. If a state file remains but no live PID/OS lock owns it, cleanup is allowed. Never break a live OS lock.

## Task 1: Secret-Safe Provider Status and In-Memory Checks

**Files:**
- Create: `src/hieronymus/secrets.py`
- Modify: `src/hieronymus/settings.py`
- Modify: `src/hieronymus/dream_providers.py`
- Modify: `tests/test_dream_providers.py`
- Modify: `tests/test_settings.py`

- [ ] **Step 1: Add failing provider/status secret tests**

Add these tests to `tests/test_dream_providers.py`:

```python
from dataclasses import replace

from hieronymus.dream_providers import ProviderRegistry
from hieronymus.settings import ProviderSettings, load_settings, save_settings


def test_provider_status_reports_key_presence_without_key_value(config, monkeypatch):
    monkeypatch.setenv("HIERONYMUS_OPENAI_TEST_KEY", "sk-live-secret-value")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_TEST_KEY",
            base_url="https://api.example.test/v1",
        ),
    )
    save_settings(config, settings)

    payload = ProviderRegistry().status_payload(config)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["api_key_env"] == "HIERONYMUS_OPENAI_TEST_KEY"
    assert openai["api_key_present"] is True
    assert "sk-live-secret-value" not in repr(payload)


def test_provider_status_can_use_unsaved_in_memory_settings(config, monkeypatch):
    monkeypatch.setenv("DRAFT_OPENAI_KEY", "draft-secret")
    saved = load_settings(config)
    draft = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            enabled=True,
            api_key_env="DRAFT_OPENAI_KEY",
            model="draft-model",
            base_url="https://draft.example.test/v1",
        ),
    )

    payload = ProviderRegistry().status_payload(config, settings=draft)
    openai = next(row for row in payload if row["name"] == "openai")

    assert openai["model"] == "draft-model"
    assert openai["api_key_present"] is True
    assert load_settings(config).providers["openai"].model == "gpt-4.1-mini"


def test_provider_check_uses_unsaved_in_memory_settings(config, monkeypatch):
    class Transport:
        def __init__(self):
            self.calls = []

        def post_json(self, url, *, headers, payload, timeout):
            self.calls.append((url, headers, payload, timeout))
            return type("Response", (), {"status": 200, "body": "{}"})()

    monkeypatch.setenv("DRAFT_OPENAI_KEY", "draft-secret")
    saved = load_settings(config)
    draft = saved.with_provider(
        "openai",
        replace(
            saved.providers["openai"],
            enabled=True,
            api_key_env="DRAFT_OPENAI_KEY",
            model="draft-model",
            base_url="https://draft.example.test/v1",
            timeout_seconds=7.5,
        ),
    )
    transport = Transport()

    result = ProviderRegistry(transport=transport).check(config, "openai", settings=draft)

    assert result.ok is True
    assert result.model == "draft-model"
    assert transport.calls[0][0] == "https://draft.example.test/v1/chat/completions"
    assert transport.calls[0][3] == 7.5
    assert "draft-secret" not in repr(result.to_json_dict())
```

Add this test to `tests/test_settings.py`:

```python
from hieronymus.settings import validate_settings


def test_validate_settings_exposes_existing_validation(config):
    settings = load_settings(config).with_dreaming(
        DreamingSettings(
            active_provider="deterministic",
            autostart_enabled=False,
            min_interval_minutes=0,
            new_short_term_memory_threshold=25,
            max_cycles_per_autostart=1,
        )
    )

    with pytest.raises(SettingsError, match="min_interval_minutes must be at least 1"):
        validate_settings(settings)
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_dream_providers.py tests/test_settings.py -v
```

Expected: FAIL because `validate_settings`, `api_key_present`, and the `settings=` parameters do not exist.

- [ ] **Step 3: Add secret helpers**

Create `src/hieronymus/secrets.py`:

```python
from __future__ import annotations

import os

from hieronymus.settings import HieronymusSettings


def configured_key_env_names(settings: HieronymusSettings) -> set[str]:
    return {
        provider.api_key_env
        for provider in settings.providers.values()
        if provider.api_key_env.strip()
    }


def env_value_exists(env_name: str) -> bool:
    return bool(env_name and os.environ.get(env_name))


def redact_configured_secret_values(text: str, settings: HieronymusSettings) -> str:
    redacted = text
    for env_name in configured_key_env_names(settings):
        value = os.environ.get(env_name)
        if value and len(value) >= 4:
            redacted = redacted.replace(value, "[redacted]")
    return redacted
```

- [ ] **Step 4: Expose settings validation**

Modify `src/hieronymus/settings.py` near `save_settings`:

```python
def validate_settings(settings: HieronymusSettings) -> HieronymusSettings:
    return _validate_settings(settings)
```

- [ ] **Step 5: Allow provider status/check to use drafts**

Modify imports in `src/hieronymus/dream_providers.py`:

```python
from hieronymus.secrets import env_value_exists
from hieronymus.settings import HieronymusSettings, ProviderSettings, load_settings
```

Change `status_payload` and `check` signatures and bodies:

```python
    def status_payload(
        self,
        config: HieronymusConfig,
        *,
        settings: HieronymusSettings | None = None,
    ) -> list[dict[str, object]]:
        active_settings = settings or load_settings(config)
        statuses = []
        for metadata in self._providers:
            provider = active_settings.providers.get(metadata.name, ProviderSettings())
            configured, error = _configured_status(metadata.name, provider)
            statuses.append(
                {
                    "name": metadata.name,
                    "display_name": metadata.display_name,
                    "enabled": provider.enabled,
                    "configured": configured,
                    "model": provider.model,
                    "api_key_env": provider.api_key_env,
                    "api_key_present": env_value_exists(provider.api_key_env),
                    "base_url": provider.base_url,
                    "timeout_seconds": provider.timeout_seconds,
                    "error": error,
                }
            )
        return statuses

    def check(
        self,
        config: HieronymusConfig,
        name: str,
        temporary_api_key: str | None = None,
        *,
        settings: HieronymusSettings | None = None,
    ) -> ProviderCheckResult:
        self.metadata(name)
        if name == "deterministic":
            return ProviderCheckResult(name="deterministic", ok=True, model="")

        active_settings = settings or load_settings(config)
        provider = active_settings.providers.get(name, ProviderSettings())
        key = temporary_api_key or os.environ.get(provider.api_key_env)
        if not key:
            return ProviderCheckResult(
                name=name,
                ok=False,
                model=provider.model,
                error=f"missing environment variable: {provider.api_key_env}",
            )
```

Update `_configured_status` to use the helper:

```python
    if provider.enabled and not env_value_exists(provider.api_key_env):
        return False, f"missing environment variable: {provider.api_key_env}"
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
uv run pytest tests/test_dream_providers.py tests/test_settings.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/secrets.py src/hieronymus/settings.py src/hieronymus/dream_providers.py tests/test_dream_providers.py tests/test_settings.py
git commit -m "fix: make provider status secret-safe"
```

## Task 2: Config TUI Draft Editing and Validation

**Files:**
- Create: `src/hieronymus/tui/config_state.py`
- Modify: `src/hieronymus/tui/config_screens.py`
- Modify: `src/hieronymus/tui/styles.tcss`
- Modify: `tests/test_config_tui.py`

- [ ] **Step 1: Add failing config TUI edit tests**

Replace `tests/test_config_tui.py` with:

```python
from pathlib import Path

import pytest
from textual.widgets import DataTable, Input, Static

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui.config_app import HieronymusConfigApp


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def _detail_text(app: HieronymusConfigApp) -> str:
    return str(app.screen.query_one("#config-detail", Static).renderable)


@pytest.mark.anyio
async def test_config_tui_mounts_provider_rows(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test():
        table = app.screen.query_one("#config-table", DataTable)
        labels = [str(table.get_row(row.key)[0]) for row in table.ordered_rows]

    assert {"deterministic", "openai", "gemini", "anthropic"} <= set(labels)


@pytest.mark.anyio
async def test_config_tui_edits_provider_fields_and_saves(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        app.screen.query_one("#provider-model", Input).value = "gpt-4.1"
        app.screen.query_one("#provider-api-key-env", Input).value = "HIERONYMUS_OPENAI_KEY"
        app.screen.query_one("#provider-base-url", Input).value = "https://llm.example.test/v1"
        app.screen.query_one("#provider-timeout-seconds", Input).value = "11.5"
        await pilot.press("s")

    provider = load_settings(config).providers["openai"]
    assert provider.enabled is True
    assert provider.model == "gpt-4.1"
    assert provider.api_key_env == "HIERONYMUS_OPENAI_KEY"
    assert provider.base_url == "https://llm.example.test/v1"
    assert provider.timeout_seconds == 11.5


@pytest.mark.anyio
async def test_config_tui_edits_dreaming_fields_and_saves(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("3")
        app.screen.query_one("#dreaming-active-provider", Input).value = "gemini"
        app.screen.query_one("#dreaming-autostart-enabled", Input).value = "yes"
        app.screen.query_one("#dreaming-min-interval-minutes", Input).value = "9"
        app.screen.query_one("#dreaming-new-short-term-memory-threshold", Input).value = "3"
        app.screen.query_one("#dreaming-max-cycles-per-autostart", Input).value = "2"
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        await pilot.press("s")

    dreaming = load_settings(config).dreaming
    assert dreaming.active_provider == "gemini"
    assert dreaming.autostart_enabled is True
    assert dreaming.min_interval_minutes == 9
    assert dreaming.new_short_term_memory_threshold == 3
    assert dreaming.max_cycles_per_autostart == 2
    assert load_settings(config).providers["gemini"].enabled is True


@pytest.mark.anyio
async def test_config_tui_reload_discards_unsaved_edits(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-model", Input).value = "unsaved-model"
        assert "unsaved" in _detail_text(app)
        await pilot.press("r")
        assert app.screen.query_one("#provider-model", Input).value == "gpt-4.1-mini"
        assert "unsaved" not in _detail_text(app)


@pytest.mark.anyio
async def test_config_tui_validation_failure_does_not_save(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        app.screen.query_one("#dreaming-min-interval-minutes", Input).value = "0"
        await pilot.press("s")
        detail = _detail_text(app)

    assert "min_interval_minutes must be at least 1" in detail
    assert not config.settings_path.exists()


@pytest.mark.anyio
async def test_config_tui_detail_never_shows_raw_api_key_value(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-api-key-env", Input).value = "HIERONYMUS_OPENAI_KEY"
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        detail = _detail_text(app)

    assert "HIERONYMUS_OPENAI_KEY" in detail
    assert "api_key_present: yes" in detail
    assert "raw-secret-value" not in detail
```

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
uv run pytest tests/test_config_tui.py -v
```

Expected: FAIL because the input widgets and draft validation do not exist.

- [ ] **Step 3: Create config draft helpers**

Create `src/hieronymus/tui/config_state.py`:

```python
from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any

from hieronymus.settings import (
    DreamingSettings,
    HieronymusSettings,
    ProviderSettings,
    SettingsError,
    validate_settings,
)


@dataclass(frozen=True)
class ConfigDraft:
    saved: HieronymusSettings
    edited: HieronymusSettings
    errors: tuple[str, ...] = ()
    check_result: str = ""

    @property
    def has_unsaved_changes(self) -> bool:
        return self.saved != self.edited

    def with_edited(self, settings: HieronymusSettings) -> ConfigDraft:
        return ConfigDraft(
            saved=self.saved,
            edited=settings,
            errors=(),
            check_result=self.check_result,
        )

    def with_errors(self, errors: list[str]) -> ConfigDraft:
        return ConfigDraft(
            saved=self.saved,
            edited=self.edited,
            errors=tuple(errors),
            check_result=self.check_result,
        )

    def with_check_result(self, result: str) -> ConfigDraft:
        return ConfigDraft(
            saved=self.saved,
            edited=self.edited,
            errors=self.errors,
            check_result=result,
        )


def parse_bool(field_name: str, raw: str) -> bool:
    value = raw.strip().lower()
    if value in {"yes", "true", "1", "on"}:
        return True
    if value in {"no", "false", "0", "off"}:
        return False
    raise SettingsError(f"{field_name} must be yes or no")


def parse_int(field_name: str, raw: str) -> int:
    try:
        return int(raw.strip())
    except ValueError as error:
        raise SettingsError(f"{field_name} must be an integer") from error


def parse_float(field_name: str, raw: str) -> float:
    try:
        return float(raw.strip())
    except ValueError as error:
        raise SettingsError(f"{field_name} must be a number") from error


def apply_provider_form(
    settings: HieronymusSettings,
    name: str,
    values: dict[str, str],
) -> HieronymusSettings:
    provider = settings.providers.get(name, ProviderSettings())
    base_url = values["base_url"].strip()
    updated = replace(
        provider,
        enabled=parse_bool(f"providers.{name}.enabled", values["enabled"]),
        model=values["model"].strip(),
        api_key_env=values["api_key_env"].strip(),
        base_url=base_url or None,
        timeout_seconds=parse_float(
            f"providers.{name}.timeout_seconds",
            values["timeout_seconds"],
        ),
    )
    return settings.with_provider(name, updated)


def apply_dreaming_form(
    settings: HieronymusSettings,
    values: dict[str, str],
) -> HieronymusSettings:
    dreaming = DreamingSettings(
        active_provider=values["active_provider"].strip(),
        autostart_enabled=parse_bool("autostart_enabled", values["autostart_enabled"]),
        min_interval_minutes=parse_int(
            "min_interval_minutes",
            values["min_interval_minutes"],
        ),
        new_short_term_memory_threshold=parse_int(
            "new_short_term_memory_threshold",
            values["new_short_term_memory_threshold"],
        ),
        max_cycles_per_autostart=parse_int(
            "max_cycles_per_autostart",
            values["max_cycles_per_autostart"],
        ),
    )
    return settings.with_dreaming(dreaming)


def validate_draft(settings: HieronymusSettings) -> list[str]:
    try:
        validate_settings(settings)
    except SettingsError as error:
        return [str(error)]
    return []


def yes_no(value: bool) -> str:
    return "yes" if value else "no"


def field_value(value: Any) -> str:
    if isinstance(value, bool):
        return yes_no(value)
    if value is None:
        return ""
    return str(value)
```

- [ ] **Step 4: Replace config screen with editable draft form**

Modify `src/hieronymus/tui/config_screens.py` so imports include `Input`, helper functions, and `SettingsError`:

```python
from textual.widgets import DataTable, Footer, Input, Static

from hieronymus.secrets import env_value_exists
from hieronymus.settings import (
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)
from hieronymus.tui.config_state import (
    ConfigDraft,
    apply_dreaming_form,
    apply_provider_form,
    field_value,
    validate_draft,
    yes_no,
)
```

Change `__init__` to keep saved and edited state:

```python
        settings = load_settings(config)
        self.draft = ConfigDraft(saved=settings, edited=settings)
        self._syncing_form = False
```

Change `compose()` to add the editable form beside the table:

```python
    def compose(self) -> ComposeResult:
        yield Static("Providers", id="config-title")
        with Horizontal(id="workspace"):
            yield DataTable(id="config-table")
            with Vertical(id="config-form"):
                yield Static("", id="config-detail")
                yield Input(id="provider-enabled")
                yield Input(id="provider-model")
                yield Input(id="provider-api-key-env")
                yield Input(id="provider-base-url")
                yield Input(id="provider-timeout-seconds")
                yield Input(id="dreaming-active-provider")
                yield Input(id="dreaming-autostart-enabled")
                yield Input(id="dreaming-min-interval-minutes")
                yield Input(id="dreaming-new-short-term-memory-threshold")
                yield Input(id="dreaming-max-cycles-per-autostart")
        yield Footer()
```

Add `Vertical` to the imports:

```python
from textual.containers import Horizontal, Vertical
```

Add a form sync method:

```python
    def _sync_form_from_draft(self, selected_provider: str) -> None:
        provider = self.draft.edited.providers.get(selected_provider, ProviderSettings())
        dreaming = self.draft.edited.dreaming
        values = {
            "#provider-enabled": field_value(provider.enabled),
            "#provider-model": field_value(provider.model),
            "#provider-api-key-env": field_value(provider.api_key_env),
            "#provider-base-url": field_value(provider.base_url),
            "#provider-timeout-seconds": field_value(provider.timeout_seconds),
            "#dreaming-active-provider": field_value(dreaming.active_provider),
            "#dreaming-autostart-enabled": field_value(dreaming.autostart_enabled),
            "#dreaming-min-interval-minutes": field_value(dreaming.min_interval_minutes),
            "#dreaming-new-short-term-memory-threshold": field_value(
                dreaming.new_short_term_memory_threshold
            ),
            "#dreaming-max-cycles-per-autostart": field_value(
                dreaming.max_cycles_per_autostart
            ),
        }
        self._syncing_form = True
        try:
            for selector, value in values.items():
                self.query_one(selector, Input).value = value
        finally:
            self._syncing_form = False
```

Add form collection and draft application:

```python
    def _provider_form_values(self) -> dict[str, str]:
        return {
            "enabled": self.query_one("#provider-enabled", Input).value,
            "model": self.query_one("#provider-model", Input).value,
            "api_key_env": self.query_one("#provider-api-key-env", Input).value,
            "base_url": self.query_one("#provider-base-url", Input).value,
            "timeout_seconds": self.query_one("#provider-timeout-seconds", Input).value,
        }

    def _dreaming_form_values(self) -> dict[str, str]:
        return {
            "active_provider": self.query_one("#dreaming-active-provider", Input).value,
            "autostart_enabled": self.query_one("#dreaming-autostart-enabled", Input).value,
            "min_interval_minutes": self.query_one(
                "#dreaming-min-interval-minutes",
                Input,
            ).value,
            "new_short_term_memory_threshold": self.query_one(
                "#dreaming-new-short-term-memory-threshold",
                Input,
            ).value,
            "max_cycles_per_autostart": self.query_one(
                "#dreaming-max-cycles-per-autostart",
                Input,
            ).value,
        }

    def _apply_form_to_draft(self) -> bool:
        selected_provider = self._selected_provider()
        try:
            edited = apply_provider_form(
                self.draft.edited,
                selected_provider,
                self._provider_form_values(),
            )
            edited = apply_dreaming_form(edited, self._dreaming_form_values())
        except SettingsError as error:
            self.draft = self.draft.with_errors([str(error)])
            self._update_detail(selected_provider)
            return False
        self.draft = self.draft.with_edited(edited)
        return True
```

Add an input-change handler so tests and users see draft/validation state immediately:

```python
    def on_input_changed(self, message: Input.Changed) -> None:
        if self._syncing_form:
            return
        if message.input.id is None or not message.input.id.startswith(
            ("provider-", "dreaming-")
        ):
            return
        selected_provider = self._selected_provider()
        if self._apply_form_to_draft():
            self._refresh(selected_provider=selected_provider)
```

Update actions to operate on drafts:

```python
    def action_set_active(self, name: str) -> None:
        if not self._apply_form_to_draft():
            return
        provider = self.draft.edited.providers.get(name, ProviderSettings())
        if not provider.enabled:
            self.draft = self.draft.with_edited(
                self.draft.edited.with_provider(name, replace(provider, enabled=True))
            )
        self.draft = self.draft.with_edited(
            self.draft.edited.with_dreaming(
                replace(self.draft.edited.dreaming, active_provider=name)
            )
        )
        self._refresh(selected_provider=name)

    def action_save(self) -> None:
        if not self._apply_form_to_draft():
            return
        errors = validate_draft(self.draft.edited)
        if errors:
            self.draft = self.draft.with_errors(errors)
            self._update_detail(self._selected_provider())
            return
        save_settings(self.config, self.draft.edited)
        saved = load_settings(self.config)
        self.draft = ConfigDraft(saved=saved, edited=saved)
        self._refresh()

    def action_reload(self) -> None:
        settings = load_settings(self.config)
        self.draft = ConfigDraft(saved=settings, edited=settings)
        self._refresh()

    def action_check_selected(self) -> None:
        if not self._apply_form_to_draft():
            return
        selected_provider = self._selected_provider()
        result = self.registry.check(
            self.config,
            selected_provider,
            settings=self.draft.edited,
        )
        lines = [
            f"Check: {result.name}",
            f"status: {'ok' if result.ok else 'failed'}",
            f"model: {result.model or '-'}",
        ]
        if result.latency_ms is not None:
            lines.append(f"latency: {result.latency_ms}ms")
        if result.error:
            lines.append(f"error: {result.error}")
        self.draft = self.draft.with_check_result("\n".join(lines))
        self._update_detail(selected_provider)
```

Update `_provider_rows()` to use edited settings and in-memory provider status:

```python
        rows = self.registry.status_payload(self.config, settings=self.draft.edited)
```

Update `_refresh()` to sync form before detail:

```python
        self._sync_form_from_draft(selected_provider)
        self._update_detail(selected_provider)
```

Replace `_update_detail()` body with:

```python
        provider_name = selected_provider or self.draft.edited.dreaming.active_provider
        provider = self.draft.edited.providers.get(provider_name, ProviderSettings())
        dreaming = self.draft.edited.dreaming
        configured, error = _configured_status(provider_name, provider)
        detail = [
            f"settings_path: {self.config.settings_path}",
            f"database_path: {self.config.database_path}",
            f"state: {'unsaved' if self.draft.has_unsaved_changes else 'saved'}",
            "",
            "Autostart",
            f"enabled: {yes_no(dreaming.autostart_enabled)}",
            f"active_provider: {dreaming.active_provider}",
            f"min_interval_minutes: {dreaming.min_interval_minutes}",
            f"new_short_term_memory_threshold: {dreaming.new_short_term_memory_threshold}",
            f"max_cycles_per_autostart: {dreaming.max_cycles_per_autostart}",
            "",
            f"Selected provider: {provider_name}",
            f"enabled: {yes_no(provider.enabled)}",
            f"configured: {yes_no(configured)}",
            f"model: {provider.model or '-'}",
            f"key env: {provider.api_key_env or '-'}",
            f"api_key_present: {yes_no(env_value_exists(provider.api_key_env))}",
            f"base_url: {provider.base_url or '-'}",
            f"timeout_seconds: {provider.timeout_seconds}",
            f"error: {error or '-'}",
        ]
        if self.draft.check_result:
            detail.extend(["", self.draft.check_result])
        if self.draft.errors:
            detail.extend(["", "Validation errors", *self.draft.errors])
        detail.extend(["", "Keys: 1-4 set active, s save, r reload, c check selected, q quit."])
        self.query_one("#config-detail", Static).update("\n".join(detail))
```

- [ ] **Step 5: Add config form styles**

Append to `src/hieronymus/tui/styles.tcss`:

```css
#config-form {
    width: 1fr;
    height: 100%;
    padding: 1 2;
    border-left: solid #2a313a;
    background: #13171b;
}

#config-form Input {
    margin-top: 1;
}
```

Change `#config-detail` to remove width/border duplication:

```css
#config-detail {
    height: auto;
    padding: 0;
    background: #13171b;
    color: #d8dee6;
}
```

- [ ] **Step 6: Run config TUI tests**

Run:

```bash
uv run pytest tests/test_config_tui.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/tui/config_state.py src/hieronymus/tui/config_screens.py src/hieronymus/tui/styles.tcss tests/test_config_tui.py
git commit -m "feat: edit provider settings in config tui"
```

## Task 3: Process-Safe Dream Cycle Lock

**Files:**
- Create: `src/hieronymus/dream_locks.py`
- Create: `tests/test_dream_locks.py`

- [ ] **Step 1: Add failing dream lock tests**

Create `tests/test_dream_locks.py`:

```python
from __future__ import annotations

import os
import time
from datetime import UTC, datetime, timedelta

import pytest

from hieronymus.dream_locks import (
    DreamCycleAlreadyRunning,
    dream_cycle_lock,
    dream_cycle_paths,
    read_dream_cycle_state,
)


def test_dream_cycle_lock_acquires_and_releases(config):
    with dream_cycle_lock(config, owner="manual") as state:
        assert state.owner == "manual"
        assert state.pid == os.getpid()
        assert read_dream_cycle_state(config).owner == "manual"

    assert read_dream_cycle_state(config) is None


def test_second_dream_cycle_lock_fails_while_active(config):
    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(DreamCycleAlreadyRunning, match="dream cycle already running"):
            with dream_cycle_lock(config, owner="autostart"):
                raise AssertionError("second lock must not acquire")


def test_dream_cycle_lock_releases_after_exception(config):
    with pytest.raises(RuntimeError, match="provider failed"):
        with dream_cycle_lock(config, owner="manual"):
            raise RuntimeError("provider failed")

    with dream_cycle_lock(config, owner="manual") as state:
        assert state.owner == "manual"


def test_dream_cycle_lock_wait_blocks_until_release(config):
    with dream_cycle_lock(config, owner="manual"):
        started = time.monotonic()
        with pytest.raises(DreamCycleAlreadyRunning):
            with dream_cycle_lock(config, owner="manual", wait=False):
                pass
        assert time.monotonic() - started < 1


def test_stale_state_with_dead_pid_is_cleaned_conservatively(config):
    paths = dream_cycle_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    paths.state_json.write_text(
        (
            '{"owner":"manual","pid":-1,"started_at":"'
            + (datetime.now(UTC) - timedelta(hours=1)).isoformat()
            + '","token":"stale"}'
        ),
        encoding="utf-8",
    )

    assert read_dream_cycle_state(config) is None
    assert not paths.state_json.exists()
```

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
uv run pytest tests/test_dream_locks.py -v
```

Expected: FAIL because `hieronymus.dream_locks` does not exist.

- [ ] **Step 3: Implement dream lock module**

Create `src/hieronymus/dream_locks.py`:

```python
from __future__ import annotations

import fcntl
import json
import os
import threading
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.service_state import is_pid_running


@dataclass(frozen=True)
class DreamCyclePaths:
    config_root: Path
    lock_file: Path
    state_json: Path


@dataclass(frozen=True)
class DreamCycleState:
    owner: str
    pid: int
    started_at: str
    token: str

    def to_json_dict(self) -> dict[str, object]:
        return asdict(self)

    @classmethod
    def from_json_dict(cls, payload: dict[str, object]) -> DreamCycleState:
        return cls(
            owner=str(payload["owner"]),
            pid=int(payload["pid"]),
            started_at=str(payload["started_at"]),
            token=str(payload["token"]),
        )


class DreamCycleAlreadyRunning(ValueError):
    def __init__(self, state: DreamCycleState | None = None) -> None:
        self.state = state
        detail = f" by {state.owner} pid {state.pid}" if state is not None else ""
        super().__init__(f"dream cycle already running{detail}")


_LOCKS_GUARD = threading.Lock()
_LOCAL_LOCKS: dict[Path, threading.Lock] = {}


def dream_cycle_paths(config: HieronymusConfig) -> DreamCyclePaths:
    root = config.config_root
    return DreamCyclePaths(
        config_root=root,
        lock_file=root / "dream-cycle.lock",
        state_json=root / "dream-cycle.json",
    )


def _local_lock(path: Path) -> threading.Lock:
    with _LOCKS_GUARD:
        lock = _LOCAL_LOCKS.get(path)
        if lock is None:
            lock = threading.Lock()
            _LOCAL_LOCKS[path] = lock
        return lock


def _write_state(paths: DreamCyclePaths, state: DreamCycleState) -> None:
    tmp = paths.state_json.with_name(f"{paths.state_json.name}.tmp-{os.getpid()}")
    tmp.write_text(
        json.dumps(state.to_json_dict(), ensure_ascii=False, sort_keys=True),
        encoding="utf-8",
    )
    tmp.replace(paths.state_json)


def _read_state_file(paths: DreamCyclePaths) -> DreamCycleState | None:
    try:
        payload = json.loads(paths.state_json.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    try:
        return DreamCycleState.from_json_dict(payload)
    except (KeyError, TypeError, ValueError):
        return None


def read_dream_cycle_state(config: HieronymusConfig) -> DreamCycleState | None:
    paths = dream_cycle_paths(config)
    state = _read_state_file(paths)
    if state is None:
        return None
    if is_pid_running(state.pid):
        return state
    try:
        paths.state_json.unlink()
    except FileNotFoundError:
        pass
    return None


@contextmanager
def dream_cycle_lock(
    config: HieronymusConfig,
    *,
    owner: str,
    wait: bool = False,
) -> Iterator[DreamCycleState]:
    paths = dream_cycle_paths(config)
    paths.config_root.mkdir(parents=True, exist_ok=True)
    local_lock = _local_lock(paths.lock_file)
    local_acquired = local_lock.acquire(blocking=wait)
    if not local_acquired:
        raise DreamCycleAlreadyRunning(read_dream_cycle_state(config))

    lock_file = paths.lock_file.open("a+", encoding="utf-8")
    state = DreamCycleState(
        owner=owner,
        pid=os.getpid(),
        started_at=datetime.now(UTC).isoformat(),
        token=uuid.uuid4().hex,
    )
    try:
        flags = fcntl.LOCK_EX if wait else fcntl.LOCK_EX | fcntl.LOCK_NB
        try:
            fcntl.flock(lock_file.fileno(), flags)
        except BlockingIOError as error:
            raise DreamCycleAlreadyRunning(read_dream_cycle_state(config)) from error
        _write_state(paths, state)
        yield state
    finally:
        current = _read_state_file(paths)
        if current is not None and current.token == state.token:
            try:
                paths.state_json.unlink()
            except FileNotFoundError:
                pass
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        lock_file.close()
        local_lock.release()
```

- [ ] **Step 4: Run dream lock tests**

Run:

```bash
uv run pytest tests/test_dream_locks.py -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/dream_locks.py tests/test_dream_locks.py
git commit -m "feat: add dream cycle lock"
```

## Task 4: Route Manual Dream Runs Through the Shared Guard

**Files:**
- Modify: `src/hieronymus/dreaming.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/mcp_server.py`
- Modify: `src/hieronymus/admin.py`
- Modify: `tests/test_dreaming.py`
- Modify: `tests/test_cli.py`
- Modify: `tests/test_mcp_server.py`
- Modify: `tests/test_admin_store.py`

- [ ] **Step 1: Add failing lock integration tests for manual paths**

Add this test to `tests/test_dreaming.py`:

```python
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock


def test_dreaming_rejects_second_cycle_while_lock_is_active(config: HieronymusConfig) -> None:
    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(DreamCycleAlreadyRunning, match="dream cycle already running"):
            DreamService(config, DeterministicDreamProvider()).run_cycle()


def test_dreaming_releases_lock_after_provider_exception(config: HieronymusConfig) -> None:
    class FailingProvider:
        name = "failing"

        def crystallize(self, context, memories):
            raise RuntimeError("provider failed")

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(RuntimeError, match="provider failed"):
        DreamService(config, FailingProvider()).run_cycle()

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()
    assert run.status == "completed"
```

Add this test to `tests/test_cli.py`:

```python
from hieronymus.dream_locks import dream_cycle_lock


def test_dream_returns_clean_error_when_cycle_is_active(tmp_path):
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    runner = CliRunner()

    with dream_cycle_lock(config, owner="manual"):
        result = runner.invoke(main, ["--data-root", str(data_root), "dream"])

    assert result.exit_code == 1
    assert "Error: dream cycle already running" in result.output
    assert "Traceback" not in result.output
```

Add this test to `tests/test_mcp_server.py`:

```python
def test_mcp_dream_rejects_active_cycle(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()

    from hieronymus import mcp_server
    from hieronymus.dream_locks import dream_cycle_lock

    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(ValueError, match="dream cycle already running"):
            mcp_server.hieronymus_dream()
```

Add this test to `tests/test_admin_store.py`:

```python
from hieronymus.dream_locks import dream_cycle_lock


def test_admin_manual_dreaming_uses_shared_cycle_guard(config: HieronymusConfig) -> None:
    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(ValueError, match="dream cycle already running"):
            AdminStore(config).run_manual_dreaming()
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_dreaming.py tests/test_cli.py tests/test_mcp_server.py tests/test_admin_store.py -k "active_cycle or shared_cycle_guard or second_cycle or releases_lock" -v
```

Expected: FAIL because `DreamService.run_cycle()` does not acquire the new lock and CLI/MCP signatures do not support wait.

- [ ] **Step 3: Add lock support to `DreamService.run_cycle`**

Modify imports in `src/hieronymus/dreaming.py`:

```python
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import load_settings
```

Change `run_cycle` into a lock wrapper:

```python
    def run_cycle(
        self,
        *,
        owner: str = "manual",
        wait: bool = False,
        skip_when_locked: bool = False,
    ) -> DreamRunRecord:
        try:
            with dream_cycle_lock(self.config, owner=owner, wait=wait):
                return self._run_cycle_unlocked()
        except DreamCycleAlreadyRunning:
            if skip_when_locked:
                return self._record_skipped_run("dream cycle already running")
            raise
```

Add `DreamCycleAlreadyRunning` to the import:

```python
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock
```

Rename the current `run_cycle(self) -> DreamRunRecord` implementation to:

```python
    def _run_cycle_unlocked(self) -> DreamRunRecord:
```

In the exception block that stores failed errors, sanitize before writing:

```python
            settings = load_settings(self.config)
            error_message = redact_configured_secret_values(str(exc), settings)
            with connect(self.config.database_path) as conn:
                conn.execute(
                    """
                    update dream_runs
                    set status = 'failed',
                        error = ?,
                        completed_at = ?
                    where id = ?
                    """,
                    (error_message, _now(), run_id),
                )
```

Add skipped-run support:

```python
    def _record_skipped_run(self, reason: str) -> DreamRunRecord:
        now = _now()
        with connect(self.config.database_path) as conn:
            cycle_id = self._next_cycle_id(conn)
            cursor = conn.execute(
                """
                insert into dream_runs(cycle_id, status, provider, error, created_at, completed_at)
                values (?, 'skipped', ?, ?, ?, ?)
                """,
                (cycle_id, self.provider.name, reason, now, now),
            )
            run_id = int(cursor.lastrowid)
            conn.commit()
        return DreamRunRecord(
            id=run_id,
            cycle_id=cycle_id,
            status="skipped",
            provider=self.provider.name,
            error=reason,
        )
```

- [ ] **Step 4: Wire CLI and MCP explicit wait**

Modify `src/hieronymus/cli.py` imports:

```python
from hieronymus.dream_locks import DreamCycleAlreadyRunning
```

Modify the dream command decorator and signature:

```python
@click.option("--wait", is_flag=True, help="Wait for an active dream cycle to finish.")
def dream(ctx: click.Context, provider: str | None, json_output: bool, wait: bool) -> None:
```

Change the run call and exception tuple:

```python
        run = DreamService(
            ctx.obj["config"],
            dream_provider,
        ).run_cycle(wait=wait, owner="manual")
    except (KeyError, ValueError, SettingsError, DreamCycleAlreadyRunning) as error:
        _raise_click_error(error)
```

Modify `src/hieronymus/mcp_server.py`:

```python
def hieronymus_dream(provider: str | None = None, wait: bool = False) -> dict[str, int | str]:
    """Run a dream cycle over completed sessions."""
    config = _load_validated_config()
    run = DreamService(config, resolve_provider(config, provider)).run_cycle(
        owner="mcp",
        wait=wait,
    )
```

Modify `src/hieronymus/admin.py`:

```python
        run = DreamService(self.config, resolve_provider(self.config)).run_cycle(owner="admin")
```

- [ ] **Step 5: Run manual path tests**

Run:

```bash
uv run pytest tests/test_dreaming.py tests/test_cli.py tests/test_mcp_server.py tests/test_admin_store.py -k "active_cycle or shared_cycle_guard or second_cycle or releases_lock" -v
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/dreaming.py src/hieronymus/cli.py src/hieronymus/mcp_server.py src/hieronymus/admin.py tests/test_dreaming.py tests/test_cli.py tests/test_mcp_server.py tests/test_admin_store.py
git commit -m "fix: serialize manual dream cycles"
```

## Task 5: Autostart Skip State and Service Status

**Files:**
- Modify: `src/hieronymus/dream_autostart.py`
- Modify: `src/hieronymus/service_http.py`
- Modify: `tests/test_dream_autostart.py`
- Modify: `tests/test_service_http.py`

- [ ] **Step 1: Add failing autostart/status tests**

Add this test to `tests/test_dream_autostart.py`:

```python
from hieronymus.dream_locks import dream_cycle_lock


def test_autostart_skips_and_records_when_cycle_is_active(config: HieronymusConfig) -> None:
    _enable_autostart(config, new_short_term_memory_threshold=1)
    _completed_session(config, _context(config), memories=1)
    now = datetime(2026, 6, 7, 12, 0, tzinfo=UTC)

    with dream_cycle_lock(config, owner="manual"):
        result = DreamAutostart(config).run_due(now=now)
        status = DreamAutostart(config).status()

    assert result == {"ran": False, "reason": "cycle-active", "cycles": 0}
    assert status["cycle_active"] is True
    assert status["active_cycle"]["owner"] == "manual"
    assert status["last_skip_reason"] == "cycle-active"
    assert status["last_skipped_at"] == now.isoformat()

    with connect(config.database_path) as conn:
        run = conn.execute("select status, error from dream_runs").fetchone()
    assert run["status"] == "skipped"
    assert run["error"] == "dream cycle already running"
```

Update `tests/test_service_http.py::test_status_endpoint_returns_paths_and_pid` assertions:

```python
    assert payload["dreaming"]["cycle_active"] is False
    assert payload["dreaming"]["active_cycle"] is None
    assert payload["dreaming"]["last_skip_reason"] == ""
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_dream_autostart.py tests/test_service_http.py -v
```

Expected: FAIL because autostart state does not include skipped/active fields.

- [ ] **Step 3: Extend autostart state**

Modify `AutostartState` in `src/hieronymus/dream_autostart.py`:

```python
@dataclass(frozen=True)
class AutostartState:
    last_started_at: datetime | None = None
    last_error: str = ""
    last_skipped_at: datetime | None = None
    last_skip_reason: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return {
            "last_started_at": self.last_started_at.isoformat()
            if self.last_started_at is not None
            else None,
            "last_error": self.last_error,
            "last_skipped_at": self.last_skipped_at.isoformat()
            if self.last_skipped_at is not None
            else None,
            "last_skip_reason": self.last_skip_reason,
        }
```

Modify `load_autostart_state`:

```python
    return AutostartState(
        last_started_at=_parse_datetime(payload.get("last_started_at")),
        last_error=str(payload.get("last_error", "")),
        last_skipped_at=_parse_datetime(payload.get("last_skipped_at")),
        last_skip_reason=str(payload.get("last_skip_reason", "")),
    )
```

- [ ] **Step 4: Add active lock fields to status**

Modify imports:

```python
from hieronymus.dream_locks import read_dream_cycle_state
```

Modify `DreamAutostart.status()`:

```python
        active_cycle = read_dream_cycle_state(self.config)
        return {
            "enabled": settings.dreaming.autostart_enabled,
            "active_provider": settings.dreaming.active_provider,
            "min_interval_minutes": settings.dreaming.min_interval_minutes,
            "new_short_term_memory_threshold": settings.dreaming.new_short_term_memory_threshold,
            "max_cycles_per_autostart": settings.dreaming.max_cycles_per_autostart,
            "pending_completed_sessions": pending_completed_sessions,
            "pending_short_term_memories": pending_short_term_memories,
            "last_started_at": state.to_json_dict()["last_started_at"],
            "last_error": state.last_error,
            "last_skipped_at": state.to_json_dict()["last_skipped_at"],
            "last_skip_reason": state.last_skip_reason,
            "cycle_active": active_cycle is not None,
            "active_cycle": active_cycle.to_json_dict() if active_cycle is not None else None,
        }
```

- [ ] **Step 5: Make autostart skip active cycles**

Change the autostart run loop in `run_due()`:

```python
            cycles = 0
            attempted_run = True
            service = DreamService(self.config, resolve_provider(self.config))
            for _ in range(settings.dreaming.max_cycles_per_autostart):
                _pending_completed_sessions, pending_short_term_memories = self._pending_counts()
                if pending_short_term_memories == 0:
                    break
                run = service.run_cycle(owner="autostart", skip_when_locked=True)
                if run.status == "skipped":
                    save_autostart_state(
                        self.config,
                        AutostartState(
                            last_started_at=state.last_started_at,
                            last_error="",
                            last_skipped_at=now,
                            last_skip_reason="cycle-active",
                        ),
                    )
                    return {"ran": False, "reason": "cycle-active", "cycles": cycles}
                cycles += 1
            save_autostart_state(self.config, AutostartState(last_started_at=now))
            return {"ran": True, "reason": reason, "cycles": cycles}
```

- [ ] **Step 6: Run autostart/status tests**

Run:

```bash
uv run pytest tests/test_dream_autostart.py tests/test_service_http.py -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/dream_autostart.py src/hieronymus/service_http.py tests/test_dream_autostart.py tests/test_service_http.py
git commit -m "fix: report skipped dream autostart cycles"
```

## Task 6: Secret Leak Regression Coverage

**Files:**
- Modify: `tests/test_cli.py`
- Modify: `tests/test_doctor.py`
- Modify: `tests/test_dreaming.py`
- Modify: `src/hieronymus/doctor.py`
- Modify: `src/hieronymus/dreaming.py`

- [ ] **Step 1: Add failing no-secret regression tests**

Add this test to `tests/test_cli.py`:

```python
def test_config_json_does_not_include_raw_api_key_value(tmp_path, monkeypatch):
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_KEY",
            base_url="https://api.example.test/v1",
        ),
    )
    save_settings(config, settings)

    result = CliRunner().invoke(main, ["--data-root", str(data_root), "config", "--json"])

    assert result.exit_code == 0
    assert "HIERONYMUS_OPENAI_KEY" in result.output
    assert "raw-secret-value" not in result.output
```

Add this test to `tests/test_doctor.py`:

```python
def test_doctor_json_does_not_include_raw_api_key_value(config, monkeypatch):
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_KEY",
        ),
    ).with_dreaming(
        DreamingSettings(active_provider="openai")
    )
    save_settings(config, settings)

    payload = report_to_json(Doctor(config).run())

    assert "HIERONYMUS_OPENAI_KEY" in repr(payload)
    assert "raw-secret-value" not in repr(payload)
```

Add this test to `tests/test_dreaming.py`:

```python
def test_dream_error_records_redact_configured_api_key_value(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_PROVIDER_KEY", "raw-secret-value")
    save_settings(
        config,
        load_settings(config).with_provider(
            "openai",
            ProviderSettings(
                enabled=True,
                model="gpt-4.1-mini",
                api_key_env="HIERONYMUS_PROVIDER_KEY",
            ),
        ),
    )

    class LeakyProvider:
        name = "leaky"

        def crystallize(self, context, memories):
            raise RuntimeError("provider rejected raw-secret-value")

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(RuntimeError, match="raw-secret-value"):
        DreamService(config, LeakyProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select error from dream_runs").fetchone()

    assert run["error"] == "provider rejected [redacted]"
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
uv run pytest tests/test_cli.py tests/test_doctor.py tests/test_dreaming.py -k "raw_api_key_value or redact_configured_api_key" -v
```

Expected: FAIL until imports are added and redaction is wired everywhere.

- [ ] **Step 3: Add test imports**

Add missing imports where needed:

```python
from hieronymus.settings import DreamingSettings, ProviderSettings, load_settings, save_settings
```

- [ ] **Step 4: Redact doctor findings defensively**

Modify `src/hieronymus/doctor.py` imports:

```python
from hieronymus.secrets import redact_configured_secret_values
```

In `_check_settings_and_providers`, after loading settings, add a local helper:

```python
        def safe(message: str) -> str:
            return redact_configured_secret_values(message, settings)
```

Wrap provider-related messages:

```python
                    message=safe(f"Active dream provider is disabled: {active_name}"),
```

```python
                    message=safe(
                        "Missing environment variable for active dream provider: "
                        f"{active.api_key_env}"
                    ),
```

```python
                message=safe(f"Active dream provider is configured: {active_name}"),
```

The current messages should not contain values; this defensive wrapper prevents future regressions.

- [ ] **Step 5: Run no-secret tests**

Run:

```bash
uv run pytest tests/test_cli.py tests/test_doctor.py tests/test_dreaming.py -k "raw_api_key_value or redact_configured_api_key" -v
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/doctor.py src/hieronymus/dreaming.py tests/test_cli.py tests/test_doctor.py tests/test_dreaming.py
git commit -m "test: guard against api key leaks"
```

## Task 7: Documentation and Final Verification

**Files:**
- Modify: `docs/memory-dreaming.md`
- Modify: `docs/usage.md`
- Modify: `README.md`

- [ ] **Step 1: Update memory dreaming docs**

Add this section to `docs/memory-dreaming.md`:

````markdown
## Dream Cycle Concurrency

Dream cycles are serialized per Hieronymus data root. CLI, MCP, admin TUI, and daemon autostart all share the same `dream-cycle.lock` file under the config root, plus an in-process guard for service threads.

Manual runs fail fast when another cycle is active:

```bash
hiero dream
```

Use explicit waiting only when the caller is prepared to block:

```bash
hiero dream --wait
```

Autostart never waits. If a scheduled cycle is due while another cycle is active, it records a skipped dream run and reports `reason: cycle-active` in service status.

The `dream-cycle.json` state file is informational. Hieronymus removes stale state only when the recorded PID is no longer running; it never breaks a live OS lock.
````

- [ ] **Step 2: Update usage docs**

Add this section to `docs/usage.md`:

````markdown
## Editing Dream Provider Settings

Run:

```bash
hiero config
```

The config TUI edits provider fields (`enabled`, `model`, `api_key_env`, `base_url`, `timeout_seconds`) and dreaming automation fields (`active_provider`, `autostart_enabled`, `min_interval_minutes`, `new_short_term_memory_threshold`, `max_cycles_per_autostart`).

Edits stay in memory until saved. Reload discards unsaved edits and reads `settings.toml` again. Provider checks use the edited in-memory settings. API key values are never stored or displayed; the TUI shows only the configured environment variable name and whether that variable exists.
````

- [ ] **Step 3: Update README command summary**

In `README.md`, ensure the command list includes:

```markdown
- `hiero config` edits dream provider and autostart settings in a local TUI.
- `hiero config --json` prints secret-safe provider and dreaming status for automation.
- `hiero dream --wait` waits for an active dream cycle instead of failing fast.
```

- [ ] **Step 4: Run full verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all commands PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/memory-dreaming.md docs/usage.md README.md
git commit -m "docs: document config tui and dream locks"
```

## Self-Review Notes

- Spec coverage: config TUI provider fields are covered in Task 2; automation fields are covered in Task 2; in-memory checks and reload behavior are covered in Tasks 1 and 2; secret handling is covered in Tasks 1, 2, and 6; dream-cycle serialization is covered in Tasks 3, 4, and 5; active/skipped status is covered in Task 5; docs and stale lock behavior are covered in Tasks 3 and 7.
- Placeholder scan: this plan has no deferred implementation markers. Each task includes concrete tests, code shape, commands, expected results, and commit commands.
- Type consistency: `DreamCycleAlreadyRunning`, `DreamCycleState`, `ConfigDraft`, `validate_settings`, `ProviderRegistry.status_payload(settings=...)`, and `ProviderRegistry.check(settings=...)` are introduced before later tasks use them.

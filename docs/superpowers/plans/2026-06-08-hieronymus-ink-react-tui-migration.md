# Hieronymus Ink/React TUI Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Textual admin and config TUIs with an Ink/React terminal UI that talks to the existing Python backend through a typed local JSON-RPC bridge.

**Architecture:** Python remains authoritative for settings, storage, domain validation, dreaming, strict terminology, and admin mutations. TypeScript owns only terminal rendering, keyboard handling, dialogs, local UI state, and runtime validation of JSON payload shapes. The first transport is stdio JSON-RPC launched by the Python CLI, with Textual kept behind `HIERONYMUS_TUI=textual` until Ink parity tests pass.

**Tech Stack:** Python 3.12, Click, SQLite/FTS5, pytest, ruff, TypeScript, React, Ink, Zod, Vitest, pnpm.

---

## Scope Check

This migration touches the config interface and admin interface, but they are not independent enough for separate plans because both require the same JSON-RPC bridge, CLI launch path, frontend project, packaging, and runtime error contract. The tasks below keep the shared bridge first, then deliver the config and admin flows as separately testable slices.

The config UI must not list every dreaming provider as editable rows. It should let the user choose one remote compatible family: `openai`, `gemini`, or `anthropic`; edit API path/base URL where the provider supports it; edit model; and show model suggestions when the provider API supports fetching available models. Python settings may still retain the existing provider dictionary internally for compatibility.

## File Structure

- Create `src/hieronymus/tui_bridge/__init__.py`: package marker and public bridge entry point.
- Create `src/hieronymus/tui_bridge/protocol.py`: JSON-RPC request/response typed dictionaries and JSON-safe dataclass conversion helpers.
- Create `src/hieronymus/tui_bridge/errors.py`: stable error codes and secret-safe exception conversion.
- Create `src/hieronymus/tui_bridge/config_api.py`: config method handlers; wraps settings, provider registry, config draft helpers, secret redaction, and model suggestions.
- Create `src/hieronymus/tui_bridge/admin_api.py`: admin method handlers; wraps `AdminStore`, applies filters, returns refreshed snapshots, and never writes SQL directly.
- Create `src/hieronymus/tui_bridge/server.py`: stdio JSON-RPC loop and dispatch table for `admin.*` and `config.*`.
- Modify `src/hieronymus/dream_providers.py`: add provider model suggestion support with safe fallback payloads and no secret leakage.
- Modify `src/hieronymus/cli.py`: route `hiero admin` and `hiero config` to Ink when requested, keep `--json`, and keep Textual fallback.
- Modify `src/hieronymus/doctor.py`: report Node.js and pnpm availability once Ink runtime is introduced.
- Modify `pyproject.toml`: include bundled frontend build artifacts in package data if needed by Hatch.
- Create `frontend/package.json`: Ink frontend scripts and dependencies.
- Create `frontend/tsconfig.json`: TypeScript configuration for NodeNext/React JSX.
- Create `frontend/vitest.config.ts`: Vitest configuration.
- Create `frontend/src/main.tsx`: command entry point invoked by Python CLI.
- Create `frontend/src/rpc/client.ts`: stdio JSON-RPC client that spawns the Python bridge command.
- Create `frontend/src/rpc/schema.ts`: Zod schemas mirroring bridge payloads.
- Create `frontend/src/app/App.tsx`: routes admin/config mode to the right screen.
- Create `frontend/src/app/routes.ts`: route type definitions.
- Create `frontend/src/config/ConfigScreen.tsx`: config screen state machine.
- Create `frontend/src/config/ProviderSelector.tsx`: one-of-three provider family selector.
- Create `frontend/src/config/ConfigForm.tsx`: provider/dreaming form controls.
- Create `frontend/src/admin/AdminScreen.tsx`: admin screen state machine.
- Create `frontend/src/admin/AdminTable.tsx`: admin row list and selection.
- Create `frontend/src/admin/DetailPane.tsx`: detail renderer.
- Create `frontend/src/admin/CommandPalette.tsx`: scoped command palette.
- Create `frontend/src/admin/dialogs.tsx`: add/edit/filter/delete/merge/split/supersede dialogs.
- Create `frontend/src/ui/FocusableList.tsx`: shared list navigation helper.
- Create `frontend/src/ui/StatusLine.tsx`: shared status/error footer.
- Create `frontend/src/ui/KeyHelp.tsx`: shared key binding help line.
- Create `tests/test_tui_bridge_protocol.py`: bridge envelope and dispatch tests.
- Create `tests/test_tui_bridge_config.py`: config bridge tests.
- Create `tests/test_tui_bridge_admin.py`: admin bridge tests.
- Create `tests/test_cli_ink_tui.py`: CLI launch selection tests.
- Create `frontend/src/rpc/client.test.ts`: RPC client tests.
- Create `frontend/src/rpc/schema.test.ts`: runtime schema tests.
- Create `frontend/src/config/ConfigScreen.test.tsx`: config state transition tests.
- Create `frontend/src/admin/AdminScreen.test.tsx`: admin state transition tests.
- Modify `docs/usage.md`: document Ink runtime, feature flag, provider-family config UX, and any keyboard differences before default switch.

---

### Task 1: JSON-RPC Protocol and Safe Error Envelopes

**Files:**
- Create: `src/hieronymus/tui_bridge/__init__.py`
- Create: `src/hieronymus/tui_bridge/protocol.py`
- Create: `src/hieronymus/tui_bridge/errors.py`
- Create: `tests/test_tui_bridge_protocol.py`

- [ ] **Step 1: Write failing protocol tests**

Add `tests/test_tui_bridge_protocol.py`:

```python
from dataclasses import dataclass

from hieronymus.settings import default_settings
from hieronymus.tui_bridge.errors import error_payload
from hieronymus.tui_bridge.protocol import (
    RpcError,
    RpcRequest,
    dataclass_to_json,
    error_response,
    parse_request,
    success_response,
)


@dataclass(frozen=True)
class Child:
    name: str


@dataclass(frozen=True)
class Parent:
    id: int
    child: Child
    tags: tuple[str, ...]


def test_parse_request_accepts_valid_json_rpc_object() -> None:
    request = parse_request('{"id":"1","method":"admin.snapshot","params":{"view":"Crystals"}}')

    assert request == RpcRequest(
        id="1",
        method="admin.snapshot",
        params={"view": "Crystals"},
    )


def test_parse_request_rejects_non_object_params() -> None:
    response = error_response("1", RpcError("invalid_request", "params must be an object"))

    assert response == {
        "id": "1",
        "ok": False,
        "error": {"code": "invalid_request", "message": "params must be an object"},
    }


def test_success_response_wraps_result() -> None:
    assert success_response("9", {"ready": True}) == {
        "id": "9",
        "ok": True,
        "result": {"ready": True},
    }


def test_dataclass_to_json_recurses_without_private_state() -> None:
    assert dataclass_to_json(Parent(id=4, child=Child("x"), tags=("a", "b"))) == {
        "id": 4,
        "child": {"name": "x"},
        "tags": ["a", "b"],
    }


def test_error_payload_redacts_configured_secret_values(monkeypatch) -> None:
    settings = default_settings()
    monkeypatch.setenv("OPENAI_API_KEY", "raw-secret-value")

    payload = error_payload(
        ValueError("provider rejected raw-secret-value"),
        settings=settings,
    )

    assert payload == {
        "code": "validation_error",
        "message": "provider rejected [redacted]",
    }
```

- [ ] **Step 2: Run protocol tests to verify failure**

Run:

```bash
uv run pytest tests/test_tui_bridge_protocol.py -v
```

Expected: FAIL with `ModuleNotFoundError: No module named 'hieronymus.tui_bridge'`.

- [ ] **Step 3: Add protocol and error modules**

Create `src/hieronymus/tui_bridge/__init__.py`:

```python
from hieronymus.tui_bridge.server import run_stdio

__all__ = ["run_stdio"]
```

Create `src/hieronymus/tui_bridge/protocol.py`:

```python
from __future__ import annotations

import json
from dataclasses import fields, is_dataclass
from typing import Any, NamedTuple


class RpcRequest(NamedTuple):
    id: str
    method: str
    params: dict[str, object]


class RpcError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


def parse_request(line: str) -> RpcRequest:
    try:
        payload = json.loads(line)
    except json.JSONDecodeError as error:
        raise RpcError("invalid_json", f"invalid JSON: {error.msg}") from error
    if type(payload) is not dict:
        raise RpcError("invalid_request", "request must be an object")
    request_id = payload.get("id")
    method = payload.get("method")
    params = payload.get("params", {})
    if type(request_id) is not str or not request_id:
        raise RpcError("invalid_request", "id must be a non-empty string")
    if type(method) is not str or not method:
        raise RpcError("invalid_request", "method must be a non-empty string")
    if type(params) is not dict:
        raise RpcError("invalid_request", "params must be an object")
    return RpcRequest(id=request_id, method=method, params=params)


def dataclass_to_json(value: Any) -> Any:
    if is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: dataclass_to_json(getattr(value, field.name))
            for field in fields(value)
            if not field.name.startswith("_")
        }
    if isinstance(value, tuple):
        return [dataclass_to_json(item) for item in value]
    if isinstance(value, list):
        return [dataclass_to_json(item) for item in value]
    if isinstance(value, dict):
        return {str(key): dataclass_to_json(item) for key, item in value.items()}
    return value


def success_response(request_id: str, result: dict[str, object]) -> dict[str, object]:
    return {"id": request_id, "ok": True, "result": result}


def error_response(request_id: str | None, error: RpcError) -> dict[str, object]:
    return {
        "id": request_id,
        "ok": False,
        "error": {"code": error.code, "message": error.message},
    }
```

Create `src/hieronymus/tui_bridge/errors.py`:

```python
from __future__ import annotations

from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import HieronymusSettings, SettingsError
from hieronymus.tui_bridge.protocol import RpcError


def error_code(error: Exception) -> str:
    if isinstance(error, RpcError):
        return error.code
    if isinstance(error, SettingsError | ValueError | KeyError):
        return "validation_error"
    return "internal_error"


def display_message(error: Exception, *, settings: HieronymusSettings | None = None) -> str:
    if isinstance(error, RpcError):
        message = error.message
    elif isinstance(error, KeyError) and error.args:
        message = str(error.args[0])
    elif isinstance(error, SettingsError | ValueError):
        message = str(error)
    else:
        message = "Unexpected backend error"
    if settings is not None:
        return redact_configured_secret_values(message, settings)
    return message


def error_payload(
    error: Exception,
    *,
    settings: HieronymusSettings | None = None,
) -> dict[str, str]:
    return {
        "code": error_code(error),
        "message": display_message(error, settings=settings),
    }
```

- [ ] **Step 4: Run protocol tests to verify pass**

Run:

```bash
uv run pytest tests/test_tui_bridge_protocol.py -v
```

Expected: PASS all tests in `tests/test_tui_bridge_protocol.py`.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/tui_bridge/__init__.py src/hieronymus/tui_bridge/protocol.py src/hieronymus/tui_bridge/errors.py tests/test_tui_bridge_protocol.py
git commit -m "feat: add TUI bridge protocol envelopes"
```

---

### Task 2: Config Bridge with One Provider Family and Model Suggestions

**Files:**
- Create: `src/hieronymus/tui_bridge/config_api.py`
- Modify: `src/hieronymus/dream_providers.py`
- Create: `tests/test_tui_bridge_config.py`
- Modify: `tests/test_dream_providers.py`

- [ ] **Step 1: Write failing config bridge tests**

Add `tests/test_tui_bridge_config.py`:

```python
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui_bridge.config_api import ConfigBridge


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def test_config_bootstrap_returns_one_remote_provider_selector(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.bootstrap({})

    assert [choice["name"] for choice in payload["provider_choices"]] == [
        "openai",
        "gemini",
        "anthropic",
    ]
    assert payload["selected_provider"] == "openai"
    assert payload["form_values"]["provider"]["api_path"] == "https://api.openai.com/v1"
    assert "deterministic" not in [choice["name"] for choice in payload["provider_choices"]]


def test_config_select_provider_enables_only_selected_remote_provider(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.select_provider({"provider": "gemini", "draft": {}})

    assert payload["selected_provider"] == "gemini"
    providers = payload["draft"]["providers"]
    assert providers["gemini"]["enabled"] is True
    assert providers["openai"]["enabled"] is False
    assert providers["anthropic"]["enabled"] is False
    assert payload["draft"]["dreaming"]["active_provider"] == "gemini"


def test_config_update_draft_uses_api_path_alias_for_base_url(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1",
                "api_key_env": "HIERONYMUS_OPENAI_KEY",
                "api_path": "https://llm.example.test/v1",
                "timeout_seconds": "12.5",
            },
            "dreaming": {
                "autostart_enabled": "yes",
                "min_interval_minutes": "9",
                "new_short_term_memory_threshold": "3",
                "max_cycles_per_autostart": "2",
            },
        }
    )

    assert payload["validation"]["ok"] is True
    assert payload["form_values"]["provider"]["api_path"] == "https://llm.example.test/v1"
    assert payload["draft"]["providers"]["openai"]["base_url"] == "https://llm.example.test/v1"


def test_config_save_persists_valid_selected_provider(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = ConfigBridge(config)
    draft = bridge.update_draft(
        {
            "selected_provider": "gemini",
            "provider": {
                "model": "gemini-2.5-flash",
                "api_key_env": "GEMINI_API_KEY",
                "api_path": "",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "30",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
        }
    )["draft"]

    payload = bridge.save({"draft": draft})

    settings = load_settings(config)
    assert payload["validation"]["ok"] is True
    assert settings.dreaming.active_provider == "gemini"
    assert settings.providers["gemini"].enabled is True
    assert settings.providers["openai"].enabled is False


def test_config_save_rejects_invalid_dreaming_threshold(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))
    draft = bridge.update_draft(
        {
            "selected_provider": "openai",
            "provider": {
                "model": "gpt-4.1-mini",
                "api_key_env": "OPENAI_API_KEY",
                "api_path": "https://api.openai.com/v1",
                "timeout_seconds": "30",
            },
            "dreaming": {
                "autostart_enabled": "no",
                "min_interval_minutes": "0",
                "new_short_term_memory_threshold": "25",
                "max_cycles_per_autostart": "1",
            },
        }
    )["draft"]

    payload = bridge.save({"draft": draft})

    assert payload["validation"] == {
        "ok": False,
        "errors": ["min_interval_minutes must be at least 1"],
    }


def test_config_check_provider_redacts_error(tmp_path: Path, monkeypatch) -> None:
    class Registry:
        def list_model_suggestions(self, *args, **kwargs):
            return {"provider": "openai", "models": [], "source": "unavailable", "error": ""}

        def check(self, *args, **kwargs):
            class Result:
                def to_json_dict(self):
                    return {
                        "name": "openai",
                        "ok": False,
                        "model": "gpt-4.1-mini",
                        "error": "provider returned raw-secret-value",
                        "latency_ms": 10,
                    }

            return Result()

    config = _config(tmp_path)
    monkeypatch.setenv("OPENAI_API_KEY", "raw-secret-value")
    bridge = ConfigBridge(config, registry=Registry())

    payload = bridge.check_provider({"selected_provider": "openai", "draft": {}})

    assert payload["check_result"]["error"] == "provider returned [redacted]"
    assert "raw-secret-value" not in repr(payload)


def test_config_model_suggestions_fall_back_to_defaults(tmp_path: Path) -> None:
    bridge = ConfigBridge(_config(tmp_path))

    payload = bridge.model_suggestions({"selected_provider": "anthropic", "draft": {}})

    assert payload["suggestions"]["provider"] == "anthropic"
    assert "claude-3-5-haiku-latest" in payload["suggestions"]["models"]
```

- [ ] **Step 2: Run config bridge tests to verify failure**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py -v
```

Expected: FAIL with `ModuleNotFoundError` or `ImportError` for `ConfigBridge`.

- [ ] **Step 3: Add provider model suggestions**

Modify `src/hieronymus/dream_providers.py`:

```python
@dataclass(frozen=True)
class ModelSuggestionResult:
    provider: str
    models: list[str]
    source: str
    error: str = ""

    def to_json_dict(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": self.models,
            "source": self.source,
            "error": self.error,
        }
```

Add this method to `ProviderRegistry`:

```python
    def list_model_suggestions(
        self,
        config: HieronymusConfig,
        name: str,
        *,
        settings: HieronymusSettings | None = None,
    ) -> ModelSuggestionResult:
        self.metadata(name)
        defaults = {
            "openai": ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
            "gemini": ["gemini-2.5-flash", "gemini-2.5-pro"],
            "anthropic": ["claude-3-5-haiku-latest", "claude-3-7-sonnet-latest"],
            "deterministic": [""],
        }
        if name in {"anthropic", "deterministic"}:
            return ModelSuggestionResult(
                provider=name,
                models=defaults[name],
                source="defaults",
            )
        active_settings = settings or load_settings(config)
        provider = active_settings.providers.get(name, ProviderSettings())
        key = os.environ.get(provider.api_key_env)
        if not key:
            return ModelSuggestionResult(
                provider=name,
                models=defaults[name],
                source="defaults",
                error=f"missing environment variable: {provider.api_key_env}",
            )
        try:
            if name == "openai":
                base_url = (provider.base_url or "https://api.openai.com/v1").rstrip("/")
                response = self._transport.get_json(
                    f"{base_url}/models",
                    headers={"Authorization": f"Bearer {key}"},
                    timeout=provider.timeout_seconds,
                )
                if 200 <= response.status < 300:
                    payload = json.loads(response.body)
                    models = sorted(
                        str(item["id"])
                        for item in payload.get("data", [])
                        if type(item) is dict and type(item.get("id")) is str
                    )
                    if models:
                        return ModelSuggestionResult(name, models, "api")
            if name == "gemini":
                response = self._transport.get_json(
                    "https://generativelanguage.googleapis.com/v1beta/models",
                    headers={"x-goog-api-key": key},
                    timeout=provider.timeout_seconds,
                )
                if 200 <= response.status < 300:
                    payload = json.loads(response.body)
                    models = sorted(
                        str(item["name"]).removeprefix("models/")
                        for item in payload.get("models", [])
                        if type(item) is dict and type(item.get("name")) is str
                    )
                    if models:
                        return ModelSuggestionResult(name, models, "api")
        except Exception:
            return ModelSuggestionResult(
                provider=name,
                models=defaults[name],
                source="defaults",
                error="model suggestions unavailable",
            )
        return ModelSuggestionResult(
            provider=name,
            models=defaults[name],
            source="defaults",
            error="model suggestions unavailable",
        )
```

Extend `HTTPTransport` and `UrllibTransport`:

```python
class HTTPTransport(Protocol):
    def post_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        payload: dict[str, object],
        timeout: float,
    ) -> HTTPResponse: ...

    def get_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        timeout: float,
    ) -> HTTPResponse: ...
```

```python
    def get_json(
        self,
        url: str,
        *,
        headers: dict[str, str],
        timeout: float,
    ) -> HTTPResponse:
        request = urllib.request.Request(
            url,
            headers={**headers, "Accept": "application/json"},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                body = response.read().decode("utf-8", errors="replace")
                return HTTPResponse(status=response.status, body=body)
        except urllib.error.HTTPError as error:
            body = error.read().decode("utf-8", errors="replace")
            return HTTPResponse(status=error.code, body=body)
        except urllib.error.URLError:
            return HTTPResponse(status=0, body="network error")
```

- [ ] **Step 4: Add config bridge implementation**

Create `src/hieronymus/tui_bridge/config_api.py`:

```python
from __future__ import annotations

from dataclasses import replace

from hieronymus.config import HieronymusConfig
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import (
    DreamingSettings,
    HieronymusSettings,
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)
from hieronymus.tui.config_state import (
    apply_dreaming_form,
    apply_provider_form,
    field_value,
    validate_draft,
)
from hieronymus.tui_bridge.protocol import dataclass_to_json

REMOTE_PROVIDER_CHOICES = ("openai", "gemini", "anthropic")


class ConfigBridge:
    def __init__(self, config: HieronymusConfig, registry: ProviderRegistry | None = None) -> None:
        self.config = config
        self.registry = registry or ProviderRegistry()

    def bootstrap(self, params: dict[str, object]) -> dict[str, object]:
        settings = load_settings(self.config)
        selected = self._selected_remote_provider(settings, params.get("selected_provider"))
        return self._payload(settings, selected)

    def select_provider(self, params: dict[str, object]) -> dict[str, object]:
        provider = self._require_provider(params.get("provider"))
        settings = self._settings_from_draft(params.get("draft"))
        settings = self._select_only_provider(settings, provider)
        return self._payload(settings, provider)

    def update_draft(self, params: dict[str, object]) -> dict[str, object]:
        selected = self._require_provider(params.get("selected_provider"))
        settings = self._settings_from_draft(params.get("draft"))
        provider_values = self._provider_form(params.get("provider"), settings, selected)
        dreaming_values = self._dreaming_form(params.get("dreaming"), settings, selected)
        try:
            edited = apply_provider_form(settings, selected, provider_values)
            edited = apply_dreaming_form(edited, dreaming_values)
            edited = self._select_only_provider(edited, selected)
            validation = self._validation(edited)
        except SettingsError as error:
            edited = settings
            validation = {"ok": False, "errors": [str(error)]}
        return self._payload(edited, selected, validation=validation)

    def save(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_draft(params.get("draft"))
        validation = self._validation(settings)
        if not validation["ok"]:
            selected = self._selected_remote_provider(settings, None)
            return self._payload(settings, selected, validation=validation)
        save_settings(self.config, settings)
        saved = load_settings(self.config)
        selected = self._selected_remote_provider(saved, None)
        return self._payload(saved, selected, validation={"ok": True, "errors": []})

    def reload(self, params: dict[str, object]) -> dict[str, object]:
        settings = load_settings(self.config)
        selected = self._selected_remote_provider(settings, params.get("selected_provider"))
        return self._payload(settings, selected)

    def check_provider(self, params: dict[str, object]) -> dict[str, object]:
        selected = self._require_provider(params.get("selected_provider"))
        settings = self._settings_from_draft(params.get("draft"))
        result = self.registry.check(self.config, selected, settings=settings).to_json_dict()
        if result.get("error"):
            result["error"] = redact_configured_secret_values(str(result["error"]), settings)
        payload = self._payload(settings, selected)
        payload["check_result"] = result
        return payload

    def model_suggestions(self, params: dict[str, object]) -> dict[str, object]:
        selected = self._require_provider(params.get("selected_provider"))
        settings = self._settings_from_draft(params.get("draft"))
        result = self.registry.list_model_suggestions(
            self.config,
            selected,
            settings=settings,
        ).to_json_dict()
        if result.get("error"):
            result["error"] = redact_configured_secret_values(str(result["error"]), settings)
        payload = self._payload(settings, selected)
        payload["suggestions"] = result
        return payload

    def _payload(
        self,
        settings: HieronymusSettings,
        selected: str,
        *,
        validation: dict[str, object] | None = None,
    ) -> dict[str, object]:
        suggestions = self.registry.list_model_suggestions(
            self.config,
            selected,
            settings=settings,
        ).to_json_dict()
        return {
            "config_paths": {
                "config_root": str(self.config.config_root),
                "settings_path": str(self.config.settings_path),
                "database_path": str(self.config.database_path),
            },
            "provider_choices": self._provider_choices(),
            "selected_provider": selected,
            "draft": settings.to_json_dict(),
            "form_values": self._form_values(settings, selected),
            "validation": validation or self._validation(settings),
            "suggestions": suggestions,
            "detail": self._detail(settings, selected, validation or self._validation(settings)),
        }

    def _provider_choices(self) -> list[dict[str, object]]:
        names = set(REMOTE_PROVIDER_CHOICES)
        return [
            {
                "name": metadata.name,
                "display_name": metadata.display_name,
                "supports_api_path": metadata.supports_base_url,
            }
            for metadata in self.registry.list()
            if metadata.name in names
        ]

    def _form_values(self, settings: HieronymusSettings, selected: str) -> dict[str, object]:
        provider = settings.providers.get(selected, ProviderSettings())
        dreaming = settings.dreaming
        return {
            "provider": {
                "model": field_value(provider.model),
                "api_key_env": field_value(provider.api_key_env),
                "api_path": field_value(provider.base_url),
                "timeout_seconds": field_value(provider.timeout_seconds),
            },
            "dreaming": {
                "autostart_enabled": field_value(dreaming.autostart_enabled),
                "min_interval_minutes": field_value(dreaming.min_interval_minutes),
                "new_short_term_memory_threshold": field_value(
                    dreaming.new_short_term_memory_threshold
                ),
                "max_cycles_per_autostart": field_value(dreaming.max_cycles_per_autostart),
            },
        }

    def _detail(
        self,
        settings: HieronymusSettings,
        selected: str,
        validation: dict[str, object],
    ) -> dict[str, object]:
        provider = settings.providers.get(selected, ProviderSettings())
        return {
            "title": f"{selected} dreaming provider",
            "fields": [
                ["settings_path", str(self.config.settings_path)],
                ["active_provider", settings.dreaming.active_provider],
                ["model", provider.model or "-"],
                ["api_key_env", provider.api_key_env or "-"],
                ["api_path", provider.base_url or "-"],
                ["timeout_seconds", str(provider.timeout_seconds)],
            ],
            "errors": validation["errors"],
        }

    def _validation(self, settings: HieronymusSettings) -> dict[str, object]:
        errors = validate_draft(settings)
        return {"ok": not errors, "errors": errors}

    def _settings_from_draft(self, draft: object) -> HieronymusSettings:
        if not draft:
            return load_settings(self.config)
        if type(draft) is not dict:
            raise ValueError("draft must be an object")
        dreaming_payload = draft.get("dreaming", {})
        providers_payload = draft.get("providers", {})
        if type(dreaming_payload) is not dict or type(providers_payload) is not dict:
            raise ValueError("draft must contain object fields")
        dreaming = DreamingSettings(**dreaming_payload)
        providers = {
            str(name): ProviderSettings(**payload)
            for name, payload in providers_payload.items()
            if type(payload) is dict
        }
        return HieronymusSettings(dreaming=dreaming, providers=providers)

    def _provider_form(
        self,
        raw: object,
        settings: HieronymusSettings,
        selected: str,
    ) -> dict[str, str]:
        provider = settings.providers.get(selected, ProviderSettings())
        values = {
            "enabled": "yes",
            "model": field_value(provider.model),
            "api_key_env": field_value(provider.api_key_env),
            "base_url": field_value(provider.base_url),
            "timeout_seconds": field_value(provider.timeout_seconds),
        }
        if type(raw) is dict:
            for key in ("model", "api_key_env", "timeout_seconds"):
                if key in raw:
                    values[key] = str(raw[key])
            if "api_path" in raw:
                values["base_url"] = str(raw["api_path"])
        return values

    def _dreaming_form(
        self,
        raw: object,
        settings: HieronymusSettings,
        selected: str,
    ) -> dict[str, str]:
        dreaming = settings.dreaming
        values = {
            "active_provider": selected,
            "autostart_enabled": field_value(dreaming.autostart_enabled),
            "min_interval_minutes": field_value(dreaming.min_interval_minutes),
            "new_short_term_memory_threshold": field_value(
                dreaming.new_short_term_memory_threshold
            ),
            "max_cycles_per_autostart": field_value(dreaming.max_cycles_per_autostart),
        }
        if type(raw) is dict:
            for key in values:
                if key in raw and key != "active_provider":
                    values[key] = str(raw[key])
        return values

    def _select_only_provider(
        self,
        settings: HieronymusSettings,
        selected: str,
    ) -> HieronymusSettings:
        providers = {}
        for name, provider in settings.providers.items():
            if name in REMOTE_PROVIDER_CHOICES:
                providers[name] = replace(provider, enabled=name == selected)
            else:
                providers[name] = provider
        return HieronymusSettings(
            dreaming=replace(settings.dreaming, active_provider=selected),
            providers=providers,
        )

    def _selected_remote_provider(self, settings: HieronymusSettings, raw: object) -> str:
        if type(raw) is str and raw in REMOTE_PROVIDER_CHOICES:
            return raw
        if settings.dreaming.active_provider in REMOTE_PROVIDER_CHOICES:
            return settings.dreaming.active_provider
        return "openai"

    def _require_provider(self, raw: object) -> str:
        if type(raw) is str and raw in REMOTE_PROVIDER_CHOICES:
            return raw
        raise ValueError("selected_provider must be openai, gemini, or anthropic")
```

- [ ] **Step 5: Add provider suggestion transport tests**

Append to `tests/test_dream_providers.py`:

```python
def test_openai_model_suggestions_use_models_endpoint(tmp_path, monkeypatch) -> None:
    class Transport:
        def __init__(self):
            self.requests = []

        def post_json(self, *args, **kwargs):
            raise AssertionError("post_json should not be used for model suggestions")

        def get_json(self, url, *, headers, timeout):
            self.requests.append({"url": url, "headers": headers, "timeout": timeout})
            return HTTPResponse(
                status=200,
                body='{"data":[{"id":"gpt-4.1"},{"id":"gpt-4.1-mini"}]}',
            )

    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config)
    monkeypatch.setenv("OPENAI_API_KEY", "secret-openai")
    transport = Transport()

    result = ProviderRegistry(transport).list_model_suggestions(
        config,
        "openai",
        settings=settings,
    )

    assert result.to_json_dict() == {
        "provider": "openai",
        "models": ["gpt-4.1", "gpt-4.1-mini"],
        "source": "api",
        "error": "",
    }
    assert transport.requests[0]["url"] == "https://api.openai.com/v1/models"
    assert "secret-openai" not in repr(result.to_json_dict())
```

Ensure the file imports `HTTPResponse`, `HieronymusConfig`, `ProviderRegistry`, and `load_settings`.

- [ ] **Step 6: Run config and provider tests**

Run:

```bash
uv run pytest tests/test_tui_bridge_config.py tests/test_dream_providers.py -v
```

Expected: PASS all tests in both files.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/tui_bridge/config_api.py src/hieronymus/dream_providers.py tests/test_tui_bridge_config.py tests/test_dream_providers.py
git commit -m "feat: add config bridge provider selection"
```

---

### Task 3: Admin Bridge Methods and Snapshot Refresh

**Files:**
- Create: `src/hieronymus/tui_bridge/admin_api.py`
- Create: `tests/test_tui_bridge_admin.py`

- [ ] **Step 1: Write failing admin bridge tests**

Add `tests/test_tui_bridge_admin.py`:

```python
from pathlib import Path

import pytest

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.concepts import ConceptProposalStore
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui_bridge.admin_api import AdminBridge


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def _seed(config: HieronymusConfig) -> int:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )
    return CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        title="Guild Ledger",
        text="Guild ledger detail marker.",
    )


def test_admin_bootstrap_returns_views_stats_and_initial_snapshot(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    payload = AdminBridge(config).bootstrap({})

    assert payload["views"] == list(ADMIN_VIEWS)
    assert payload["default_view"] == "Crystals"
    assert payload["stats"]["series"] == 1
    assert payload["snapshot"]["view"] == "Crystals"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger"


def test_admin_snapshot_filters_crystal_status(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)
    AdminStore(config).delete_crystal(crystal_id, evidence="test")

    payload = AdminBridge(config).snapshot(
        {"view": "Crystals", "filters": {"status": "active"}}
    )

    assert payload["snapshot"]["rows"] == []
    assert payload["snapshot"]["filters"] == ["status=active"]


def test_admin_delete_requires_confirmation(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    with pytest.raises(ValueError, match="delete requires confirmation"):
        AdminBridge(config).delete_crystal({"id": crystal_id, "confirmed": False})


def test_admin_delete_mutates_through_store_and_refreshes_snapshot(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).delete_crystal({"id": crystal_id, "confirmed": True})

    assert payload["result"]["action"] == "delete"
    assert CrystalStore(config).get(crystal_id).status == "archived"
    assert payload["snapshot"]["selected"]["status"] == "archived"


def test_admin_edit_crystal_refreshes_selected_detail(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).edit_crystal(
        {"id": crystal_id, "title": "Guild Ledger Notes", "text": "Keep term stable."}
    )

    assert payload["result"]["message"] == "Crystal edited"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger Notes"
    assert payload["snapshot"]["detail"]["body"] == "Keep term stable."


def test_admin_proposal_approval_refreshes_proposal_view(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    proposal_id = ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        concept_text="Sense",
        source_form="センス",
        canonical_rendering="сенс",
        rationale="Palette proposal fixture.",
    )

    payload = AdminBridge(config).approve_proposal({"id": proposal_id})

    assert payload["result"]["entity_type"] == "strict_term"
    assert payload["snapshot"]["view"] == "Proposals"
    assert payload["snapshot"]["selected"]["status"] == "approved"
```

- [ ] **Step 2: Run admin bridge tests to verify failure**

Run:

```bash
uv run pytest tests/test_tui_bridge_admin.py -v
```

Expected: FAIL with `ModuleNotFoundError` or `ImportError` for `AdminBridge`.

- [ ] **Step 3: Add admin bridge implementation**

Create `src/hieronymus/tui_bridge/admin_api.py`:

```python
from __future__ import annotations

from dataclasses import replace

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.admin_models import ActionResult, AdminSnapshot
from hieronymus.config import HieronymusConfig
from hieronymus.tui_bridge.protocol import dataclass_to_json


class AdminBridge:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.store = AdminStore(config)

    def bootstrap(self, params: dict[str, object]) -> dict[str, object]:
        view = self._view(params.get("view"), default="Crystals")
        return {
            "views": list(ADMIN_VIEWS),
            "default_view": view,
            "stats": self.store.stats().as_dict(),
            "service": self.store.status_payload()["service"],
            "snapshot": self._snapshot_payload(view, None, params.get("filters")),
        }

    def snapshot(self, params: dict[str, object]) -> dict[str, object]:
        view = self._view(params.get("view"), default="Crystals")
        selected_id = params.get("selected_id")
        return {
            "stats": self.store.stats().as_dict(),
            "snapshot": self._snapshot_payload(view, selected_id, params.get("filters")),
        }

    def add_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self.store.add_crystal(
            series_slug=self._str(params, "series_slug"),
            source_language=self._str(params, "source_language"),
            target_language=self._str(params, "target_language"),
            crystal_type=self._str(params, "type"),
            title=self._str(params, "title"),
            text=self._str(params, "text"),
            tags=self._tags(params.get("tags")),
        )
        return self._action_payload(
            ActionResult("crystal", crystal_id, "add", "Crystal added"),
            view="Crystals",
            selected_id=crystal_id,
        )

    def edit_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        result = self.store.edit_crystal(
            crystal_id,
            title=self._str(params, "title"),
            text=self._str(params, "text"),
        )
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def merge_crystals(self, params: dict[str, object]) -> dict[str, object]:
        ids = params.get("ids")
        if type(ids) is not list:
            raise ValueError("ids must be a list")
        merged_id = self.store.merge_crystals(
            [int(item) for item in ids],
            title=self._str(params, "title"),
            text=self._str(params, "text"),
        )
        return self._action_payload(
            ActionResult("crystal", merged_id, "merge", "Crystals merged"),
            view="Crystals",
            selected_id=merged_id,
        )

    def split_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        parts = [
            {"title": self._str(params, "part_one_title"), "text": self._str(params, "part_one_text")},
            {"title": self._str(params, "part_two_title"), "text": self._str(params, "part_two_text")},
        ]
        new_ids = self.store.split_crystal(crystal_id, parts=parts)
        return {
            "result": {"entity_type": "crystal", "entity_id": new_ids, "action": "split", "message": "Crystal split"},
            "stats": self.store.stats().as_dict(),
            "snapshot": self._snapshot_payload("Crystals", new_ids[0], None),
        }

    def supersede_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        result = self.store.supersede_crystal(
            crystal_id,
            replacement_id=self._int(params, "replacement_id"),
            evidence="Superseded from Ink admin TUI",
        )
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def reinforce_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        result = self.store.reinforce_crystal(crystal_id, evidence="Reinforced from Ink admin TUI")
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def decay_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        result = self.store.decay_crystal(crystal_id, evidence="Decayed from Ink admin TUI")
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def deprecate_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = self._int(params, "id")
        result = self.store.deprecate_crystal(crystal_id, evidence="Deprecated from Ink admin TUI")
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def delete_crystal(self, params: dict[str, object]) -> dict[str, object]:
        if params.get("confirmed") is not True:
            raise ValueError("delete requires confirmation")
        crystal_id = self._int(params, "id")
        result = self.store.delete_crystal(crystal_id, evidence="Deleted from Ink admin TUI")
        return self._action_payload(result, view="Crystals", selected_id=crystal_id)

    def approve_proposal(self, params: dict[str, object]) -> dict[str, object]:
        proposal_id = self._int(params, "id")
        term_id = self.store.approve_proposal(proposal_id)
        result = ActionResult("strict_term", term_id, "approve", "Proposal approved")
        return self._action_payload(result, view="Proposals", selected_id=proposal_id)

    def reject_proposal(self, params: dict[str, object]) -> dict[str, object]:
        proposal_id = self._int(params, "id")
        result = self.store.reject_proposal(proposal_id, evidence="Rejected from Ink admin TUI")
        return self._action_payload(result, view="Proposals", selected_id=proposal_id)

    def provenance(self, params: dict[str, object]) -> dict[str, object]:
        return {"detail": dataclass_to_json(self.store.provenance_for_crystal(self._int(params, "crystal_id")))}

    def recall_reasons(self, params: dict[str, object]) -> dict[str, object]:
        return {"detail": self.store.recall_reasons_for_crystal(self._int(params, "crystal_id"))}

    def run_manual_dreaming(self, params: dict[str, object]) -> dict[str, object]:
        run = self.store.run_manual_dreaming()
        return {
            "result": {"run_id": run.id},
            "stats": self.store.stats().as_dict(),
            "snapshot": self._snapshot_payload("Dream Runs", run.id, None),
        }

    def dream_review(self, params: dict[str, object]) -> dict[str, object]:
        return {"detail": dataclass_to_json(self.store.dream_review(self._int(params, "run_id")))}

    def _snapshot_payload(
        self,
        view: str,
        selected_id: object,
        raw_filters: object,
    ) -> dict[str, object]:
        snapshot = self.store.snapshot(view, selected_id=selected_id)
        filtered = self._apply_filters(snapshot, raw_filters)
        return dataclass_to_json(filtered)

    def _apply_filters(self, snapshot: AdminSnapshot, raw_filters: object) -> AdminSnapshot:
        if type(raw_filters) is not dict or not raw_filters:
            return snapshot
        rows = snapshot.rows
        labels: list[str] = []
        status = raw_filters.get("status")
        if type(status) is str and status:
            rows = [row for row in rows if row.status == status]
            labels.append(f"status={status}")
        kind = raw_filters.get("kind")
        if type(kind) is str and kind:
            rows = [row for row in rows if row.kind == kind]
            labels.append(f"kind={kind}")
        selected = snapshot.selected if snapshot.selected in rows else (rows[0] if rows else None)
        detail = self.store.snapshot(snapshot.view, selected_id=selected.id if selected else None).detail
        return replace(snapshot, rows=rows, selected=selected, detail=detail, filters=labels)

    def _action_payload(
        self,
        result: ActionResult,
        *,
        view: str,
        selected_id: int | str,
    ) -> dict[str, object]:
        return {
            "result": dataclass_to_json(result),
            "stats": self.store.stats().as_dict(),
            "snapshot": self._snapshot_payload(view, selected_id, None),
        }

    def _view(self, raw: object, *, default: str) -> str:
        if raw is None:
            return default
        if type(raw) is str and raw in ADMIN_VIEWS:
            return raw
        raise ValueError(f"unknown admin view: {raw}")

    def _int(self, params: dict[str, object], key: str) -> int:
        value = params.get(key)
        if type(value) is int:
            return value
        if type(value) is str and value.isdecimal():
            return int(value)
        raise ValueError(f"{key} must be an integer")

    def _str(self, params: dict[str, object], key: str) -> str:
        value = params.get(key)
        if type(value) is str:
            return value
        raise ValueError(f"{key} must be a string")

    def _tags(self, raw: object) -> tuple[str, ...]:
        if raw is None:
            return ()
        if type(raw) is str:
            return tuple(tag.strip() for tag in raw.split(",") if tag.strip())
        if type(raw) is list:
            return tuple(str(tag).strip() for tag in raw if str(tag).strip())
        raise ValueError("tags must be a string or list")
```

- [ ] **Step 4: Run admin bridge tests**

Run:

```bash
uv run pytest tests/test_tui_bridge_admin.py -v
```

Expected: PASS all tests in `tests/test_tui_bridge_admin.py`.

- [ ] **Step 5: Commit**

```bash
git add src/hieronymus/tui_bridge/admin_api.py tests/test_tui_bridge_admin.py
git commit -m "feat: add admin TUI bridge methods"
```

---

### Task 4: Stdio Bridge Server and CLI Launch Selection

**Files:**
- Create: `src/hieronymus/tui_bridge/server.py`
- Modify: `src/hieronymus/cli.py`
- Create: `tests/test_cli_ink_tui.py`
- Modify: `tests/test_cli.py`

- [ ] **Step 1: Write failing server and CLI tests**

Add `tests/test_cli_ink_tui.py`:

```python
import json

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.tui_bridge.server import dispatch


def test_bridge_dispatch_config_bootstrap(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": "1", "method": "config.bootstrap", "params": {}},
    )

    assert response["id"] == "1"
    assert response["ok"] is True
    assert response["result"]["selected_provider"] == "openai"


def test_bridge_dispatch_unknown_method_returns_error(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    response = dispatch(
        config,
        {"id": "1", "method": "missing.method", "params": {}},
    )

    assert response == {
        "id": "1",
        "ok": False,
        "error": {"code": "method_not_found", "message": "unknown method: missing.method"},
    }


def test_cli_bridge_command_reads_one_request(tmp_path, monkeypatch) -> None:
    runner = CliRunner()
    request = json.dumps({"id": "1", "method": "config.bootstrap", "params": {}})

    result = runner.invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "tui-bridge"],
        input=request + "\n",
    )

    assert result.exit_code == 0
    response = json.loads(result.output)
    assert response["id"] == "1"
    assert response["ok"] is True


def test_cli_ink_config_launches_frontend_when_requested(tmp_path, monkeypatch) -> None:
    calls = []

    def fake_run(command, check):
        calls.append(command)

    monkeypatch.setenv("HIERONYMUS_TUI", "ink")
    monkeypatch.setattr("hieronymus.cli.subprocess.run", fake_run)
    monkeypatch.setattr("hieronymus.cli._frontend_entrypoint", lambda: "/tmp/hiero-ink.js")

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 0
    assert calls[0][0] == "node"
    assert calls[0][1] == "/tmp/hiero-ink.js"
    assert calls[0][2:] == ["config", "--bridge-command", "hiero"]


def test_cli_json_config_bypasses_ink_launcher(tmp_path, monkeypatch) -> None:
    def fail_run(*args, **kwargs):
        raise AssertionError("JSON output must not launch frontend")

    monkeypatch.setenv("HIERONYMUS_TUI", "ink")
    monkeypatch.setattr("hieronymus.cli.subprocess.run", fail_run)

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config", "--json"],
    )

    assert result.exit_code == 0
    assert '"settings_path"' in result.output
```

- [ ] **Step 2: Run CLI tests to verify failure**

Run:

```bash
uv run pytest tests/test_cli_ink_tui.py -v
```

Expected: FAIL because `tui-bridge`, `dispatch`, and Ink launch helpers do not exist.

- [ ] **Step 3: Add stdio bridge server**

Create `src/hieronymus/tui_bridge/server.py`:

```python
from __future__ import annotations

import json
import sys
from collections.abc import Callable

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.config_api import ConfigBridge
from hieronymus.tui_bridge.errors import error_payload
from hieronymus.tui_bridge.protocol import (
    RpcError,
    error_response,
    parse_request,
    success_response,
)

Handler = Callable[[dict[str, object]], dict[str, object]]


def dispatch(config: HieronymusConfig, raw: dict[str, object]) -> dict[str, object]:
    request_id = raw.get("id") if type(raw.get("id")) is str else None
    settings = None
    try:
        request = parse_request(json.dumps(raw))
        handler = _handlers(config).get(request.method)
        if handler is None:
            raise RpcError("method_not_found", f"unknown method: {request.method}")
        return success_response(request.id, handler(request.params))
    except Exception as error:
        try:
            settings = load_settings(config)
        except Exception:
            settings = None
        payload = error_payload(error, settings=settings)
        return error_response(
            request_id,
            RpcError(payload["code"], payload["message"]),
        )


def run_stdio(config: HieronymusConfig) -> None:
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            raw = json.loads(line)
            if type(raw) is not dict:
                raise RpcError("invalid_request", "request must be an object")
            response = dispatch(config, raw)
        except Exception as error:
            payload = error_payload(error)
            response = error_response(None, RpcError(payload["code"], payload["message"]))
        sys.stdout.write(json.dumps(response, sort_keys=True) + "\n")
        sys.stdout.flush()


def _handlers(config: HieronymusConfig) -> dict[str, Handler]:
    admin = AdminBridge(config)
    config_bridge = ConfigBridge(config)
    return {
        "admin.bootstrap": admin.bootstrap,
        "admin.snapshot": admin.snapshot,
        "admin.filter": admin.snapshot,
        "admin.add_crystal": admin.add_crystal,
        "admin.edit_crystal": admin.edit_crystal,
        "admin.merge_crystals": admin.merge_crystals,
        "admin.split_crystal": admin.split_crystal,
        "admin.supersede_crystal": admin.supersede_crystal,
        "admin.reinforce_crystal": admin.reinforce_crystal,
        "admin.decay_crystal": admin.decay_crystal,
        "admin.deprecate_crystal": admin.deprecate_crystal,
        "admin.delete_crystal": admin.delete_crystal,
        "admin.approve_proposal": admin.approve_proposal,
        "admin.reject_proposal": admin.reject_proposal,
        "admin.provenance": admin.provenance,
        "admin.recall_reasons": admin.recall_reasons,
        "admin.run_manual_dreaming": admin.run_manual_dreaming,
        "admin.dream_review": admin.dream_review,
        "config.bootstrap": config_bridge.bootstrap,
        "config.select_provider": config_bridge.select_provider,
        "config.update_draft": config_bridge.update_draft,
        "config.save": config_bridge.save,
        "config.reload": config_bridge.reload,
        "config.check_provider": config_bridge.check_provider,
        "config.model_suggestions": config_bridge.model_suggestions,
    }
```

- [ ] **Step 4: Add CLI command and Ink launch helper**

Modify `src/hieronymus/cli.py` imports:

```python
import os
import shutil
import sys
from pathlib import Path
```

Add helpers near `_subprocess_error_message`:

```python
def _tui_mode() -> str:
    value = os.environ.get("HIERONYMUS_TUI", "textual").strip().lower()
    if value not in {"textual", "ink"}:
        raise click.ClickException("HIERONYMUS_TUI must be textual or ink")
    return value


def _frontend_entrypoint() -> str:
    candidate = Path(__file__).resolve().parent / "frontend" / "dist" / "main.js"
    if candidate.exists():
        return str(candidate)
    return str(Path.cwd() / "frontend" / "dist" / "main.js")


def _launch_ink(mode: str) -> None:
    node = shutil.which("node") or "node"
    subprocess.run(
        [node, _frontend_entrypoint(), mode, "--bridge-command", "hiero"],
        check=True,
    )
```

Add a hidden bridge command:

```python
@main.command("tui-bridge", hidden=True)
@click.pass_context
def tui_bridge_command(ctx: click.Context) -> None:
    from hieronymus.tui_bridge.server import run_stdio

    run_stdio(ctx.obj["config"])
```

Change `config_command` non-JSON branch:

```python
    if not json_output:
        if _tui_mode() == "ink":
            _launch_ink("config")
            return
        HieronymusConfigApp(config).run()
        return
```

Change the `admin` command non-JSON branch in the same style:

```python
    if not json_output:
        if _tui_mode() == "ink":
            _launch_ink("admin")
            return
        HieronymusAdminApp(config).run()
        return
```

- [ ] **Step 5: Run CLI tests**

Run:

```bash
uv run pytest tests/test_cli_ink_tui.py tests/test_cli.py -v
```

Expected: PASS all selected tests.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/tui_bridge/server.py src/hieronymus/cli.py tests/test_cli_ink_tui.py tests/test_cli.py
git commit -m "feat: launch Ink TUI through bridge"
```

---

### Task 5: Frontend Project, RPC Client, and Runtime Schemas

**Files:**
- Create: `frontend/package.json`
- Create: `frontend/tsconfig.json`
- Create: `frontend/vitest.config.ts`
- Create: `frontend/src/rpc/client.ts`
- Create: `frontend/src/rpc/schema.ts`
- Create: `frontend/src/rpc/client.test.ts`
- Create: `frontend/src/rpc/schema.test.ts`

- [ ] **Step 1: Create frontend package files**

Create `frontend/package.json`:

```json
{
  "name": "hieronymus-ink-tui",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "tsc -p tsconfig.json",
    "test": "vitest run",
    "format": "prettier --check src",
    "typecheck": "tsc -p tsconfig.json --noEmit"
  },
  "dependencies": {
    "ink": "^5.0.1",
    "react": "^18.3.1",
    "zod": "^3.25.0"
  },
  "devDependencies": {
    "@types/node": "^22.0.0",
    "@types/react": "^18.3.0",
    "prettier": "^3.3.0",
    "typescript": "^5.5.0",
    "vitest": "^2.0.0"
  }
}
```

Create `frontend/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "rootDir": "src",
    "outDir": "dist",
    "types": ["node", "vitest"]
  },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

Create `frontend/vitest.config.ts`:

```ts
import {defineConfig} from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
  },
});
```

- [ ] **Step 2: Write failing RPC/schema tests**

Create `frontend/src/rpc/schema.test.ts`:

```ts
import {describe, expect, it} from 'vitest';
import {AdminSnapshotSchema, ConfigBootstrapSchema, RpcResponseSchema} from './schema.js';

describe('runtime schemas', () => {
  it('parses config bootstrap payload with one provider selector', () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        config_root: '/tmp/h',
        settings_path: '/tmp/h/settings.toml',
        database_path: '/tmp/h/hieronymus.sqlite3',
      },
      provider_choices: [
        {name: 'openai', display_name: 'OpenAI compatible', supports_api_path: true},
      ],
      selected_provider: 'openai',
      draft: {dreaming: {active_provider: 'openai'}, providers: {}},
      form_values: {provider: {model: 'gpt-4.1-mini'}, dreaming: {}},
      validation: {ok: true, errors: []},
      suggestions: {provider: 'openai', models: ['gpt-4.1-mini'], source: 'defaults', error: ''},
      detail: {title: 'openai dreaming provider', fields: [], errors: []},
    });

    expect(payload.selected_provider).toBe('openai');
  });

  it('rejects config provider choices outside supported families', () => {
    expect(() =>
      ConfigBootstrapSchema.parse({
        config_paths: {},
        provider_choices: [{name: 'deterministic', display_name: 'Deterministic', supports_api_path: false}],
        selected_provider: 'deterministic',
        draft: {dreaming: {}, providers: {}},
        form_values: {provider: {}, dreaming: {}},
        validation: {ok: true, errors: []},
        suggestions: {provider: 'deterministic', models: [], source: 'defaults', error: ''},
        detail: {title: '', fields: [], errors: []},
      }),
    ).toThrow();
  });

  it('parses admin snapshots', () => {
    const snapshot = AdminSnapshotSchema.parse({
      view: 'Crystals',
      rows: [],
      selected: null,
      detail: {title: 'Empty', subtitle: '', body: '', fields: []},
      filters: [],
    });

    expect(snapshot.view).toBe('Crystals');
  });

  it('parses success and error envelopes', () => {
    expect(RpcResponseSchema.parse({id: '1', ok: true, result: {ready: true}}).ok).toBe(true);
    expect(
      RpcResponseSchema.parse({
        id: '1',
        ok: false,
        error: {code: 'validation_error', message: 'text must not be empty'},
      }).ok,
    ).toBe(false);
  });
});
```

Create `frontend/src/rpc/client.test.ts`:

```ts
import {EventEmitter} from 'node:events';
import {Readable, Writable} from 'node:stream';
import {describe, expect, it} from 'vitest';
import {JsonRpcClient} from './client.js';

class FakeProcess extends EventEmitter {
  stdin: Writable;
  stdout: Readable;
  stderr: Readable;
  writes: string[] = [];

  constructor() {
    super();
    this.stdin = new Writable({
      write: (chunk, _encoding, callback) => {
        this.writes.push(String(chunk));
        callback();
      },
    });
    this.stdout = new Readable({read() {}});
    this.stderr = new Readable({read() {}});
  }
}

describe('JsonRpcClient', () => {
  it('writes request envelopes and resolves matching response', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('config.bootstrap', {});
    expect(proc.writes[0]).toContain('"method":"config.bootstrap"');
    proc.stdout.push('{"id":"1","ok":true,"result":{"ready":true}}\n');

    await expect(pending).resolves.toEqual({ready: true});
  });

  it('rejects backend error envelopes', async () => {
    const proc = new FakeProcess();
    const client = new JsonRpcClient(proc as never);

    const pending = client.request('admin.edit_crystal', {});
    proc.stdout.push(
      '{"id":"1","ok":false,"error":{"code":"validation_error","message":"text must not be empty"}}\n',
    );

    await expect(pending).rejects.toThrow('text must not be empty');
  });
});
```

- [ ] **Step 3: Run frontend tests to verify failure**

Run:

```bash
pnpm --dir frontend install
pnpm --dir frontend test
```

Expected: install succeeds; tests fail because `schema.ts` and `client.ts` do not exist.

- [ ] **Step 4: Add runtime schemas**

Create `frontend/src/rpc/schema.ts`:

```ts
import {z} from 'zod';

export const ProviderNameSchema = z.enum(['openai', 'gemini', 'anthropic']);

export const RpcResponseSchema = z.discriminatedUnion('ok', [
  z.object({
    id: z.string(),
    ok: z.literal(true),
    result: z.record(z.unknown()),
  }),
  z.object({
    id: z.string().nullable(),
    ok: z.literal(false),
    error: z.object({
      code: z.string(),
      message: z.string(),
    }),
  }),
]);

export const AdminRowSchema = z.object({
  id: z.union([z.number(), z.string()]),
  kind: z.string(),
  label: z.string(),
  status: z.string(),
  scope: z.string(),
  language_pair: z.string(),
  quality_label: z.string().default(''),
  tags: z.array(z.string()).default([]),
});

export const AdminDetailSchema = z.object({
  title: z.string(),
  subtitle: z.string(),
  body: z.string(),
  fields: z.array(z.tuple([z.string(), z.string()])).default([]),
});

export const AdminSnapshotSchema = z.object({
  view: z.string(),
  rows: z.array(AdminRowSchema),
  selected: AdminRowSchema.nullable(),
  detail: AdminDetailSchema,
  filters: z.array(z.string()),
});

export const ConfigBootstrapSchema = z.object({
  config_paths: z.record(z.string()),
  provider_choices: z.array(
    z.object({
      name: ProviderNameSchema,
      display_name: z.string(),
      supports_api_path: z.boolean(),
    }),
  ),
  selected_provider: ProviderNameSchema,
  draft: z.object({
    dreaming: z.record(z.unknown()),
    providers: z.record(z.record(z.unknown())),
  }),
  form_values: z.object({
    provider: z.record(z.string()),
    dreaming: z.record(z.string()),
  }),
  validation: z.object({
    ok: z.boolean(),
    errors: z.array(z.string()),
  }),
  suggestions: z.object({
    provider: ProviderNameSchema,
    models: z.array(z.string()),
    source: z.string(),
    error: z.string(),
  }),
  detail: z.object({
    title: z.string(),
    fields: z.array(z.tuple([z.string(), z.string()])),
    errors: z.array(z.string()),
  }),
});

export type ProviderName = z.infer<typeof ProviderNameSchema>;
export type AdminRow = z.infer<typeof AdminRowSchema>;
export type AdminSnapshot = z.infer<typeof AdminSnapshotSchema>;
export type ConfigBootstrap = z.infer<typeof ConfigBootstrapSchema>;
```

- [ ] **Step 5: Add JSON-RPC client**

Create `frontend/src/rpc/client.ts`:

```ts
import {spawn, type ChildProcessWithoutNullStreams} from 'node:child_process';
import {createInterface} from 'node:readline';
import {RpcResponseSchema} from './schema.js';

type Pending = {
  resolve: (value: Record<string, unknown>) => void;
  reject: (error: Error) => void;
};

export class JsonRpcClient {
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();

  constructor(private readonly proc: ChildProcessWithoutNullStreams) {
    const lines = createInterface({input: proc.stdout});
    lines.on('line', (line) => this.receive(line));
    proc.stderr.on('data', (chunk) => {
      process.stderr.write(chunk);
    });
  }

  request(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>> {
    const id = String(this.nextId++);
    const payload = JSON.stringify({id, method, params});
    this.proc.stdin.write(`${payload}\n`);
    return new Promise((resolve, reject) => {
      this.pending.set(id, {resolve, reject});
    });
  }

  close(): void {
    this.proc.stdin.end();
  }

  private receive(line: string): void {
    const response = RpcResponseSchema.parse(JSON.parse(line));
    if (response.id === null) {
      throw new Error(response.error.message);
    }
    const pending = this.pending.get(response.id);
    if (!pending) {
      return;
    }
    this.pending.delete(response.id);
    if (response.ok) {
      pending.resolve(response.result);
    } else {
      pending.reject(new Error(response.error.message));
    }
  }
}

export function createBridgeClient(command: string): JsonRpcClient {
  const proc = spawn(command, ['tui-bridge'], {
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  return new JsonRpcClient(proc);
}
```

- [ ] **Step 6: Run frontend tests and build**

Run:

```bash
pnpm --dir frontend test
pnpm --dir frontend build
```

Expected: PASS Vitest tests and TypeScript build.

- [ ] **Step 7: Commit**

```bash
git add frontend/package.json frontend/tsconfig.json frontend/vitest.config.ts frontend/src/rpc/client.ts frontend/src/rpc/schema.ts frontend/src/rpc/client.test.ts frontend/src/rpc/schema.test.ts pnpm-lock.yaml
git commit -m "feat: add Ink frontend RPC foundation"
```

---

### Task 6: Ink Config Screen

**Files:**
- Create: `frontend/src/app/App.tsx`
- Create: `frontend/src/app/routes.ts`
- Create: `frontend/src/main.tsx`
- Create: `frontend/src/config/ConfigScreen.tsx`
- Create: `frontend/src/config/ProviderSelector.tsx`
- Create: `frontend/src/config/ConfigForm.tsx`
- Create: `frontend/src/ui/StatusLine.tsx`
- Create: `frontend/src/ui/KeyHelp.tsx`
- Create: `frontend/src/config/ConfigScreen.test.tsx`

- [ ] **Step 1: Write config screen tests**

Create `frontend/src/config/ConfigScreen.test.tsx`:

```tsx
import React from 'react';
import {describe, expect, it} from 'vitest';
import {render} from 'ink-testing-library';
import {ConfigScreen} from './ConfigScreen.js';

function payload() {
  return {
    config_paths: {settings_path: '/tmp/settings.toml'},
    provider_choices: [
      {name: 'openai' as const, display_name: 'OpenAI compatible', supports_api_path: true},
      {name: 'gemini' as const, display_name: 'Gemini', supports_api_path: false},
      {name: 'anthropic' as const, display_name: 'Anthropic', supports_api_path: false},
    ],
    selected_provider: 'openai' as const,
    draft: {dreaming: {active_provider: 'openai'}, providers: {}},
    form_values: {
      provider: {
        model: 'gpt-4.1-mini',
        api_key_env: 'OPENAI_API_KEY',
        api_path: 'https://api.openai.com/v1',
        timeout_seconds: '30',
      },
      dreaming: {
        autostart_enabled: 'no',
        min_interval_minutes: '30',
        new_short_term_memory_threshold: '25',
        max_cycles_per_autostart: '1',
      },
    },
    validation: {ok: true, errors: []},
    suggestions: {provider: 'openai' as const, models: ['gpt-4.1-mini'], source: 'defaults', error: ''},
    detail: {title: 'openai dreaming provider', fields: [['api_key_env', 'OPENAI_API_KEY']] as [string, string][], errors: []},
  };
}

describe('ConfigScreen', () => {
  it('renders one provider family selector instead of provider rows', () => {
    const app = render(<ConfigScreen initial={payload()} client={undefined} />);

    expect(app.lastFrame()).toContain('OpenAI compatible');
    expect(app.lastFrame()).toContain('Gemini');
    expect(app.lastFrame()).toContain('Anthropic');
    expect(app.lastFrame()).not.toContain('Deterministic');
  });

  it('renders model suggestions when present', () => {
    const app = render(<ConfigScreen initial={payload()} client={undefined} />);

    expect(app.lastFrame()).toContain('gpt-4.1-mini');
  });
});
```

Add `ink-testing-library` to `frontend/package.json` dev dependencies:

```json
"ink-testing-library": "^3.0.0"
```

- [ ] **Step 2: Run config screen tests to verify failure**

Run:

```bash
pnpm --dir frontend test -- ConfigScreen
```

Expected: FAIL because config components do not exist.

- [ ] **Step 3: Add route and app shell**

Create `frontend/src/app/routes.ts`:

```ts
export type AppMode = 'admin' | 'config';
```

Create `frontend/src/app/App.tsx`:

```tsx
import React, {useEffect, useState} from 'react';
import {Text} from 'ink';
import {ConfigBootstrapSchema, type ConfigBootstrap} from '../rpc/schema.js';
import {ConfigScreen} from '../config/ConfigScreen.js';
import type {JsonRpcClient} from '../rpc/client.js';
import type {AppMode} from './routes.js';

type Props = {
  mode: AppMode;
  client: JsonRpcClient;
};

export function App({mode, client}: Props) {
  const [configInitial, setConfigInitial] = useState<ConfigBootstrap | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    if (mode === 'config') {
      client
        .request('config.bootstrap', {})
        .then((payload) => setConfigInitial(ConfigBootstrapSchema.parse(payload)))
        .catch((err: Error) => setError(err.message));
    }
  }, [client, mode]);

  if (error) {
    return <Text color="red">{error}</Text>;
  }
  if (mode === 'config' && configInitial) {
    return <ConfigScreen initial={configInitial} client={client} />;
  }
  return <Text>Loading {mode}...</Text>;
}
```

Create `frontend/src/main.tsx`:

```tsx
#!/usr/bin/env node
import React from 'react';
import {render} from 'ink';
import {App} from './app/App.js';
import {createBridgeClient} from './rpc/client.js';
import type {AppMode} from './app/routes.js';

const [modeArg, bridgeFlag, bridgeCommand] = process.argv.slice(2);
const mode = modeArg === 'admin' ? 'admin' : 'config';
if (bridgeFlag !== '--bridge-command' || !bridgeCommand) {
  throw new Error('Usage: main.js <admin|config> --bridge-command <command>');
}

const client = createBridgeClient(bridgeCommand);
render(<App mode={mode as AppMode} client={client} />);
```

- [ ] **Step 4: Add config components**

Create `frontend/src/config/ProviderSelector.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';
import type {ConfigBootstrap, ProviderName} from '../rpc/schema.js';

type Props = {
  choices: ConfigBootstrap['provider_choices'];
  selected: ProviderName;
};

export function ProviderSelector({choices, selected}: Props) {
  return (
    <Box flexDirection="column" width={24}>
      {choices.map((choice) => (
        <Text key={choice.name} color={choice.name === selected ? 'cyan' : undefined}>
          {choice.name === selected ? '>' : ' '} {choice.display_name}
        </Text>
      ))}
    </Box>
  );
}
```

Create `frontend/src/config/ConfigForm.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';
import type {ConfigBootstrap} from '../rpc/schema.js';

type Props = {
  payload: ConfigBootstrap;
};

export function ConfigForm({payload}: Props) {
  const provider = payload.form_values.provider;
  const dreaming = payload.form_values.dreaming;
  return (
    <Box flexDirection="column">
      <Text bold>Provider</Text>
      <Text>model: {provider.model || '-'}</Text>
      <Text>api_key_env: {provider.api_key_env || '-'}</Text>
      <Text>api_path: {provider.api_path || '-'}</Text>
      <Text>timeout_seconds: {provider.timeout_seconds || '-'}</Text>
      <Text bold>Dreaming</Text>
      <Text>autostart_enabled: {dreaming.autostart_enabled || 'no'}</Text>
      <Text>min_interval_minutes: {dreaming.min_interval_minutes || '-'}</Text>
      <Text>new_short_term_memory_threshold: {dreaming.new_short_term_memory_threshold || '-'}</Text>
      <Text>max_cycles_per_autostart: {dreaming.max_cycles_per_autostart || '-'}</Text>
    </Box>
  );
}
```

Create `frontend/src/ui/StatusLine.tsx`:

```tsx
import React from 'react';
import {Text} from 'ink';

type Props = {
  message: string;
  error?: boolean;
};

export function StatusLine({message, error = false}: Props) {
  return <Text color={error ? 'red' : 'green'}>{message}</Text>;
}
```

Create `frontend/src/ui/KeyHelp.tsx`:

```tsx
import React from 'react';
import {Text} from 'ink';

export function KeyHelp({keys}: {keys: string[]}) {
  return <Text dimColor>{keys.join('  ')}</Text>;
}
```

Create `frontend/src/config/ConfigScreen.tsx`:

```tsx
import React, {useState} from 'react';
import {Box, Text, useInput} from 'ink';
import {ConfigBootstrapSchema, type ConfigBootstrap, type ProviderName} from '../rpc/schema.js';
import type {JsonRpcClient} from '../rpc/client.js';
import {ConfigForm} from './ConfigForm.js';
import {ProviderSelector} from './ProviderSelector.js';
import {KeyHelp} from '../ui/KeyHelp.js';
import {StatusLine} from '../ui/StatusLine.js';

type Props = {
  initial: ConfigBootstrap;
  client: JsonRpcClient | undefined;
};

const providerKeys: ProviderName[] = ['openai', 'gemini', 'anthropic'];

export function ConfigScreen({initial, client}: Props) {
  const [payload, setPayload] = useState(initial);
  const [status, setStatus] = useState('ready');

  useInput((input) => {
    if (input === '1' || input === '2' || input === '3') {
      void selectProvider(providerKeys[Number(input) - 1]);
    }
    if (input === 's') {
      void save();
    }
    if (input === 'r') {
      void reload();
    }
    if (input === 'c') {
      void check();
    }
  });

  async function selectProvider(provider: ProviderName) {
    if (!client) return;
    const next = await client.request('config.select_provider', {
      provider,
      draft: payload.draft,
    });
    setPayload(ConfigBootstrapSchema.parse(next));
    setStatus(`selected ${provider}`);
  }

  async function save() {
    if (!client) return;
    const next = await client.request('config.save', {draft: payload.draft});
    setPayload(ConfigBootstrapSchema.parse(next));
    setStatus('saved');
  }

  async function reload() {
    if (!client) return;
    const next = await client.request('config.reload', {
      selected_provider: payload.selected_provider,
    });
    setPayload(ConfigBootstrapSchema.parse(next));
    setStatus('reloaded');
  }

  async function check() {
    if (!client) return;
    const next = await client.request('config.check_provider', {
      selected_provider: payload.selected_provider,
      draft: payload.draft,
    });
    setPayload(ConfigBootstrapSchema.parse(next));
    setStatus('checked provider');
  }

  return (
    <Box flexDirection="column">
      <Text bold>Hieronymus Config</Text>
      <Box gap={2}>
        <ProviderSelector choices={payload.provider_choices} selected={payload.selected_provider} />
        <ConfigForm payload={payload} />
      </Box>
      <Text bold>Model suggestions</Text>
      <Text>{payload.suggestions.models.join(', ') || '-'}</Text>
      {payload.detail.errors.map((error) => (
        <Text key={error} color="red">
          {error}
        </Text>
      ))}
      <StatusLine message={status} />
      <KeyHelp keys={['1 openai', '2 gemini', '3 anthropic', 's save', 'r reload', 'c check', 'q quit']} />
    </Box>
  );
}
```

- [ ] **Step 5: Run config frontend tests and build**

Run:

```bash
pnpm --dir frontend test -- ConfigScreen
pnpm --dir frontend build
```

Expected: PASS selected tests and TypeScript build.

- [ ] **Step 6: Commit**

```bash
git add frontend/package.json frontend/src/app frontend/src/main.tsx frontend/src/config frontend/src/ui frontend/src/config/ConfigScreen.test.tsx
git commit -m "feat: add Ink config screen"
```

---

### Task 7: Ink Admin Screen, Table, Detail Pane, and Commands

**Files:**
- Modify: `frontend/src/app/App.tsx`
- Create: `frontend/src/admin/AdminScreen.tsx`
- Create: `frontend/src/admin/AdminTable.tsx`
- Create: `frontend/src/admin/DetailPane.tsx`
- Create: `frontend/src/admin/CommandPalette.tsx`
- Create: `frontend/src/admin/dialogs.tsx`
- Create: `frontend/src/ui/FocusableList.tsx`
- Create: `frontend/src/admin/AdminScreen.test.tsx`

- [ ] **Step 1: Write admin screen tests**

Create `frontend/src/admin/AdminScreen.test.tsx`:

```tsx
import React from 'react';
import {describe, expect, it} from 'vitest';
import {render} from 'ink-testing-library';
import {AdminScreen} from './AdminScreen.js';

function bootstrap() {
  return {
    views: ['Concepts', 'Renderings', 'Crystals', 'Lessons', 'Short-Term Sessions', 'Dream Runs', 'Proposals', 'Audit Log'],
    default_view: 'Crystals',
    stats: {series: 1, crystals: 1, lessons: 0, short_term_memories: 0, sessions: 0, dream_runs: 0, pending_proposals: 0, audit_events: 0},
    service: {running: false},
    snapshot: {
      view: 'Crystals',
      rows: [
        {id: 1, kind: 'concept', label: 'Guild Ledger', status: 'active', scope: 'only-sense-online', language_pair: 'ja -> ru', quality_label: '', tags: []},
      ],
      selected: {id: 1, kind: 'concept', label: 'Guild Ledger', status: 'active', scope: 'only-sense-online', language_pair: 'ja -> ru', quality_label: '', tags: []},
      detail: {title: 'Guild Ledger', subtitle: 'concept', body: 'Guild ledger detail marker.', fields: []},
      filters: [],
    },
  };
}

describe('AdminScreen', () => {
  it('renders views, stats, table row, and detail', () => {
    const app = render(<AdminScreen initial={bootstrap()} client={undefined} />);

    expect(app.lastFrame()).toContain('Crystals');
    expect(app.lastFrame()).toContain('series 1');
    expect(app.lastFrame()).toContain('Guild Ledger');
    expect(app.lastFrame()).toContain('Guild ledger detail marker.');
  });

  it('shows crystal commands for crystal view', () => {
    const app = render(<AdminScreen initial={bootstrap()} client={undefined} showCommands />);

    expect(app.lastFrame()).toContain('reinforce');
    expect(app.lastFrame()).toContain('delete');
    expect(app.lastFrame()).not.toContain('approve');
  });
});
```

- [ ] **Step 2: Run admin frontend tests to verify failure**

Run:

```bash
pnpm --dir frontend test -- AdminScreen
```

Expected: FAIL because admin components do not exist.

- [ ] **Step 3: Add admin components**

Create `frontend/src/ui/FocusableList.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';

type Props<T> = {
  items: T[];
  selectedIndex: number;
  label: (item: T) => string;
};

export function FocusableList<T>({items, selectedIndex, label}: Props<T>) {
  return (
    <Box flexDirection="column">
      {items.map((item, index) => (
        <Text key={`${index}-${label(item)}`} color={index === selectedIndex ? 'cyan' : undefined}>
          {index === selectedIndex ? '>' : ' '} {label(item)}
        </Text>
      ))}
    </Box>
  );
}
```

Create `frontend/src/admin/AdminTable.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';
import type {AdminRow} from '../rpc/schema.js';

export function AdminTable({rows, selectedId}: {rows: AdminRow[]; selectedId: string | number | null}) {
  return (
    <Box flexDirection="column" width={48}>
      {rows.map((row) => (
        <Text key={String(row.id)} color={row.id === selectedId ? 'cyan' : undefined}>
          {row.id === selectedId ? '>' : ' '} {row.label} [{row.status}] {row.quality_label}
        </Text>
      ))}
    </Box>
  );
}
```

Create `frontend/src/admin/DetailPane.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';
import type {AdminSnapshot} from '../rpc/schema.js';

export function DetailPane({detail}: {detail: AdminSnapshot['detail']}) {
  return (
    <Box flexDirection="column" width={60}>
      <Text bold>{detail.title}</Text>
      <Text dimColor>{detail.subtitle}</Text>
      <Text>{detail.body}</Text>
      {detail.fields.map(([name, value]) => (
        <Text key={name}>
          {name}: {value}
        </Text>
      ))}
    </Box>
  );
}
```

Create `frontend/src/admin/CommandPalette.tsx`:

```tsx
import React from 'react';
import {Box, Text} from 'ink';

const COMMANDS: Record<string, string[]> = {
  Crystals: ['add', 'edit', 'delete', 'merge', 'split', 'deprecate', 'supersede', 'reinforce', 'decay', 'inspect provenance', 'inspect recall reason'],
  Lessons: ['add', 'edit', 'delete', 'merge', 'split', 'deprecate', 'supersede', 'reinforce', 'decay', 'promote local lesson', 'activate global lesson', 'inspect provenance', 'inspect recall reason'],
  'Dream Runs': ['run manual dreaming', 'review dream outputs'],
  Proposals: ['approve', 'reject'],
};

export function commandsForView(view: string): string[] {
  return COMMANDS[view] ?? [];
}

export function CommandPalette({view}: {view: string}) {
  return (
    <Box flexDirection="column">
      <Text bold>Commands</Text>
      {commandsForView(view).map((command) => (
        <Text key={command}>{command}</Text>
      ))}
    </Box>
  );
}
```

Create `frontend/src/admin/dialogs.tsx`:

```tsx
export type DialogKind =
  | 'add'
  | 'edit'
  | 'filter'
  | 'delete'
  | 'merge'
  | 'split'
  | 'supersede'
  | 'none';

export type DialogState = {
  kind: DialogKind;
  error: string;
};

export const closedDialog: DialogState = {kind: 'none', error: ''};
```

Create `frontend/src/admin/AdminScreen.tsx`:

```tsx
import React, {useState} from 'react';
import {Box, Text, useInput} from 'ink';
import {AdminSnapshotSchema, type AdminSnapshot} from '../rpc/schema.js';
import type {JsonRpcClient} from '../rpc/client.js';
import {AdminTable} from './AdminTable.js';
import {CommandPalette} from './CommandPalette.js';
import {DetailPane} from './DetailPane.js';
import {KeyHelp} from '../ui/KeyHelp.js';
import {StatusLine} from '../ui/StatusLine.js';

type Bootstrap = {
  views: string[];
  default_view: string;
  stats: Record<string, number>;
  service: Record<string, unknown>;
  snapshot: AdminSnapshot;
};

type Props = {
  initial: Bootstrap;
  client: JsonRpcClient | undefined;
  showCommands?: boolean;
};

export function AdminScreen({initial, client, showCommands = false}: Props) {
  const [snapshot, setSnapshot] = useState(initial.snapshot);
  const [stats, setStats] = useState(initial.stats);
  const [commandsOpen, setCommandsOpen] = useState(showCommands);
  const [status, setStatus] = useState('ready');

  useInput((input, key) => {
    if (/^[1-8]$/.test(input)) {
      const view = initial.views[Number(input) - 1];
      if (view) void switchView(view);
    }
    if (input === 'p' && key.ctrl) {
      setCommandsOpen((value) => !value);
    }
    if (input === 'd') {
      void mutate('admin.delete_crystal', {id: snapshot.selected?.id, confirmed: true});
    }
    if (input === '+') {
      void mutate('admin.reinforce_crystal', {id: snapshot.selected?.id});
    }
    if (input === '-') {
      void mutate('admin.decay_crystal', {id: snapshot.selected?.id});
    }
  });

  async function switchView(view: string) {
    if (!client) return;
    const response = await client.request('admin.snapshot', {view});
    const next = AdminSnapshotSchema.parse(response.snapshot);
    setSnapshot(next);
    setStats(response.stats as Record<string, number>);
    setStatus(`view ${view}`);
  }

  async function mutate(method: string, params: Record<string, unknown>) {
    if (!client || params.id === undefined || params.id === null) return;
    const response = await client.request(method, params);
    const next = AdminSnapshotSchema.parse(response.snapshot);
    setSnapshot(next);
    setStats(response.stats as Record<string, number>);
    setStatus(String((response.result as Record<string, unknown>).message ?? 'updated'));
  }

  return (
    <Box flexDirection="column">
      <Text bold>Hieronymus Admin</Text>
      <Text>{initial.views.join(' | ')}</Text>
      <Text>
        series {stats.series ?? 0} crystals {stats.crystals ?? 0} lessons {stats.lessons ?? 0} proposals {stats.pending_proposals ?? 0}
      </Text>
      <Box gap={2}>
        <AdminTable rows={snapshot.rows} selectedId={snapshot.selected?.id ?? null} />
        <DetailPane detail={snapshot.detail} />
      </Box>
      {commandsOpen ? <CommandPalette view={snapshot.view} /> : null}
      <StatusLine message={status} />
      <KeyHelp keys={['1-8 views', 'f filter', 'e edit', '+ reinforce', '- decay', 'd delete', 'ctrl+p commands', 'q quit']} />
    </Box>
  );
}
```

- [ ] **Step 4: Wire admin route in app shell**

Modify `frontend/src/app/App.tsx`:

```tsx
import React, {useEffect, useState} from 'react';
import {Text} from 'ink';
import {AdminSnapshotSchema, ConfigBootstrapSchema, type ConfigBootstrap} from '../rpc/schema.js';
import {AdminScreen} from '../admin/AdminScreen.js';
import {ConfigScreen} from '../config/ConfigScreen.js';
import type {JsonRpcClient} from '../rpc/client.js';
import type {AppMode} from './routes.js';

type AdminBootstrap = {
  views: string[];
  default_view: string;
  stats: Record<string, number>;
  service: Record<string, unknown>;
  snapshot: unknown;
};

type Props = {
  mode: AppMode;
  client: JsonRpcClient;
};

export function App({mode, client}: Props) {
  const [configInitial, setConfigInitial] = useState<ConfigBootstrap | null>(null);
  const [adminInitial, setAdminInitial] = useState<AdminBootstrap | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    if (mode === 'config') {
      client
        .request('config.bootstrap', {})
        .then((payload) => setConfigInitial(ConfigBootstrapSchema.parse(payload)))
        .catch((err: Error) => setError(err.message));
    }
    if (mode === 'admin') {
      client
        .request('admin.bootstrap', {})
        .then((payload) => {
          const snapshot = AdminSnapshotSchema.parse(payload.snapshot);
          setAdminInitial({...payload, snapshot} as AdminBootstrap);
        })
        .catch((err: Error) => setError(err.message));
    }
  }, [client, mode]);

  if (error) {
    return <Text color="red">{error}</Text>;
  }
  if (mode === 'config' && configInitial) {
    return <ConfigScreen initial={configInitial} client={client} />;
  }
  if (mode === 'admin' && adminInitial) {
    return <AdminScreen initial={adminInitial as never} client={client} />;
  }
  return <Text>Loading {mode}...</Text>;
}
```

- [ ] **Step 5: Run admin frontend tests and build**

Run:

```bash
pnpm --dir frontend test -- AdminScreen
pnpm --dir frontend build
```

Expected: PASS selected tests and TypeScript build.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/app/App.tsx frontend/src/admin frontend/src/ui/FocusableList.tsx
git commit -m "feat: add Ink admin screen"
```

---

### Task 8: Doctor, Packaging, and Documentation

**Files:**
- Modify: `src/hieronymus/doctor.py`
- Modify: `pyproject.toml`
- Modify: `docs/usage.md`
- Modify: `README.md`
- Create: `tests/test_doctor_ink_runtime.py`

- [ ] **Step 1: Write failing doctor runtime test**

Add `tests/test_doctor_ink_runtime.py`:

```python
from hieronymus.doctor import Doctor


def test_doctor_reports_node_and_pnpm_runtime(config, monkeypatch) -> None:
    monkeypatch.setattr("hieronymus.doctor.shutil.which", lambda name: f"/usr/bin/{name}")

    payload = Doctor(config).report().to_json_dict()
    check_names = [check["name"] for check in payload["checks"]]

    assert "node runtime" in check_names
    assert "pnpm frontend package manager" in check_names
```

- [ ] **Step 2: Run doctor test to verify failure**

Run:

```bash
uv run pytest tests/test_doctor_ink_runtime.py -v
```

Expected: FAIL because doctor does not report Node or pnpm.

- [ ] **Step 3: Add runtime checks**

Modify `src/hieronymus/doctor.py`:

```python
import shutil
```

Add checks in `Doctor.report()` next to existing environment checks:

```python
        checks.append(
            self._check(
                "node runtime",
                shutil.which("node") is not None,
                "Node.js is available for the Ink TUI",
                "Node.js was not found; set HIERONYMUS_TUI=textual or install Node.js",
            )
        )
        checks.append(
            self._check(
                "pnpm frontend package manager",
                shutil.which("pnpm") is not None,
                "pnpm is available for frontend development",
                "pnpm was not found; frontend development commands will not run",
            )
        )
```

- [ ] **Step 4: Include built frontend artifact in package**

Modify `pyproject.toml`:

```toml
[tool.hatch.build.targets.wheel]
packages = ["src/hieronymus"]
artifacts = ["frontend/dist/main.js"]
```

If Hatch rejects `artifacts` for this project layout, use this package-data form instead:

```toml
[tool.hatch.build.targets.wheel.force-include]
"frontend/dist/main.js" = "hieronymus/frontend/dist/main.js"
```

- [ ] **Step 5: Update docs**

Add to `docs/usage.md`:

```markdown
### Ink TUI Preview

The interactive admin and config interfaces can run through the Ink/React frontend:

```bash
HIERONYMUS_TUI=ink hiero config
HIERONYMUS_TUI=ink hiero admin
```

The Textual implementation remains available during the migration:

```bash
HIERONYMUS_TUI=textual hiero config
HIERONYMUS_TUI=textual hiero admin
```

The Ink config screen edits one remote dreaming provider family at a time:
OpenAI compatible, Gemini, or Anthropic. The selected family becomes
`dreaming.active_provider`; the other remote families are disabled. API keys are
still read only from environment variables and are not displayed or persisted.
OpenAI-compatible providers expose `api_path`; Gemini and Anthropic use their
default API paths. Model suggestions are fetched from the provider API when the
provider supports listing models and the configured environment variable is
present; otherwise Hieronymus shows built-in suggestions.
```

Add to `README.md` development section:

```markdown
Frontend TUI development uses pnpm:

```bash
pnpm --dir frontend install
pnpm --dir frontend test
pnpm --dir frontend build
```
```

- [ ] **Step 6: Run doctor/docs-adjacent tests and build**

Run:

```bash
uv run pytest tests/test_doctor_ink_runtime.py tests/test_doctor.py -v
pnpm --dir frontend build
```

Expected: PASS selected Python tests and frontend build.

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/doctor.py pyproject.toml docs/usage.md README.md tests/test_doctor_ink_runtime.py
git commit -m "docs: document Ink TUI runtime"
```

---

### Task 9: End-to-End Verification and Default-Switch Gate

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `tests/test_cli_ink_tui.py`
- Modify: `docs/usage.md`

- [ ] **Step 1: Add default-stays-Textual gate test**

Append to `tests/test_cli_ink_tui.py`:

```python
def test_cli_defaults_to_textual_until_parity_switch(tmp_path, monkeypatch) -> None:
    launched = {"textual": False}

    class FakeConfigApp:
        def __init__(self, config):
            self.config = config

        def run(self):
            launched["textual"] = True

    monkeypatch.delenv("HIERONYMUS_TUI", raising=False)
    monkeypatch.setattr("hieronymus.cli.HieronymusConfigApp", FakeConfigApp)

    result = CliRunner().invoke(
        main,
        ["--data-root", str(tmp_path / "hieronymus"), "config"],
    )

    assert result.exit_code == 0
    assert launched["textual"] is True
```

- [ ] **Step 2: Run the gate test**

Run:

```bash
uv run pytest tests/test_cli_ink_tui.py -v
```

Expected: PASS. This locks the migration phase: Ink is opt-in until parity is accepted.

- [ ] **Step 3: Run full Python verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: PASS all commands.

- [ ] **Step 4: Run frontend verification**

Run:

```bash
pnpm --dir frontend test
pnpm --dir frontend build
```

Expected: PASS all frontend tests and build.

- [ ] **Step 5: Manual smoke tests**

Run:

```bash
HIERONYMUS_TUI=ink hiero config
```

Expected:
- Screen title is `Hieronymus Config`.
- Provider selector shows OpenAI compatible, Gemini, and Anthropic.
- Deterministic is not shown as an editable provider family.
- `s`, `r`, and `c` are visible in key help.
- Raw API key values do not appear.

Run:

```bash
HIERONYMUS_TUI=ink hiero admin
```

Expected:
- Screen title is `Hieronymus Admin`.
- Views line includes Concepts through Audit Log.
- Stats line renders counts.
- Crystals table and detail pane render without a traceback.
- Command help includes `ctrl+p commands`.

- [ ] **Step 6: Document default-switch criteria**

Add to `docs/usage.md`:

```markdown
### TUI Default Switch Criteria

Ink remains opt-in until the following pass in the same release candidate:

- `uv run pytest`
- `uv run ruff check .`
- `uv run ruff format --check .`
- `pnpm --dir frontend test`
- `pnpm --dir frontend build`
- Manual smoke tests for `HIERONYMUS_TUI=ink hiero config`
- Manual smoke tests for `HIERONYMUS_TUI=ink hiero admin`

After the default changes to Ink, Textual remains available with
`HIERONYMUS_TUI=textual` for one release cycle.
```

- [ ] **Step 7: Commit**

```bash
git add src/hieronymus/cli.py tests/test_cli_ink_tui.py docs/usage.md
git commit -m "test: gate Ink TUI default switch"
```

---

## Self-Review

**Spec coverage:** The plan covers the required JSON-RPC envelope, display-safe errors, config methods, admin methods, CLI feature flag, TypeScript structure, Python bridge structure, packaging/runtime checks, test strategy, and migration phases. The user’s updated config requirement is covered by Task 2 and Task 6: the config UI selects one remote provider family instead of listing all dreaming providers, exposes API path/model fields, and fetches model suggestions when supported.

**Placeholder scan:** The plan avoids banned placeholder phrasing. Each code-changing step includes concrete code or exact replacement snippets and each verification step includes commands and expected outcomes.

**Type consistency:** Python method names match the bridge dispatch table. TypeScript schemas use `ProviderName = openai | gemini | anthropic`, matching the config bridge. Admin snapshot field names match `AdminSnapshot`, `AdminRow`, and `AdminDetail` dataclass fields after `dataclass_to_json`.

**Residual risks:** Task 6 and Task 7 provide a functional first Ink surface but do not yet make every dialog fully ergonomic. The bridge methods are complete enough for frontend wiring; later polishing should deepen Ink dialog tests for add, merge, split, supersede, filtering, provenance, recall reasons, and dream review before switching Ink to default.

from __future__ import annotations

import math
import tomllib
from dataclasses import dataclass, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig


class SettingsError(ValueError):
    """Raised when persisted Hieronymus settings cannot be used."""


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
    base_url: str | None = None
    timeout_seconds: float = 30.0

    def to_json_dict(self) -> dict[str, object]:
        payload: dict[str, object] = {
            "enabled": self.enabled,
            "model": self.model,
            "api_key_env": self.api_key_env,
            "timeout_seconds": self.timeout_seconds,
        }
        if self.base_url is not None:
            payload["base_url"] = self.base_url
        return payload


@dataclass(frozen=True)
class HieronymusSettings:
    dreaming: DreamingSettings
    providers: dict[str, ProviderSettings]

    def with_provider(
        self,
        name: str,
        provider: ProviderSettings,
    ) -> HieronymusSettings:
        return replace(self, providers={**self.providers, name: provider})

    def with_dreaming(self, dreaming: DreamingSettings) -> HieronymusSettings:
        return replace(self, dreaming=dreaming)

    def to_json_dict(self) -> dict[str, object]:
        return {
            "dreaming": self.dreaming.to_json_dict(),
            "providers": {
                name: provider.to_json_dict() for name, provider in self.providers.items()
            },
        }


def load_settings(config: HieronymusConfig) -> HieronymusSettings:
    if not config.settings_path.exists():
        return _validate_settings(default_settings())

    try:
        payload = tomllib.loads(config.settings_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise SettingsError(f"settings.toml is not valid TOML: {error}") from error

    return _validate_settings(_settings_from_payload(payload))


def save_settings(config: HieronymusConfig, settings: HieronymusSettings) -> None:
    settings = _validate_settings(settings)
    config.config_root.mkdir(parents=True, exist_ok=True)

    atomic_write_text(config.settings_path, tomli_w.dumps(settings.to_json_dict()))


def validate_settings(settings: HieronymusSettings) -> HieronymusSettings:
    return _validate_settings(settings)


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


def _settings_from_payload(payload: dict[str, Any]) -> HieronymusSettings:
    _validate_unknown_keys(
        payload,
        allowed=frozenset({"dreaming", "providers"}),
        prefix=None,
    )
    defaults = default_settings()

    dreaming_payload = _dict_payload(payload.get("dreaming"), "dreaming")
    _validate_unknown_keys(
        dreaming_payload,
        allowed=_field_names(DreamingSettings),
        prefix="dreaming",
    )
    _validate_dreaming_payload(dreaming_payload)
    dreaming = replace(defaults.dreaming, **dreaming_payload)

    providers = dict(defaults.providers)
    providers_payload = _dict_payload(payload.get("providers"), "providers")
    for name, raw_provider in providers_payload.items():
        provider_payload = _dict_payload(raw_provider, f"providers.{name}")
        _validate_unknown_keys(
            provider_payload,
            allowed=_field_names(ProviderSettings),
            prefix=f"providers.{name}",
        )
        _validate_provider_payload(name, provider_payload)
        provider_default = providers.get(name, ProviderSettings())
        providers[name] = replace(provider_default, **provider_payload)

    return HieronymusSettings(dreaming=dreaming, providers=providers)


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise SettingsError(f"{field_name} must be a table")
    return value


def _validate_settings(settings: HieronymusSettings) -> HieronymusSettings:
    _validate_dreaming_settings(settings.dreaming)
    for name, provider in settings.providers.items():
        _validate_provider_settings(name, provider)

    if settings.dreaming.active_provider not in settings.providers:
        raise SettingsError(
            f"active provider is not configured: {settings.dreaming.active_provider}",
        )

    _validate_minimum(
        "min_interval_minutes",
        settings.dreaming.min_interval_minutes,
    )
    _validate_minimum(
        "new_short_term_memory_threshold",
        settings.dreaming.new_short_term_memory_threshold,
    )
    _validate_minimum(
        "max_cycles_per_autostart",
        settings.dreaming.max_cycles_per_autostart,
    )

    for name, provider in settings.providers.items():
        if name == "deterministic" or not provider.enabled:
            continue
        if not provider.model:
            raise SettingsError(f"enabled provider must have a model: {name}")
        if not provider.api_key_env:
            raise SettingsError(f"enabled provider must have an api_key_env: {name}")

    return settings


def _field_names(model: type[object]) -> frozenset[str]:
    return frozenset(field.name for field in fields(model))


def _validate_unknown_keys(
    payload: dict[str, Any],
    *,
    allowed: frozenset[str],
    prefix: str | None,
) -> None:
    for key in payload:
        if key not in allowed:
            setting = key if prefix is None else f"{prefix}.{key}"
            raise SettingsError(f"unknown setting: {setting}")


def _validate_dreaming_payload(payload: dict[str, Any]) -> None:
    if "active_provider" in payload:
        _require_exact_str("active_provider", payload["active_provider"])
    if "autostart_enabled" in payload:
        _require_exact_bool("autostart_enabled", payload["autostart_enabled"])
    if "min_interval_minutes" in payload:
        _require_exact_int("min_interval_minutes", payload["min_interval_minutes"])
    if "new_short_term_memory_threshold" in payload:
        _require_exact_int(
            "new_short_term_memory_threshold",
            payload["new_short_term_memory_threshold"],
        )
    if "max_cycles_per_autostart" in payload:
        _require_exact_int(
            "max_cycles_per_autostart",
            payload["max_cycles_per_autostart"],
        )


def _validate_provider_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"providers.{name}"
    if "enabled" in payload:
        _require_exact_bool(f"{prefix}.enabled", payload["enabled"])
    if "model" in payload:
        _require_exact_str(f"{prefix}.model", payload["model"])
    if "api_key_env" in payload:
        _require_exact_str(f"{prefix}.api_key_env", payload["api_key_env"])
    if "base_url" in payload:
        _require_optional_exact_str(f"{prefix}.base_url", payload["base_url"])
    if "timeout_seconds" in payload:
        payload["timeout_seconds"] = _coerce_positive_float(
            f"{prefix}.timeout_seconds",
            payload["timeout_seconds"],
        )


def _validate_dreaming_settings(dreaming: DreamingSettings) -> None:
    _require_exact_str("active_provider", dreaming.active_provider)
    _require_exact_bool("autostart_enabled", dreaming.autostart_enabled)
    _require_exact_int("min_interval_minutes", dreaming.min_interval_minutes)
    _require_exact_int(
        "new_short_term_memory_threshold",
        dreaming.new_short_term_memory_threshold,
    )
    _require_exact_int(
        "max_cycles_per_autostart",
        dreaming.max_cycles_per_autostart,
    )


def _validate_provider_settings(name: str, provider: ProviderSettings) -> None:
    prefix = f"providers.{name}"
    _require_exact_bool(f"{prefix}.enabled", provider.enabled)
    _require_exact_str(f"{prefix}.model", provider.model)
    _require_exact_str(f"{prefix}.api_key_env", provider.api_key_env)
    _require_optional_exact_str(f"{prefix}.base_url", provider.base_url)
    _require_positive_float(f"{prefix}.timeout_seconds", provider.timeout_seconds)


def _require_exact_int(field_name: str, value: object) -> None:
    if type(value) is not int:
        raise SettingsError(f"{field_name} must be an integer")


def _require_exact_bool(field_name: str, value: object) -> None:
    if type(value) is not bool:
        raise SettingsError(f"{field_name} must be a boolean")


def _require_exact_str(field_name: str, value: object) -> None:
    if type(value) is not str:
        raise SettingsError(f"{field_name} must be a string")


def _require_optional_exact_str(field_name: str, value: object) -> None:
    if value is not None and type(value) is not str:
        raise SettingsError(f"{field_name} must be a string or null")


def _coerce_positive_float(field_name: str, value: object) -> float:
    if type(value) not in (int, float):
        raise SettingsError(f"{field_name} must be a number")
    value = float(value)
    if not math.isfinite(value):
        raise SettingsError(f"{field_name} must be finite and greater than 0")
    if value <= 0:
        raise SettingsError(f"{field_name} must be greater than 0")
    return value


def _require_positive_float(field_name: str, value: object) -> None:
    _coerce_positive_float(field_name, value)


def _validate_minimum(field_name: str, value: int) -> None:
    if value < 1:
        raise SettingsError(f"{field_name} must be at least 1")

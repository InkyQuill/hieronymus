from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass, replace
from typing import Any

import tomli_w

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

    def to_json_dict(self) -> dict[str, object]:
        payload: dict[str, object] = {
            "enabled": self.enabled,
            "model": self.model,
            "api_key_env": self.api_key_env,
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

    tmp_path = config.config_root / f"settings.toml.tmp-{os.getpid()}"
    tmp_path.write_text(
        tomli_w.dumps(settings.to_json_dict()),
        encoding="utf-8",
    )
    tmp_path.replace(config.settings_path)


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
    defaults = default_settings()

    dreaming_payload = _dict_payload(payload.get("dreaming"), "dreaming")
    dreaming = replace(defaults.dreaming, **dreaming_payload)

    providers = dict(defaults.providers)
    providers_payload = _dict_payload(payload.get("providers"), "providers")
    for name, raw_provider in providers_payload.items():
        provider_payload = _dict_payload(raw_provider, f"providers.{name}")
        provider_default = providers.get(name, ProviderSettings())
        providers[name] = replace(provider_default, **provider_payload)

    return HieronymusSettings(dreaming=dreaming, providers=providers)


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise SettingsError(f"{field_name} must be a table")
    return value


def _validate_settings(settings: HieronymusSettings) -> HieronymusSettings:
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


def _validate_minimum(field_name: str, value: int) -> None:
    if value < 1:
        raise SettingsError(f"{field_name} must be at least 1")

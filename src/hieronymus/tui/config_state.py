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
            check_result="",
        )

    def with_errors(self, errors: list[str]) -> ConfigDraft:
        return ConfigDraft(
            saved=self.saved,
            edited=self.edited,
            errors=tuple(errors),
            check_result="",
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

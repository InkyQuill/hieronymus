from __future__ import annotations

import math
import re
import tomllib
from dataclasses import dataclass, field, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import DreamConfigError, load_dream_config, save_dream_config

SUPPORTED_PROVIDER_TYPES = frozenset({"anthropic", "google", "ollama", "openai"})
PROVIDER_ID_PATTERN = re.compile(r"^[A-Za-z0-9_-]+$")


class ProviderCatalogError(ValueError):
    """Raised when provider.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ProviderProfile:
    name: str = ""
    type: str = ""
    url: str = ""
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
        return {
            "provider": self.provider,
            "model": self.model,
        }


@dataclass(frozen=True)
class ProviderCatalog:
    providers: dict[str, ProviderProfile] = field(default_factory=dict)
    defaults: ProviderDefaults = field(default_factory=ProviderDefaults)

    def with_provider(self, name: str, provider: ProviderProfile) -> ProviderCatalog:
        return replace(self, providers={**self.providers, name: provider})

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        return {
            **{
                name: provider.to_payload(redact=redact)
                for name, provider in self.providers.items()
            },
            "defaults": self.defaults.to_payload(),
        }


def default_provider_catalog() -> ProviderCatalog:
    return ProviderCatalog()


def load_provider_catalog(config: HieronymusConfig) -> ProviderCatalog:
    catalog = _load_provider_catalog_file(config)
    catalog = _migrate_legacy_dream_providers(config, catalog)
    if _provider_config_uses_legacy_gemini_type(config):
        save_provider_catalog(config, catalog)
    return catalog


def _load_provider_catalog_file(config: HieronymusConfig) -> ProviderCatalog:
    if not config.provider_config_path.exists():
        return validate_provider_catalog(default_provider_catalog())

    try:
        payload = tomllib.loads(config.provider_config_path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ProviderCatalogError(f"provider.conf could not be read: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise ProviderCatalogError(f"provider.conf is not valid TOML: {error}") from error

    return validate_provider_catalog(_provider_catalog_from_payload(payload))


def _migrate_legacy_dream_providers(
    config: HieronymusConfig,
    catalog: ProviderCatalog,
) -> ProviderCatalog:
    providers_payload = _legacy_dream_provider_payload(config)
    if not providers_payload:
        return catalog

    migrated = migrate_dream_provider_payload(providers_payload, existing=catalog)
    save_provider_catalog(config, migrated)
    try:
        save_dream_config(config, load_dream_config(config))
    except DreamConfigError:
        pass
    return migrated


def _legacy_dream_provider_payload(config: HieronymusConfig) -> dict[str, object]:
    if not config.dream_config_path.exists():
        return {}
    try:
        payload = tomllib.loads(config.dream_config_path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ProviderCatalogError(f"dream.conf could not be read: {error}") from error
    except tomllib.TOMLDecodeError:
        return {}
    providers = payload.get("providers")
    if providers is None:
        return {}
    return _dict_payload(providers, "providers")


def save_provider_catalog(config: HieronymusConfig, catalog: ProviderCatalog) -> None:
    catalog = validate_provider_catalog(catalog)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.provider_config_path, tomli_w.dumps(catalog.to_payload()))


def redacted_provider_catalog_payload(catalog: ProviderCatalog) -> dict[str, object]:
    return catalog.to_payload(redact=True)


def migrate_dream_provider_payload(
    providers_payload: dict[str, object],
    *,
    existing: ProviderCatalog,
) -> ProviderCatalog:
    next_catalog = existing
    for profile_id, raw_profile in providers_payload.items():
        _validate_provider_id(profile_id)
        table = _dict_payload(raw_profile, f"providers.{profile_id}")
        _validate_unknown_keys(
            table,
            allowed=frozenset(
                {"name", "type", "endpoint", "url", "api_key", "key", "timeout_seconds"},
            ),
            prefix=f"providers.{profile_id}",
        )
        existing_profile = next_catalog.providers.get(profile_id)
        if existing_profile is not None:
            table.setdefault("name", existing_profile.name)
            table.setdefault("type", existing_profile.type)
            table.setdefault("url", existing_profile.url)
            table.setdefault("key", existing_profile.key)
            table.setdefault("timeout_seconds", existing_profile.timeout_seconds)
        _validate_dream_provider_payload(profile_id, table)
        _require_migration_profile_fields(profile_id, table)
        profile = ProviderProfile(
            name=table.get("name", profile_id.replace("_", " ").title()),
            type=table["type"],
            url=table.get("endpoint", table.get("url", "")),
            key=table.get("api_key", table.get("key", "")),
            timeout_seconds=_coerce_positive_float(
                f"providers.{profile_id}.timeout_seconds",
                table.get("timeout_seconds", 30.0),
            ),
        )
        if existing_profile is not None and existing_profile != profile:
            raise ProviderCatalogError(
                f"dream.conf migration would overwrite provider profile: {profile_id}",
            )
        next_catalog = next_catalog.with_provider(profile_id, profile)
    return validate_provider_catalog(next_catalog)


def validate_provider_catalog(catalog: ProviderCatalog) -> ProviderCatalog:
    if not isinstance(catalog, ProviderCatalog):
        raise ProviderCatalogError("provider catalog has invalid type")

    if type(catalog.providers) is not dict:
        raise ProviderCatalogError("providers must be a mapping")
    for name, provider in catalog.providers.items():
        _validate_provider_id(name)
        if not isinstance(provider, ProviderProfile):
            raise ProviderCatalogError(f"providers.{name} has invalid profile type")
        _validate_provider_profile(name, provider)

    if not isinstance(catalog.defaults, ProviderDefaults):
        raise ProviderCatalogError("defaults has invalid type")
    _validate_provider_defaults(catalog.defaults, catalog.providers)
    return catalog


def _provider_catalog_from_payload(payload: dict[str, Any]) -> ProviderCatalog:
    defaults_payload = _dict_payload(payload.get("defaults"), "defaults")
    _validate_unknown_keys(
        defaults_payload,
        allowed=_field_names(ProviderDefaults),
        prefix="defaults",
    )
    _validate_defaults_payload(defaults_payload)

    providers: dict[str, ProviderProfile] = {}
    for name, raw_provider in payload.items():
        if name == "defaults":
            continue
        _validate_provider_id(name)
        provider_payload = _dict_payload(raw_provider, name)
        _validate_unknown_keys(
            provider_payload,
            allowed=_field_names(ProviderProfile),
            prefix=name,
        )
        _validate_provider_payload(name, provider_payload)
        _require_profile_fields(name, provider_payload)
        provider_payload.setdefault("name", name)
        profile = replace(ProviderProfile(), **provider_payload)
        if profile.type == "gemini":
            profile = replace(profile, type="google")
        providers[name] = profile

    return ProviderCatalog(
        providers=providers,
        defaults=replace(ProviderDefaults(), **defaults_payload),
    )


def _provider_config_uses_legacy_gemini_type(config: HieronymusConfig) -> bool:
    if not config.provider_config_path.exists():
        return False
    try:
        payload = tomllib.loads(config.provider_config_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return False
    return any(
        type(profile) is dict and profile.get("type") == "gemini"
        for name, profile in payload.items()
        if name != "defaults"
    )


def _validate_provider_profile(name: str, provider: ProviderProfile) -> None:
    prefix = f"providers.{name}"
    _require_exact_str(f"{prefix}.name", provider.name)
    _require_exact_str(f"{prefix}.type", provider.type)
    _require_exact_str(f"{prefix}.url", provider.url)
    _require_exact_str(f"{prefix}.key", provider.key)
    _require_positive_float(f"{prefix}.timeout_seconds", provider.timeout_seconds)
    if not provider.name:
        raise ProviderCatalogError(f"{prefix}.name is required")
    if provider.type not in SUPPORTED_PROVIDER_TYPES:
        raise ProviderCatalogError(f"unsupported provider type for {name}: {provider.type}")
    if not provider.url:
        raise ProviderCatalogError(f"{prefix}.url is required")


def _validate_provider_defaults(
    defaults: ProviderDefaults,
    providers: dict[str, ProviderProfile],
) -> None:
    _require_exact_str("defaults.provider", defaults.provider)
    _require_exact_str("defaults.model", defaults.model)
    if defaults.provider and defaults.provider not in providers:
        raise ProviderCatalogError(f"default provider is missing: {defaults.provider}")


def _validate_provider_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"providers.{name}"
    if "name" in payload:
        _require_exact_str(f"{prefix}.name", payload["name"])
    if "type" in payload:
        _require_exact_str(f"{prefix}.type", payload["type"])
    if "url" in payload:
        _require_exact_str(f"{prefix}.url", payload["url"])
    if "key" in payload:
        _require_exact_str(f"{prefix}.key", payload["key"])
    if "timeout_seconds" in payload:
        payload["timeout_seconds"] = _coerce_positive_float(
            f"{prefix}.timeout_seconds",
            payload["timeout_seconds"],
        )


def _validate_dream_provider_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"providers.{name}"
    if "name" in payload:
        _require_exact_str(f"{prefix}.name", payload["name"])
    if "type" in payload:
        _require_exact_str(f"{prefix}.type", payload["type"])
    if "endpoint" in payload:
        _require_exact_str(f"{prefix}.endpoint", payload["endpoint"])
    if "url" in payload:
        _require_exact_str(f"{prefix}.url", payload["url"])
    if "api_key" in payload:
        _require_exact_str(f"{prefix}.api_key", payload["api_key"])
    if "key" in payload:
        _require_exact_str(f"{prefix}.key", payload["key"])
    if "timeout_seconds" in payload:
        payload["timeout_seconds"] = _coerce_positive_float(
            f"{prefix}.timeout_seconds",
            payload["timeout_seconds"],
        )


def _validate_defaults_payload(payload: dict[str, Any]) -> None:
    if "provider" in payload:
        _require_exact_str("defaults.provider", payload["provider"])
    if "model" in payload:
        _require_exact_str("defaults.model", payload["model"])


def _require_profile_fields(name: str, payload: dict[str, Any]) -> None:
    for field_name in ("type", "url"):
        if field_name not in payload:
            raise ProviderCatalogError(f"providers.{name}.{field_name} is required")


def _require_migration_profile_fields(name: str, payload: dict[str, Any]) -> None:
    if "type" not in payload:
        raise ProviderCatalogError(f"providers.{name}.type is required")
    if "endpoint" not in payload and "url" not in payload:
        raise ProviderCatalogError(f"providers.{name}.url is required")


def _validate_provider_id(name: object) -> None:
    if type(name) is not str:
        raise ProviderCatalogError("providers keys must be strings")
    if not name or name == "defaults" or not PROVIDER_ID_PATTERN.fullmatch(name):
        raise ProviderCatalogError(f"invalid provider id: {name}")


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise ProviderCatalogError(f"{field_name} must be a table")
    return value


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
            raise ProviderCatalogError(f"unknown provider config setting: {setting}")


def _require_exact_str(field_name: str, value: object) -> None:
    if type(value) is not str:
        raise ProviderCatalogError(f"{field_name} must be a string")


def _coerce_positive_float(field_name: str, value: object) -> float:
    if type(value) not in (int, float):
        raise ProviderCatalogError(f"{field_name} must be a number")
    value = float(value)
    if not math.isfinite(value):
        raise ProviderCatalogError(f"{field_name} must be finite and greater than 0")
    if value <= 0:
        raise ProviderCatalogError(f"{field_name} must be greater than 0")
    return value


def _require_positive_float(field_name: str, value: object) -> None:
    _coerce_positive_float(field_name, value)

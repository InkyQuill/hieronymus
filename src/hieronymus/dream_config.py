from __future__ import annotations

import math
import tomllib
from dataclasses import dataclass, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

SUPPORTED_PROVIDER_TYPES = frozenset({"openai", "anthropic", "gemini", "ollama"})


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
        dreaming = {
            "enabled": self.enabled,
            "schedule_interval_minutes": self.schedule_interval_minutes,
            "min_pending_short_term_memories": self.min_pending_short_term_memories,
            "max_pending_short_term_memories": self.max_pending_short_term_memories,
            "max_short_term_memories_per_cycle": self.max_short_term_memories_per_cycle,
            "not_enough_memories_cycle_threshold": (self.not_enough_memories_cycle_threshold),
            "general_prompt": self.general_prompt,
        }
        providers = {
            name: provider.to_payload(redact=redact) for name, provider in self.providers.items()
        }
        workflows = {name: workflow.to_payload() for name, workflow in self.workflows.items()}
        return {
            "dreaming": dreaming,
            "providers": providers,
            "workflows": workflows,
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
            "anthropic": ProviderProfile(
                type="anthropic",
                endpoint="https://api.anthropic.com",
            ),
            "openai": ProviderProfile(
                type="openai",
                endpoint="https://api.openai.com/v1",
            ),
            "gemini": ProviderProfile(
                type="gemini",
                endpoint="https://generativelanguage.googleapis.com",
            ),
            "ollama": ProviderProfile(
                type="ollama",
                endpoint="http://localhost:11434",
            ),
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
        raise DreamConfigError(f"dream.conf is not valid TOML: {error}") from error

    return validate_dream_config(_dream_config_from_payload(payload))


def save_dream_config(config: HieronymusConfig, dream_config: DreamConfig) -> None:
    dream_config = validate_dream_config(dream_config)
    config.config_root.mkdir(parents=True, exist_ok=True)

    atomic_write_text(config.dream_config_path, tomli_w.dumps(dream_config.to_payload()))


def redacted_dream_config_payload(dream_config: DreamConfig) -> dict[str, object]:
    return dream_config.to_payload(redact=True)


def validate_dream_config(dream_config: DreamConfig) -> DreamConfig:
    _require_exact_bool("enabled", dream_config.enabled)
    _require_exact_str("general_prompt", dream_config.general_prompt)
    _require_positive_int(
        "schedule_interval_minutes",
        dream_config.schedule_interval_minutes,
    )
    _require_positive_int(
        "min_pending_short_term_memories",
        dream_config.min_pending_short_term_memories,
    )
    _require_positive_int(
        "max_pending_short_term_memories",
        dream_config.max_pending_short_term_memories,
    )
    _require_positive_int(
        "max_short_term_memories_per_cycle",
        dream_config.max_short_term_memories_per_cycle,
    )
    _require_positive_int(
        "not_enough_memories_cycle_threshold",
        dream_config.not_enough_memories_cycle_threshold,
    )
    if dream_config.max_pending_short_term_memories < dream_config.min_pending_short_term_memories:
        raise DreamConfigError(
            "max_pending_short_term_memories must be greater than or equal to "
            "min_pending_short_term_memories",
        )
    if (
        dream_config.max_short_term_memories_per_cycle
        > dream_config.max_pending_short_term_memories
    ):
        raise DreamConfigError(
            "max_short_term_memories_per_cycle must be less than or equal to "
            "max_pending_short_term_memories",
        )

    _require_profile_mapping(
        "providers",
        dream_config.providers,
        ProviderProfile,
    )
    _require_profile_mapping(
        "workflows",
        dream_config.workflows,
        WorkflowProfile,
    )
    for name, provider in dream_config.providers.items():
        _validate_provider_profile(name, provider)
    for name, workflow in dream_config.workflows.items():
        _validate_workflow_profile(name, workflow, dream_config.providers)

    return dream_config


def _dream_config_from_payload(payload: dict[str, Any]) -> DreamConfig:
    _validate_unknown_keys(
        payload,
        allowed=frozenset({"dreaming", "providers", "workflows"}),
        prefix=None,
    )
    defaults = default_dream_config()

    dreaming = _dict_payload(payload.get("dreaming"), "dreaming")
    dreaming_keys = _field_names(DreamConfig) - frozenset({"providers", "workflows"})
    _validate_unknown_keys(
        dreaming,
        allowed=dreaming_keys,
        prefix="dreaming",
    )
    _validate_dreaming_payload(dreaming)

    providers = dict(defaults.providers)
    providers_payload = _dict_payload(payload.get("providers"), "providers")
    for name, raw_provider in providers_payload.items():
        provider_payload = _dict_payload(raw_provider, f"providers.{name}")
        _validate_unknown_keys(
            provider_payload,
            allowed=_field_names(ProviderProfile),
            prefix=f"providers.{name}",
        )
        _validate_provider_payload(name, provider_payload)
        provider_default = providers.get(name)
        if provider_default is None:
            if "type" not in provider_payload:
                raise DreamConfigError(f"providers.{name}.type is required")
            provider_default = ProviderProfile(type=provider_payload["type"])
        providers[name] = replace(provider_default, **provider_payload)

    workflows = dict(defaults.workflows)
    workflows_payload = _dict_payload(payload.get("workflows"), "workflows")
    for name, raw_workflow in workflows_payload.items():
        workflow_payload = _dict_payload(raw_workflow, f"workflows.{name}")
        _validate_unknown_keys(
            workflow_payload,
            allowed=_field_names(WorkflowProfile),
            prefix=f"workflows.{name}",
        )
        _validate_workflow_payload(name, workflow_payload)
        workflow_default = workflows.get(
            name,
            WorkflowProfile(provider="", model="", enabled=False),
        )
        workflows[name] = replace(workflow_default, **workflow_payload)

    return DreamConfig(
        enabled=dreaming.get("enabled", defaults.enabled),
        schedule_interval_minutes=dreaming.get(
            "schedule_interval_minutes",
            defaults.schedule_interval_minutes,
        ),
        min_pending_short_term_memories=dreaming.get(
            "min_pending_short_term_memories",
            defaults.min_pending_short_term_memories,
        ),
        max_pending_short_term_memories=dreaming.get(
            "max_pending_short_term_memories",
            defaults.max_pending_short_term_memories,
        ),
        max_short_term_memories_per_cycle=dreaming.get(
            "max_short_term_memories_per_cycle",
            defaults.max_short_term_memories_per_cycle,
        ),
        not_enough_memories_cycle_threshold=dreaming.get(
            "not_enough_memories_cycle_threshold",
            defaults.not_enough_memories_cycle_threshold,
        ),
        general_prompt=dreaming.get("general_prompt", defaults.general_prompt),
        providers=providers,
        workflows=workflows,
    )


def _validate_dreaming_payload(payload: dict[str, Any]) -> None:
    if "enabled" in payload:
        _require_exact_bool("enabled", payload["enabled"])
    if "schedule_interval_minutes" in payload:
        _require_exact_int(
            "schedule_interval_minutes",
            payload["schedule_interval_minutes"],
        )
    if "min_pending_short_term_memories" in payload:
        _require_exact_int(
            "min_pending_short_term_memories",
            payload["min_pending_short_term_memories"],
        )
    if "max_pending_short_term_memories" in payload:
        _require_exact_int(
            "max_pending_short_term_memories",
            payload["max_pending_short_term_memories"],
        )
    if "max_short_term_memories_per_cycle" in payload:
        _require_exact_int(
            "max_short_term_memories_per_cycle",
            payload["max_short_term_memories_per_cycle"],
        )
    if "not_enough_memories_cycle_threshold" in payload:
        _require_exact_int(
            "not_enough_memories_cycle_threshold",
            payload["not_enough_memories_cycle_threshold"],
        )
    if "general_prompt" in payload:
        _require_exact_str("general_prompt", payload["general_prompt"])


def _validate_provider_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"providers.{name}"
    if "type" in payload:
        _require_exact_str(f"{prefix}.type", payload["type"])
    if "endpoint" in payload:
        _require_exact_str(f"{prefix}.endpoint", payload["endpoint"])
    if "api_key" in payload:
        _require_exact_str(f"{prefix}.api_key", payload["api_key"])
    if "timeout_seconds" in payload:
        payload["timeout_seconds"] = _coerce_positive_float(
            f"{prefix}.timeout_seconds",
            payload["timeout_seconds"],
        )


def _validate_workflow_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"workflows.{name}"
    if "provider" in payload:
        _require_exact_str(f"{prefix}.provider", payload["provider"])
    if "model" in payload:
        _require_exact_str(f"{prefix}.model", payload["model"])
    if "enabled" in payload:
        _require_exact_bool(f"{prefix}.enabled", payload["enabled"])


def _validate_provider_profile(name: str, provider: ProviderProfile) -> None:
    prefix = f"providers.{name}"
    _require_exact_str(f"{prefix}.type", provider.type)
    _require_exact_str(f"{prefix}.endpoint", provider.endpoint)
    _require_exact_str(f"{prefix}.api_key", provider.api_key)
    _require_positive_float(f"{prefix}.timeout_seconds", provider.timeout_seconds)
    if provider.type not in SUPPORTED_PROVIDER_TYPES:
        raise DreamConfigError(f"unsupported provider type for {name}: {provider.type}")


def _validate_workflow_profile(
    name: str,
    workflow: WorkflowProfile,
    providers: dict[str, ProviderProfile],
) -> None:
    prefix = f"workflows.{name}"
    _require_exact_str(f"{prefix}.provider", workflow.provider)
    _require_exact_str(f"{prefix}.model", workflow.model)
    _require_exact_bool(f"{prefix}.enabled", workflow.enabled)
    if workflow.enabled and workflow.provider not in providers:
        raise DreamConfigError(
            f"referenced provider profile is missing: {name}.{workflow.provider}",
        )
    if workflow.enabled and not workflow.model:
        raise DreamConfigError(f"enabled workflow must have a model: {name}")


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise DreamConfigError(f"{field_name} must be a table")
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
            raise DreamConfigError(f"unknown dream config setting: {setting}")


def _require_profile_mapping(
    field_name: str,
    value: object,
    profile_type: type[object],
) -> None:
    if type(value) is not dict:
        raise DreamConfigError(f"{field_name} must be a mapping")
    for name, profile in value.items():
        if type(name) is not str:
            raise DreamConfigError(f"{field_name} keys must be strings")
        if not isinstance(profile, profile_type):
            raise DreamConfigError(f"{field_name}.{name} has invalid profile type")


def _require_exact_int(field_name: str, value: object) -> None:
    if type(value) is not int:
        raise DreamConfigError(f"{field_name} must be an integer")


def _require_positive_int(field_name: str, value: object) -> None:
    _require_exact_int(field_name, value)
    if value < 1:
        raise DreamConfigError(f"{field_name} must be at least 1")


def _require_exact_bool(field_name: str, value: object) -> None:
    if type(value) is not bool:
        raise DreamConfigError(f"{field_name} must be a boolean")


def _require_exact_str(field_name: str, value: object) -> None:
    if type(value) is not str:
        raise DreamConfigError(f"{field_name} must be a string")


def _coerce_positive_float(field_name: str, value: object) -> float:
    if type(value) not in (int, float):
        raise DreamConfigError(f"{field_name} must be a number")
    value = float(value)
    if not math.isfinite(value):
        raise DreamConfigError(f"{field_name} must be finite and greater than 0")
    if value <= 0:
        raise DreamConfigError(f"{field_name} must be greater than 0")
    return value


def _require_positive_float(field_name: str, value: object) -> None:
    _coerce_positive_float(field_name, value)

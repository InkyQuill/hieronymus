from __future__ import annotations

import tomllib
from dataclasses import dataclass, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig


class DreamConfigError(ValueError):
    """Raised when dream.conf cannot be loaded or used."""


@dataclass(frozen=True)
class WorkflowProfile:
    provider: str
    model: str
    enabled: bool = True
    max_records_per_pass: int = 500

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "model": self.model,
            "enabled": self.enabled,
            "max_records_per_pass": self.max_records_per_pass,
        }


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
    max_short_term_memories_per_run: int
    max_long_term_records_affected_per_run: int
    max_relation_records_per_pass: int
    general_prompt: str
    workflows: dict[str, WorkflowProfile]

    def with_workflow(self, name: str, workflow: WorkflowProfile) -> DreamConfig:
        # This programmatic convenience is intentionally not part of the persisted
        # schema: `_dream_config_from_payload` rejects the removed alpha name.
        name = {
            "crystallization": "knowledge_crystals",
            "relation_discovery": "relations",
            "reinforcement_compaction": "reinforcement",
        }.get(name, name)
        return replace(self, workflows={**self.workflows, name: workflow})

    def to_payload(self, *, redact: bool = False) -> dict[str, object]:
        del redact
        dreaming = {
            "enabled": self.enabled,
            "schedule_interval_minutes": self.schedule_interval_minutes,
            "min_pending_short_term_memories": self.min_pending_short_term_memories,
            "max_pending_short_term_memories": self.max_pending_short_term_memories,
            "max_short_term_memories_per_cycle": self.max_short_term_memories_per_cycle,
            "not_enough_memories_cycle_threshold": (self.not_enough_memories_cycle_threshold),
            "max_changed_crystals_per_cycle": self.max_changed_crystals_per_cycle,
            "max_related_concepts_per_cycle": self.max_related_concepts_per_cycle,
            "max_related_crystals_per_concept": self.max_related_crystals_per_concept,
            "max_total_affected_crystals": self.max_total_affected_crystals,
            "max_short_term_memories_per_run": self.max_short_term_memories_per_run,
            "max_long_term_records_affected_per_run": self.max_long_term_records_affected_per_run,
            "max_relation_records_per_pass": self.max_relation_records_per_pass,
            "general_prompt": self.general_prompt,
        }
        workflows = {name: workflow.to_payload() for name, workflow in self.workflows.items()}
        return {
            "dreaming": dreaming,
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
        max_changed_crystals_per_cycle=200,
        max_related_concepts_per_cycle=80,
        max_related_crystals_per_concept=20,
        max_total_affected_crystals=500,
        max_short_term_memories_per_run=500,
        max_long_term_records_affected_per_run=1000,
        max_relation_records_per_pass=1000,
        general_prompt=(
            "Use English as the primary searchable memory language. Preserve Japanese, "
            "Russian, and other languages only as terms, names, renderings, quoted "
            "evidence, or metadata. Long-term crystals must be 1-2 sentences. "
            "Short-term memories must be 1-6 sentences."
        ),
        workflows={
            "concepts": WorkflowProfile(
                provider="",
                model="",
                enabled=False,
            ),
            **{
                name: WorkflowProfile(provider="", model="", enabled=False)
                for name in (
                    "terminology_candidates",
                    "rule_crystals",
                    "knowledge_crystals",
                    "relations",
                    "reinforcement",
                    "coverage_audit",
                )
            },
        },
    )


def load_dream_config(config: HieronymusConfig) -> DreamConfig:
    if not config.dream_config_path.exists():
        return validate_dream_config(default_dream_config())

    try:
        payload = tomllib.loads(config.dream_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise DreamConfigError(f"dream.conf is not valid TOML: {error}") from error

    dream_config = validate_dream_config(_dream_config_from_payload(payload))
    raw_workflows = payload.get("workflows")
    if type(raw_workflows) is dict and set(raw_workflows) != set(dream_config.workflows):
        save_dream_config(config, dream_config)
    return dream_config


def save_dream_config(config: HieronymusConfig, dream_config: DreamConfig) -> None:
    dream_config = validate_dream_config(dream_config)
    config.config_root.mkdir(parents=True, exist_ok=True)

    atomic_write_text(config.dream_config_path, tomli_w.dumps(dream_config.to_payload()))


def redacted_dream_config_payload(dream_config: DreamConfig) -> dict[str, object]:
    return dream_config.to_payload(redact=True)


def validate_dream_config(dream_config: DreamConfig) -> DreamConfig:
    expected_workflows = set(default_dream_config().workflows)
    if set(dream_config.workflows) != expected_workflows:
        raise DreamConfigError("workflows must contain exactly the seven Dream passes")
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
    _require_positive_int(
        "max_changed_crystals_per_cycle",
        dream_config.max_changed_crystals_per_cycle,
    )
    _require_positive_int(
        "max_related_concepts_per_cycle",
        dream_config.max_related_concepts_per_cycle,
    )
    _require_positive_int(
        "max_related_crystals_per_concept",
        dream_config.max_related_crystals_per_concept,
    )
    _require_positive_int(
        "max_total_affected_crystals",
        dream_config.max_total_affected_crystals,
    )
    _require_positive_int(
        "max_short_term_memories_per_run",
        dream_config.max_short_term_memories_per_run,
    )
    _require_positive_int(
        "max_long_term_records_affected_per_run",
        dream_config.max_long_term_records_affected_per_run,
    )
    _require_positive_int(
        "max_relation_records_per_pass",
        dream_config.max_relation_records_per_pass,
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
        "workflows",
        dream_config.workflows,
        WorkflowProfile,
    )
    if set(dream_config.workflows) != expected_workflows:
        raise DreamConfigError("workflows must contain exactly the seven Dream passes")
    for name, workflow in dream_config.workflows.items():
        _validate_workflow_profile(name, workflow)

    return dream_config


def _dream_config_from_payload(payload: dict[str, Any]) -> DreamConfig:
    _validate_unknown_keys(
        payload,
        allowed=frozenset({"dreaming", "providers", "workflows"}),
        prefix=None,
    )
    defaults = default_dream_config()

    dreaming = _dict_payload(payload.get("dreaming"), "dreaming")
    dreaming_keys = _field_names(DreamConfig) - frozenset({"workflows"})
    _validate_unknown_keys(
        dreaming,
        allowed=dreaming_keys,
        prefix="dreaming",
    )
    _validate_dreaming_payload(dreaming)

    _dict_payload(payload.get("providers"), "providers")

    workflows = dict(defaults.workflows)
    workflows_payload = _dict_payload(payload.get("workflows"), "workflows")
    for name, raw_workflow in workflows_payload.items():
        if name not in workflows:
            raise DreamConfigError("workflows must contain exactly the seven Dream passes")
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
        max_changed_crystals_per_cycle=dreaming.get(
            "max_changed_crystals_per_cycle",
            defaults.max_changed_crystals_per_cycle,
        ),
        max_related_concepts_per_cycle=dreaming.get(
            "max_related_concepts_per_cycle",
            defaults.max_related_concepts_per_cycle,
        ),
        max_related_crystals_per_concept=dreaming.get(
            "max_related_crystals_per_concept",
            defaults.max_related_crystals_per_concept,
        ),
        max_total_affected_crystals=dreaming.get(
            "max_total_affected_crystals",
            defaults.max_total_affected_crystals,
        ),
        max_short_term_memories_per_run=dreaming.get(
            "max_short_term_memories_per_run",
            defaults.max_short_term_memories_per_run,
        ),
        max_long_term_records_affected_per_run=dreaming.get(
            "max_long_term_records_affected_per_run",
            defaults.max_long_term_records_affected_per_run,
        ),
        max_relation_records_per_pass=dreaming.get(
            "max_relation_records_per_pass",
            defaults.max_relation_records_per_pass,
        ),
        general_prompt=dreaming.get("general_prompt", defaults.general_prompt),
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
    if "max_changed_crystals_per_cycle" in payload:
        _require_exact_int(
            "max_changed_crystals_per_cycle",
            payload["max_changed_crystals_per_cycle"],
        )
    if "max_related_concepts_per_cycle" in payload:
        _require_exact_int(
            "max_related_concepts_per_cycle",
            payload["max_related_concepts_per_cycle"],
        )
    if "max_related_crystals_per_concept" in payload:
        _require_exact_int(
            "max_related_crystals_per_concept",
            payload["max_related_crystals_per_concept"],
        )
    if "max_total_affected_crystals" in payload:
        _require_exact_int(
            "max_total_affected_crystals",
            payload["max_total_affected_crystals"],
        )
    if "general_prompt" in payload:
        _require_exact_str("general_prompt", payload["general_prompt"])


def _validate_workflow_payload(name: str, payload: dict[str, Any]) -> None:
    prefix = f"workflows.{name}"
    if "provider" in payload:
        _require_exact_str(f"{prefix}.provider", payload["provider"])
    if "model" in payload:
        _require_exact_str(f"{prefix}.model", payload["model"])
    if "enabled" in payload:
        _require_exact_bool(f"{prefix}.enabled", payload["enabled"])


def _validate_workflow_profile(name: str, workflow: WorkflowProfile) -> None:
    prefix = f"workflows.{name}"
    _require_exact_str(f"{prefix}.provider", workflow.provider)
    _require_exact_str(f"{prefix}.model", workflow.model)
    _require_exact_bool(f"{prefix}.enabled", workflow.enabled)
    _require_positive_int(f"{prefix}.max_records_per_pass", workflow.max_records_per_pass)
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

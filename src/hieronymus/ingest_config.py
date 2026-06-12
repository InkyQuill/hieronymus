from __future__ import annotations

import tomllib
from dataclasses import dataclass, fields, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig


class IngestConfigError(ValueError):
    """Raised when ingest.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ShortMemoryLimits:
    warning_sentence_count: int = 6
    rejection_sentence_count: int = 30
    warning_symbol_count: int = 0
    rejection_symbol_count: int = 0

    def to_payload(self) -> dict[str, int]:
        return {
            "warning_sentence_count": self.warning_sentence_count,
            "rejection_sentence_count": self.rejection_sentence_count,
            "warning_symbol_count": self.warning_symbol_count,
            "rejection_symbol_count": self.rejection_symbol_count,
        }


@dataclass(frozen=True)
class LearnLimits:
    max_block_chars: int = 1200

    def to_payload(self) -> dict[str, int]:
        return {"max_block_chars": self.max_block_chars}


@dataclass(frozen=True)
class IngestConfig:
    short_memory: ShortMemoryLimits = ShortMemoryLimits()
    learn: LearnLimits = LearnLimits()

    def to_payload(self) -> dict[str, object]:
        return {
            "short_memory": self.short_memory.to_payload(),
            "learn": self.learn.to_payload(),
        }

    def with_short_memory(self, limits: ShortMemoryLimits) -> IngestConfig:
        return replace(self, short_memory=limits)

    def with_learn(self, limits: LearnLimits) -> IngestConfig:
        return replace(self, learn=limits)


def default_ingest_config() -> IngestConfig:
    return IngestConfig()


def load_ingest_config(config: HieronymusConfig) -> IngestConfig:
    if not config.ingest_config_path.exists():
        return validate_ingest_config(default_ingest_config())

    try:
        payload = tomllib.loads(config.ingest_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise IngestConfigError(f"ingest.conf is not valid TOML: {error}") from error

    return validate_ingest_config(_ingest_config_from_payload(payload))


def save_ingest_config(config: HieronymusConfig, ingest_config: IngestConfig) -> None:
    ingest_config = validate_ingest_config(ingest_config)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.ingest_config_path, tomli_w.dumps(ingest_config.to_payload()))


def validate_ingest_config(ingest_config: IngestConfig) -> IngestConfig:
    _validate_int_model("short_memory", ingest_config.short_memory)
    _validate_int_model("learn", ingest_config.learn)
    _require_minimum(
        "short_memory.warning_sentence_count",
        ingest_config.short_memory.warning_sentence_count,
        1,
    )
    _require_minimum(
        "short_memory.rejection_sentence_count",
        ingest_config.short_memory.rejection_sentence_count,
        1,
    )
    _require_minimum(
        "short_memory.warning_symbol_count",
        ingest_config.short_memory.warning_symbol_count,
        0,
    )
    _require_minimum(
        "short_memory.rejection_symbol_count",
        ingest_config.short_memory.rejection_symbol_count,
        0,
    )
    _require_minimum("learn.max_block_chars", ingest_config.learn.max_block_chars, 1)
    if (
        ingest_config.short_memory.rejection_sentence_count
        < ingest_config.short_memory.warning_sentence_count
    ):
        raise IngestConfigError(
            "short_memory.rejection_sentence_count must be greater than or equal to "
            "short_memory.warning_sentence_count",
        )
    if (
        ingest_config.short_memory.warning_symbol_count
        and ingest_config.short_memory.rejection_symbol_count
        and ingest_config.short_memory.rejection_symbol_count
        < ingest_config.short_memory.warning_symbol_count
    ):
        # A zero symbol threshold is treated as disabled, so compare only active limits.
        raise IngestConfigError(
            "short_memory.rejection_symbol_count must be greater than or equal to "
            "short_memory.warning_symbol_count",
        )
    return ingest_config


def _ingest_config_from_payload(payload: dict[str, Any]) -> IngestConfig:
    _validate_unknown_keys(payload, allowed=frozenset({"short_memory", "learn"}), prefix=None)
    defaults = default_ingest_config()

    short_memory_payload = _dict_payload(payload.get("short_memory"), "short_memory")
    learn_payload = _dict_payload(payload.get("learn"), "learn")
    _validate_unknown_keys(
        short_memory_payload,
        allowed=_field_names(ShortMemoryLimits),
        prefix="short_memory",
    )
    _validate_unknown_keys(learn_payload, allowed=_field_names(LearnLimits), prefix="learn")
    _validate_short_memory_payload(short_memory_payload)
    _validate_learn_payload(learn_payload)
    return IngestConfig(
        short_memory=replace(defaults.short_memory, **short_memory_payload),
        learn=replace(defaults.learn, **learn_payload),
    )


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise IngestConfigError(f"{field_name} must be a table")
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
            raise IngestConfigError(f"unknown ingest config setting: {setting}")


def _validate_short_memory_payload(payload: dict[str, Any]) -> None:
    for field in fields(ShortMemoryLimits):
        if field.name in payload:
            _require_exact_int(f"short_memory.{field.name}", payload[field.name])


def _validate_learn_payload(payload: dict[str, Any]) -> None:
    if "max_block_chars" in payload:
        _require_exact_int("learn.max_block_chars", payload["max_block_chars"])


def _validate_int_model(prefix: str, model: object) -> None:
    for field in fields(model):
        value = getattr(model, field.name)
        if type(value) is not int:
            raise IngestConfigError(f"{prefix}.{field.name} must be an integer")


def _require_exact_int(field_name: str, value: object) -> None:
    if type(value) is not int:
        raise IngestConfigError(f"{field_name} must be an integer")


def _require_minimum(field_name: str, value: int, minimum: int) -> None:
    if value < minimum:
        raise IngestConfigError(f"{field_name} must be at least {minimum}")

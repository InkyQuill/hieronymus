from __future__ import annotations

import os
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass, fields, replace
from typing import Any

from hieronymus.config import HieronymusConfig

DEFAULT_SEMANTIC_MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
MIN_BATCH_SIZE = 1
MAX_BATCH_SIZE = 256
MIN_CANDIDATE_MULTIPLIER = 1
MAX_CANDIDATE_MULTIPLIER = 100
MIN_FUSION_CONSTANT = 1
MAX_FUSION_CONSTANT = 1_000
MIN_DIMENSIONS = 1
MAX_DIMENSIONS = 65_536


class SemanticConfigError(ValueError):
    """Raised when semantic retrieval configuration is invalid."""


@dataclass(frozen=True)
class SemanticConfig:
    enabled: bool = True
    provider: str = "local"
    model: str = DEFAULT_SEMANTIC_MODEL
    revision: str | None = None
    dimensions: int = 384
    batch_size: int = 32
    candidate_multiplier: int = 4
    fusion_constant: int = 60

    @property
    def fts_only(self) -> bool:
        return not self.enabled


_ENVIRONMENT_FIELDS = {
    "HIERONYMUS_SEMANTIC_ENABLED": "enabled",
    "HIERONYMUS_SEMANTIC_PROVIDER": "provider",
    "HIERONYMUS_SEMANTIC_MODEL": "model",
    "HIERONYMUS_SEMANTIC_REVISION": "revision",
    "HIERONYMUS_SEMANTIC_DIMENSIONS": "dimensions",
    "HIERONYMUS_SEMANTIC_BATCH_SIZE": "batch_size",
    "HIERONYMUS_SEMANTIC_CANDIDATE_MULTIPLIER": "candidate_multiplier",
    "HIERONYMUS_SEMANTIC_FUSION_CONSTANT": "fusion_constant",
}


def load_semantic_config(
    config: HieronymusConfig,
    *,
    environ: Mapping[str, str] | None = None,
) -> SemanticConfig:
    semantic = _load_semantic_config_file(config)
    semantic = _apply_environment(semantic, os.environ if environ is None else environ)
    return validate_semantic_config(semantic)


def validate_semantic_config(config: SemanticConfig) -> SemanticConfig:
    if type(config.enabled) is not bool:
        raise SemanticConfigError("semantic.enabled must be a boolean")
    provider = _require_nonblank_string("semantic.provider", config.provider)
    model = _require_nonblank_string("semantic.model", config.model)
    revision = config.revision
    if revision is not None:
        revision = _require_nonblank_string("semantic.revision", revision)
    _require_bounded_int("semantic.dimensions", config.dimensions, MIN_DIMENSIONS, MAX_DIMENSIONS)
    _require_bounded_int("semantic.batch_size", config.batch_size, MIN_BATCH_SIZE, MAX_BATCH_SIZE)
    _require_bounded_int(
        "semantic.candidate_multiplier",
        config.candidate_multiplier,
        MIN_CANDIDATE_MULTIPLIER,
        MAX_CANDIDATE_MULTIPLIER,
    )
    _require_bounded_int(
        "semantic.fusion_constant",
        config.fusion_constant,
        MIN_FUSION_CONSTANT,
        MAX_FUSION_CONSTANT,
    )
    return replace(config, provider=provider, model=model, revision=revision)


def _load_semantic_config_file(config: HieronymusConfig) -> SemanticConfig:
    if not config.semantic_config_path.exists():
        return SemanticConfig()
    try:
        payload = tomllib.loads(config.semantic_config_path.read_text(encoding="utf-8"))
    except OSError as error:
        raise SemanticConfigError(f"semantic.conf could not be read: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise SemanticConfigError(f"semantic.conf is not valid TOML: {error}") from error

    semantic_payload = payload.get("semantic", {})
    if type(semantic_payload) is not dict:
        raise SemanticConfigError("semantic must be a table")
    allowed = {field.name for field in fields(SemanticConfig)}
    for key in semantic_payload:
        if key not in allowed:
            raise SemanticConfigError(f"unknown semantic config setting: semantic.{key}")
    return validate_semantic_config(SemanticConfig(**semantic_payload))


def _apply_environment(config: SemanticConfig, environ: Mapping[str, str]) -> SemanticConfig:
    overrides: dict[str, Any] = {}
    for environment_name, field_name in _ENVIRONMENT_FIELDS.items():
        if environment_name not in environ:
            continue
        value = environ[environment_name]
        if field_name == "enabled":
            overrides[field_name] = _parse_environment_bool(environment_name, value)
        elif field_name in {"dimensions", "batch_size", "candidate_multiplier", "fusion_constant"}:
            overrides[field_name] = _parse_environment_int(environment_name, value)
        elif field_name == "revision" and not value.strip():
            overrides[field_name] = None
        else:
            overrides[field_name] = value
    updated = replace(config, **overrides)
    try:
        return validate_semantic_config(updated)
    except SemanticConfigError as error:
        changed_field = next(
            (field for field in overrides if f"semantic.{field}" in str(error)), None
        )
        if changed_field is None:
            raise
        environment_name = next(
            name for name, field in _ENVIRONMENT_FIELDS.items() if field == changed_field
        )
        raise SemanticConfigError(f"{environment_name}: {error}") from error


def _parse_environment_bool(name: str, value: str) -> bool:
    normalized = value.strip().casefold()
    if normalized in {"1", "true", "yes", "on"}:
        return True
    if normalized in {"0", "false", "no", "off"}:
        return False
    raise SemanticConfigError(f"{name} must be true or false")


def _parse_environment_int(name: str, value: str) -> int:
    try:
        return int(value)
    except ValueError as error:
        raise SemanticConfigError(f"{name} must be an integer") from error


def _require_nonblank_string(name: str, value: object) -> str:
    if type(value) is not str or not value.strip():
        raise SemanticConfigError(f"{name} must be a non-blank string")
    return value.strip()


def _require_bounded_int(name: str, value: object, minimum: int, maximum: int) -> None:
    if type(value) is not int:
        raise SemanticConfigError(f"{name} must be an integer")
    if not minimum <= value <= maximum:
        raise SemanticConfigError(f"{name} must be between {minimum} and {maximum}")

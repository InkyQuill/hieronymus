from __future__ import annotations

import tomllib
from dataclasses import dataclass, replace
from typing import Any

import tomli_w

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

UPDATE_CHANNELS = frozenset({"stable", "dev"})


class ReleaseConfigError(ValueError):
    """Raised when release.conf cannot be loaded or used."""


@dataclass(frozen=True)
class ReleaseConfig:
    update_channel: str = "stable"

    @property
    def update_target(self) -> str:
        return "main" if self.update_channel == "dev" else "latest"

    @property
    def allows_dev_updates(self) -> bool:
        return self.update_channel == "dev"

    def to_payload(self) -> dict[str, object]:
        return {"updates": {"channel": self.update_channel}}

    def with_update_channel(self, update_channel: str) -> ReleaseConfig:
        return replace(self, update_channel=update_channel)


def default_release_config() -> ReleaseConfig:
    return ReleaseConfig()


def load_release_config(config: HieronymusConfig) -> ReleaseConfig:
    if not config.release_config_path.exists():
        return validate_release_config(default_release_config())

    try:
        payload = tomllib.loads(config.release_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise ReleaseConfigError(f"release.conf is not valid TOML: {error}") from error

    return validate_release_config(_release_config_from_payload(payload))


def save_release_config(config: HieronymusConfig, release_config: ReleaseConfig) -> None:
    release_config = validate_release_config(release_config)
    config.config_root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(config.release_config_path, tomli_w.dumps(release_config.to_payload()))


def validate_release_config(release_config: ReleaseConfig) -> ReleaseConfig:
    _require_exact_str("updates.channel", release_config.update_channel)
    if release_config.update_channel not in UPDATE_CHANNELS:
        allowed = ", ".join(sorted(UPDATE_CHANNELS))
        raise ReleaseConfigError(f"updates.channel must be one of: {allowed}")
    return release_config


def _release_config_from_payload(payload: dict[str, Any]) -> ReleaseConfig:
    _validate_unknown_keys(payload, allowed=frozenset({"updates"}), prefix=None)
    updates = _dict_payload(payload.get("updates"), "updates")
    _validate_unknown_keys(updates, allowed=frozenset({"channel"}), prefix="updates")
    if "channel" not in updates:
        return default_release_config()
    return ReleaseConfig(update_channel=updates["channel"])


def _dict_payload(value: object, field_name: str) -> dict[str, Any]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise ReleaseConfigError(f"{field_name} must be a table")
    return value


def _validate_unknown_keys(
    payload: dict[str, Any],
    *,
    allowed: frozenset[str],
    prefix: str | None,
) -> None:
    for key in payload:
        if key not in allowed:
            setting = key if prefix is None else f"{prefix}.{key}"
            raise ReleaseConfigError(f"unknown release config setting: {setting}")


def _require_exact_str(field_name: str, value: object) -> None:
    if type(value) is not str:
        raise ReleaseConfigError(f"{field_name} must be a string")

from __future__ import annotations

import os

from hieronymus.dream_config import DreamConfig


def configured_secret_values(dream_config: DreamConfig) -> set[str]:
    return {
        value
        for provider in dream_config.providers.values()
        if (value := getattr(provider, "api_key", "").strip())
    }


def env_value_exists(env_name: str) -> bool:
    return bool(env_name and os.environ.get(env_name))


def redact_configured_secret_values(text: str, dream_config: DreamConfig) -> str:
    redacted = text
    for value in sorted(configured_secret_values(dream_config), key=len, reverse=True):
        redacted = redacted.replace(value, "[redacted]")
    return redacted

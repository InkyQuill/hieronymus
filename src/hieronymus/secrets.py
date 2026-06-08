from __future__ import annotations

import os

from hieronymus.settings import HieronymusSettings


def configured_key_env_names(settings: HieronymusSettings) -> set[str]:
    return {
        provider.api_key_env
        for provider in settings.providers.values()
        if provider.api_key_env.strip()
    }


def env_value_exists(env_name: str) -> bool:
    return bool(env_name and os.environ.get(env_name))


def redact_configured_secret_values(text: str, settings: HieronymusSettings) -> str:
    redacted = text
    for env_name in configured_key_env_names(settings):
        value = os.environ.get(env_name)
        if value and len(value) >= 4:
            redacted = redacted.replace(value, "[redacted]")
    return redacted

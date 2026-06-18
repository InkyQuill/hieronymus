from __future__ import annotations

import os

from hieronymus.provider_config import ProviderCatalog


def configured_secret_values(provider_catalog: ProviderCatalog) -> set[str]:
    return {
        value for provider in provider_catalog.providers.values() if (value := provider.key.strip())
    }


def env_value_exists(env_name: str) -> bool:
    return bool(env_name and os.environ.get(env_name))


def redact_configured_secret_values(text: str, provider_catalog: ProviderCatalog) -> str:
    redacted = text
    for value in sorted(configured_secret_values(provider_catalog), key=len, reverse=True):
        redacted = redacted.replace(value, "[redacted]")
    return redacted

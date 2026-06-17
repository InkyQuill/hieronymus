from hieronymus.provider_config import ProviderCatalog, ProviderProfile
from hieronymus.secrets import redact_configured_secret_values


def test_redact_configured_secret_values_replaces_longer_prefix_first():
    provider_catalog = ProviderCatalog(
        providers={
            "openai": ProviderProfile(type="openai", key="secret"),
            "gemini": ProviderProfile(type="google", key="secret-suffix"),
        }
    )

    redacted = redact_configured_secret_values(
        "provider returned secret-suffix and secret",
        provider_catalog,
    )

    assert redacted == "provider returned [redacted] and [redacted]"
    assert "suffix" not in redacted


def test_redact_configured_secret_values_ignores_short_api_keys():
    provider_catalog = ProviderCatalog(
        providers={
            "openai": ProviderProfile(type="openai", key="abc"),
        }
    )

    redacted = redact_configured_secret_values("provider returned abc", provider_catalog)

    assert redacted == "provider returned abc"

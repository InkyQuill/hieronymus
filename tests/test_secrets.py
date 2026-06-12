from hieronymus.dream_config import ProviderProfile, default_dream_config
from hieronymus.secrets import redact_configured_secret_values


def test_redact_configured_secret_values_replaces_longer_prefix_first():
    dream_config = (
        default_dream_config()
        .with_provider(
            "openai",
            ProviderProfile(type="openai", api_key="secret"),
        )
        .with_provider(
            "gemini",
            ProviderProfile(type="gemini", api_key="secret-suffix"),
        )
    )

    redacted = redact_configured_secret_values(
        "provider returned secret-suffix and secret",
        dream_config,
    )

    assert redacted == "provider returned [redacted] and [redacted]"
    assert "suffix" not in redacted

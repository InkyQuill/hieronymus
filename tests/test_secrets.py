from hieronymus.config import HieronymusConfig
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import ProviderSettings, load_settings


def test_redact_configured_secret_values_replaces_longer_prefix_first(tmp_path, monkeypatch):
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = (
        load_settings(config)
        .with_provider(
            "openai",
            ProviderSettings(api_key_env="SHORT_SECRET"),
        )
        .with_provider(
            "gemini",
            ProviderSettings(api_key_env="LONG_SECRET"),
        )
    )
    monkeypatch.setenv("SHORT_SECRET", "secret")
    monkeypatch.setenv("LONG_SECRET", "secret-suffix")

    redacted = redact_configured_secret_values(
        "provider returned secret-suffix and secret",
        settings,
    )

    assert redacted == "provider returned [redacted] and [redacted]"
    assert "suffix" not in redacted

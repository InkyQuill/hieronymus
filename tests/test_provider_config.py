from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderCatalogError,
    ProviderDefaults,
    ProviderProfile,
    load_provider_catalog,
    save_provider_catalog,
    validate_provider_catalog,
)


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def test_load_provider_catalog_defaults_when_missing(tmp_path: Path) -> None:
    catalog = load_provider_catalog(_config(tmp_path))

    assert catalog.providers == {}
    assert catalog.defaults == ProviderDefaults(provider="", model="")


def test_save_and_load_provider_catalog_round_trips_profiles_and_defaults(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "deepseek-api": ProviderProfile(
                    name="Deepseek",
                    type="openai",
                    url="https://api.deepseek.com",
                    key="raw-secret",
                    timeout_seconds=45.0,
                ),
                "local-ollama": ProviderProfile(
                    name="Ollama",
                    type="openai",
                    url="http://127.0.0.1:6000/v1",
                    key="",
                    timeout_seconds=30.0,
                ),
            },
            defaults=ProviderDefaults(
                provider="deepseek-api",
                model="deepseek-v4-flash",
            ),
        ),
    )

    assert load_provider_catalog(config) == ProviderCatalog(
        providers={
            "deepseek-api": ProviderProfile(
                name="Deepseek",
                type="openai",
                url="https://api.deepseek.com",
                key="raw-secret",
                timeout_seconds=45.0,
            ),
            "local-ollama": ProviderProfile(
                name="Ollama",
                type="openai",
                url="http://127.0.0.1:6000/v1",
                key="",
                timeout_seconds=30.0,
            ),
        },
        defaults=ProviderDefaults(
            provider="deepseek-api",
            model="deepseek-v4-flash",
        ),
    )


def test_provider_catalog_validates_default_provider_exists() -> None:
    with pytest.raises(ProviderCatalogError, match="default provider is missing"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={},
                defaults=ProviderDefaults(provider="deepseek-api", model="deepseek-v4-flash"),
            ),
        )


def test_provider_catalog_rejects_unknown_provider_type() -> None:
    with pytest.raises(ProviderCatalogError, match="unsupported provider type"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={
                    "bad": ProviderProfile(
                        name="Bad",
                        type="made-up",
                        url="https://example.test",
                    )
                },
                defaults=ProviderDefaults(),
            ),
        )

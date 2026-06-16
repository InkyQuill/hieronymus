import tomllib
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


def _write_provider_config(config: HieronymusConfig, raw_config: str) -> None:
    config.config_root.mkdir(parents=True)
    config.provider_config_path.write_text(raw_config, encoding="utf-8")


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

    raw = config.provider_config_path.read_text(encoding="utf-8")
    assert "[deepseek-api]" in raw
    assert "[local-ollama]" in raw
    assert "[defaults]" in raw
    assert "[providers." not in raw
    payload = tomllib.loads(raw)
    assert "providers" not in payload
    assert payload["deepseek-api"]["key"] == "raw-secret"

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


def test_load_provider_catalog_defaults_provider_name_to_table_id(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    _write_provider_config(
        config,
        "[deepseek]\n"
        'type = "openai"\n'
        'url = "https://api.deepseek.com"\n',
    )

    assert load_provider_catalog(config).providers["deepseek"] == ProviderProfile(
        name="deepseek",
        type="openai",
        url="https://api.deepseek.com",
    )


@pytest.mark.parametrize("provider_id", ["", "defaults"])
def test_provider_catalog_rejects_invalid_provider_ids(provider_id: str) -> None:
    with pytest.raises(ProviderCatalogError, match="invalid provider id"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={
                    provider_id: ProviderProfile(
                        name="Bad",
                        type="openai",
                        url="https://example.test",
                    )
                },
                defaults=ProviderDefaults(),
            ),
        )


def test_provider_catalog_allows_default_model_without_default_provider() -> None:
    assert validate_provider_catalog(
        ProviderCatalog(
            providers={},
            defaults=ProviderDefaults(provider="", model="deepseek-v4-flash"),
        ),
    ) == ProviderCatalog(
        providers={},
        defaults=ProviderDefaults(provider="", model="deepseek-v4-flash"),
    )

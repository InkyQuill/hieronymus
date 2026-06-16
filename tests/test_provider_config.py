import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderCatalogError,
    ProviderDefaults,
    ProviderProfile,
    default_provider_catalog,
    load_provider_catalog,
    redacted_provider_catalog_payload,
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


def test_default_provider_catalog_returns_empty_providers_and_defaults() -> None:
    assert default_provider_catalog() == ProviderCatalog(
        providers={},
        defaults=ProviderDefaults(provider="", model=""),
    )


def test_load_provider_catalog_rejects_invalid_toml(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _write_provider_config(config, "[deepseek\n")

    with pytest.raises(ProviderCatalogError, match="provider.conf is not valid TOML"):
        load_provider_catalog(config)


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


def test_redacted_provider_catalog_payload_redacts_keys() -> None:
    assert redacted_provider_catalog_payload(
        ProviderCatalog(
            providers={
                "deepseek-api": ProviderProfile(
                    name="Deepseek",
                    type="openai",
                    url="https://api.deepseek.com",
                    key="raw-secret",
                ),
                "local-ollama": ProviderProfile(
                    name="Ollama",
                    type="ollama",
                    url="http://127.0.0.1:11434",
                    key="",
                ),
            },
            defaults=ProviderDefaults(provider="deepseek-api", model="deepseek-chat"),
        ),
    ) == {
        "deepseek-api": {
            "name": "Deepseek",
            "type": "openai",
            "url": "https://api.deepseek.com",
            "key": "***",
            "timeout_seconds": 30.0,
        },
        "local-ollama": {
            "name": "Ollama",
            "type": "ollama",
            "url": "http://127.0.0.1:11434",
            "key": "",
            "timeout_seconds": 30.0,
        },
        "defaults": {"provider": "deepseek-api", "model": "deepseek-chat"},
    }


def test_provider_catalog_validates_default_provider_exists() -> None:
    with pytest.raises(ProviderCatalogError, match="default provider is missing"):
        validate_provider_catalog(
            ProviderCatalog(
                providers={},
                defaults=ProviderDefaults(provider="deepseek-api", model="deepseek-v4-flash"),
            ),
        )


@pytest.mark.parametrize("provider_type", ["openai", "google", "anthropic", "ollama"])
def test_provider_catalog_accepts_supported_provider_types(provider_type: str) -> None:
    assert validate_provider_catalog(
        ProviderCatalog(
            providers={
                provider_type: ProviderProfile(
                    name=provider_type,
                    type=provider_type,
                    url="https://example.test",
                ),
            },
            defaults=ProviderDefaults(),
        ),
    ) == ProviderCatalog(
        providers={
            provider_type: ProviderProfile(
                name=provider_type,
                type=provider_type,
                url="https://example.test",
            ),
        },
        defaults=ProviderDefaults(),
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


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        ("[deepseek]\nurl = \"https://api.deepseek.com\"\n", "deepseek.type is required"),
        ("[deepseek]\ntype = \"openai\"\n", "deepseek.url is required"),
    ],
)
def test_load_provider_catalog_rejects_missing_required_provider_fields(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = _config(tmp_path)
    _write_provider_config(config, raw_config)

    with pytest.raises(ProviderCatalogError, match=error):
        load_provider_catalog(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            'extra = "nope"\n',
            "unknown provider config setting: deepseek.extra",
        ),
        (
            "[defaults]\n"
            'provider = ""\n'
            'model = ""\n'
            'extra = "nope"\n',
            "unknown provider config setting: defaults.extra",
        ),
    ],
)
def test_load_provider_catalog_rejects_unknown_keys(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = _config(tmp_path)
    _write_provider_config(config, raw_config)

    with pytest.raises(ProviderCatalogError, match=error):
        load_provider_catalog(config)


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


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[deepseek]\n"
            "name = 1\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n',
            "providers.deepseek.name must be a string",
        ),
        (
            "[deepseek]\n"
            "type = 1\n"
            'url = "https://api.deepseek.com"\n',
            "providers.deepseek.type must be a string",
        ),
        (
            "[deepseek]\n"
            'type = "openai"\n'
            "url = 1\n",
            "providers.deepseek.url must be a string",
        ),
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            "key = 1\n",
            "providers.deepseek.key must be a string",
        ),
        (
            "[defaults]\n"
            "provider = 1\n",
            "defaults.provider must be a string",
        ),
        (
            "[defaults]\n"
            "model = 1\n",
            "defaults.model must be a string",
        ),
    ],
)
def test_load_provider_catalog_rejects_field_type_mismatches(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = _config(tmp_path)
    _write_provider_config(config, raw_config)

    with pytest.raises(ProviderCatalogError, match=error):
        load_provider_catalog(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            'timeout_seconds = "slow"\n',
            "providers.deepseek.timeout_seconds must be a number",
        ),
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            "timeout_seconds = 0\n",
            "providers.deepseek.timeout_seconds must be greater than 0",
        ),
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            "timeout_seconds = -1\n",
            "providers.deepseek.timeout_seconds must be greater than 0",
        ),
        (
            "[deepseek]\n"
            'type = "openai"\n'
            'url = "https://api.deepseek.com"\n'
            "timeout_seconds = inf\n",
            "providers.deepseek.timeout_seconds must be finite and greater than 0",
        ),
    ],
)
def test_load_provider_catalog_rejects_invalid_timeout_values(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = _config(tmp_path)
    _write_provider_config(config, raw_config)

    with pytest.raises(ProviderCatalogError, match=error):
        load_provider_catalog(config)

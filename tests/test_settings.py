from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.settings import (
    DreamingSettings,
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)


def test_load_settings_returns_defaults_when_file_is_missing(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    settings = load_settings(config)

    assert config.settings_path == config.config_root / "settings.toml"
    assert settings.dreaming.active_provider == "deterministic"
    assert settings.dreaming.autostart_enabled is False
    assert settings.dreaming.min_interval_minutes == 30
    assert settings.dreaming.new_short_term_memory_threshold == 25
    assert settings.dreaming.max_cycles_per_autostart == 1
    assert settings.providers["deterministic"].enabled is True
    assert settings.providers["openai"].model == "gpt-4.1-mini"
    assert settings.providers["openai"].base_url == "https://api.openai.com/v1"
    assert settings.providers["openai"].api_key_env == "OPENAI_API_KEY"
    assert settings.providers["gemini"].model == "gemini-2.5-flash"
    assert settings.providers["gemini"].api_key_env == "GEMINI_API_KEY"
    assert settings.providers["anthropic"].model == "claude-3-5-haiku-latest"
    assert settings.providers["anthropic"].api_key_env == "ANTHROPIC_API_KEY"


def test_save_settings_writes_toml_without_secret_values(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    settings = load_settings(config).with_provider(
        "openai",
        ProviderSettings(
            enabled=True,
            model="gpt-4.1-mini",
            api_key_env="HIERONYMUS_OPENAI_KEY",
            base_url="https://llm.example.test/v1",
        ),
    )

    save_settings(config, settings)

    raw = config.settings_path.read_text(encoding="utf-8")
    assert "secret" not in raw.lower()
    assert "HIERONYMUS_OPENAI_KEY" in raw
    payload = tomllib.loads(raw)
    assert payload["providers"]["openai"]["enabled"] is True
    assert payload["providers"]["openai"]["base_url"] == "https://llm.example.test/v1"


def test_load_settings_rejects_malformed_toml(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text("[dreaming\n", encoding="utf-8")

    with pytest.raises(SettingsError, match="settings.toml is not valid TOML"):
        load_settings(config)


def test_load_settings_rejects_invalid_active_provider(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text(
        "[dreaming]\nactive_provider = 'missing'\n",
        encoding="utf-8",
    )

    with pytest.raises(SettingsError, match="active provider is not configured: missing"):
        load_settings(config)


def test_load_settings_rejects_non_positive_dreaming_values(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.settings_path.write_text(
        "[dreaming]\n"
        "min_interval_minutes = 0\n"
        "new_short_term_memory_threshold = 25\n"
        "max_cycles_per_autostart = 1\n",
        encoding="utf-8",
    )

    with pytest.raises(SettingsError, match="min_interval_minutes must be at least 1"):
        load_settings(config)


def test_settings_to_json_masks_key_source_only(tmp_path: Path) -> None:
    settings = load_settings(HieronymusConfig(data_root=tmp_path / "hieronymus"))

    payload = settings.to_json_dict()

    assert payload["dreaming"]["active_provider"] == "deterministic"
    assert payload["providers"]["openai"]["api_key_env"] == "OPENAI_API_KEY"
    assert "api_key" not in payload["providers"]["openai"]


def test_settings_with_dreaming_returns_replaced_copy(tmp_path: Path) -> None:
    settings = load_settings(HieronymusConfig(data_root=tmp_path / "hieronymus"))
    dreaming = DreamingSettings(
        active_provider="openai",
        autostart_enabled=True,
        min_interval_minutes=45,
        new_short_term_memory_threshold=50,
        max_cycles_per_autostart=2,
    )

    updated = settings.with_dreaming(dreaming)

    assert updated.dreaming == dreaming
    assert settings.dreaming.active_provider == "deterministic"

from pathlib import Path

import pytest
from textual.widgets import DataTable, Input, Static

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui.config_app import HieronymusConfigApp


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def _detail_text(app: HieronymusConfigApp) -> str:
    return str(app.screen.query_one("#config-detail", Static).renderable)


@pytest.mark.anyio
async def test_config_tui_mounts_provider_rows(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test():
        table = app.screen.query_one("#config-table", DataTable)
        labels = [str(table.get_row(row.key)[0]) for row in table.ordered_rows]

    assert {"deterministic", "openai", "gemini", "anthropic"} <= set(labels)


@pytest.mark.anyio
async def test_config_tui_edits_provider_fields_and_saves(
    config: HieronymusConfig,
) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        app.screen.query_one("#provider-model", Input).value = "gpt-4.1"
        app.screen.query_one("#provider-api-key-env", Input).value = "HIERONYMUS_OPENAI_KEY"
        app.screen.query_one("#provider-base-url", Input).value = "https://llm.example.test/v1"
        app.screen.query_one("#provider-timeout-seconds", Input).value = "11.5"
        await pilot.press("s")

    provider = load_settings(config).providers["openai"]
    assert provider.enabled is True
    assert provider.model == "gpt-4.1"
    assert provider.api_key_env == "HIERONYMUS_OPENAI_KEY"
    assert provider.base_url == "https://llm.example.test/v1"
    assert provider.timeout_seconds == 11.5


@pytest.mark.anyio
async def test_config_tui_edits_dreaming_fields_and_saves(
    config: HieronymusConfig,
) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("3")
        app.screen.query_one("#dreaming-active-provider", Input).value = "gemini"
        app.screen.query_one("#dreaming-autostart-enabled", Input).value = "yes"
        app.screen.query_one("#dreaming-min-interval-minutes", Input).value = "9"
        app.screen.query_one("#dreaming-new-short-term-memory-threshold", Input).value = "3"
        app.screen.query_one("#dreaming-max-cycles-per-autostart", Input).value = "2"
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        await pilot.press("s")

    dreaming = load_settings(config).dreaming
    assert dreaming.active_provider == "gemini"
    assert dreaming.autostart_enabled is True
    assert dreaming.min_interval_minutes == 9
    assert dreaming.new_short_term_memory_threshold == 3
    assert dreaming.max_cycles_per_autostart == 2
    assert load_settings(config).providers["gemini"].enabled is True


@pytest.mark.anyio
async def test_config_tui_reload_discards_unsaved_edits(
    config: HieronymusConfig,
) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-model", Input).value = "unsaved-model"
        assert "unsaved" in _detail_text(app)
        await pilot.press("r")
        assert app.screen.query_one("#provider-model", Input).value == "gpt-4.1-mini"
        assert "unsaved" not in _detail_text(app)


@pytest.mark.anyio
async def test_config_tui_validation_failure_does_not_save(
    config: HieronymusConfig,
) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        app.screen.query_one("#dreaming-min-interval-minutes", Input).value = "0"
        await pilot.press("s")
        detail = _detail_text(app)

    assert "min_interval_minutes must be at least 1" in detail
    assert not config.settings_path.exists()


@pytest.mark.anyio
async def test_config_tui_detail_never_shows_raw_api_key_value(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("HIERONYMUS_OPENAI_KEY", "raw-secret-value")
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        app.screen.query_one("#provider-api-key-env", Input).value = "HIERONYMUS_OPENAI_KEY"
        app.screen.query_one("#provider-enabled", Input).value = "yes"
        detail = _detail_text(app)

    assert "HIERONYMUS_OPENAI_KEY" in detail
    assert "api_key_present: yes" in detail
    assert "raw-secret-value" not in detail

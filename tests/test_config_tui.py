from pathlib import Path

import pytest
from textual.widgets import DataTable

from hieronymus.config import HieronymusConfig
from hieronymus.settings import load_settings
from hieronymus.tui.config_app import HieronymusConfigApp


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


@pytest.fixture
def config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


@pytest.mark.anyio
async def test_config_tui_mounts_provider_rows(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test():
        table = app.screen.query_one("#config-table", DataTable)
        labels = [str(table.get_row(row.key)[0]) for row in table.ordered_rows]

    assert {"deterministic", "openai", "gemini", "anthropic"} <= set(labels)


@pytest.mark.anyio
async def test_config_tui_can_save_active_provider(config: HieronymusConfig) -> None:
    app = HieronymusConfigApp(config)

    async with app.run_test() as pilot:
        await pilot.press("2")
        await pilot.press("s")

    assert load_settings(config).dreaming.active_provider == "openai"

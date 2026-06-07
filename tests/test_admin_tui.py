import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui.app import HieronymusAdminApp


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def _plain_text(widget) -> str:
    renderable = widget.render()
    return getattr(renderable, "plain", str(renderable))


def _query(app: HieronymusAdminApp, selector: str):
    return app.screen.query_one(selector)


def _seed(config: HieronymusConfig) -> None:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Keep inventory UI labels compact.",
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        title="Guild Ledger",
        text="Guild ledger detail marker belongs to the second row.",
    )


@pytest.mark.anyio
async def test_tui_starts_with_navigation_stats_table_and_detail(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test():
        assert app.title == "Hieronymus Admin"
        assert _query(app, "#view-tabs").has_focus
        assert "Crystals" in _plain_text(_query(app, "#view-tabs"))
        assert "series 1" in _plain_text(_query(app, "#stats"))
        detail_text = _plain_text(_query(app, "#detail"))
        assert "Inventory UI" in detail_text
        assert "Guild Ledger" not in detail_text
        table = _query(app, "#entries")
        assert table.row_count == 2


@pytest.mark.anyio
async def test_tui_updates_detail_when_table_row_highlight_changes(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        detail_text = _plain_text(_query(app, "#detail"))
        assert "Inventory UI" in detail_text
        assert "Guild Ledger" not in detail_text

        table = _query(app, "#entries")
        table.move_cursor(row=1)
        await pilot.pause()

        detail_text = _plain_text(_query(app, "#detail"))
        assert "Guild Ledger" in detail_text
        assert "Guild ledger detail marker" in detail_text
        assert "Inventory UI" not in detail_text


@pytest.mark.anyio
async def test_tui_switches_views_with_number_keys(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("3")
        assert app.active_view == "Crystals"
        assert _query(app, "#entries").row_count == 2
        await pilot.press("4")
        assert app.active_view == "Lessons"
        assert _query(app, "#entries").row_count == 1
        detail_text = _plain_text(_query(app, "#detail"))
        assert "Inventory UI" in detail_text
        assert "Guild Ledger" not in detail_text

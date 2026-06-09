import pytest
from textual.widgets import Input, ListView, TextArea

from hieronymus.admin import AdminStore
from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui.app import HieronymusAdminApp
from hieronymus.workspace import WorkspaceStore


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def _plain_text(widget) -> str:
    renderable = widget.render()
    return getattr(renderable, "plain", str(renderable))


def _query(app: HieronymusAdminApp, selector: str):
    return app.screen.query_one(selector)


def _command_labels(app: HieronymusAdminApp) -> set[str]:
    return {_plain_text(label) for label in app.screen.query("#command-list Label")}


def _command_labels_ordered(app: HieronymusAdminApp) -> list[str]:
    return [_plain_text(label) for label in app.screen.query("#command-list Label")]


def _seed(config: HieronymusConfig) -> tuple[int, int]:
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
    crystal_store = CrystalStore(config)
    lesson_id = crystal_store.add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Keep inventory UI labels compact.",
    )
    concept_id = crystal_store.add_crystal(
        context,
        crystal_type="concept",
        title="Guild Ledger",
        text="Guild ledger detail marker belongs to the second row.",
    )
    return lesson_id, concept_id


def _seed_proposal(config: HieronymusConfig) -> None:
    ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        concept_text="Sense",
        source_form="センス",
        canonical_rendering="сенс",
        rationale="Palette proposal fixture.",
    )


def _seed_completed_short_term_memory(config: HieronymusConfig) -> None:
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translation",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Keep inventory UI labels compact in dream outputs.",
    )
    workspace.complete_session(session.id)


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


@pytest.mark.anyio
async def test_tui_opens_filter_and_edit_dialogs_from_keyboard(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("f")
        assert app.screen.id == "filter-dialog"

        await pilot.press("escape")
        assert app.screen.id != "filter-dialog"

        await pilot.press("e")
        assert app.screen.id == "edit-dialog"


@pytest.mark.anyio
async def test_tui_command_palette_lists_admin_commands(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("ctrl+p")

        assert app.screen.id == "command-dialog"
        commands = _command_labels(app)
        assert {
            "add",
            "reinforce",
            "edit",
            "delete",
            "merge",
            "split",
            "supersede",
            "inspect provenance",
            "inspect recall reason",
        } <= commands
        assert {"approve", "reject"}.isdisjoint(commands)
        assert "run manual dreaming" not in commands


@pytest.mark.anyio
async def test_tui_command_palette_lists_proposal_commands(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    _seed_proposal(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("7")
        await pilot.press("ctrl+p")

        assert app.screen.id == "command-dialog"
        commands = _command_labels(app)
        assert {"approve", "reject"} <= commands
        assert {"reinforce", "edit", "delete"}.isdisjoint(commands)


@pytest.mark.anyio
@pytest.mark.parametrize(
    ("view_key", "expected"),
    [
        ("1", set()),
        (
            "3",
            {
                "add",
                "edit",
                "delete",
                "merge",
                "split",
                "deprecate",
                "supersede",
                "reinforce",
                "decay",
                "inspect provenance",
                "inspect recall reason",
            },
        ),
        (
            "4",
            {
                "add",
                "edit",
                "delete",
                "merge",
                "split",
                "deprecate",
                "supersede",
                "reinforce",
                "decay",
                "promote local lesson",
                "activate global lesson",
                "inspect provenance",
                "inspect recall reason",
            },
        ),
        ("6", {"run manual dreaming", "review dream outputs"}),
        ("7", {"approve", "reject"}),
    ],
)
async def test_tui_command_palette_commands_are_view_executable(
    config: HieronymusConfig,
    view_key: str,
    expected: set[str],
) -> None:
    _seed(config)
    _seed_proposal(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press(view_key)
        await pilot.press("ctrl+p")

        assert _command_labels(app) == expected


@pytest.mark.anyio
async def test_tui_confirms_destructive_delete_before_mutating(
    config: HieronymusConfig,
) -> None:
    lesson_id, _ = _seed(config)
    app = HieronymusAdminApp(config)
    store = AdminStore(config)
    crystal_store = CrystalStore(config)

    async with app.run_test() as pilot:
        await pilot.press("delete")
        assert app.screen.id == "confirm-dialog"
        await pilot.click("#confirm-cancel")
        await pilot.pause()

        assert crystal_store.get(lesson_id).status == "active"
        assert store.stats().audit_events == 0

        await pilot.press("delete")
        assert app.screen.id == "confirm-dialog"
        await pilot.click("#confirm-confirm")
        await pilot.pause()

        assert crystal_store.get(lesson_id).status == "archived"
        assert store.stats().audit_events == 1
        detail_text = _plain_text(_query(app, "#detail"))
        assert "archived" in detail_text


@pytest.mark.anyio
async def test_tui_edit_save_updates_crystal_and_refreshes_detail(
    config: HieronymusConfig,
) -> None:
    _, concept_id = _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        table = _query(app, "#entries")
        table.move_cursor(row=1)
        await pilot.pause()

        await pilot.press("e")
        assert app.screen.id == "edit-dialog"
        assert app.screen.query_one("#edit-title", Input).value == "Guild Ledger"
        text_area = app.screen.query_one("#edit-text", TextArea)
        assert text_area.text == "Guild ledger detail marker belongs to the second row."
        assert not text_area.text.startswith("Guild Ledger\n\n")

        app.screen.query_one("#edit-title", Input).value = "Guild Ledger Notes"
        text_area.load_text("Keep guild ledger terminology stable.")
        await pilot.click("#edit-save")
        await pilot.pause()

        record = CrystalStore(config).get(concept_id)
        assert record.title == "Guild Ledger Notes"
        assert record.text == "Keep guild ledger terminology stable."
        detail_text = _plain_text(_query(app, "#detail"))
        assert "Guild Ledger Notes" in detail_text
        assert "Keep guild ledger terminology stable." in detail_text
        assert "Inventory UI" not in detail_text


@pytest.mark.anyio
async def test_tui_command_palette_dispatches_supported_crystal_command(
    config: HieronymusConfig,
) -> None:
    lesson_id, _ = _seed(config)
    app = HieronymusAdminApp(config)
    store = AdminStore(config)
    crystal_store = CrystalStore(config)

    before = crystal_store.get(lesson_id)
    async with app.run_test() as pilot:
        await pilot.press("ctrl+p")
        assert app.screen.id == "command-dialog"
        command_list = app.screen.query_one("#command-list", ListView)
        command_list.index = _command_labels_ordered(app).index("reinforce")
        await pilot.press("enter")
        await pilot.pause()

        after = crystal_store.get(lesson_id)
        assert after.strength > before.strength
        assert after.confidence > before.confidence
        assert store.stats().audit_events == 1


@pytest.mark.anyio
async def test_lessons_view_clears_stale_type_filter_in_dialog(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        app.screen.filters = {"type": "concept"}
        await pilot.press("4")
        await pilot.pause()

        assert app.active_view == "Lessons"
        assert _query(app, "#entries").row_count == 1
        detail_text = _plain_text(_query(app, "#detail"))
        assert "Inventory UI" in detail_text
        assert "Guild Ledger" not in detail_text

        await pilot.press("f")
        assert app.screen.id == "filter-dialog"
        assert len(app.screen.query("#filter-type")) == 0


@pytest.mark.anyio
async def test_tui_add_command_creates_audited_crystal(config: HieronymusConfig) -> None:
    _seed(config)
    app = HieronymusAdminApp(config)
    store = AdminStore(config)

    async with app.run_test() as pilot:
        app.screen._dispatch_command("add")
        await pilot.pause()

        assert app.screen.id == "form-dialog"
        app.screen.dismiss(
            {
                "series_slug": "only-sense-online",
                "source_language": "ja",
                "target_language": "ru",
                "type": "concept",
                "title": "Added Note",
                "text": "Added from the admin TUI.",
                "tags": "",
            }
        )
        await pilot.pause()

        assert store.stats().crystals == 3
        assert store.stats().audit_events == 1
        assert "Added Note" in _plain_text(_query(app, "#detail"))


@pytest.mark.anyio
async def test_tui_filters_proposals_by_status(config: HieronymusConfig) -> None:
    _seed(config)
    _seed_proposal(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("7")
        await pilot.press("f")
        assert app.screen.id == "filter-dialog"
        app.screen.dismiss({"status": "approved"})
        await pilot.pause()

        assert _query(app, "#entries").row_count == 0


@pytest.mark.anyio
async def test_tui_manual_dream_command_switches_to_dream_runs(
    config: HieronymusConfig,
) -> None:
    _seed(config)
    _seed_completed_short_term_memory(config)
    app = HieronymusAdminApp(config)

    async with app.run_test() as pilot:
        await pilot.press("6")
        await pilot.press("ctrl+p")
        command_list = app.screen.query_one("#command-list", ListView)
        command_list.index = _command_labels_ordered(app).index("run manual dreaming")
        await pilot.press("enter")
        await pilot.pause()

        assert app.active_view == "Dream Runs"
        assert _query(app, "#entries").row_count == 1

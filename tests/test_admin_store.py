import pytest

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        volume="01",
        chapter="002",
        tags=("style",),
    )


def test_status_payload_reports_admin_counts(config: HieronymusConfig) -> None:
    context = _context(config)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
        strength=0.75,
        confidence=0.8,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Prefer concise labels in menu chrome.",
    )
    archived_memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Archived short-term note.",
    )
    with connect(config.database_path) as conn:
        conn.execute(
            """
            update short_term_memories
            set archived_at = '2026-06-07T00:00:00+00:00'
            where id = ?
            """,
            (archived_memory_id,),
        )
        conn.commit()

    payload = AdminStore(config).status_payload()

    assert payload["counts"]["series"] == 1
    assert payload["counts"]["crystals"] == 1
    assert payload["counts"]["lessons"] == 1
    assert payload["counts"]["short_term_memories"] == 1
    assert payload["counts"]["sessions"] == 1
    assert payload["counts"]["pending_proposals"] == 0
    assert payload["service"]["running"] is False


def test_list_crystals_filters_by_series_type_status_and_tags(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    other = TranslationContext(
        series_slug="another-series",
        source_language="ja",
        target_language="ru",
        task_type="translation",
    )
    Registry(config).create_series(
        slug="another-series",
        title="Another Series",
        source_language="ja",
        target_language="ru",
    )
    store = CrystalStore(config)
    wanted_id = store.add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
        strength=0.75,
        confidence=0.8,
    )
    store.add_crystal(other, crystal_type="erudition", title="Other", text="Other note.")

    rows = AdminStore(config).list_crystals(
        series_slug="only-sense-online",
        crystal_type="lesson",
        status="active",
        tags=("style",),
    )

    assert [row.id for row in rows] == [wanted_id]
    assert rows[0].label == "Inventory UI"
    assert rows[0].quality_label == "80% conf / 75% str"


def test_view_snapshot_contains_rows_selection_detail_and_filter_labels(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Use compact Russian nouns for inventory UI labels.",
    )

    snapshot = AdminStore(config).snapshot("Crystals", selected_id=crystal_id)

    assert snapshot.view == "Crystals"
    assert snapshot.rows[0].id == crystal_id
    assert snapshot.selected.id == crystal_id
    assert "Inventory UI" in snapshot.detail.body
    assert snapshot.filters == []


def test_view_snapshot_selects_row_when_selected_id_is_string(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="First Crystal",
        text="First crystal body.",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Second Crystal",
        text="Second crystal body.",
    )

    snapshot = AdminStore(config).snapshot("Crystals", selected_id=str(crystal_id))

    assert snapshot.selected is not None
    assert snapshot.selected.id == crystal_id
    assert "Second Crystal" in snapshot.detail.body


def test_status_payload_counts_memory_events_for_audit_log_fallback(
    config: HieronymusConfig,
) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        conn.execute(
            """
            insert into memory_events(
              event_type,
              source_role,
              evidence,
              created_at
            )
            values ('activation', 'assistant', 'Applied crystal', '2026-06-07T00:00:00+00:00')
            """
        )
        conn.commit()

    assert store.status_payload()["counts"]["audit_events"] == 1
    assert len(store.snapshot("Audit Log").rows) == 1


@pytest.mark.parametrize("view", ADMIN_VIEWS)
def test_snapshot_smoke_for_admin_views(config: HieronymusConfig, view: str) -> None:
    snapshot = AdminStore(config).snapshot(view)

    assert snapshot.view == view

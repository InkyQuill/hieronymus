from pathlib import Path

import pytest

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui_bridge.admin_api import AdminBridge


def _config(tmp_path: Path) -> HieronymusConfig:
    return HieronymusConfig(data_root=tmp_path / "hieronymus")


def _seed(config: HieronymusConfig) -> int:
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
    return CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        title="Guild Ledger",
        text="Guild ledger detail marker.",
    )


def test_admin_bootstrap_returns_views_stats_and_initial_snapshot(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    payload = AdminBridge(config).bootstrap({})

    assert payload["views"] == list(ADMIN_VIEWS)
    assert payload["default_view"] == "Crystals"
    assert payload["stats"]["series"] == 1
    assert payload["snapshot"]["view"] == "Crystals"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger"


def test_admin_snapshot_filters_crystal_status(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)
    AdminStore(config).delete_crystal(crystal_id, evidence="test")

    payload = AdminBridge(config).snapshot({"view": "Crystals", "filters": {"status": "active"}})

    assert payload["snapshot"]["rows"] == []
    assert payload["snapshot"]["filters"] == ["status=active"]


def test_admin_delete_requires_confirmation(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    with pytest.raises(ValueError, match="delete requires confirmation"):
        AdminBridge(config).delete_crystal({"id": crystal_id, "confirmed": False})


def test_admin_delete_mutates_through_store_and_refreshes_snapshot(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).delete_crystal({"id": crystal_id, "confirmed": True})

    assert payload["result"]["action"] == "delete"
    assert CrystalStore(config).get(crystal_id).status == "archived"
    assert payload["snapshot"]["selected"]["status"] == "archived"


def test_admin_edit_crystal_refreshes_selected_detail(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).edit_crystal(
        {"id": crystal_id, "title": "Guild Ledger Notes", "text": "Keep term stable."}
    )

    assert payload["result"]["message"] == "Crystal edited"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger Notes"
    assert payload["snapshot"]["detail"]["body"] == "Keep term stable."


def test_admin_proposal_approval_refreshes_proposal_view(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    proposal_id = ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        concept_text="Sense",
        source_form="センス",
        canonical_rendering="сенс",
        rationale="Palette proposal fixture.",
    )

    payload = AdminBridge(config).approve_proposal({"id": proposal_id})

    assert payload["result"]["entity_type"] == "strict_term"
    assert payload["snapshot"]["view"] == "Proposals"
    assert payload["snapshot"]["selected"]["status"] == "approved"

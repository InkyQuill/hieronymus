from pathlib import Path

import pytest

from hieronymus.admin import ADMIN_VIEW_KEYS, ADMIN_VIEWS, AdminStore
from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dreaming import DreamRunRecord
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.workspace import WorkspaceStore


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
    assert payload["view_keys"] == list(ADMIN_VIEW_KEYS)
    assert payload["view_labels"]["concepts"] == "Concepts"
    assert payload["view_labels"]["dream_audits"] == "Dream Audits"
    assert payload["default_view"] == "Crystals"
    assert payload["default_view_key"] == "crystals"
    assert payload["stats"]["series"] == 1
    assert payload["short_term_status"]["pending_count"] == 0
    assert payload["dream_status"]["state"] in {"IDLE", "WORKING", "DISABLED"}
    assert payload["config_editor"]["providers"]
    assert payload["config_editor"]["workflows"]
    assert "general" in payload["config_editor"]["prompts"]
    assert "max_pending_short_term_memories" in payload["config_editor"]["thresholds"]
    assert payload["snapshot"]["view"] == "Crystals"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger"


def test_admin_bridge_exposes_memory_and_dream_status(config: HieronymusConfig) -> None:
    api = AdminBridge(config)
    payload = api.dashboard()

    assert "short_term_status" in payload
    assert "dream_status" in payload
    assert payload["dream_status"]["state"] in {"IDLE", "WORKING", "DISABLED"}
    assert "Concepts" in payload["views"]
    assert "Dream Audits" in payload["views"]
    assert "concepts" in payload["view_keys"]
    assert "dream_audits" in payload["view_keys"]


def test_admin_bridge_survives_malformed_dream_config(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    _seed(config)
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text("[dreaming\n", encoding="utf-8")
    api = AdminBridge(config)

    bootstrap = api.bootstrap({})
    snapshot = api.snapshot({"view": "Crystals"})

    assert bootstrap["snapshot"]["selected"]["label"] == "Guild Ledger"
    assert bootstrap["dream_status"]["state"] == "DISABLED"
    assert "dream.conf is not valid TOML" in bootstrap["dream_status"]["reason"]
    assert "dream.conf is not valid TOML" in bootstrap["dream_config_error"]
    assert snapshot["snapshot"]["selected"]["label"] == "Guild Ledger"
    assert snapshot["dream_status"]["state"] == "DISABLED"
    assert "dream.conf is not valid TOML" in snapshot["dream_config_error"]


def test_admin_snapshot_accepts_machine_view_key(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    payload = AdminBridge(config).snapshot({"view": "crystals"})

    assert payload["snapshot"]["view"] == "Crystals"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger"


def test_admin_snapshot_filters_crystal_status(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)
    AdminStore(config).delete_crystal(crystal_id, evidence="test")

    payload = AdminBridge(config).snapshot({"view": "Crystals", "filters": {"status": "active"}})

    assert payload["snapshot"]["rows"] == []
    assert payload["snapshot"]["filters"] == ["status=active"]


def test_admin_snapshot_accepts_type_filter_alias(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    series = Registry(config).create_series(
        slug="another-series",
        title="Another Series",
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
        title="Lesson Row",
        text="Lesson row marker.",
    )

    payload = AdminBridge(config).snapshot({"view": "Crystals", "filters": {"type": "lesson"}})

    assert payload["snapshot"]["filters"] == ["type=lesson"]
    assert [row["label"] for row in payload["snapshot"]["rows"]] == ["Lesson Row"]


def test_admin_snapshot_filters_crystals_by_series_slug(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    series = Registry(config).create_series(
        slug="another-series",
        title="Another Series",
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
        crystal_type="concept",
        title="Other Ledger",
        text="Other series marker.",
    )

    payload = AdminBridge(config).snapshot(
        {"view": "Crystals", "filters": {"series_slug": "another-series"}}
    )

    assert payload["snapshot"]["filters"] == ["series_slug=another-series"]
    assert [row["label"] for row in payload["snapshot"]["rows"]] == ["Other Ledger"]


def test_admin_snapshot_crystal_filter_finds_matches_after_unfiltered_page(
    tmp_path: Path,
) -> None:
    config = _config(tmp_path)
    _seed(config)
    store = CrystalStore(config)
    filler_context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translation",
    )
    for index in range(200):
        store.add_crystal(
            filler_context,
            crystal_type="concept",
            title=f"Filler {index:03d}",
            text="Filler crystal marker.",
        )
    series = Registry(config).create_series(
        slug="late-series",
        title="Late Series",
        source_language="ja",
        target_language="ru",
    )
    late_context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )
    store.add_crystal(
        late_context,
        crystal_type="concept",
        title="Late Ledger",
        text="Late body marker.",
    )

    payload = AdminBridge(config).snapshot(
        {"view": "Crystals", "filters": {"series_slug": "late-series"}}
    )

    assert [row["label"] for row in payload["snapshot"]["rows"]] == ["Late Ledger"]
    assert payload["snapshot"]["selected"]["label"] == "Late Ledger"
    assert payload["snapshot"]["detail"]["title"] == "Late Ledger"
    assert payload["snapshot"]["detail"]["body"] == "Late body marker."
    assert ["Series", "late-series"] in payload["snapshot"]["detail"]["fields"]


def test_admin_snapshot_filters_crystals_by_tags(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translation",
        tags=("glossary", "reviewed"),
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        title="Tagged Ledger",
        text="Tagged ledger marker.",
    )

    payload = AdminBridge(config).snapshot(
        {"view": "Crystals", "filters": {"tags": ["glossary", "reviewed"]}}
    )

    assert payload["snapshot"]["filters"] == ["tags=glossary,reviewed"]
    assert [row["label"] for row in payload["snapshot"]["rows"]] == ["Tagged Ledger"]


def test_admin_snapshot_splits_comma_separated_tag_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translation",
        tags=("glossary", "reviewed"),
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        title="Comma Tagged Ledger",
        text="Comma tagged ledger marker.",
    )

    payload = AdminBridge(config).snapshot(
        {"view": "Crystals", "filters": {"tags": "glossary, reviewed"}}
    )

    assert payload["snapshot"]["filters"] == ["tags=glossary,reviewed"]
    assert [row["label"] for row in payload["snapshot"]["rows"]] == ["Comma Tagged Ledger"]


def test_admin_lessons_snapshot_rejects_type_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Lessons: type"):
        AdminBridge(config).snapshot({"view": "Lessons", "filters": {"type": "concept"}})


def test_admin_crystal_snapshot_rejects_confidence_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Crystals: confidence"):
        AdminBridge(config).snapshot({"view": "Crystals", "filters": {"confidence": "60"}})


def test_admin_crystal_snapshot_rejects_language_pair_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Crystals: language_pair"):
        AdminBridge(config).snapshot({"view": "Crystals", "filters": {"language_pair": "ja->ru"}})


def test_admin_lesson_snapshot_rejects_strength_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Lessons: strength"):
        AdminBridge(config).snapshot({"view": "Lessons", "filters": {"strength": "60%"}})


def test_admin_proposals_snapshot_rejects_series_slug_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Proposals: series_slug"):
        AdminBridge(config).snapshot(
            {"view": "Proposals", "filters": {"series_slug": "only-sense-online"}}
        )


def test_admin_proposals_snapshot_rejects_confidence_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Proposals: confidence"):
        AdminBridge(config).snapshot({"view": "Proposals", "filters": {"confidence": "60"}})


def test_admin_proposals_snapshot_rejects_status_filter(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Proposals: status"):
        AdminBridge(config).snapshot({"view": "Proposals", "filters": {"status": "pending"}})


def test_admin_snapshot_rejects_unknown_filter_key(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter: unknown"):
        AdminBridge(config).snapshot({"view": "Crystals", "filters": {"unknown": "value"}})


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


def test_admin_delete_validates_filters_before_mutating(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    with pytest.raises(ValueError, match="unsupported admin filter for Crystals: confidence"):
        AdminBridge(config).delete_crystal(
            {
                "id": crystal_id,
                "confirmed": True,
                "filters": {"confidence": "60"},
            }
        )

    assert CrystalStore(config).get(crystal_id).status == "active"


def test_admin_delete_validates_view_before_mutating(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    with pytest.raises(ValueError, match="unsupported admin view: Bad"):
        AdminBridge(config).delete_crystal({"id": crystal_id, "confirmed": True, "view": "Bad"})

    assert CrystalStore(config).get(crystal_id).status == "active"


def test_admin_edit_crystal_refreshes_selected_detail(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).edit_crystal(
        {"id": crystal_id, "title": "Guild Ledger Notes", "text": "Keep term stable."}
    )

    assert payload["result"]["message"] == "Crystal edited"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger Notes"
    assert payload["snapshot"]["detail"]["body"] == "Keep term stable."


def test_admin_crystal_mutation_preserves_lessons_view(tmp_path: Path) -> None:
    config = _config(tmp_path)
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
    lesson_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Lesson Ledger",
        text="Lesson ledger detail marker.",
    )

    payload = AdminBridge(config).reinforce_crystal({"id": lesson_id, "view": "Lessons"})

    assert payload["snapshot"]["view"] == "Lessons"
    assert payload["snapshot"]["selected"]["label"] == "Lesson Ledger"


def test_admin_empty_evidence_uses_default(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    AdminBridge(config).reinforce_crystal({"id": crystal_id, "evidence": "  "})

    with connect(config.database_path) as conn:
        event = conn.execute("select evidence from memory_events").fetchone()
    assert event["evidence"] == "Reinforced from admin bridge"


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

    assert payload["result"]["entity_type"] == "crystal"
    assert payload["snapshot"]["view"] == "Proposals"
    assert payload["snapshot"]["selected"]["status"] == "approved"


def test_admin_proposal_approval_rejects_filtered_refresh(tmp_path: Path) -> None:
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

    with pytest.raises(ValueError, match="unsupported admin filter for Proposals: status"):
        AdminBridge(config).approve_proposal({"id": proposal_id, "filters": {"status": "pending"}})


def test_admin_add_crystal_accepts_type_alias(tmp_path: Path) -> None:
    config = _config(tmp_path)
    _seed(config)

    payload = AdminBridge(config).add_crystal(
        {
            "series_slug": "only-sense-online",
            "source_language": "ja",
            "target_language": "ru",
            "type": "concept",
            "title": "Guild Register",
            "text": "Use guild register for ledger adjacent notes.",
        }
    )

    assert payload["result"]["action"] == "add"
    assert payload["snapshot"]["selected"]["kind"] == "concept"
    assert payload["snapshot"]["selected"]["label"] == "Guild Register"


def test_admin_split_crystal_accepts_named_part_params(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).split_crystal(
        {
            "id": crystal_id,
            "part_one_title": "Guild Ledger Term",
            "part_one_text": "Keep guild ledger as the accounting term.",
            "part_two_title": "Guild Ledger Context",
            "part_two_text": "Use context notes for guild accounting scenes.",
        }
    )

    assert payload["result"]["action"] == "split"
    assert payload["snapshot"]["selected"]["label"] == "Guild Ledger Term"
    assert CrystalStore(config).get(crystal_id).status == "archived"


def test_admin_provenance_accepts_crystal_id_param(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).provenance({"crystal_id": crystal_id})

    assert payload["provenance"]["title"] == "Guild Ledger"


def test_admin_recall_reasons_accepts_crystal_id_param(tmp_path: Path) -> None:
    config = _config(tmp_path)
    crystal_id = _seed(config)

    payload = AdminBridge(config).recall_reasons({"crystal_id": crystal_id})

    assert payload["reasons"] == []


def test_admin_dream_review_uses_run_id_param(tmp_path: Path) -> None:
    config = _config(tmp_path)

    with pytest.raises(KeyError, match="unknown dream run: 999"):
        AdminBridge(config).dream_review({"run_id": 999})


def test_admin_dream_all_runs_when_no_completed_pending_memory(tmp_path: Path) -> None:
    config = _config(tmp_path)
    bridge = AdminBridge(config)

    payload = bridge.run_manual_dreaming({})

    with connect(config.database_path) as conn:
        row = conn.execute(
            "select * from dream_runs where id = ?",
            (payload["result"]["id"],),
        ).fetchone()
    assert row["status"] == "completed"
    assert row["input_count"] == 0


def test_admin_dream_all_ignores_service_minimum_when_pending_exists(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = _config(tmp_path)
    _seed(config)
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
        text="Keep guild ledger stable.",
    )
    workspace.complete_session(session.id)
    calls: dict[str, object] = {}

    class FakeDreamService:
        def __init__(self, config: HieronymusConfig, provider: object) -> None:
            calls["config"] = config
            calls["provider"] = provider

        def run_all(self, **kwargs: object) -> DreamRunRecord:
            calls["run_all"] = kwargs
            return DreamRunRecord(
                id=123,
                cycle_id=456,
                status="completed",
                provider="fake",
                input_count=1,
                created_crystal_count=0,
                proposal_count=0,
            )

    monkeypatch.setattr("hieronymus.admin.DreamService", FakeDreamService)

    payload = AdminBridge(config).run_manual_dreaming({})

    assert calls["config"] == config
    assert calls["run_all"] == {"owner": "admin", "ignore_minimum": True}
    assert payload["result"]["id"] == 123

import pytest

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dream_audit import DreamAuditStore
from hieronymus.dream_locks import dream_cycle_lock
from hieronymus.dreaming import DreamRunRecord
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
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
    assert payload["header"]["version"].endswith("α")
    assert payload["header"]["version"].startswith("v0.")
    assert payload["service"]["running"] is False
    assert payload["short_term_status"]["pending_count"] == 0
    assert payload["short_term_status"]["urgent"] is False
    assert payload["dream_status"] == {
        "state": "DISABLED",
        "current_phase": "",
        "progress": 0.0,
        "run_id": None,
        "cycle_id": None,
        "owner": "",
        "started_at": "",
    }
    assert "concepts" in payload["view_keys"]
    assert "dream_audits" in payload["view_keys"]


def test_status_payload_survives_malformed_dream_config(
    config: HieronymusConfig,
) -> None:
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text("[dreaming\n", encoding="utf-8")

    payload = AdminStore(config).status_payload()

    assert payload["short_term_status"] == {
        "pending_count": 0,
        "min_pending_short_term_memories": 20,
        "max_pending_short_term_memories": 200,
        "urgent": False,
        "drain_in_progress": False,
        "drain_completed": 0,
        "drain_remaining": 0,
        "drain_total": 0,
        "drain_progress": 0.0,
    }
    assert payload["dream_status"]["state"] == "DISABLED"
    assert payload["dream_status"]["current_phase"] == ""
    assert payload["dream_status"]["progress"] == 0.0
    assert "dream.conf is not valid TOML" in payload["dream_status"]["reason"]
    assert "dream.conf is not valid TOML" in payload["dream_config_error"]


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


def test_status_payload_counts_audit_log_before_memory_event_fallback(
    config: HieronymusConfig,
) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        conn.execute(
            """
            insert into audit_log(
              action,
              entity_type,
              entity_id,
              note,
              created_at
            )
            values ('edit', 'crystal', '1', 'Edited crystal', '2026-06-07T00:00:00+00:00')
            """
        )
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
    row = store.snapshot("Audit Log").rows[0]
    assert row.kind == "edit"
    assert row.label == "Edited crystal"
    assert row.status == "crystal"
    assert row.scope == "1"


@pytest.mark.parametrize("view", ADMIN_VIEWS)
def test_snapshot_smoke_for_admin_views(config: HieronymusConfig, view: str) -> None:
    snapshot = AdminStore(config).snapshot(view)

    assert snapshot.view == view


def test_admin_snapshot_exposes_dream_audit_entries(config: HieronymusConfig) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        cursor = conn.execute(
            """
            insert into dream_runs(cycle_id, status, provider, created_at)
            values (1, 'running', 'test', '2026-06-09T00:00:00+00:00')
            """
        )
        dream_run_id = int(cursor.lastrowid)
        conn.commit()
    audit_id = DreamAuditStore(config).append(
        dream_run_id=dream_run_id,
        phase_run_id=None,
        event_type="provider_request",
        severity="warning",
        summary="sent request",
        payload={
            "model": "claude-test",
            "headers": {"Authorization": "Bearer secret"},
        },
    )

    snapshot = store.snapshot("Dream Audits", selected_id=audit_id)

    assert snapshot.view == "Dream Audits"
    assert snapshot.selected is not None
    assert snapshot.selected.id == audit_id
    assert snapshot.selected.kind == "dream audit"
    assert snapshot.selected.label == "provider_request: sent request"
    assert snapshot.selected.status == "warning"
    assert snapshot.selected.scope == f"dream:{dream_run_id}"
    assert snapshot.detail.title == "provider_request: sent request"
    assert snapshot.detail.subtitle == "warning"
    assert snapshot.detail.fields == (
        ("Dream run", str(dream_run_id)),
        ("Phase run", ""),
        ("Severity", "warning"),
        ("Created", snapshot.selected.quality_label),
    )
    assert snapshot.detail.body == (
        '{\n  "headers": {\n    "Authorization": "[REDACTED]"\n  },\n  "model": "claude-test"\n}'
    )


def test_snapshot_accepts_machine_view_key(config: HieronymusConfig) -> None:
    snapshot = AdminStore(config).snapshot("dream_audits")

    assert snapshot.view == "Dream Audits"


def test_dream_audit_detail_survives_malformed_payload_json(
    config: HieronymusConfig,
) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        dream_run_id = conn.execute(
            """
            insert into dream_runs(cycle_id, status, provider, created_at)
            values (1, 'running', 'test', '2026-06-09T00:00:00+00:00')
            """
        ).lastrowid
        audit_id = conn.execute(
            """
            insert into dream_audit_entries(
              dream_run_id,
              phase_run_id,
              event_type,
              severity,
              summary,
              payload_json,
              created_at
            )
            values (?, null, 'provider_output', 'warning', 'malformed', ?, ?)
            """,
            (dream_run_id, "{not json", "2026-06-09T00:00:00+00:00"),
        ).lastrowid
        conn.commit()

    snapshot = store.snapshot("Dream Audits", selected_id=audit_id)

    assert snapshot.detail.title == "provider_output: malformed"
    assert '"_invalid_json": "{not json"' in snapshot.detail.body


def test_admin_exposes_crystal_provenance_and_recall_reason(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="term-note",
        text="Inventory UI labels should stay compact.",
        source_ref="mentor:v1c2",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Inventory UI labels should stay compact.",
        source_memory_ids=[memory_id],
    )
    RecallService(config).recall(session.id, context, "inventory compact", limit=1)

    admin = AdminStore(config)
    provenance = admin.provenance_for_crystal(crystal_id)
    recall = admin.recall_reasons_for_crystal(crystal_id)

    assert provenance.title == "Inventory UI"
    assert provenance.sources[0]["source_ref"] == "mentor:v1c2"
    assert "Inventory UI labels should stay compact." in provenance.sources[0]["text"]
    assert recall[0]["query"] == "inventory compact"
    assert recall[0]["reason"]


def test_admin_runs_manual_dreaming_and_reviews_outputs(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Keep crafting result messages quiet and precise.",
    )
    workspace.complete_session(session.id)

    admin = AdminStore(config)
    run = admin.run_manual_dreaming()
    review = admin.dream_review(run.id)

    assert run.status == "completed"
    assert run.provider == "deterministic"
    assert review.run_id == run.id
    assert review.consumed_memories == ["Keep crafting result messages quiet and precise."]
    assert review.created_crystals == ["Keep crafting result messages quiet and precise."]
    assert review.failed_outputs == []
    assert review.validation_errors == []
    with connect(config.database_path) as conn:
        audit = conn.execute(
            """
            select note
            from audit_log
            where action = 'run'
              and entity_type = 'dream'
              and entity_id = ?
            """,
            (run.id,),
        ).fetchone()
    assert audit["note"] == f"Manual dream run {run.cycle_id} with provider deterministic"


def test_admin_manual_dreaming_runs_without_completed_pending_memory(
    config: HieronymusConfig,
) -> None:
    admin = AdminStore(config)

    run = admin.run_manual_dreaming()

    with connect(config.database_path) as conn:
        row = conn.execute("select * from dream_runs where id = ?", (run.id,)).fetchone()
    assert row["status"] == "completed"
    assert row["input_count"] == 0


def test_admin_manual_dreaming_uses_ignore_minimum_when_pending_exists(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Manual dreaming should bypass the service minimum.",
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

    run = AdminStore(config).run_manual_dreaming()

    assert calls["config"] == config
    assert calls["run_all"] == {"owner": "admin", "ignore_minimum": True}
    assert run.id == 123


def test_admin_manual_dreaming_uses_shared_cycle_guard(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Pending memory lets the lock guard run.",
    )
    workspace.complete_session(session.id)

    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(ValueError, match="dream cycle already running"):
            AdminStore(config).run_manual_dreaming()


def test_dream_review_excludes_manual_decay_audit_from_unrelated_run(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Manual decay",
        text="Manual decay should not appear in dream review.",
        strength=0.5,
        confidence=0.5,
        status="archived",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Unrelated dream input.",
    )
    workspace.complete_session(session.id)

    admin = AdminStore(config)
    admin.decay_crystal(crystal_id, evidence="Manual correction outside dreaming.")
    run = admin.run_manual_dreaming()

    assert admin.dream_review(run.id).decayed_crystals == []


def test_dream_review_reports_real_cycle_decay_and_excludes_manual_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    decayed_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Real cycle decay",
        text="Real cycle decay should appear in dream review.",
        strength=0.5,
        confidence=0.5,
    )
    manual_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Manual-only decay",
        text="Manual-only audit should not appear in dream review.",
        strength=0.5,
        confidence=0.5,
        status="archived",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Dream input that triggers a real cycle.",
    )
    workspace.complete_session(session.id)

    admin = AdminStore(config)
    admin.decay_crystal(manual_id, evidence="Manual correction outside dreaming.")
    run = admin.run_manual_dreaming()
    review = admin.dream_review(run.id)

    assert decayed_id != manual_id
    assert review.decayed_crystals == ["Real cycle decay"]


def test_dream_review_reports_confidence_only_cycle_decay(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Confidence-only decay",
        text="Confidence-only cycle decay should appear in dream review.",
        strength=0.0,
        confidence=0.5,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Dream input that triggers confidence-only decay.",
    )
    workspace.complete_session(session.id)

    admin = AdminStore(config)
    run = admin.run_manual_dreaming()
    review = admin.dream_review(run.id)

    with connect(config.database_path) as conn:
        event = conn.execute(
            """
            select strength_delta, confidence_delta
            from memory_events
            where crystal_id = ?
              and event_type = 'cycle_decay'
              and cycle_id = ?
            """,
            (crystal_id, run.cycle_id),
        ).fetchone()

    assert event["strength_delta"] == pytest.approx(0.0)
    assert event["confidence_delta"] < 0
    assert review.decayed_crystals == ["Confidence-only decay"]


def test_dream_review_reports_only_cycle_scoped_decay_events(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    included_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Cycle decay",
        text="Cycle-scoped decay should appear.",
        strength=0.5,
        confidence=0.5,
        status="archived",
    )
    excluded_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Other cycle decay",
        text="Other cycle decay should not appear.",
        strength=0.5,
        confidence=0.5,
        status="archived",
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="lesson",
        text="Dream input for scoped review.",
    )
    workspace.complete_session(session.id)
    run = AdminStore(config).run_manual_dreaming()

    with connect(config.database_path) as conn:
        conn.execute(
            """
            insert into memory_events(
              crystal_id,
              session_id,
              event_type,
              source_role,
              evidence,
              strength_delta,
              confidence_delta,
              applied,
              cycle_id,
              created_at
            )
            values (
              ?,
              null,
              'cycle_decay',
              'system',
              'cycle decay',
              -0.1,
              -0.12,
              1,
              ?,
              '2026-06-07T00:00:00+00:00'
            )
            """,
            (included_id, run.cycle_id),
        )
        conn.execute(
            """
            insert into memory_events(
              crystal_id,
              session_id,
              event_type,
              source_role,
              evidence,
              strength_delta,
              confidence_delta,
              applied,
              cycle_id,
              created_at
            )
            values (
              ?,
              null,
              'cycle_decay',
              'system',
              'cycle decay',
              -0.1,
              -0.12,
              1,
              ?,
              '2026-06-07T00:00:01+00:00'
            )
            """,
            (excluded_id, run.cycle_id + 1),
        )
        conn.commit()

    review = AdminStore(config).dream_review(run.id)

    assert review.decayed_crystals == ["Cycle decay"]

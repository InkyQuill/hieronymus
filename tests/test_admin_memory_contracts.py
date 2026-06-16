from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from hieronymus.admin import AdminStore
from hieronymus.concepts import ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dream_audit import DreamAuditStore
from hieronymus.dream_config import default_dream_config
from hieronymus.dreaming import DreamRunRecord, DreamService
from hieronymus.llm_cache import CachedModels, ModelCacheEntry, save_model_cache
from hieronymus.memory_models import TranslationContext
from hieronymus.provider_config import ProviderCatalog, ProviderProfile, save_provider_catalog
from hieronymus.registry import Registry
from hieronymus.scoring import FeedbackStore
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.server import dispatch
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
        semantic_tags=("style",),
    )


def _active_session(config: HieronymusConfig) -> int:
    return WorkspaceStore(config).start_session(_context(config)).id


def test_memory_contracts_expose_header_status_controls_and_config(
    config: HieronymusConfig,
) -> None:
    save_provider_catalog(
        config,
        ProviderCatalog(
            providers={
                "anthropic": ProviderProfile(
                    name="Anthropic",
                    type="anthropic",
                    url="https://api.anthropic.com",
                    key="secret-anthropic",
                ),
            },
        ),
    )
    stale = (datetime.now(UTC) - timedelta(days=3)).isoformat()
    save_model_cache(
        config,
        CachedModels(
            providers={
                "anthropic": ModelCacheEntry(
                    provider="anthropic",
                    models=("other-model",),
                    fetched_at=stale,
                    identity="old",
                )
            }
        ),
    )

    payload = AdminBridge(config).memory_contracts({})

    assert payload["header"]["product"] == "Hieronymus"
    assert payload["header"]["logo"]["alt"] == "Hieronymus feather logo"
    assert payload["crystals"]["actions"] == ["reinforce", "decay"]
    assert "add_user_correction" in payload["short_term_memories"]["actions"]
    assert "merge" in payload["concepts"]["actions"]
    assert payload["short_term_status"]["pending_count"] == 0
    assert payload["dream_status"]["state"] == "DISABLED"
    editor = payload["config_editor"]
    assert "providers" in editor
    assert "workflows" in editor
    assert editor["prompts"]["general"] == default_dream_config().general_prompt
    assert "max_pending_short_term_memories" in editor["thresholds"]
    warning_codes = {warning["code"] for warning in editor["model_cache_warnings"]}
    assert "model_cache_identity_mismatch" in warning_codes
    assert "model_cache_stale" in warning_codes
    assert "workflow_model_not_cached" in warning_codes


def test_audit_list_detail_displays_dream_cycle_with_multiple_phases(
    config: HieronymusConfig,
) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        run_id = int(
            conn.execute(
                """
                insert into dream_runs(cycle_id, status, provider, created_at, completed_at)
                values (10, 'completed', 'deterministic', ?, ?)
                """,
                ("2026-06-10T00:00:00+00:00", "2026-06-10T00:00:10+00:00"),
            ).lastrowid
        )
        crystallization_id = int(
            conn.execute(
                """
                insert into dream_phase_runs(
                  dream_run_id,
                  phase,
                  provider_profile,
                  provider_type,
                  model,
                  status,
                  input_count,
                  output_count,
                  created_at,
                  completed_at
                )
                values (?, 'crystallization', 'deterministic', 'deterministic', '', 'completed',
                        2, 1, ?, ?)
                """,
                (run_id, "2026-06-10T00:00:01+00:00", "2026-06-10T00:00:04+00:00"),
            ).lastrowid
        )
        maintenance_id = int(
            conn.execute(
                """
                insert into dream_phase_runs(
                  dream_run_id,
                  phase,
                  provider_profile,
                  provider_type,
                  model,
                  status,
                  input_count,
                  output_count,
                  created_at,
                  completed_at
                )
                values (?, 'maintenance', 'deterministic', 'deterministic', '', 'completed',
                        0, 2, ?, ?)
                """,
                (run_id, "2026-06-10T00:00:05+00:00", "2026-06-10T00:00:06+00:00"),
            ).lastrowid
        )
        conn.commit()

    audit = DreamAuditStore(config)
    first_audit_id = audit.append(
        dream_run_id=run_id,
        phase_run_id=crystallization_id,
        event_type="phase_completed",
        severity="info",
        summary="completed crystallization phase",
        payload={"phase_name": "crystallization", "accepted_entries": {"crystals": 1}},
    )
    second_audit_id = audit.append(
        dream_run_id=run_id,
        phase_run_id=maintenance_id,
        event_type="phase_completed",
        severity="info",
        summary="completed maintenance phase",
        payload={"phase_name": "maintenance", "decayed_crystals": [12, 13]},
    )

    snapshot = store.snapshot("Dream Audits", selected_id=first_audit_id)
    assert [row.id for row in snapshot.rows] == [second_audit_id, first_audit_id]
    assert {row.label for row in snapshot.rows} == {
        "phase_completed: completed crystallization phase",
        "phase_completed: completed maintenance phase",
    }
    assert snapshot.detail.fields[1] == ("Phase run", str(crystallization_id))
    assert '"phase_name": "crystallization"' in snapshot.detail.body


def test_admin_status_reports_running_maintenance_phase_for_completed_run(
    config: HieronymusConfig,
) -> None:
    store = AdminStore(config)
    with connect(config.database_path) as conn:
        run_id = int(
            conn.execute(
                """
                insert into dream_runs(
                  cycle_id,
                  status,
                  provider,
                  input_count,
                  created_crystal_count,
                  proposal_count,
                  created_at,
                  completed_at
                )
                values (12, 'completed', 'deterministic', 3, 1, 0, ?, ?)
                """,
                ("2026-06-10T00:00:00+00:00", "2026-06-10T00:00:06+00:00"),
            ).lastrowid
        )
        conn.execute(
            """
            insert into dream_phase_runs(
              dream_run_id,
              phase,
              provider_profile,
              provider_type,
              model,
              status,
              input_count,
              output_count,
              created_at,
              completed_at
            )
            values (?, 'crystallization', 'deterministic', 'deterministic', '',
                    'completed', 3, 1, ?, ?)
            """,
            (run_id, "2026-06-10T00:00:01+00:00", "2026-06-10T00:00:05+00:00"),
        )
        conn.execute(
            """
            insert into dream_phase_runs(
              dream_run_id,
              phase,
              provider_profile,
              provider_type,
              model,
              status,
              input_count,
              output_count,
              created_at
            )
            values (?, 'maintenance', 'deterministic', 'deterministic', '',
                    'running', 0, 0, ?)
            """,
            (run_id, "2026-06-10T00:00:07+00:00"),
        )
        conn.commit()

    payload = store.status_payload()

    assert payload["dream_status"] == {
        "state": "WORKING",
        "current_phase": "maintenance",
        "progress": 0.9,
        "run_id": run_id,
        "cycle_id": 12,
        "owner": "",
        "started_at": "",
    }
    assert payload["short_term_status"]["drain_in_progress"] is True
    assert payload["short_term_status"]["drain_completed"] == 3
    assert payload["short_term_status"]["drain_progress"] == 1.0


def test_admin_status_reports_maintenance_during_real_maintenance_path(
    config: HieronymusConfig,
    monkeypatch,
) -> None:
    class EmptyDreamProvider:
        name = "empty"

        def crystallize(self, context, memories):
            return {}

    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Maintenance target",
        text="Maintenance target should be reinforced.",
        strength=0.5,
        confidence=0.5,
    )
    FeedbackStore(config).record(
        crystal_id,
        event_type="used_in_translation",
        source_role="system",
        evidence="sample admin status during maintenance",
    )
    samples: list[dict[str, object]] = []
    original_apply_passive_events = DreamService._apply_passive_events

    def sample_status_during_passive_events(
        self,
        conn,
        cycle_id: int,
        *,
        max_changed_crystals: int,
    ):
        samples.append(AdminStore(self.config).status_payload()["dream_status"])
        return original_apply_passive_events(
            self,
            conn,
            cycle_id,
            max_changed_crystals=max_changed_crystals,
        )

    monkeypatch.setattr(
        DreamService,
        "_apply_passive_events",
        sample_status_during_passive_events,
    )

    run = DreamService(config, EmptyDreamProvider()).run_all(owner="admin")

    assert run.status == "completed"
    assert samples == [
        {
            "state": "WORKING",
            "current_phase": "maintenance",
            "progress": 0.9,
            "run_id": run.id,
            "cycle_id": run.cycle_id,
            "owner": "admin",
            "started_at": samples[0]["started_at"],
        }
    ]
    assert isinstance(samples[0]["started_at"], str)
    assert samples[0]["started_at"]


def test_user_correction_creates_short_term_memory_and_not_rule_crystal(
    config: HieronymusConfig,
) -> None:
    session_id = _active_session(config)

    result = AdminBridge(config).add_user_correction(
        {
            "session_id": session_id,
            "text": "User told me to render センス as сенс in UI labels.",
            "semantic_tags": ["terminology"],
            "rule_intent": "translation_rendering",
        }
    )

    memory_id = result["result"]["entity_id"]
    with connect(config.database_path) as conn:
        memory = conn.execute(
            "select * from short_term_memories where id = ?",
            (memory_id,),
        ).fetchone()
        crystal_count = conn.execute(
            "select count(*) from crystals where crystal_type = 'rule'",
        ).fetchone()[0]
    assert memory["kind"] == "correction"
    assert memory["source_role"] == "user"
    assert memory["source_credibility"] == "user_rule"
    assert memory["rule_intent"] == "translation_rendering"
    assert crystal_count == 0


def test_short_term_list_and_remove_contract(config: HieronymusConfig) -> None:
    session_id = _active_session(config)
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session_id,
        source_role="user",
        kind="lesson",
        text="Keep inventory messages concise.",
    )

    bridge = AdminBridge(config)
    listed = bridge.list_short_term_memories({})["short_term_memories"]
    removed = bridge.remove_short_term_memory({"id": memory_id})
    listed_after = bridge.list_short_term_memories({})["short_term_memories"]

    assert [item["id"] for item in listed] == [memory_id]
    assert removed["result"]["action"] == "remove"
    assert listed_after == []


def test_dream_all_calls_drain_service_even_without_pending_memory(
    config: HieronymusConfig,
    monkeypatch,
) -> None:
    calls: dict[str, object] = {}

    class FakeDreamService:
        def __init__(self, config: HieronymusConfig, provider: object) -> None:
            calls["config"] = config
            calls["provider"] = provider

        def run_all(self, **kwargs: object):
            calls["run_all"] = kwargs
            return DreamRunRecord(
                id=123,
                cycle_id=456,
                status="completed",
                provider="fake",
                input_count=0,
                created_crystal_count=0,
                proposal_count=0,
            )

    monkeypatch.setattr("hieronymus.admin.DreamService", FakeDreamService)

    payload = AdminBridge(config).run_manual_dreaming({})

    assert calls["run_all"] == {"owner": "admin", "ignore_minimum": True}
    assert payload["result"]["id"] == 123


def test_concept_and_facet_admin_commands_call_primitive_store(
    config: HieronymusConfig,
    monkeypatch,
) -> None:
    calls: list[tuple[str, tuple[object, ...], dict[str, object]]] = []

    @dataclass(frozen=True)
    class Concept:
        id: int = 7
        canonical_name: str = "Guild Ledger"
        description: str = "Accounting concept."
        status: str = "candidate"
        confidence: float = 0.4
        scope_type: str = "global"
        scope_key: str = ""
        tags: tuple[str, ...] = ("accounting",)
        merged_into_concept_id: int | None = None

    @dataclass(frozen=True)
    class Facet:
        id: int = 11
        concept_id: int = 7
        language: str = "ru"
        facet_type: str = "rendering"
        value: str = "гильдейская книга"
        confidence: float = 0.6
        source_crystal_id: int | None = None
        language_tags: tuple[str, ...] = ("ru",)
        story_scopes: tuple[str, ...] = ()
        semantic_tags: tuple[str, ...] = ()
        is_canonical: bool = False

        @property
        def kind(self) -> str:
            return self.facet_type

    class FakeConceptStore:
        def __init__(self, config: HieronymusConfig) -> None:
            calls.append(("init", (config,), {}))

        def create_concept(self, *args: object, **kwargs: object) -> Concept:
            calls.append(("create_concept", args, kwargs))
            return Concept()

        def get(self, *args: object, **kwargs: object) -> Concept:
            calls.append(("get", args, kwargs))
            return Concept()

        def update_concept(self, *args: object, **kwargs: object) -> Concept:
            calls.append(("update_concept", args, kwargs))
            return Concept()

        def rename_concept(self, *args: object, **kwargs: object) -> Concept:
            calls.append(("rename_concept", args, kwargs))
            return Concept(canonical_name="Guild Register")

        def merge_concepts(self, *args: object, **kwargs: object) -> None:
            calls.append(("merge_concepts", args, kwargs))

        def archive_concept(self, *args: object, **kwargs: object) -> None:
            calls.append(("archive_concept", args, kwargs))

        def list_facets(self, *args: object, **kwargs: object) -> list[Facet]:
            calls.append(("list_facets", args, kwargs))
            return [Facet()]

        def add_facet(self, *args: object, **kwargs: object) -> Facet:
            calls.append(("add_facet", args, kwargs))
            return Facet()

        def update_facet(self, *args: object, **kwargs: object) -> Facet:
            calls.append(("update_facet", args, kwargs))
            return Facet(value="гильдейский реестр")

        def set_canonical_facet(self, *args: object, **kwargs: object) -> None:
            calls.append(("set_canonical_facet", args, kwargs))

    monkeypatch.setattr("hieronymus.admin.ConceptStore", FakeConceptStore)
    store = AdminStore(config)

    store.add_concept(canonical_name="Guild Ledger", semantic_tags=("accounting",))
    store.update_concept(7, description="Updated.")
    store.reinforce_concept(7, evidence="Confirmed.")
    store.decay_concept(7, evidence="Contradicted.")
    store.rename_concept(7, canonical_name="Guild Register")
    store.merge_concepts(7, 8, reason="Duplicate.")
    store.archive_concept(7, reason="Obsolete.")
    store.list_concept_facets(7)
    store.add_concept_facet(7, value="гильдейская книга", facet_type="rendering")
    store.update_concept_facet(11, value="гильдейский реестр")
    store.set_canonical_concept_facet(7, 11)

    called = [name for name, _args, _kwargs in calls]
    assert "create_concept" in called
    assert called.count("update_concept") == 3
    assert "rename_concept" in called
    assert "merge_concepts" in called
    assert "archive_concept" in called
    assert "list_facets" in called
    assert "add_facet" in called
    assert "update_facet" in called
    assert "set_canonical_facet" in called


def test_concept_facet_bridge_dispatches_to_admin_contract(
    config: HieronymusConfig,
) -> None:
    concept_id = (
        ConceptStore(config)
        .create_concept(
            "Guild Ledger",
            semantic_tags=("accounting",),
        )
        .id
    )

    add_response = dispatch(
        config,
        {
            "id": "1",
            "method": "admin.add_concept_facet",
            "params": {
                "concept_id": concept_id,
                "value": "гильдейская книга",
                "facet_type": "rendering",
                "language_tags": ["ru"],
                "is_canonical": True,
            },
        },
    )
    detail_response = dispatch(
        config,
        {
            "id": "2",
            "method": "admin.concept_detail",
            "params": {"id": concept_id},
        },
    )

    assert add_response["ok"] is True
    assert detail_response["ok"] is True
    assert detail_response["result"]["concept"]["facets"][0]["value"] == "гильдейская книга"


def test_proposal_approval_upgrades_existing_concept_facet_state(
    config: HieronymusConfig,
) -> None:
    concept_id = (
        ConceptStore(config)
        .create_concept(
            "Guild Ledger",
            scope_type="series",
            scope_key="series:only-sense-online",
        )
        .id
    )
    with connect(config.database_path) as conn:
        source_facet_id = conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              confidence,
              is_canonical,
              created_at,
              updated_at
            )
            values (?, 'ja', 'name', 'ギルド台帳', 0.2, 0, ?, ?)
            """,
            (
                concept_id,
                "2026-06-10T00:00:00+00:00",
                "2026-06-10T00:00:00+00:00",
            ),
        ).lastrowid
        rendering_facet_id = conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              confidence,
              is_canonical,
              created_at,
              updated_at
            )
            values (?, 'ru', 'rendering', 'гильдейская книга', 0.2, 0, ?, ?)
            """,
            (
                concept_id,
                "2026-06-10T00:00:00+00:00",
                "2026-06-10T00:00:00+00:00",
            ),
        ).lastrowid
        conn.commit()

    proposal_id = ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        concept_text="Guild Ledger",
        source_form="ギルド台帳",
        canonical_rendering="гильдейская книга",
        approved_variants=["гильдейская книга"],
        forbidden_variants=[],
        rationale="Existing facet should be reinforced.",
    )

    AdminStore(config).approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        source_row = conn.execute(
            "select is_canonical, confidence from concept_facets where id = ?",
            (source_facet_id,),
        ).fetchone()
        rendering_row = conn.execute(
            "select is_canonical, confidence from concept_facets where id = ?",
            (rendering_facet_id,),
        ).fetchone()
    assert source_row["is_canonical"] == 1
    assert source_row["confidence"] == 0.95
    assert rendering_row["is_canonical"] == 0
    assert rendering_row["confidence"] == 0.95


def test_concepts_snapshot_uses_memory_graph_concepts(config: HieronymusConfig) -> None:
    ConceptStore(config).create_concept(
        "Guild Ledger",
        description="Guild accounting artifact.",
        semantic_tags=("accounting",),
    )
    _context(config)
    CrystalStore(config).add_crystal(
        TranslationContext(
            series_slug="only-sense-online",
            source_language="ja",
            target_language="ru",
            task_type="translation",
        ),
        crystal_type="concept",
        title="Crystal concept",
        text="This should stay in the crystal view.",
    )

    snapshot = AdminStore(config).snapshot("Concepts")

    assert [row.label for row in snapshot.rows] == ["Guild Ledger"]
    assert snapshot.detail.title == "Guild Ledger"

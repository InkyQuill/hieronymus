import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dreaming import (
    DeterministicDreamProvider,
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamService,
)
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig, *, slug: str = "only-sense-online") -> TranslationContext:
    series = Registry(config).create_series(
        slug=slug,
        title=slug.replace("-", " ").title(),
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
    )


def test_dreaming_crystallizes_completed_short_term_memory(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Use Сенс, not Чувство, for Sense in UI references.",
    )
    workspace.complete_session(session.id)

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals").fetchone()
        sources = conn.execute("select * from crystal_sources").fetchall()

    assert run.status == "completed"
    assert crystal["crystal_type"] == "lesson"
    assert crystal["title"] == "Correction"
    assert crystal["text"] == "Use Сенс, not Чувство, for Sense in UI references."
    assert sources[0]["short_term_memory_id"] == memory_id


def test_dreaming_ignores_active_sessions(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Active notes should wait for completion.",
    )

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run.status == "completed"
    assert crystal_count == 0
    assert session_row["status"] == "active"
    assert session_row["cycle_id"] is None


def test_dreaming_marks_completed_sessions_as_dreamed_with_cycle(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="style-note",
        text="Keep item crafting notes concise.",
    )
    workspace.complete_session(session.id)

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    with connect(config.database_path) as conn:
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert session_row["status"] == "dreamed"
    assert session_row["cycle_id"] == run.cycle_id


def test_dreaming_creates_next_cycle_id(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    first = workspace.start_session(context)
    workspace.add_short_term_memory(first.id, "user", "note", "First note.")
    workspace.complete_session(first.id)

    first_run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    second = workspace.start_session(context)
    workspace.add_short_term_memory(second.id, "user", "note", "Second note.")
    workspace.complete_session(second.id)
    second_run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    assert first_run.cycle_id == 1
    assert second_run.cycle_id == 2


def test_dreaming_records_failed_run_for_invalid_provider_output(
    config: HieronymusConfig,
) -> None:
    class InvalidProvider:
        name = "invalid"

        def crystallize(self, context, memories):
            return DreamOutput(
                crystals=[
                    DreamCrystalCandidate(
                        crystal_type="lesson",
                        title="Invalid",
                        text="",
                        strength=0.5,
                        confidence=0.5,
                        source_memory_ids=[memories[0].id],
                    )
                ],
                concept_proposals=[],
            )

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="text"):
        DreamService(config, InvalidProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert run["error"]
    assert run["completed_at"]
    assert crystal_count == 0
    assert session_row["status"] == "completed"
    assert session_row["cycle_id"] is None


def test_dreaming_records_failed_run_when_provider_returns_none(
    config: HieronymusConfig,
) -> None:
    class NoneProvider:
        name = "none"

        def crystallize(self, context, memories):
            return None

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="DreamOutput"):
        DreamService(config, NoneProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert "DreamOutput" in run["error"]
    assert run["completed_at"]
    assert crystal_count == 0
    assert session_row["status"] == "completed"
    assert session_row["cycle_id"] is None


def test_dreaming_records_failed_run_when_provider_returns_invalid_crystal_item(
    config: HieronymusConfig,
) -> None:
    class InvalidCrystalItemProvider:
        name = "invalid-item"

        def crystallize(self, context, memories):
            return DreamOutput(crystals=[{"text": "Not a candidate."}], concept_proposals=[])

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="DreamCrystalCandidate"):
        DreamService(config, InvalidCrystalItemProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert "DreamCrystalCandidate" in run["error"]
    assert run["completed_at"]
    assert crystal_count == 0
    assert session_row["status"] == "completed"
    assert session_row["cycle_id"] is None


def test_deterministic_provider_maps_roles_to_crystal_types(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    user_id = workspace.add_short_term_memory(session.id, "user", "correction", "User lesson.")
    mentor_id = workspace.add_short_term_memory(session.id, "mentor", "note", "Mentor lore.")
    mundane_id = workspace.add_short_term_memory(session.id, "mundane", "tm", "Mundane term.")
    system_id = workspace.add_short_term_memory(session.id, "system", "trace", "System trace.")
    memories = workspace.list_short_term_memories(session.id)

    output = DeterministicDreamProvider().crystallize(context, memories)

    assert [
        (candidate.crystal_type, candidate.source_memory_ids) for candidate in output.crystals
    ] == [
        ("lesson", [user_id]),
        ("erudition", [mentor_id]),
        ("concept", [mundane_id]),
    ]
    assert all(system_id not in candidate.source_memory_ids for candidate in output.crystals)


def test_dreaming_inserts_strict_concept_proposals_from_provider_output(
    config: HieronymusConfig,
) -> None:
    class ProposalProvider:
        name = "proposal"

        def crystallize(self, context, memories):
            return DreamOutput(
                crystals=[],
                concept_proposals=[
                    DreamConceptProposal(
                        series_slug=context.series_slug,
                        source_language=context.source_language,
                        target_language=context.target_language,
                        concept_text="Sense",
                        source_form="センス",
                        canonical_rendering="Сенс",
                        approved_variants=["Сенс"],
                        forbidden_variants=["Чувство"],
                        rationale="User corrected the term.",
                    )
                ],
            )

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mundane", "term", "センス should be Сенс.")
    workspace.complete_session(session.id)

    run = DreamService(config, ProposalProvider()).run_cycle()

    with connect(config.database_path) as conn:
        proposal = conn.execute("select * from strict_concept_proposals").fetchone()

    assert run.status == "completed"
    assert proposal["dream_run_id"] == 1
    assert proposal["status"] == "pending"
    assert proposal["concept_text"] == "Sense"
    assert proposal["approved_variants_json"] == '["Сенс"]'
    assert proposal["forbidden_variants_json"] == '["Чувство"]'

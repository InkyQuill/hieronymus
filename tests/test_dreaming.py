import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dreaming import (
    DeterministicDreamProvider,
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamService,
)
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.scoring import FeedbackStore
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


class EmptyDreamProvider:
    name = "empty"

    def crystallize(self, context, memories):
        return DreamOutput(crystals=[], concept_proposals=[])


def _add_crystal(
    config: HieronymusConfig,
    context: TranslationContext,
    *,
    text: str,
    strength: float = 0.5,
    confidence: float = 0.5,
    status: str = "active",
) -> int:
    return CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text=text,
        strength=strength,
        confidence=confidence,
        status=status,
    )


def _completed_session(config: HieronymusConfig, context: TranslationContext) -> int:
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "A completed dream input.")
    workspace.complete_session(session.id)
    return session.id


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


def test_dreaming_rolls_back_applied_outputs_when_crystal_insert_fails(
    config: HieronymusConfig,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class TwoCrystalProvider:
        name = "two-crystals"

        def crystallize(self, context, memories):
            return DreamOutput(
                crystals=[
                    DreamCrystalCandidate(
                        crystal_type="lesson",
                        title="First",
                        text="First valid candidate.",
                        strength=0.5,
                        confidence=0.5,
                        source_memory_ids=[memories[0].id],
                    ),
                    DreamCrystalCandidate(
                        crystal_type="lesson",
                        title="Second",
                        text="Second valid candidate.",
                        strength=0.5,
                        confidence=0.5,
                        source_memory_ids=[memories[0].id],
                    ),
                ],
                concept_proposals=[
                    DreamConceptProposal(
                        series_slug=context.series_slug,
                        source_language=context.source_language,
                        target_language=context.target_language,
                        concept_text="Sense",
                        source_form="センス",
                        canonical_rendering="Сенс",
                        approved_variants=["Сенс"],
                        forbidden_variants=[],
                        rationale="Valid proposal.",
                    )
                ],
            )

    calls = 0

    def fail_after_first_crystal(self, conn, context, candidate):
        nonlocal calls
        calls += 1
        crystal_id = self._insert_crystal(conn, context, candidate)
        if calls == 2:
            raise RuntimeError("apply failure")
        return crystal_id

    monkeypatch.setattr(
        DreamService,
        "_insert_crystal_for_dream",
        fail_after_first_crystal,
        raising=False,
    )

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(RuntimeError, match="apply failure"):
        DreamService(config, TwoCrystalProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        proposal_count = conn.execute("select count(*) from strict_concept_proposals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert run["completed_at"]
    assert crystal_count == 0
    assert proposal_count == 0
    assert session_row["status"] == "completed"
    assert session_row["cycle_id"] is None


def test_dreaming_rejects_concept_proposal_with_mismatched_scope(
    config: HieronymusConfig,
) -> None:
    class MismatchedProposalProvider:
        name = "mismatched-proposal"

        def crystallize(self, context, memories):
            return DreamOutput(
                crystals=[],
                concept_proposals=[
                    DreamConceptProposal(
                        series_slug="other-series",
                        source_language=context.source_language,
                        target_language=context.target_language,
                        concept_text="Sense",
                        source_form="センス",
                        canonical_rendering="Сенс",
                        approved_variants=["Сенс"],
                        forbidden_variants=[],
                        rationale="Wrong scope.",
                    )
                ],
            )

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mundane", "term", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="series_slug"):
        DreamService(config, MismatchedProposalProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        proposal_count = conn.execute("select count(*) from strict_concept_proposals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert "series_slug" in run["error"]
    assert run["completed_at"]
    assert proposal_count == 0
    assert session_row["status"] == "completed"
    assert session_row["cycle_id"] is None


def test_dreaming_rejects_concept_proposal_with_non_string_variant(
    config: HieronymusConfig,
) -> None:
    class InvalidVariantProvider:
        name = "invalid-variant"

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
                        approved_variants=["Сенс", 42],
                        forbidden_variants=[],
                        rationale="Invalid variant type.",
                    )
                ],
            )

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mundane", "term", "Valid input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="approved_variants"):
        DreamService(config, InvalidVariantProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select * from dream_runs").fetchone()
        proposal_count = conn.execute("select count(*) from strict_concept_proposals").fetchone()[0]
        session_row = conn.execute("select status, cycle_id from task_sessions").fetchone()

    assert run["status"] == "failed"
    assert "approved_variants" in run["error"]
    assert run["completed_at"]
    assert proposal_count == 0
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


def test_dreaming_decays_inactive_crystals_but_not_recalled_crystals(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    inactive_id = _add_crystal(
        config,
        context,
        text="Inactive crafting label memory.",
        strength=0.5,
        confidence=0.5,
    )
    recalled_id = _add_crystal(
        config,
        context,
        text="Guarded crafting recall memory.",
        strength=0.5,
        confidence=0.5,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    RecallService(config).recall(session.id, context, "guarded crafting", limit=1)
    workspace.complete_session(session.id)

    run = DreamService(config, EmptyDreamProvider()).run_cycle()

    inactive = CrystalStore(config).get(inactive_id)
    recalled = CrystalStore(config).get(recalled_id)
    with connect(config.database_path) as conn:
        recalled_cycle = conn.execute(
            "select last_activated_cycle from crystals where id = ?",
            (recalled_id,),
        ).fetchone()[0]

    assert inactive.strength == pytest.approx(0.47)
    assert inactive.confidence == pytest.approx(0.5)
    assert recalled.strength == pytest.approx(0.5)
    assert recalled.confidence == pytest.approx(0.5)
    assert recalled_cycle == run.cycle_id


def test_passive_positive_event_reinforces_and_prevents_decay_in_same_cycle(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(
        config,
        context,
        text="Positive passive reinforcement memory.",
        strength=0.5,
        confidence=0.5,
    )
    FeedbackStore(config).record(crystal_id, "used_in_translation", "system")
    run = DreamService(config, EmptyDreamProvider()).run_cycle()

    crystal = CrystalStore(config).get(crystal_id)
    with connect(config.database_path) as conn:
        event = conn.execute("select applied, cycle_id from memory_events").fetchone()
        last_reinforced_cycle = conn.execute(
            "select last_reinforced_cycle from crystals where id = ?",
            (crystal_id,),
        ).fetchone()[0]

    assert crystal.strength == pytest.approx(0.55)
    assert crystal.confidence == pytest.approx(0.52)
    assert event["applied"] == 1
    assert event["cycle_id"] == run.cycle_id
    assert last_reinforced_cycle == run.cycle_id


def test_passive_negative_event_applies_and_does_not_protect_from_decay(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(
        config,
        context,
        text="Negative passive correction memory.",
        strength=0.5,
        confidence=0.5,
    )
    FeedbackStore(config).record(crystal_id, "caused_correction", "user")
    run = DreamService(config, EmptyDreamProvider()).run_cycle()

    crystal = CrystalStore(config).get(crystal_id)
    with connect(config.database_path) as conn:
        row = conn.execute(
            "select applied, cycle_id from memory_events where crystal_id = ?",
            (crystal_id,),
        ).fetchone()

    assert crystal.strength == pytest.approx(0.37)
    assert crystal.confidence == pytest.approx(0.38)
    assert row["applied"] == 1
    assert row["cycle_id"] == run.cycle_id


def test_confidence_decays_only_after_strength_below_threshold(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    above_id = _add_crystal(
        config,
        context,
        text="Strength lands exactly on threshold.",
        strength=0.23,
        confidence=0.5,
    )
    below_id = _add_crystal(
        config,
        context,
        text="Strength lands below threshold.",
        strength=0.22,
        confidence=0.5,
    )
    _completed_session(config, context)

    DreamService(config, EmptyDreamProvider()).run_cycle()

    above = CrystalStore(config).get(above_id)
    below = CrystalStore(config).get(below_id)
    assert above.strength == pytest.approx(0.2)
    assert above.confidence == pytest.approx(0.5)
    assert below.strength == pytest.approx(0.19)
    assert below.confidence == pytest.approx(0.49)


def test_decay_skips_archived_or_rejected_crystals(config: HieronymusConfig) -> None:
    context = _context(config)
    archived_id = _add_crystal(
        config,
        context,
        text="Archived memory.",
        strength=0.5,
        confidence=0.5,
        status="archived",
    )
    rejected_id = _add_crystal(
        config,
        context,
        text="Rejected memory.",
        strength=0.5,
        confidence=0.5,
        status="rejected",
    )
    active_id = _add_crystal(
        config,
        context,
        text="Active memory.",
        strength=0.5,
        confidence=0.5,
    )
    _completed_session(config, context)

    DreamService(config, EmptyDreamProvider()).run_cycle()

    assert CrystalStore(config).get(archived_id).strength == pytest.approx(0.5)
    assert CrystalStore(config).get(rejected_id).strength == pytest.approx(0.5)
    assert CrystalStore(config).get(active_id).strength == pytest.approx(0.47)


def test_activation_rows_get_cycle_id_when_dream_processes_session(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(
        config,
        context,
        text="Guarded crafting activation memory.",
        strength=0.5,
        confidence=0.5,
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    RecallService(config).recall(session.id, context, "guarded crafting", limit=1)
    workspace.complete_session(session.id)

    run = DreamService(config, EmptyDreamProvider()).run_cycle()

    with connect(config.database_path) as conn:
        activation = conn.execute("select crystal_id, cycle_id from crystal_activations").fetchone()
        last_activated_cycle = conn.execute(
            "select last_activated_cycle from crystals where id = ?",
            (crystal_id,),
        ).fetchone()[0]

    assert activation["crystal_id"] == crystal_id
    assert activation["cycle_id"] == run.cycle_id
    assert last_activated_cycle == run.cycle_id

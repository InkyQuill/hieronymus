import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.dream_config import (
    DreamConfigError,
    ProviderProfile,
    default_dream_config,
    save_dream_config,
)
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import (
    DeterministicDreamProvider,
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamService,
    _recover_crystal_text,
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


def _pending_short_term_memory_count(config: HieronymusConfig) -> int:
    with connect(config.database_path) as conn:
        return int(
            conn.execute(
                "select count(*) from short_term_memories where archived_at is null",
            ).fetchone()[0]
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


def test_manual_dreaming_drains_small_batch_even_below_minimum(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    _completed_session(config, context)

    run = DreamService(config, DeterministicDreamProvider()).run_all(owner="admin")

    assert run.status == "completed"
    assert run.input_count == 1
    assert _pending_short_term_memory_count(config) == 0


def test_dreaming_preserves_typed_session_metadata_for_provider(
    config: HieronymusConfig,
) -> None:
    class CapturingProvider:
        name = "capturing"

        def __init__(self) -> None:
            self.contexts = []

        def crystallize(self, context, memories):
            self.contexts.append(context)
            return DreamOutput(crystals=[], concept_proposals=[])

    base_context = _context(config)
    context = TranslationContext(
        series_slug=base_context.series_slug,
        source_language=base_context.source_language,
        target_language=base_context.target_language,
        task_type=base_context.task_type,
        volume=base_context.volume,
        chapter=base_context.chapter,
        language_tags=("ja", "ru", "en"),
        story_scopes=("arc:academy",),
        semantic_tags=("terminology",),
    )
    _completed_session(config, context)
    provider = CapturingProvider()

    run = DreamService(config, provider).run_all(owner="admin")

    assert run.status == "completed"
    assert provider.contexts[0].language_tags == ("ja", "ru", "en")
    assert provider.contexts[0].story_scopes == ("arc:academy",)
    assert provider.contexts[0].semantic_tags == ("terminology",)


def test_scheduled_dreaming_drains_all_pending_in_capped_cycles(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session_sizes = [10, 10, 20, 15]
    for session_index, memory_count in enumerate(session_sizes):
        session = workspace.start_session(context)
        for memory_index in range(memory_count):
            workspace.add_short_term_memory(
                session.id,
                source_role="user",
                kind="note",
                text=f"Drain session {session_index} memory {memory_index}.",
            )
        workspace.complete_session(session.id)

    run = DreamService(
        config,
        DeterministicDreamProvider(),
        max_short_term_memories_per_cycle=20,
    ).run_all(owner="scheduler")

    with connect(config.database_path) as conn:
        phase_rows = conn.execute(
            """
            select phase, status, input_count
            from dream_phase_runs
            order by id
            """
        ).fetchall()
        session_rows = conn.execute(
            "select status, cycle_id from task_sessions order by id",
        ).fetchall()

    assert run.status == "completed"
    assert run.input_count == 55
    assert run.created_crystal_count == 55
    assert run.proposal_count == 0
    assert [(row["phase"], row["status"], row["input_count"]) for row in phase_rows] == [
        ("crystallization", "completed", 20),
        ("crystallization", "completed", 20),
        ("crystallization", "completed", 15),
    ]
    assert _pending_short_term_memory_count(config) == 0
    assert [(row["status"], row["cycle_id"]) for row in session_rows] == [
        ("dreamed", run.cycle_id),
        ("dreamed", run.cycle_id),
        ("dreamed", run.cycle_id),
        ("dreamed", run.cycle_id),
    ]


def test_run_all_failed_later_batch_keeps_successful_batch_cycle_attribution(
    config: HieronymusConfig,
) -> None:
    class FailSecondBatchProvider:
        name = "fail-second-batch"

        def __init__(self) -> None:
            self.calls = 0

        def crystallize(self, context, memories):
            self.calls += 1
            if self.calls == 2:
                raise RuntimeError("second batch failed")
            return DreamOutput(crystals=[], concept_proposals=[])

    context = _context(config)
    workspace = WorkspaceStore(config)
    first_session = workspace.start_session(context)
    first_memory_ids = [
        workspace.add_short_term_memory(
            first_session.id,
            source_role="user",
            kind="note",
            text=f"Oversized first session memory {index}.",
        )
        for index in range(3)
    ]
    workspace.complete_session(first_session.id)
    second_session = workspace.start_session(context)
    second_memory_id = workspace.add_short_term_memory(
        second_session.id,
        source_role="user",
        kind="note",
        text="Second batch fails.",
    )
    workspace.complete_session(second_session.id)

    with pytest.raises(RuntimeError, match="second batch failed"):
        DreamService(
            config,
            FailSecondBatchProvider(),
            max_short_term_memories_per_cycle=2,
        ).run_all()

    with connect(config.database_path) as conn:
        failed_run = conn.execute(
            """
            select
              id,
              cycle_id,
              status,
              input_count,
              created_crystal_count,
              proposal_count
            from dream_runs
            """
        ).fetchone()
        first_memories = conn.execute(
            f"""
            select archived_at
            from short_term_memories
            where id in ({", ".join("?" for _ in first_memory_ids)})
            order by id
            """,
            first_memory_ids,
        ).fetchall()
        second_memory = conn.execute(
            "select archived_at from short_term_memories where id = ?",
            (second_memory_id,),
        ).fetchone()
        first_session_after_failure = conn.execute(
            "select status, cycle_id from task_sessions where id = ?",
            (first_session.id,),
        ).fetchone()
        phase_rows = conn.execute(
            "select status from dream_phase_runs order by id",
        ).fetchall()
        audit_rows = conn.execute(
            """
            select event_type, summary, phase_run_id
            from dream_audit_entries
            order by id
            """
        ).fetchall()

    assert failed_run["status"] == "failed"
    assert failed_run["input_count"] == 2
    assert failed_run["created_crystal_count"] == 0
    assert failed_run["proposal_count"] == 0
    assert [memory["archived_at"] is not None for memory in first_memories] == [
        True,
        True,
        False,
    ]
    assert second_memory["archived_at"] is None
    assert first_session_after_failure["status"] == "completed"
    assert first_session_after_failure["cycle_id"] is None
    assert [row["status"] for row in phase_rows] == ["completed", "failed"]
    assert [(row["event_type"], row["summary"]) for row in audit_rows] == [
        ("provider_request", "sent crystallization request"),
        ("provider_response", "received crystallization response"),
        ("phase_completed", "completed crystallization phase"),
        ("provider_request", "sent crystallization request"),
    ]
    assert audit_rows[2]["phase_run_id"] is not None

    later_run = DreamService(
        config,
        EmptyDreamProvider(),
        max_short_term_memories_per_cycle=1,
    ).run_all()

    with connect(config.database_path) as conn:
        first_session_after_later_run = conn.execute(
            "select status, cycle_id from task_sessions where id = ?",
            (first_session.id,),
        ).fetchone()
        second_session_after_later_run = conn.execute(
            "select status, cycle_id from task_sessions where id = ?",
            (second_session.id,),
        ).fetchone()

    assert later_run.cycle_id != failed_run["cycle_id"]
    assert first_session_after_later_run["status"] == "dreamed"
    assert first_session_after_later_run["cycle_id"] == later_run.cycle_id
    assert second_session_after_later_run["status"] == "dreamed"
    assert second_session_after_later_run["cycle_id"] == later_run.cycle_id


def test_run_all_preserves_crystal_source_links(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Keep Item Box capitalized.",
    )
    workspace.complete_session(session.id)

    run = DreamService(config, DeterministicDreamProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals").fetchone()
        source = conn.execute("select * from crystal_sources").fetchone()

    assert run.status == "completed"
    assert crystal["text"] == "Keep Item Box capitalized."
    assert source["short_term_memory_id"] == memory_id


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


def test_dreaming_rejects_second_cycle_while_lock_is_active(
    config: HieronymusConfig,
) -> None:
    with dream_cycle_lock(config, owner="manual"):
        with pytest.raises(DreamCycleAlreadyRunning, match="dream cycle already running"):
            DreamService(config, DeterministicDreamProvider()).run_cycle()


def test_dreaming_releases_lock_after_provider_exception(
    config: HieronymusConfig,
) -> None:
    class FailingProvider:
        name = "failing"

        def crystallize(self, context, memories):
            raise RuntimeError("provider failed")

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(RuntimeError, match="provider failed"):
        DreamService(config, FailingProvider()).run_cycle()

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()
    assert run.status == "completed"


def test_dreaming_records_locked_skip_without_consuming_cycle_id(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    _completed_session(config, context)

    with dream_cycle_lock(config, owner="manual"):
        skipped = DreamService(config, DeterministicDreamProvider()).run_cycle(
            skip_when_locked=True,
        )

    run = DreamService(config, DeterministicDreamProvider()).run_cycle()

    assert skipped.status == "skipped"
    assert skipped.cycle_id == -1
    assert skipped.error == "dream cycle already running"
    assert run.status == "completed"
    assert run.cycle_id == 1
    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select cycle_id, status from dream_runs order by id",
        ).fetchall()
    assert [(row["cycle_id"], row["status"]) for row in rows] == [
        (-1, "skipped"),
        (1, "completed"),
    ]


def test_dreaming_does_not_skip_provider_lock_error_after_acquiring_lock(
    config: HieronymusConfig,
) -> None:
    class ProviderLockError:
        name = "provider-lock-error"

        def crystallize(self, context, memories):
            raise DreamCycleAlreadyRunning()

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(DreamCycleAlreadyRunning, match="dream cycle already running"):
        DreamService(config, ProviderLockError()).run_cycle(skip_when_locked=True)

    with connect(config.database_path) as conn:
        run = conn.execute("select status, error from dream_runs").fetchone()
    assert run["status"] == "failed"
    assert "dream cycle already running" in run["error"]


def test_dreaming_rejects_invalid_dream_config(
    config: HieronymusConfig,
) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text("not valid toml = [", encoding="utf-8")

    with pytest.raises(DreamConfigError, match="dream.conf is not valid TOML"):
        DreamService(config, DeterministicDreamProvider())


def test_dreaming_rejects_missing_workflow_provider_without_deterministic_fallback(
    config: HieronymusConfig,
) -> None:
    config.config_root.mkdir(parents=True, exist_ok=True)
    config.dream_config_path.write_text(
        """
[dreaming]
enabled = true
schedule_interval_minutes = 30
min_pending_short_term_memories = 1
max_pending_short_term_memories = 10
max_short_term_memories_per_cycle = 1
not_enough_memories_cycle_threshold = 5
max_changed_crystals_per_cycle = 200
max_related_concepts_per_cycle = 80
max_related_crystals_per_concept = 20
max_total_affected_crystals = 500
general_prompt = "Use English as the primary searchable memory language."

[workflows.crystallization]
provider = "missing"
model = "model"
enabled = true
""",
        encoding="utf-8",
    )

    with pytest.raises(DreamConfigError, match="referenced provider profile is missing"):
        resolve_provider(config)


def test_dream_error_records_redact_configured_api_key_value(
    config: HieronymusConfig,
) -> None:
    save_dream_config(
        config,
        default_dream_config().with_provider(
            "openai",
            ProviderProfile(type="openai", api_key="raw-secret-value"),
        ),
    )

    class LeakyProvider:
        name = "leaky"

        def crystallize(self, context, memories):
            raise RuntimeError("provider rejected raw-secret-value")

    context = _context(config)
    _completed_session(config, context)

    with pytest.raises(RuntimeError, match="raw-secret-value"):
        DreamService(config, LeakyProvider()).run_cycle()

    with connect(config.database_path) as conn:
        run = conn.execute("select error from dream_runs").fetchone()

    assert run["error"] == "provider rejected [redacted]"


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


def test_dreaming_applies_malformed_dict_rule_with_penalties_and_concepts(
    config: HieronymusConfig,
) -> None:
    class MalformedDictProvider:
        name = "malformed-dict"

        def crystallize(self, context, memories):
            return {
                "rule_crystals": [
                    {
                        "body": "Keep cooking terminology practical and concrete.",
                        "kind": "rule_crystal",
                        "source_credibility": "user_rule",
                        "rule_intent": "terminology",
                        "concepts": ["Cooking"],
                    }
                ],
                "concepts": [{"label": "Cooking", "tags": ["domain:cuisine"]}],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Cooking term guidance.")
    workspace.complete_session(session.id)

    run = DreamService(config, MalformedDictProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals").fetchone()
        concept = conn.execute("select * from concepts").fetchone()
        concept_link = conn.execute("select * from crystal_concepts").fetchone()

    assert run.status == "completed"
    assert crystal["crystal_type"] == "rule"
    assert crystal["text"] == "Keep cooking terminology practical and concrete."
    assert crystal["confidence"] < 0.9
    assert crystal["source_credibility"] == "user_rule"
    assert crystal["rule_intent"] == "terminology"
    assert crystal["malformed_penalty"] > 0
    assert concept["canonical_name"] == "Cooking"
    assert concept_link["crystal_id"] == crystal["id"]
    assert concept_link["concept_id"] == concept["id"]


def test_dreaming_dict_source_credibility_controls_confidence_and_clamps_floor(
    config: HieronymusConfig,
) -> None:
    class CredibilityProvider:
        name = "credibility"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {
                        "content": "Expert confidence wins over explicit low confidence.",
                        "type": "lesson",
                        "source_credibility": "expert",
                        "confidence": 0.1,
                    },
                    {
                        "content": "User rule confidence wins over explicit low confidence.",
                        "type": "rule",
                        "source_credibility": "user_rule",
                        "confidence": 0.1,
                    },
                    {
                        "content": "Rumor confidence is clamped after heavy penalty.",
                        "type": "observation",
                        "source_credibility": "rumor",
                        "malformed_penalty": 0.5,
                    },
                ],
                "thoughts": ["Speculative thought stays low confidence."],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Credibility input.")
    workspace.complete_session(session.id)

    DreamService(config, CredibilityProvider()).run_all()

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_type, source_credibility, confidence
            from crystals
            order by id
            """
        ).fetchall()

    assert [
        (row["crystal_type"], row["source_credibility"], row["confidence"]) for row in rows
    ] == [
        ("lesson", "expert", 0.85),
        ("rule", "user_rule", 0.95),
        ("observation", "rumor", 0.05),
        ("thought", "thought", 0.2),
    ]


def test_dreaming_valid_kind_does_not_add_malformed_penalty(
    config: HieronymusConfig,
) -> None:
    class KindProvider:
        name = "kind"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {
                        "content": "Valid kind rule keeps full user-rule confidence.",
                        "kind": "rule",
                        "source_credibility": "user_rule",
                    },
                    {
                        "content": "Malformed rule crystal alias is penalized.",
                        "kind": "rule_crystal",
                        "source_credibility": "user_rule",
                    },
                ]
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Kind input.")
    workspace.complete_session(session.id)

    DreamService(config, KindProvider()).run_all()

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_type, confidence, malformed_penalty
            from crystals
            order by id
            """
        ).fetchall()

    assert [(row["crystal_type"], row["confidence"], row["malformed_penalty"]) for row in rows] == [
        ("rule", 0.95, 0.0),
        ("rule", 0.75, 0.2),
    ]


def test_dream_candidate_content_helper_requires_content() -> None:
    with pytest.raises(ValueError) as exc_info:
        _recover_crystal_text({"kind": "rule"})

    assert str(exc_info.value) == "dream candidate content is required"


def test_dreaming_rejects_dict_entries_missing_required_content(
    config: HieronymusConfig,
) -> None:
    class PartlyMalformedProvider:
        name = "partly-malformed"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {"type": "lesson"},
                    {"content": "Good recovered lesson.", "type": "lesson"},
                ],
                "rule_crystals": [{"body": ""}],
                "thoughts": [{"body": "Recoverable thought."}],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Partial malformed input.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="dream candidate content is required"):
        DreamService(config, PartlyMalformedProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]

    assert crystal_count == 0


def test_dreaming_skips_dict_candidate_with_only_invalid_source_memory_ids(
    config: HieronymusConfig,
) -> None:
    class SourceIdsProvider:
        name = "source-ids"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {
                        "content": "Bad provenance should be skipped.",
                        "type": "lesson",
                        "source_memory_ids": [999999],
                    },
                    {
                        "content": "Partly valid provenance keeps only valid links.",
                        "type": "lesson",
                        "source_memory_ids": [memories[0].id, 999999],
                    },
                    {
                        "content": "Omitted provenance links to the input batch.",
                        "type": "lesson",
                    },
                ]
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    first_id = workspace.add_short_term_memory(session.id, "user", "note", "First input.")
    second_id = workspace.add_short_term_memory(session.id, "user", "note", "Second input.")
    workspace.complete_session(session.id)

    run = DreamService(config, SourceIdsProvider()).run_all()

    with connect(config.database_path) as conn:
        crystals = conn.execute("select id, text from crystals order by id").fetchall()
        sources = conn.execute(
            """
            select crystal_id, short_term_memory_id
            from crystal_sources
            order by crystal_id, short_term_memory_id
            """
        ).fetchall()

    assert run.status == "completed"
    assert [row["text"] for row in crystals] == [
        "Partly valid provenance keeps only valid links.",
        "Omitted provenance links to the input batch.",
    ]
    assert [(row["crystal_id"], row["short_term_memory_id"]) for row in sources] == [
        (crystals[0]["id"], first_id),
        (crystals[1]["id"], first_id),
        (crystals[1]["id"], second_id),
    ]


def test_dreaming_dict_semantic_tags_update_legacy_tags_json(
    config: HieronymusConfig,
) -> None:
    class TaggedProvider:
        name = "tagged"

        def crystallize(self, context, memories):
            return {
                "crystals": [
                    {
                        "content": "Tagged dream crystal.",
                        "type": "lesson",
                        "semantic_tags": [" domain:cuisine ", "domain:cuisine"],
                    }
                ]
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Tagged input.")
    workspace.complete_session(session.id)

    DreamService(config, TaggedProvider()).run_all()

    with connect(config.database_path) as conn:
        crystal = conn.execute("select id, tags_json from crystals").fetchone()
        tag = conn.execute("select tag from crystal_semantic_tags").fetchone()

    assert crystal["tags_json"] == '["domain:cuisine"]'
    assert tag["tag"] == "domain:cuisine"


def test_dreaming_supersede_action_updates_crystals_and_records_event(
    config: HieronymusConfig,
) -> None:
    class SupersedeProvider:
        name = "supersede"

        def __init__(self, old_id: int, new_id: int) -> None:
            self.old_id = old_id
            self.new_id = new_id

        def crystallize(self, context, memories):
            return {
                "supersede": [
                    {
                        "old_crystal_id": self.old_id,
                        "new_crystal_id": self.new_id,
                        "reason": "New rule is more precise.",
                    }
                ]
            }

    context = _context(config)
    old_id = _add_crystal(config, context, text="Old cooking rule.")
    new_id = _add_crystal(config, context, text="New cooking rule.")
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "user", "note", "Supersede old rule.")
    workspace.complete_session(session.id)

    run = DreamService(config, SupersedeProvider(old_id, new_id)).run_all()

    old_crystal = CrystalStore(config).get(old_id)
    new_crystal = CrystalStore(config).get(new_id)
    with connect(config.database_path) as conn:
        event = conn.execute("select * from memory_events").fetchone()

    assert old_crystal.status == "superseded"
    assert new_crystal.supersedes_crystal_id == old_id
    assert event["event_type"] == "supersede"
    assert event["evidence"] == "New rule is more precise."
    assert event["applied"] == 1
    assert event["cycle_id"] == run.cycle_id


def test_dreaming_supersede_missing_replacement_does_not_mutate_old_crystal(
    config: HieronymusConfig,
) -> None:
    class MissingReplacementProvider:
        name = "missing-replacement"

        def __init__(self, old_id: int) -> None:
            self.old_id = old_id

        def crystallize(self, context, memories):
            return {
                "supersede": [
                    {
                        "old_crystal_id": self.old_id,
                        "new_crystal_id": 999999,
                        "reason": "Missing replacement.",
                    }
                ]
            }

    context = _context(config)
    old_id = _add_crystal(config, context, text="Old cooking rule.")
    _completed_session(config, context)

    with pytest.raises(KeyError, match="unknown crystal: 999999"):
        DreamService(config, MissingReplacementProvider(old_id)).run_all()

    old_crystal = CrystalStore(config).get(old_id)
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert old_crystal.status == "active"
    assert event_count == 0


def test_dreaming_supersede_self_reference_does_not_mutate_crystal(
    config: HieronymusConfig,
) -> None:
    class SelfReferenceProvider:
        name = "self-reference"

        def __init__(self, crystal_id: int) -> None:
            self.crystal_id = crystal_id

        def crystallize(self, context, memories):
            return {
                "supersede": [
                    {
                        "old_crystal_id": self.crystal_id,
                        "new_crystal_id": self.crystal_id,
                        "reason": "Self reference.",
                    }
                ]
            }

    context = _context(config)
    crystal_id = _add_crystal(config, context, text="Self reference rule.")
    _completed_session(config, context)

    with pytest.raises(ValueError, match="crystal cannot supersede itself"):
        DreamService(config, SelfReferenceProvider(crystal_id)).run_all()

    crystal = CrystalStore(config).get(crystal_id)
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert crystal.status == "active"
    assert crystal.supersedes_crystal_id is None
    assert event_count == 0


def test_dreaming_supersede_rejects_context_mismatch_without_mutating(
    config: HieronymusConfig,
) -> None:
    class ContextMismatchProvider:
        name = "context-mismatch"

        def __init__(self, old_id: int, new_id: int) -> None:
            self.old_id = old_id
            self.new_id = new_id

        def crystallize(self, context, memories):
            return {
                "supersede": [
                    {
                        "old_crystal_id": self.old_id,
                        "new_crystal_id": self.new_id,
                        "reason": "Mismatched replacement.",
                    }
                ]
            }

    context = _context(config)
    other_context = _context(config, slug="other-series")
    old_id = _add_crystal(config, context, text="Old cooking rule.")
    new_id = _add_crystal(config, other_context, text="Other cooking rule.")
    _completed_session(config, context)

    with pytest.raises(ValueError, match="supersede crystal series_slug does not match"):
        DreamService(config, ContextMismatchProvider(old_id, new_id)).run_all()

    old_crystal = CrystalStore(config).get(old_id)
    new_crystal = CrystalStore(config).get(new_id)
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert old_crystal.status == "active"
    assert new_crystal.supersedes_crystal_id is None
    assert event_count == 0


def test_dreaming_supersede_rejects_inactive_replacement_without_mutating(
    config: HieronymusConfig,
) -> None:
    class InactiveReplacementProvider:
        name = "inactive-replacement"

        def __init__(self, old_id: int, new_id: int) -> None:
            self.old_id = old_id
            self.new_id = new_id

        def crystallize(self, context, memories):
            return {
                "supersede": [
                    {
                        "old_crystal_id": self.old_id,
                        "new_crystal_id": self.new_id,
                        "reason": "Inactive replacement.",
                    }
                ]
            }

    context = _context(config)
    old_id = _add_crystal(config, context, text="Old cooking rule.")
    new_id = _add_crystal(
        config,
        context,
        text="Archived replacement rule.",
        status="archived",
    )
    _completed_session(config, context)

    with pytest.raises(ValueError, match="supersede crystals must be active or candidate"):
        DreamService(config, InactiveReplacementProvider(old_id, new_id)).run_all()

    old_crystal = CrystalStore(config).get(old_id)
    new_crystal = CrystalStore(config).get(new_id)
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert old_crystal.status == "active"
    assert new_crystal.status == "archived"
    assert new_crystal.supersedes_crystal_id is None
    assert event_count == 0


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


def test_active_rule_crystals_do_not_decay(config: HieronymusConfig) -> None:
    context = _context(config)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Готовка.",
        confidence=0.95,
        strength=0.6,
        status="active",
    )

    DreamService(config, DeterministicDreamProvider()).decay_candidates(
        crystal_ids=(crystal_id,),
        reason="ambient low confidence decay",
    )

    crystal = CrystalStore(config).get(crystal_id)
    assert crystal.strength == 0.6
    assert crystal.status == "active"


def test_select_ambient_decay_candidates_excludes_linked_ids_and_active_rules(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    linked_id = _add_crystal(
        config,
        context,
        text="Linked low confidence memory.",
        strength=0.3,
        confidence=0.1,
    )
    lowest_id = _add_crystal(
        config,
        context,
        text="Lowest confidence unlinked memory.",
        strength=0.3,
        confidence=0.2,
    )
    active_rule_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Готовка.",
        strength=0.3,
        confidence=0.05,
        status="active",
    )
    higher_id = _add_crystal(
        config,
        context,
        text="Higher confidence unlinked memory.",
        strength=0.3,
        confidence=0.4,
    )

    candidates = DreamService(
        config,
        DeterministicDreamProvider(),
    ).select_ambient_decay_candidates(
        recalled_crystal_ids=(linked_id, lowest_id, active_rule_id, higher_id),
        linked_crystal_ids=(linked_id,),
        limit=5,
    )

    assert candidates == (lowest_id, higher_id)


def test_apply_maintenance_reinforce_decay_clamps_and_archives_zero_confidence(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    high_id = _add_crystal(
        config,
        context,
        text="Strong reinforced memory.",
        strength=0.95,
        confidence=0.98,
    )
    low_id = _add_crystal(
        config,
        context,
        text="Weak decayed memory.",
        strength=0.03,
        confidence=0.01,
    )

    DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
        {
            "reinforce": [{"crystal_id": high_id, "strength_delta": 0.1, "confidence_delta": 0.05}],
            "decay": [{"crystal_id": low_id, "strength_delta": -0.05, "confidence_delta": -0.02}],
        },
        cycle_id=42,
    )

    high = CrystalStore(config).get(high_id)
    low = CrystalStore(config).get(low_id)
    with connect(config.database_path) as conn:
        events = conn.execute(
            """
            select crystal_id, event_type, strength_delta, confidence_delta, applied, cycle_id
            from memory_events
            order by id
            """
        ).fetchall()

    assert high.strength == 1.0
    assert high.confidence == 1.0
    assert high.status == "active"
    assert low.strength == 0.0
    assert low.confidence == 0.0
    assert low.status == "archived"
    assert [
        (row["crystal_id"], row["event_type"], row["applied"], row["cycle_id"]) for row in events
    ] == [
        (high_id, "maintenance_reinforce", 1, 42),
        (low_id, "maintenance_decay", 1, 42),
    ]
    assert events[0]["strength_delta"] == pytest.approx(0.05)
    assert events[0]["confidence_delta"] == pytest.approx(0.02)
    assert events[1]["strength_delta"] == pytest.approx(-0.03)
    assert events[1]["confidence_delta"] == pytest.approx(-0.01)


def test_apply_maintenance_rolls_back_reinforce_when_later_supersede_fails(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    reinforced_id = _add_crystal(
        config,
        context,
        text="Reinforce should roll back.",
        strength=0.4,
        confidence=0.4,
    )
    supersede_id = _add_crystal(
        config,
        context,
        text="Self supersede should fail.",
        strength=0.5,
        confidence=0.5,
    )

    with pytest.raises(ValueError, match="crystal cannot supersede itself"):
        DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
            {
                "reinforce": [
                    {
                        "crystal_id": reinforced_id,
                        "strength_delta": 0.2,
                        "confidence_delta": 0.2,
                    }
                ],
                "supersede": [
                    {
                        "old_crystal_id": supersede_id,
                        "new_crystal_id": supersede_id,
                        "reason": "Invalid self supersede.",
                    }
                ],
            },
            cycle_id=11,
        )

    reinforced = CrystalStore(config).get(reinforced_id)
    supersede = CrystalStore(config).get(supersede_id)
    with connect(config.database_path) as conn:
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert reinforced.strength == pytest.approx(0.4)
    assert reinforced.confidence == pytest.approx(0.4)
    assert supersede.status == "active"
    assert supersede.supersedes_crystal_id is None
    assert event_count == 0


def test_apply_maintenance_combines_crystals_with_deterministic_sources(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    first_id = _add_crystal(
        config,
        context,
        text="First memory for combined crystal.",
        strength=0.4,
        confidence=0.7,
    )
    second_id = _add_crystal(
        config,
        context,
        text="Second memory for combined crystal.",
        strength=0.8,
        confidence=0.5,
    )

    created_ids = DreamService(
        config,
        DeterministicDreamProvider(),
    ).apply_maintenance_actions(
        {
            "combine": [
                {
                    "source_crystal_ids": [second_id, first_id, first_id],
                    "content": "Combined memory text.",
                }
            ]
        },
        cycle_id=9,
    )

    combined = CrystalStore(config).get(created_ids["combine"][0])
    with connect(config.database_path) as conn:
        source_rows = conn.execute(
            """
            select source_crystal_id, target_crystal_id, link_type
            from crystal_links
            where target_crystal_id = ?
            order by source_crystal_id
            """,
            (combined.id,),
        ).fetchall()
        event_rows = conn.execute(
            """
            select crystal_id, event_type, evidence, cycle_id
            from memory_events
            order by id
            """
        ).fetchall()

    assert combined.text == "Combined memory text."
    assert combined.strength == pytest.approx(0.6)
    assert combined.confidence == pytest.approx(0.6)
    assert [
        (row["source_crystal_id"], row["target_crystal_id"], row["link_type"])
        for row in source_rows
    ] == [
        (first_id, combined.id, "combined_into"),
        (second_id, combined.id, "combined_into"),
    ]
    assert [(row["crystal_id"], row["event_type"], row["cycle_id"]) for row in event_rows] == [
        (combined.id, "maintenance_combine", 9)
    ]
    assert event_rows[0]["evidence"] == f"combined sources: {first_id}, {second_id}"


def test_apply_maintenance_combine_rejects_single_source(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    source_id = _add_crystal(
        config,
        context,
        text="Single source should not combine.",
    )

    with pytest.raises(ValueError, match="at least two distinct"):
        DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
            {
                "combine": [
                    {
                        "source_crystal_ids": [source_id],
                        "content": "Invalid combined memory.",
                    }
                ]
            }
        )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, text from crystals order by id",
        ).fetchall()
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert [(row["id"], row["text"]) for row in rows] == [
        (source_id, "Single source should not combine.")
    ]
    assert event_count == 0


def test_apply_maintenance_combine_rejects_duplicate_only_sources(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    source_id = _add_crystal(
        config,
        context,
        text="Duplicate-only source should not combine.",
    )

    with pytest.raises(ValueError, match="at least two distinct"):
        DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
            {
                "combine": [
                    {
                        "source_crystal_ids": [source_id, source_id],
                        "content": "Invalid combined memory.",
                    }
                ]
            }
        )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, text from crystals order by id",
        ).fetchall()
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert [(row["id"], row["text"]) for row in rows] == [
        (source_id, "Duplicate-only source should not combine.")
    ]
    assert event_count == 0


def test_apply_maintenance_combine_rejects_inactive_sources(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    active_id = _add_crystal(
        config,
        context,
        text="Active source.",
    )
    archived_id = _add_crystal(
        config,
        context,
        text="Archived source.",
        status="archived",
    )

    with pytest.raises(ValueError, match="active or candidate"):
        DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
            {
                "combine": [
                    {
                        "source_crystal_ids": [active_id, archived_id],
                        "content": "Invalid combined memory.",
                    }
                ]
            }
        )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, text, status from crystals order by id",
        ).fetchall()
        event_count = conn.execute("select count(*) from memory_events").fetchone()[0]

    assert [(row["id"], row["text"], row["status"]) for row in rows] == [
        (active_id, "Active source.", "active"),
        (archived_id, "Archived source.", "archived"),
    ]
    assert event_count == 0


def test_apply_maintenance_supersede_uses_safe_supersede_behavior(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    old_id = _add_crystal(
        config,
        context,
        text="Old rule-shaped lesson.",
    )
    new_id = _add_crystal(
        config,
        context,
        text="New rule-shaped lesson.",
    )

    DreamService(config, DeterministicDreamProvider()).apply_maintenance_actions(
        {
            "supersede": [
                {
                    "old_crystal_id": old_id,
                    "new_crystal_id": new_id,
                    "reason": "New rule is specific.",
                }
            ]
        },
        cycle_id=7,
    )

    old = CrystalStore(config).get(old_id)
    new = CrystalStore(config).get(new_id)
    with connect(config.database_path) as conn:
        event = conn.execute("select * from memory_events").fetchone()

    assert old.status == "superseded"
    assert new.supersedes_crystal_id == old_id
    assert event["event_type"] == "supersede"
    assert event["evidence"] == "New rule is specific."
    assert event["cycle_id"] == 7


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

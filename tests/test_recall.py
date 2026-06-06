import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
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
        task_type="translate",
        volume="1",
        chapter="2",
    )


def test_recall_records_activation_without_reinforcing(config: HieronymusConfig) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    crystals = CrystalStore(config)
    crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use terse inventory labels in system UI.",
        strength=0.25,
        confidence=0.75,
    )

    results = RecallService(config).recall(session.id, context, "inventory labels")

    assert results[0].crystal.id == crystal_id
    assert crystals.get(crystal_id).strength == 0.25
    assert crystals.get(crystal_id).confidence == 0.75
    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_id, session_id, recall_query, rank, score, reason, cycle_id
            from crystal_activations
            """
        ).fetchall()
    assert [dict(row) for row in rows] == [
        {
            "crystal_id": crystal_id,
            "session_id": session.id,
            "recall_query": "inventory labels",
            "rank": 1,
            "score": 1.0,
            "reason": "weighted search match",
            "cycle_id": None,
        }
    ]


def test_recall_adds_system_short_term_trace_with_metadata(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="concept",
        text="Render Sense as сенс in this series.",
    )

    RecallService(config).recall(session.id, context, "Sense")

    memories = workspace.list_short_term_memories(session.id)
    assert len(memories) == 1
    assert memories[0].source_role == "system"
    assert memories[0].kind == "recalled_crystal"
    assert memories[0].text == "Render Sense as сенс in this series."
    assert memories[0].metadata == {
        "crystal_id": crystal_id,
        "rank": 1,
        "score": 1.0,
    }


def test_recall_records_ranked_activations_ordered_by_search_results(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    crystals = CrystalStore(config)
    weaker_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.1,
        confidence=0.5,
    )
    stronger_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.9,
        confidence=0.5,
    )

    results = RecallService(config).recall(session.id, context, "guarded crafting")

    assert [(result.crystal.id, result.rank, result.score) for result in results] == [
        (stronger_id, 1, 1.0),
        (weaker_id, 2, 0.5),
    ]
    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_id, rank, score
            from crystal_activations
            order by rank
            """
        ).fetchall()
    assert [(row["crystal_id"], row["rank"], row["score"]) for row in rows] == [
        (stronger_id, 1, 1.0),
        (weaker_id, 2, 0.5),
    ]


def test_recall_unknown_session_raises_key_error(config: HieronymusConfig) -> None:
    context = _context(config)

    with pytest.raises(KeyError, match="session"):
        RecallService(config).recall(999, context, "anything")


def test_recall_requires_positive_limit(config: HieronymusConfig) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)

    with pytest.raises(ValueError, match="limit"):
        RecallService(config).recall(session.id, context, "anything", limit=0)


def test_recall_requires_active_session(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match="active"):
        RecallService(config).recall(session.id, context, "anything")

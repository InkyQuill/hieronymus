import pytest

from hieronymus.config import HieronymusConfig
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
        task_type="translate",
        volume="1",
        chapter="2",
    )


def test_workspace_records_short_term_memory(config: HieronymusConfig) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    memory_id = store.add_short_term_memory(
        session.id,
        source_role="mundane",
        kind="translation-memory",
        text="Render Sense as сенс in this series.",
        source_ref="v1c2",
        metadata={"importance": 4, "tags": ["term"]},
    )

    memories = store.list_short_term_memories(session.id)

    assert len(memories) == 1
    assert memories[0].id == memory_id
    assert memories[0].source_role == "mundane"
    assert memories[0].kind == "translation-memory"
    assert memories[0].text == "Render Sense as сенс in this series."
    assert memories[0].source_ref == "v1c2"
    assert memories[0].metadata == {"importance": 4, "tags": ["term"]}


def test_short_term_memory_accepts_positional_public_fields(
    config: HieronymusConfig,
) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    memory_id = store.add_short_term_memory(session.id, "user", "note", "Some text")

    memories = store.list_short_term_memories(session.id)
    assert memories[0].id == memory_id
    assert memories[0].source_role == "user"
    assert memories[0].kind == "note"
    assert memories[0].text == "Some text"


def test_complete_session_marks_session_completed(config: HieronymusConfig) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    store.complete_session(session.id)

    with connect(config.database_path) as conn:
        row = conn.execute(
            "select status, completed_at from task_sessions where id = ?",
            (session.id,),
        ).fetchone()

    assert row["status"] == "completed"
    assert row["completed_at"]


def test_short_term_memory_rejects_unknown_role(config: HieronymusConfig) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    with pytest.raises(ValueError, match="source_role"):
        store.add_short_term_memory(
            session.id,
            source_role="agent",
            kind="note",
            text="Keep the register formal.",
        )


def test_short_term_memory_requires_existing_active_session(
    config: HieronymusConfig,
) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))
    store.complete_session(session.id)

    with pytest.raises(KeyError, match="session"):
        store.add_short_term_memory(
            999,
            source_role="user",
            kind="note",
            text="Unknown sessions cannot receive memories.",
        )

    with pytest.raises(ValueError, match="active"):
        store.add_short_term_memory(
            session.id,
            source_role="user",
            kind="note",
            text="Completed sessions cannot receive memories.",
        )


def test_short_term_memory_is_searchable_in_fts(config: HieronymusConfig) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))
    memory_id = store.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep инвентарь for inventory UI references.",
    )

    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select rowid
            from short_term_memories_fts
            where short_term_memories_fts match ?
            """,
            ("инвентарь",),
        ).fetchone()

    assert row["rowid"] == memory_id

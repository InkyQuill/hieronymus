from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(
    config: HieronymusConfig,
    *,
    tags: tuple[str, ...] = (),
) -> TranslationContext:
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
        volume="5",
        chapter="5",
        tags=tags,
    )


def test_recall_returns_short_and_long_term_results_by_source(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep Sense untranslated in inventory UI labels.",
        metadata={"origin": "chapter-note"},
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Render Sense as сенс in glossary entries.",
        title="Sense Rendering",
    )

    results = RecallService(config).recall(session.id, context, "Sense", limit=5)

    assert {result.source for result in results} == {"short_term", "long_term"}
    short_result = next(result for result in results if result.source == "short_term")
    long_result = next(result for result in results if result.source == "long_term")
    assert short_result.short_term_memory is not None
    assert short_result.short_term_memory.id == memory_id
    assert short_result.short_term_memory.text == "Keep Sense untranslated in inventory UI labels."
    assert short_result.short_term_memory.metadata["origin"] == "chapter-note"
    assert short_result.crystal is None
    assert long_result.crystal is not None
    assert long_result.crystal.id == crystal_id
    assert long_result.crystal.text == "Render Sense as сенс in glossary entries."
    assert long_result.short_term_memory is None
    assert [result.rank for result in results] == list(range(1, len(results) + 1))


def test_story_scope_boosts_long_term_crystals_without_filtering(
    config: HieronymusConfig,
) -> None:
    context = _context(config, tags=("Book 5 Chapter 5",))
    crystals = CrystalStore(config)
    unscoped_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
    )
    matching_scope_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
        story_scopes=("Book 5 Chapter 5",),
    )
    other_scope_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
        story_scopes=("Book 4 Chapter 1",),
    )
    session = WorkspaceStore(config).start_session(context)

    results = RecallService(config).recall(
        session.id,
        context,
        "guarded crafting",
        limit=10,
    )

    assert [result.crystal.id for result in results] == [
        matching_scope_id,
        unscoped_id,
        other_scope_id,
    ]
    assert results[0].score > results[1].score
    assert [result.rank for result in results] == [1, 2, 3]
    assert {result.crystal.id for result in results} == {
        matching_scope_id,
        unscoped_id,
        other_scope_id,
    }


def test_hydrated_story_scopes_boost_long_term_crystals_without_legacy_tags(
    config: HieronymusConfig,
) -> None:
    base_context = _context(config)
    context = TranslationContext(
        series_slug=base_context.series_slug,
        source_language=base_context.source_language,
        target_language=base_context.target_language,
        task_type=base_context.task_type,
        volume=base_context.volume,
        chapter=base_context.chapter,
        story_scopes=("Book 5 Chapter 5",),
        semantic_tags=(),
    )
    crystals = CrystalStore(config)
    unscoped_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
    )
    matching_scope_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
        story_scopes=("Book 5 Chapter 5",),
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    hydrated_context = workspace.get_session(session.id).context

    results = RecallService(config).recall(
        session.id,
        hydrated_context,
        "guarded crafting",
        limit=2,
    )

    assert hydrated_context.tags == ()
    assert hydrated_context.story_scopes == ("Book 5 Chapter 5",)
    assert [result.crystal.id for result in results] == [matching_scope_id, unscoped_id]
    assert results[0].score > results[1].score


def test_story_scope_boost_applies_before_final_limit(
    config: HieronymusConfig,
) -> None:
    context = _context(config, tags=("Book 5 Chapter 5",))
    crystals = CrystalStore(config)
    first_unscoped_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.9,
        confidence=0.9,
    )
    second_unscoped_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.9,
        confidence=0.9,
    )
    matching_scope_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.5,
        confidence=0.5,
        story_scopes=("Book 5 Chapter 5",),
    )
    session = WorkspaceStore(config).start_session(context)

    results = RecallService(config).recall(
        session.id,
        context,
        "guarded crafting",
        limit=2,
    )

    assert [result.crystal.id for result in results] == [matching_scope_id, first_unscoped_id]
    assert second_unscoped_id not in {result.crystal.id for result in results}


def test_combined_recall_applies_final_limit_before_activation_records(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    for index in range(5):
        crystals.add_crystal(
            context,
            crystal_type="lesson",
            text=f"Use compact inventory labels for menu entry {index}.",
            strength=0.9,
            confidence=0.9,
        )
    for index in range(5):
        workspace.add_short_term_memory(
            session.id,
            source_role="mentor",
            kind="note",
            text=f"Compact inventory labels should stay terse in menu note {index}.",
        )

    results = RecallService(config).recall(
        session.id,
        context,
        "compact inventory labels",
        limit=3,
    )

    returned_long_term_count = sum(1 for result in results if result.source == "long_term")
    with connect(config.database_path) as conn:
        activation_count = conn.execute("select count(*) from crystal_activations").fetchone()[0]

    assert len(results) == 3
    assert [result.rank for result in results] == [1, 2, 3]
    assert activation_count == returned_long_term_count
    assert activation_count <= 3


def test_short_term_recall_preserves_fts_relevance_order(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    weak_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels.",
    )
    strong_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory inventory inventory inventory labels.",
    )

    results = RecallService(config).recall(
        session.id,
        context,
        "inventory",
        limit=2,
    )

    assert [result.short_term_memory.id for result in results] == [strong_id, weak_id]
    assert results[0].score > results[1].score


def test_short_term_recall_excludes_other_active_sessions(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    other_session = workspace.start_session(context)
    current_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels stay terse.",
    )
    other_id = workspace.add_short_term_memory(
        other_session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels use another session note.",
    )

    results = RecallService(config).recall(session.id, context, "Sense inventory", limit=5)

    assert [result.short_term_memory.id for result in results] == [current_id]
    assert other_id not in {result.short_term_memory.id for result in results}


def test_short_term_recall_excludes_archived_memories(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    active_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels stay terse.",
    )
    archived_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels use archived note.",
    )
    with connect(config.database_path) as conn:
        conn.execute(
            "update short_term_memories set archived_at = ? where id = ?",
            ("2026-06-09T00:00:00+00:00", archived_id),
        )
        conn.commit()

    results = RecallService(config).recall(session.id, context, "Sense inventory", limit=5)

    assert [result.short_term_memory.id for result in results] == [active_id]
    assert archived_id not in {result.short_term_memory.id for result in results}


def test_short_term_recall_escapes_operator_like_query_characters(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Sense inventory labels stay terse.",
    )

    results = RecallService(config).recall(
        session.id,
        context,
        '"Sense" OR (inventory) NEAR labels* !!!',
        limit=5,
    )

    assert [result.short_term_memory.id for result in results] == [memory_id]

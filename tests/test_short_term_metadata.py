from hieronymus.config import HieronymusConfig, load_config
from hieronymus.db import connect
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="JA",
        target_language="RU",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
        tags=(" term ", "ui"),
        language_tags=("ja", "ru", "en"),
        story_scopes=("volume:1", "chapter:2", "arc:academy"),
        semantic_tags=("ui", "term"),
    )


def test_translation_context_normalizes_compatibility_fields() -> None:
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="JA",
        target_language="RU",
        task_type="translate",
        volume=" 1 ",
        chapter=" 2 ",
        tags=(" term ", "", "ui"),
    )

    assert context.language_tags == ("ja", "ru")
    assert context.story_scopes == ("volume:1", "chapter:2")
    assert context.semantic_tags == ("term", "ui")


def test_translation_context_explicit_empty_canonical_metadata_stays_empty() -> None:
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translate",
        volume="1",
        chapter="2",
        tags=("term",),
        language_tags=(),
        story_scopes=(),
        semantic_tags=(),
    )

    assert context.language_tags == ()
    assert context.story_scopes == ()
    assert context.semantic_tags == ()


def test_session_creation_writes_typed_metadata(config: HieronymusConfig) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    loaded = store.get_session(session.id)

    assert loaded.context.language_tags == ("ja", "ru", "en")
    assert loaded.context.story_scopes == ("volume:1", "chapter:2", "arc:academy")
    assert loaded.context.semantic_tags == ("ui", "term")
    with connect(config.database_path) as conn:
        language_tags = conn.execute(
            """
            select language_tag
            from task_session_language_tags
            where session_id = ?
            order by language_tag
            """,
            (session.id,),
        ).fetchall()
        story_scopes = conn.execute(
            """
            select story_scope
            from task_session_story_scopes
            where session_id = ?
            order by story_scope
            """,
            (session.id,),
        ).fetchall()
        semantic_tags = conn.execute(
            """
            select semantic_tag
            from task_session_semantic_tags
            where session_id = ?
            order by semantic_tag
            """,
            (session.id,),
        ).fetchall()

    assert [row["language_tag"] for row in language_tags] == ["en", "ja", "ru"]
    assert [row["story_scope"] for row in story_scopes] == [
        "arc:academy",
        "chapter:2",
        "volume:1",
    ]
    assert [row["semantic_tag"] for row in semantic_tags] == ["term", "ui"]


def test_user_rule_short_term_memory_persists_typed_metadata(
    config: HieronymusConfig,
) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))

    memory_id = store.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Use Sense as the canonical game-system term.",
        language_tags=[" RU ", "en", "ru"],
        story_scopes=["chapter:2", " scene:boss "],
        semantic_tags=[" term ", "ui", "term"],
        source_credibility="user_rule",
        rule_intent="terminology",
        soft_origin="inline-correction",
    )

    memory = store.list_short_term_memories(session.id)[0]

    assert memory.id == memory_id
    assert memory.language_tags == ("ru", "en")
    assert memory.story_scopes == ("chapter:2", "scene:boss")
    assert memory.semantic_tags == ("term", "ui")
    assert memory.source_credibility == "user_rule"
    assert memory.rule_intent == "terminology"
    assert memory.soft_origin == "inline-correction"
    assert memory.metadata == {
        "language_tags": ["ru", "en"],
        "rule_intent": "terminology",
        "semantic_tags": ["term", "ui"],
        "sentence_count": 1,
        "soft_origin": "inline-correction",
        "source_credibility": "user_rule",
        "story_scopes": ["chapter:2", "scene:boss"],
    }

    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select source_credibility, rule_intent, soft_origin
            from short_term_memories
            where id = ?
            """,
            (memory_id,),
        ).fetchone()

    assert dict(row) == {
        "source_credibility": "user_rule",
        "rule_intent": "terminology",
        "soft_origin": "inline-correction",
    }


def test_short_term_memory_search_returns_typed_metadata(
    config: HieronymusConfig,
) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))
    store.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Keep inventory labels concise.",
        language_tags=["ru"],
        story_scopes=["chapter:2"],
        semantic_tags=["ui"],
        source_credibility="user_suggestion",
        rule_intent="style",
    )

    results = store.search_short_term_memories(session.id, "inventory")

    assert len(results) == 1
    assert results[0].language_tags == ("ru",)
    assert results[0].story_scopes == ("chapter:2",)
    assert results[0].semantic_tags == ("ui",)
    assert results[0].source_credibility == "user_suggestion"
    assert results[0].rule_intent == "style"


def test_short_term_memory_delete_cascades_typed_metadata(
    config: HieronymusConfig,
) -> None:
    store = WorkspaceStore(config)
    session = store.start_session(_context(config))
    memory_id = store.add_short_term_memory(
        session.id,
        source_role="user",
        kind="note",
        text="Cascade this metadata.",
        language_tags=["ru"],
        story_scopes=["chapter:2"],
        semantic_tags=["cleanup"],
    )

    with connect(config.database_path) as conn:
        conn.execute("delete from short_term_memories where id = ?", (memory_id,))
        conn.commit()
        counts = {
            table: conn.execute(
                f"select count(*) from {table} where memory_id = ?",
                (memory_id,),
            ).fetchone()[0]
            for table in (
                "short_term_memory_language_tags",
                "short_term_memory_story_scopes",
                "short_term_memory_semantic_tags",
            )
        }

    assert counts == {
        "short_term_memory_language_tags": 0,
        "short_term_memory_story_scopes": 0,
        "short_term_memory_semantic_tags": 0,
    }


def test_memory_store_add_short_term_memory_helper_returns_record(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = MemoryStore(config, context)

    memory = store.add_short_term_memory(
        session_id=None,
        content="Prefer Sense for センス in system text.",
        language_tags=["RU"],
        story_scopes=["chapter:2"],
        semantic_tags=["term"],
        source_credibility="user_rule",
        rule_intent="terminology",
    )

    assert memory.text == "Prefer Sense for センス in system text."
    assert memory.language_tags == ("ru",)
    assert memory.story_scopes == ("chapter:2",)
    assert memory.semantic_tags == ("term",)
    assert memory.source_credibility == "user_rule"
    assert memory.rule_intent == "terminology"


def test_mcp_short_term_add_exposes_typed_metadata(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    session_id = mcp_server.hieronymus_session_start(series.slug)["session_id"]
    added = mcp_server.hieronymus_short_term_add(
        session_id,
        source_role="user",
        kind="correction",
        text="Use Sense in game UI.",
        language_tags=["EN", "ja"],
        story_scopes=["chapter:1"],
        semantic_tags=["ui"],
        source_credibility="user_suggestion",
        rule_intent="terminology",
        soft_origin="mcp-test",
        metadata={"line": 12},
    )

    memory = WorkspaceStore(config).list_short_term_memories(session_id)[0]

    assert added == {"memory_id": 1}
    assert memory.language_tags == ("en", "ja")
    assert memory.story_scopes == ("chapter:1",)
    assert memory.semantic_tags == ("ui",)
    assert memory.source_credibility == "user_suggestion"
    assert memory.rule_intent == "terminology"
    assert memory.soft_origin == "mcp-test"
    assert memory.metadata == {
        "language_tags": ["en", "ja"],
        "line": 12,
        "rule_intent": "terminology",
        "semantic_tags": ["ui"],
        "sentence_count": 1,
        "soft_origin": "mcp-test",
        "source_credibility": "user_suggestion",
        "story_scopes": ["chapter:1"],
    }


def test_mcp_recall_accepts_hydrated_typed_session_context(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    config = load_config()
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language="ja",
        target_language="en",
        task_type="translation",
        language_tags=("ja", "en", "fr"),
        story_scopes=("arc:academy",),
        semantic_tags=("ui",),
    )
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="note",
        text="Keep Sense in UI labels.",
    )

    from hieronymus import mcp_server

    recalled = mcp_server.hieronymus_recall(session.id, series.slug, "Sense labels")

    assert recalled[0]["short_term_memory"]["id"] == memory_id

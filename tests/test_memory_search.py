import pytest

from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import TranslationContext
from hieronymus.rag import RagStore
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _create_memory_store(config) -> MemoryStore:
    series = Registry(config).create_series(
        slug="death-march",
        title="Death March to the Parallel World Rhapsody",
        source_language="ja",
        target_language="en",
    )
    context = TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
    )
    return MemoryStore(config, context)


def test_memory_search_returns_relevant_entries(config):
    store = _create_memory_store(config)
    memory_id = store.add(
        kind="translation_rationale",
        text="Satou's system messages should stay concise and game-like.",
        source_ref="user:2026-06-06",
        importance=4,
    )

    results = store.search("system messages")

    assert results[0].id == memory_id
    assert results[0].kind == "translation_rationale"
    assert results[0]["text"] == "Satou's system messages should stay concise and game-like."
    assert results[0].text == "Satou's system messages should stay concise and game-like."
    assert results[0].importance == 4
    assert results[0].source_ref == "user:2026-06-06"


def test_legacy_memory_add_is_searchable_as_short_term_then_long_term(config):
    memory = MemoryStore(config)

    memory_id = memory.add(
        series_slug="only-sense-online",
        kind="translation_rationale",
        text="Use Yun for ユン.",
        source_ref="chapter-1",
        importance=4,
    )

    assert memory_id == 1
    results = memory.search("only-sense-online", "Yun")
    assert results[0]["text"] == "Use Yun for ユン."
    assert results[0].kind == "translation_rationale"
    assert results[0].importance == 4
    assert results[0].source_ref == "chapter-1"
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from crystals").fetchone()[0] == 0
        row = conn.execute("select kind, metadata_json from short_term_memories").fetchone()
    assert row["kind"] == "note"
    assert '"legacy_kind": "translation_rationale"' in row["metadata_json"]
    assert '"importance": 4' in row["metadata_json"]
    assert '"storage_semantics": "short_term_until_dreamed"' in row["metadata_json"]


def test_memory_search_unknown_series_is_read_only_for_blank_and_nonblank_queries(config):
    memory = MemoryStore(config)

    assert memory.search("missing-series", "   ") == []
    assert memory.search("missing-series", "Yun") == []
    with connect(config.database_path) as conn:
        assert conn.execute("select count(*) from series").fetchone()[0] == 0
        assert conn.execute("select count(*) from task_sessions").fetchone()[0] == 0


def test_memory_search_without_active_session_falls_back_to_short_and_long_term_fts(config):
    store = _create_memory_store(config)
    short_id = store.add(
        kind="translation_rationale",
        text="Yun keeps her name in village dialogue.",
        importance=4,
    )
    with connect(config.database_path) as conn:
        conn.execute("update task_sessions set status = 'completed', completed_at = created_at")
        conn.commit()
    CrystalStore(config).add_crystal(
        TranslationContext(
            series_slug="death-march",
            source_language="ja",
            target_language="en",
            task_type="translation",
        ),
        crystal_type="observation",
        text="Yun is a recurring merchant contact.",
        title="name_note",
        strength=0.2,
        confidence=0.5,
    )

    results = store.search("Yun", limit=10)

    assert {result.text for result in results} == {
        "Yun keeps her name in village dialogue.",
        "Yun is a recurring merchant contact.",
    }
    assert next(result for result in results if result.id == short_id).source_ref == ""


def test_memory_search_fallback_filters_short_term_by_full_context(config):
    series = Registry(config).create_series(
        slug="death-march",
        title="Death March",
        source_language="ja",
        target_language="en",
    )
    current = MemoryStore(
        config,
        TranslationContext(
            series_slug=series.slug,
            source_language="ja",
            target_language="en",
            task_type="translation",
            volume="1",
            chapter="1",
        ),
    )
    other = MemoryStore(
        config,
        TranslationContext(
            series_slug=series.slug,
            source_language="ja",
            target_language="en",
            task_type="translation",
            volume="2",
            chapter="1",
        ),
    )
    current.add(kind="translation_rationale", text="Yun belongs to the first volume.")
    other.add(kind="translation_rationale", text="Yun belongs to the second volume.")
    with connect(config.database_path) as conn:
        conn.execute("update task_sessions set status = 'completed', completed_at = created_at")
        conn.commit()

    results = current.search("Yun", limit=10)

    assert [result.text for result in results] == ["Yun belongs to the first volume."]


def test_memory_search_encodes_colliding_long_term_ids_without_changing_short_term_id(config):
    store = _create_memory_store(config)
    short_id = store.add(
        kind="translation_rationale",
        text="Yun keeps her short-term naming note.",
        importance=5,
    )
    with connect(config.database_path) as conn:
        conn.execute("update task_sessions set status = 'completed', completed_at = created_at")
        conn.commit()
    crystal_id = CrystalStore(config).add_crystal(
        TranslationContext(
            series_slug="death-march",
            source_language="ja",
            target_language="en",
            task_type="translation",
        ),
        crystal_type="observation",
        text="Yun keeps her long-term naming note.",
        title="translation_rationale",
        strength=1.0,
        confidence=0.5,
    )
    assert short_id == crystal_id

    results = store.search("Yun", limit=10)

    assert [result.id for result in results] == [short_id, -crystal_id]
    assert [result.text for result in results] == [
        "Yun keeps her short-term naming note.",
        "Yun keeps her long-term naming note.",
    ]


def test_memory_search_encodes_colliding_ids_in_active_recall_results(config):
    store = _create_memory_store(config)
    short_id = store.add(
        kind="translation_rationale",
        text="Yun keeps her active short-term note.",
        importance=5,
    )
    crystal_id = CrystalStore(config).add_crystal(
        TranslationContext(
            series_slug="death-march",
            source_language="ja",
            target_language="en",
            task_type="translation",
        ),
        crystal_type="observation",
        text="Yun keeps her active long-term note.",
        title="translation_rationale",
        strength=1.0,
        confidence=0.5,
    )
    assert short_id == crystal_id

    results = store.search("Yun", limit=10)

    assert [result.id for result in results] == [short_id, -crystal_id]


def test_memory_search_ignores_rag_only_active_recall_results(config, tmp_path):
    store = _create_memory_store(config)
    assert store.context is not None
    WorkspaceStore(config).start_session(store.context)
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nYun,Юн\n", encoding="utf-8")
    RagStore(config).import_file(store.context.series_slug, source, source_ref="glossary.csv")

    results = store.search("Yun", limit=5)

    assert results == []


@pytest.mark.parametrize("query", ["game-like", "Satou's", "system OR"])
def test_memory_search_treats_operator_heavy_query_as_plain_text(config, query):
    store = _create_memory_store(config)
    store.add(
        kind="translation_rationale",
        text="Satou's system messages should stay concise and game-like.",
    )

    results = store.search(query)

    assert [result.text for result in results] == [
        "Satou's system messages should stay concise and game-like."
    ]


def test_memory_search_returns_empty_list_for_blank_query(config):
    store = _create_memory_store(config)
    store.add(kind="translation_rationale", text="Satou's system messages stay concise.")

    assert store.search("   ") == []


def test_memory_search_orders_matches_by_importance_desc(config):
    store = _create_memory_store(config)
    store.add(kind="translation_rationale", text="System messages use dry wording.", importance=1)
    store.add(
        kind="translation_rationale", text="System messages use compact wording.", importance=5
    )

    results = store.search("system messages")

    assert [result.importance for result in results] == [5, 1]


@pytest.mark.parametrize("limit", [0, -1])
def test_memory_search_rejects_non_positive_limit(config, limit):
    store = _create_memory_store(config)

    with pytest.raises(ValueError, match="limit must be at least 1"):
        store.search("system", limit=limit)


def test_memory_search_caps_large_limit(config):
    store = _create_memory_store(config)
    for index in range(55):
        store.add(kind="translation_rationale", text=f"System memory entry {index}.")

    results = store.search("system", limit=1000)

    assert len(results) == 50


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        (
            {"kind": "   ", "text": "System messages stay concise."},
            "kind must not be empty",
        ),
        ({"kind": "translation_rationale", "text": "   "}, "text must not be empty"),
    ],
)
def test_memory_add_rejects_blank_required_text(config, kwargs, message):
    store = _create_memory_store(config)

    with pytest.raises(ValueError, match=message):
        store.add(**kwargs)

import pytest

from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry


def _create_memory_store(config) -> MemoryStore:
    series = Registry(config).create_series(
        slug="death-march",
        title="Death March to the Parallel World Rhapsody",
        source_language="ja",
        target_language="en",
    )
    return MemoryStore(series.database_path)


def test_memory_search_returns_relevant_entries(config):
    store = _create_memory_store(config)
    store.add(
        kind="translation_rationale",
        text="Satou's system messages should stay concise and game-like.",
        source_ref="user:2026-06-06",
        importance=4,
    )

    results = store.search("system messages")

    assert results[0].text == "Satou's system messages should stay concise and game-like."


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

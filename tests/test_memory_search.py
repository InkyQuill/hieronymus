from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry


def test_memory_search_returns_relevant_entries(config):
    series = Registry(config).create_series(
        slug="death-march",
        title="Death March to the Parallel World Rhapsody",
        source_language="ja",
        target_language="en",
    )
    store = MemoryStore(series.database_path)
    store.add(
        kind="translation_rationale",
        text="Satou's system messages should stay concise and game-like.",
        source_ref="user:2026-06-06",
        importance=4,
    )

    results = store.search("system messages")

    assert results[0].text == "Satou's system messages should stay concise and game-like."

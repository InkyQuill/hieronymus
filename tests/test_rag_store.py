from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.rag import RagStore
from hieronymus.registry import Registry


def _series(config: HieronymusConfig) -> str:
    return (
        Registry(config)
        .create_series(
            slug="only-sense-online",
            title="Only Sense Online",
            source_language="ja",
            target_language="ru",
        )
        .slug
    )


def test_import_file_indexes_chunks_and_searches(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.\n\nCooking Talent appears here.", encoding="utf-8")

    result = RagStore(config).import_file(
        series_slug,
        path,
        source_ref="chapter-5.txt",
        source_type="auto",
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("chapter:source",),
    )
    hits = RagStore(config).search(series_slug, "Cooking Talent", limit=5)

    assert result.chunk_count == 2
    assert result.skipped is False
    assert [hit.chunk.text for hit in hits] == ["Cooking Talent appears here."]
    assert hits[0].reason == "rag project text match"
    assert hits[0].chunk.source_ref == "chapter-5.txt"
    assert hits[0].chunk.language_tags == ("ja", "ru")
    assert hits[0].chunk.story_scopes == ("book:5/chapter:5",)
    assert hits[0].chunk.semantic_tags == ("chapter:source",)


def test_import_file_skips_unchanged_checksum(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.", encoding="utf-8")
    store = RagStore(config)

    first = store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")
    second = store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    assert first.skipped is False
    assert second.skipped is True
    assert second.chunk_count == first.chunk_count


def test_same_checksum_with_changed_format_reindexes_source(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    text_path = tmp_path / "source.txt"
    markdown_path = tmp_path / "source.md"
    content = "# Sense\n\nSense menu note."
    text_path.write_text(content, encoding="utf-8")
    markdown_path.write_text(content, encoding="utf-8")
    store = RagStore(config)

    first = store.import_file(series_slug, text_path, source_ref="source", source_type="auto")
    second = store.import_file(
        series_slug,
        markdown_path,
        source_ref="source",
        source_type="auto",
    )
    hit = store.search(series_slug, "Sense menu", limit=1)[0]

    assert first.source.content_type == "txt"
    assert second.skipped is False
    assert second.source.source_type == "markdown"
    assert second.source.content_type == "md"
    assert hit.chunk.chunk_kind == "markdown_section"
    assert hit.chunk.location == "Sense paragraph 1"


def test_reimport_with_same_checksum_refreshes_chunk_tags(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.", encoding="utf-8")
    store = RagStore(config)
    store.import_file(
        series_slug,
        path,
        source_ref="chapter.txt",
        source_type="auto",
        language_tags=("ja",),
        story_scopes=("book:5/chapter:1",),
        semantic_tags=("draft",),
    )

    result = store.import_file(
        series_slug,
        path,
        source_ref="chapter.txt",
        source_type="auto",
        language_tags=("ru",),
        story_scopes=("book:5/chapter:2",),
        semantic_tags=("approved",),
    )
    hit = store.search(series_slug, "Sense", limit=5)[0]

    assert result.skipped is True
    assert hit.chunk.language_tags == ("ru",)
    assert hit.chunk.story_scopes == ("book:5/chapter:2",)
    assert hit.chunk.semantic_tags == ("approved",)


def test_changed_import_replaces_old_chunks(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    store = RagStore(config)
    path.write_text("Old Sense note.", encoding="utf-8")
    store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    path.write_text("New Cooking Talent note.", encoding="utf-8")
    store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    assert store.search(series_slug, "Old", limit=5) == []
    assert [hit.chunk.text for hit in store.search(series_slug, "Cooking Talent", limit=5)] == [
        "New Cooking Talent note."
    ]


def test_failed_changed_import_preserves_old_chunks(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    good_path = tmp_path / "glossary.json"
    store = RagStore(config)
    good_path.write_text('[{"source": "Sense", "target": "Сенс"}]', encoding="utf-8")
    store.import_file(series_slug, good_path, source_ref="glossary.json", source_type="auto")

    good_path.write_text("{broken json", encoding="utf-8")
    with pytest.raises(ValueError, match="invalid RAG source"):
        store.import_file(series_slug, good_path, source_ref="glossary.json", source_type="auto")

    assert [
        hit.chunk.metadata["source"] for hit in store.search(series_slug, "Sense", limit=5)
    ] == ["Sense"]


def test_glossary_hits_get_glossary_reason(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс\n", encoding="utf-8")

    RagStore(config).import_file(series_slug, path, source_ref="glossary.csv", source_type="auto")
    hits = RagStore(config).search(series_slug, "Sense", limit=5)

    assert hits[0].reason == "rag glossary match"
    assert hits[0].chunk.chunk_kind == "glossary_entry"
    assert hits[0].score > 0


def test_search_boosts_matching_rag_chunk_metadata(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    store = RagStore(config)
    general_path = tmp_path / "general.txt"
    scoped_path = tmp_path / "scoped.txt"
    general_path.write_text("Sense shared evidence.", encoding="utf-8")
    scoped_path.write_text("Sense shared evidence.", encoding="utf-8")
    store.import_file(series_slug, general_path, source_ref="general.txt", source_type="auto")
    store.import_file(
        series_slug,
        scoped_path,
        source_ref="scoped.txt",
        source_type="auto",
        language_tags=("ru",),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )

    hits = store.search(
        series_slug,
        "Sense shared evidence",
        limit=2,
        language_tags=("ru",),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )

    assert [hit.chunk.source_ref for hit in hits] == ["scoped.txt", "general.txt"]
    assert hits[0].score > hits[1].score


def test_default_source_ref_keeps_same_basename_paths_distinct(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    first_path = tmp_path / "first" / "chapter.txt"
    second_path = tmp_path / "second" / "chapter.txt"
    first_path.parent.mkdir()
    second_path.parent.mkdir()
    first_path.write_text("Alpha Sense note.", encoding="utf-8")
    second_path.write_text("Beta Cooking Talent note.", encoding="utf-8")
    store = RagStore(config)

    first = store.import_file(series_slug, first_path, source_type="auto")
    second = store.import_file(series_slug, second_path, source_type="auto")

    assert first.source.source_ref == str(first_path)
    assert second.source.source_ref == str(second_path)
    assert [hit.chunk.text for hit in store.search(series_slug, "Alpha", limit=5)] == [
        "Alpha Sense note."
    ]
    assert [hit.chunk.text for hit in store.search(series_slug, "Beta", limit=5)] == [
        "Beta Cooking Talent note."
    ]


def test_search_caps_large_limit_at_fifty(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text(
        "\n\n".join(f"Sense indexed note {index}." for index in range(60)),
        encoding="utf-8",
    )
    store = RagStore(config)
    store.import_file(series_slug, path, source_type="auto")

    assert len(store.search(series_slug, "Sense", limit=1000)) == 50

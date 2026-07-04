import hashlib
import sqlite3
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.rag import RagStore, load_rag_file
from hieronymus.rag_models import RagChunkRecord, RagImportResult, RagSourceRecord
from hieronymus.registry import Registry

NOW = "2026-01-01T00:00:00Z"


def _insert_series(conn: sqlite3.Connection, slug: str) -> None:
    conn.execute(
        """
        insert into series(
          slug,
          title,
          default_source_language,
          default_target_language,
          created_at,
          updated_at
        )
        values (?, ?, 'ja', 'ru', ?, ?)
        """,
        (slug, slug.upper(), NOW, NOW),
    )


def _insert_rag_source(
    conn: sqlite3.Connection,
    *,
    source_id: int = 1,
    series_slug: str = "oso",
    source_ref: str = "glossary.csv",
) -> None:
    conn.execute(
        """
        insert into rag_sources(
          id,
          series_slug,
          source_ref,
          source_type,
          content_type,
          checksum,
          created_at,
          updated_at
        )
        values (?, ?, ?, 'glossary', 'csv', 'abc', ?, ?)
        """,
        (source_id, series_slug, source_ref, NOW, NOW),
    )


def _insert_rag_chunk(
    conn: sqlite3.Connection,
    *,
    chunk_id: int = 1,
    source_id: int = 1,
    series_slug: str = "oso",
    text: str = "Sense moonstone",
    display_text: str = "Sense moonstone",
    location: str = "row 2",
) -> None:
    conn.execute(
        """
        insert into rag_chunks(
          id,
          source_id,
          series_slug,
          chunk_kind,
          text,
          display_text,
          location,
          created_at
        )
        values (?, ?, ?, 'glossary_entry', ?, ?, ?, ?)
        """,
        (chunk_id, source_id, series_slug, text, display_text, location, NOW),
    )


def _fts_match_count(conn: sqlite3.Connection, query: str) -> int:
    row = conn.execute(
        """
        select count(*) as count
        from rag_chunks_fts
        where rag_chunks_fts match ?
        """,
        (query,),
    ).fetchone()
    return int(row["count"])


def test_rag_schema_is_created_idempotently(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        apply_migration(conn, "global.sql")
        table_names = {
            row["name"]
            for row in conn.execute(
                """
                select name
                from sqlite_master
                where type in ('table', 'virtual')
                  and name like 'rag_%'
                """
            ).fetchall()
        }

    assert {
        "rag_sources",
        "rag_chunks",
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
        "rag_chunks_fts",
    } <= table_names


def test_rag_chunk_series_must_match_source_series(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        _insert_series(conn, "oso")
        _insert_series(conn, "mti")
        _insert_rag_source(conn, series_slug="oso")

        with pytest.raises(sqlite3.IntegrityError):
            _insert_rag_chunk(conn, series_slug="mti")


def test_rag_chunk_fts_triggers_sync_insert_update_and_delete(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        _insert_series(conn, "oso")
        _insert_rag_source(conn, series_slug="oso")

        _insert_rag_chunk(conn, text="Sense moonstone", display_text="Moonstone entry")

        assert _fts_match_count(conn, "moonstone") == 1

        conn.execute(
            """
            update rag_chunks
            set text = 'Sense sunstone',
                display_text = 'Sunstone entry'
            where id = 1
            """
        )

        assert _fts_match_count(conn, "moonstone") == 0
        assert _fts_match_count(conn, "sunstone") == 1

        conn.execute("delete from rag_chunks where id = 1")

        assert _fts_match_count(conn, "sunstone") == 0


def test_rag_dataclasses_expose_payload_fields() -> None:
    source = RagSourceRecord(
        id=1,
        series_slug="oso",
        source_ref="glossary.csv",
        source_type="glossary",
        content_type="csv",
        checksum="abc",
        metadata={},
    )
    chunk = RagChunkRecord(
        id=2,
        source_id=source.id,
        series_slug="oso",
        source_ref=source.source_ref,
        chunk_kind="glossary_entry",
        text="Sense -> Сенс",
        display_text="Sense -> Сенс",
        location="row 2",
        metadata={"source": "Sense", "target": "Сенс"},
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )
    result = RagImportResult(source=source, chunk_count=1, skipped=False)

    assert result.source.source_ref == "glossary.csv"
    assert chunk.title == "glossary.csv row 2"
    assert chunk.kind == "glossary_entry"


def test_text_file_is_chunked_by_paragraph(tmp_path: Path) -> None:
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.\n\nCooking Talent appears here.", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.checksum == hashlib.sha256(path.read_bytes()).hexdigest()
    assert parsed.content_type == "txt"
    assert parsed.source_type == "text"
    assert [chunk.text for chunk in parsed.chunks] == [
        "Sense menu note.",
        "Cooking Talent appears here.",
    ]
    assert [chunk.location for chunk in parsed.chunks] == ["paragraph 1", "paragraph 2"]


def test_text_file_splits_oversized_paragraphs(tmp_path: Path) -> None:
    path = tmp_path / "chapter.txt"
    path.write_text(("Sense " * 230).strip(), encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert len(parsed.chunks) > 1
    assert all(len(chunk.text) <= 1200 for chunk in parsed.chunks)


def test_markdown_file_preserves_heading_location(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text(
        "# Glossary\n\nSense stays untranslated.\n\n## Skills\n\nEnchant is a skill.",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.content_type == "md"
    assert parsed.source_type == "markdown"
    assert [(chunk.location, chunk.text) for chunk in parsed.chunks] == [
        ("Glossary paragraph 1", "Sense stays untranslated."),
        ("Glossary > Skills paragraph 2", "Enchant is a skill."),
    ]


def test_markdown_file_splits_oversized_paragraphs(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text("# Glossary\n\n" + ("Sense " * 230).strip(), encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert len(parsed.chunks) > 1
    assert all(len(chunk.text) <= 1200 for chunk in parsed.chunks)


def test_markdown_split_paragraphs_do_not_shift_following_locations(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text(
        "# Glossary\n\n" + ("Sense " * 230).strip() + "\n\nNext paragraph.",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.chunks[-1].location == "Glossary paragraph 2"


def test_csv_file_turns_rows_into_glossary_chunks(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text(
        "source,target,category\nSense,Сенс,skill\nEnchant,Зачарование,skill\n",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.content_type == "csv"
    assert parsed.chunks[0].chunk_kind == "glossary_entry"
    assert parsed.chunks[0].location == "row 2"
    assert parsed.chunks[0].metadata == {
        "source": "Sense",
        "target": "Сенс",
        "category": "skill",
    }
    assert "Sense" in parsed.chunks[0].text
    assert "Сенс" in parsed.chunks[0].text


def test_csv_file_rejects_rows_with_extra_fields(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс,extra\n", encoding="utf-8")

    with pytest.raises(ValueError):
        load_rag_file(path, source_type="auto")


def test_glossary_file_rejects_invalid_source_type(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс\n", encoding="utf-8")

    with pytest.raises(ValueError):
        load_rag_file(path, source_type="bad")


def test_json_file_accepts_list_of_glossary_entries(tmp_path: Path) -> None:
    path = tmp_path / "glossary.json"
    path.write_text(
        '[{"source": "Sense", "target": "Сенс", "note": "menu term"}]',
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.chunks[0].location == "entry 1"
    assert parsed.chunks[0].metadata["note"] == "menu term"


def test_yaml_file_accepts_mapping_entries(tmp_path: Path) -> None:
    path = tmp_path / "glossary.yaml"
    path.write_text("Sense:\n  target: Сенс\n  category: skill\n", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.chunks[0].metadata == {
        "key": "Sense",
        "target": "Сенс",
        "category": "skill",
    }


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

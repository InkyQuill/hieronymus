import sqlite3
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.rag import load_rag_file
from hieronymus.rag_models import RagChunkRecord, RagImportResult, RagSourceRecord

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

    assert parsed.content_type == "txt"
    assert parsed.source_type == "text"
    assert [chunk.text for chunk in parsed.chunks] == [
        "Sense menu note.",
        "Cooking Talent appears here.",
    ]
    assert [chunk.location for chunk in parsed.chunks] == ["paragraph 1", "paragraph 2"]


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

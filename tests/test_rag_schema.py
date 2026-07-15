import sqlite3

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

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

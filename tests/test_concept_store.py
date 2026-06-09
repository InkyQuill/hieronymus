import sqlite3

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

_NOW = "2026-06-09T00:00:00+00:00"


def test_memory_design_tables_exist(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        table_names = {
            row["name"]
            for row in conn.execute(
                "select name from sqlite_master where type in ('table', 'view')"
            )
        }

    assert {
        "concepts",
        "concepts_fts",
        "concept_facets",
        "concept_semantic_tags",
        "concept_renames",
        "crystal_concepts",
        "crystal_story_scopes",
        "crystal_semantic_tags",
        "dream_audit_entries",
        "dream_phase_runs",
    }.issubset(table_names)


def test_concepts_fts_searches_inserted_concept(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(
            conn,
            canonical_name="Sense",
            description="A game-like aptitude category.",
        )
        conn.execute(
            "insert into concepts_fts(rowid, canonical_name, description) values (?, ?, ?)",
            (concept_id, "Sense", "A game-like aptitude category."),
        )
        rows = conn.execute(
            "select rowid from concepts_fts where concepts_fts match ?",
            ("aptitude",),
        ).fetchall()

    assert [row["rowid"] for row in rows] == [concept_id]


def test_concept_semantic_tags_cascade_when_concept_is_deleted(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(conn)
        conn.execute(
            """
            insert into concept_semantic_tags(concept_id, tag, created_at)
            values (?, 'term:system', ?)
            """,
            (concept_id, _NOW),
        )
        conn.execute("delete from concepts where id = ?", (concept_id,))
        remaining = conn.execute("select count(*) from concept_semantic_tags").fetchone()[0]

    assert remaining == 0


def test_concept_facet_source_crystal_nulls_when_crystal_is_deleted(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(conn)
        crystal_id = _insert_crystal(conn)
        facet_id = _insert_concept_facet(conn, concept_id, source_crystal_id=crystal_id)

        conn.execute("delete from crystals where id = ?", (crystal_id,))
        row = conn.execute(
            "select source_crystal_id from concept_facets where id = ?",
            (facet_id,),
        ).fetchone()

    assert row["source_crystal_id"] is None


def test_duplicate_concept_semantic_tag_is_rejected(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(conn)
        conn.execute(
            """
            insert into concept_semantic_tags(concept_id, tag, created_at)
            values (?, 'term:system', ?)
            """,
            (concept_id, _NOW),
        )

        with pytest.raises(sqlite3.IntegrityError):
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, created_at)
                values (?, 'term:system', ?)
                """,
                (concept_id, _NOW),
            )


def test_duplicate_crystal_concept_link_is_rejected(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(conn)
        crystal_id = _insert_crystal(conn)
        conn.execute(
            """
            insert into crystal_concepts(crystal_id, concept_id, link_type, created_at)
            values (?, ?, 'mentions', ?)
            """,
            (crystal_id, concept_id, _NOW),
        )

        with pytest.raises(sqlite3.IntegrityError):
            conn.execute(
                """
                insert into crystal_concepts(crystal_id, concept_id, link_type, created_at)
                values (?, ?, 'mentions', ?)
                """,
                (crystal_id, concept_id, _NOW),
            )


def _insert_concept(
    conn,
    *,
    canonical_name: str = "Sense",
    description: str = "A game-like aptitude category.",
) -> int:
    cursor = conn.execute(
        """
        insert into concepts(canonical_name, description, created_at, updated_at)
        values (?, ?, ?, ?)
        """,
        (canonical_name, description, _NOW, _NOW),
    )
    return int(cursor.lastrowid)


def _insert_crystal(conn) -> int:
    cursor = conn.execute(
        """
        insert into crystals(
          crystal_type,
          text,
          title,
          scope_type,
          scope_key,
          series_slug,
          source_language,
          target_language,
          strength,
          confidence,
          status,
          created_at,
          updated_at
        )
        values (
          'lesson',
          'Use terse inventory labels.',
          'Inventory labels',
          'series',
          'series:only-sense-online',
          'only-sense-online',
          'ja',
          'ru',
          0.5,
          0.8,
          'active',
          ?,
          ?
        )
        """,
        (_NOW, _NOW),
    )
    return int(cursor.lastrowid)


def _insert_concept_facet(
    conn,
    concept_id: int,
    *,
    source_crystal_id: int | None = None,
) -> int:
    cursor = conn.execute(
        """
        insert into concept_facets(
          concept_id,
          language,
          facet_type,
          value,
          source_crystal_id,
          created_at,
          updated_at
        )
        values (?, 'ja', 'source_form', 'Sense', ?, ?, ?)
        """,
        (concept_id, source_crystal_id, _NOW, _NOW),
    )
    return int(cursor.lastrowid)

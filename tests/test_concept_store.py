import sqlite3

import pytest

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import TranslationContext

_NOW = "2026-06-09T00:00:00+00:00"


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translate",
    )


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


def test_concept_store_creates_vague_then_solid_concept(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)

    concept_id = store.create_or_reinforce(
        " Sense ",
        description="A game-like aptitude category.",
        tags=(" term:skill ", "term:skill", "", "domain:system"),
        confidence_delta=0.4,
    )
    first = store.get(concept_id)

    assert first.canonical_name == "Sense"
    assert first.description == "A game-like aptitude category."
    assert first.status == "vague"
    assert first.confidence == 0.4
    assert first.tags == ("domain:system", "term:skill")

    reinforced_id = store.create_or_reinforce(
        "Sense",
        description="A reinforced description.",
        tags=("term:skill", "plot:core"),
        confidence_delta=0.4,
    )
    reinforced = store.get(reinforced_id)

    assert reinforced_id == concept_id
    assert reinforced.description == "A reinforced description."
    assert reinforced.status == "solid"
    assert reinforced.confidence == 0.8
    assert reinforced.tags == ("domain:system", "plot:core", "term:skill")


def test_concept_store_reinforces_only_matching_scope(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)

    global_id = store.create_or_reinforce(
        "Sense",
        confidence_delta=0.3,
        scope_type="global",
        scope_key="",
    )
    project_id = store.create_or_reinforce(
        "Sense",
        confidence_delta=0.6,
        scope_type="project",
        scope_key="oso",
    )

    reinforced_id = store.create_or_reinforce(
        "Sense",
        confidence_delta=0.2,
        scope_type="project",
        scope_key="oso",
    )

    assert global_id != project_id
    assert reinforced_id == project_id
    assert store.get(global_id).confidence == 0.3
    assert store.get(project_id).confidence == 0.8
    assert store.get(global_id).scope_type == "global"
    assert store.get(global_id).scope_key == ""
    assert store.get(project_id).scope_type == "project"
    assert store.get(project_id).scope_key == "oso"


def test_concept_store_rejects_empty_canonical_name(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)

    with pytest.raises(ValueError, match="concept canonical_name must not be empty"):
        store.create_or_reinforce("   ")


def test_concept_store_links_crystal_to_multiple_concepts(
    config: HieronymusConfig,
) -> None:
    concept_store = ConceptStore(config)
    first_id = concept_store.create_or_reinforce("Sense")
    second_id = concept_store.create_or_reinforce("Crafting")
    crystal_id = CrystalStore(config).add_crystal(
        _context(),
        crystal_type="lesson",
        text="Sense and crafting terminology stay stable.",
    )

    concept_store.link_crystal(crystal_id, first_id, link_type="mentions", confidence=0.4)
    concept_store.link_crystal(crystal_id, second_id, link_type="mentions", confidence=0.8)
    concept_store.link_crystal(crystal_id, first_id, link_type="mentions", confidence=0.9)

    assert concept_store.concept_ids_for_crystal(crystal_id) == (first_id, second_id)
    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select confidence
            from crystal_concepts
            where crystal_id = ? and concept_id = ?
            """,
            (crystal_id, first_id),
        ).fetchone()

    assert row["confidence"] == 0.9


def test_concepts_fts_searches_inserted_concept(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(
            conn,
            canonical_name="Sense",
            description="A game-like aptitude category.",
        )
        rows = conn.execute(
            "select rowid from concepts_fts where concepts_fts match ?",
            ("aptitude",),
        ).fetchall()

    assert [row["rowid"] for row in rows] == [concept_id]


def test_concepts_fts_updates_when_concept_changes(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(
            conn,
            canonical_name="Sense",
            description="A game-like aptitude category.",
        )

        conn.execute(
            """
            update concepts
            set canonical_name = 'Artisan Focus',
                description = 'A crafting concentration category.',
                updated_at = ?
            where id = ?
            """,
            (_NOW, concept_id),
        )
        old_rows = conn.execute(
            "select rowid from concepts_fts where concepts_fts match ?",
            ("aptitude",),
        ).fetchall()
        new_rows = conn.execute(
            "select rowid from concepts_fts where concepts_fts match ?",
            ("concentration",),
        ).fetchall()

    assert old_rows == []
    assert [row["rowid"] for row in new_rows] == [concept_id]


def test_concepts_fts_deletes_when_concept_is_deleted(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(
            conn,
            canonical_name="Sense",
            description="A game-like aptitude category.",
        )

        conn.execute("delete from concepts where id = ?", (concept_id,))
        rows = conn.execute(
            "select rowid from concepts_fts where concepts_fts match ?",
            ("aptitude",),
        ).fetchall()

    assert rows == []


def test_concept_insert_helper_uses_global_scope_defaults(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        concept_id = _insert_concept(conn)
        row = conn.execute(
            "select scope_type, scope_key from concepts where id = ?",
            (concept_id,),
        ).fetchone()

    assert dict(row) == {"scope_type": "global", "scope_key": ""}


def test_duplicate_concept_identity_in_same_scope_is_rejected(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        _insert_concept(conn, canonical_name="Sense")

        with pytest.raises(sqlite3.IntegrityError):
            _insert_concept(conn, canonical_name="Sense")


def test_same_concept_name_can_exist_in_different_scopes(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        global_id = _insert_concept(conn, canonical_name="Sense")
        project_id = _insert_concept(
            conn,
            canonical_name="Sense",
            scope_type="project",
            scope_key="oso",
        )
        rows = conn.execute(
            """
            select id, scope_type, scope_key
            from concepts
            where canonical_name = 'Sense'
            order by id
            """
        ).fetchall()

    assert [dict(row) for row in rows] == [
        {"id": global_id, "scope_type": "global", "scope_key": ""},
        {"id": project_id, "scope_type": "project", "scope_key": "oso"},
    ]


def test_global_concept_scope_rejects_non_empty_key(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")

        with pytest.raises(sqlite3.IntegrityError):
            _insert_concept(
                conn,
                canonical_name="Sense",
                scope_type="global",
                scope_key="oso",
            )


def test_non_global_concept_scope_requires_key(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")

        with pytest.raises(sqlite3.IntegrityError):
            _insert_concept(
                conn,
                canonical_name="Sense",
                scope_type="project",
                scope_key="",
            )


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
    scope_type: str | None = None,
    scope_key: str | None = None,
) -> int:
    if scope_type is None and scope_key is None:
        cursor = conn.execute(
            """
            insert into concepts(canonical_name, description, created_at, updated_at)
            values (?, ?, ?, ?)
            """,
            (canonical_name, description, _NOW, _NOW),
        )
        return int(cursor.lastrowid)

    cursor = conn.execute(
        """
        insert into concepts(
          canonical_name,
          description,
          scope_type,
          scope_key,
          created_at,
          updated_at
        )
        values (?, ?, ?, ?, ?, ?)
        """,
        (canonical_name, description, scope_type or "global", scope_key or "", _NOW, _NOW),
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

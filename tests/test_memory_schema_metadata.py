from __future__ import annotations

import sqlite3

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

EXPECTED_METADATA_TABLES = {
    "series_language_tags",
    "task_session_language_tags",
    "task_session_story_scopes",
    "task_session_semantic_tags",
    "short_term_memory_language_tags",
    "short_term_memory_story_scopes",
    "short_term_memory_semantic_tags",
    "crystal_language_tags",
    "concept_facet_language_tags",
    "concept_facet_story_scopes",
    "concept_facet_semantic_tags",
}


EXPECTED_COMPATIBILITY_COLUMNS = {
    "concept_facets": {
        "is_canonical",
        "superseded_at",
    },
    "concepts": {
        "merged_into_concept_id",
    },
    "short_term_memories": {
        "source_credibility",
        "rule_intent",
        "soft_origin",
    },
    "crystals": {
        "soft_origin",
        "is_inferred",
    },
}


def test_global_schema_creates_metadata_tables(tmp_path):
    db_path = tmp_path / "global.db"
    with connect(db_path) as conn:
        apply_migration(conn, "global.sql")

        tables = {
            row["name"]
            for row in conn.execute(
                "select name from sqlite_master where type = 'table'"
            ).fetchall()
        }

    assert EXPECTED_METADATA_TABLES <= tables


def test_global_schema_adds_compatibility_columns_without_dropping_rows(tmp_path):
    db_path = tmp_path / "global.db"
    with connect(db_path) as conn:
        _create_previous_schema(conn)
        _insert_previous_schema_rows(conn)

        apply_migration(conn, "global.sql")

        for table, expected_columns in EXPECTED_COMPATIBILITY_COLUMNS.items():
            columns = {row["name"] for row in conn.execute(f"pragma table_info({table})")}
            assert expected_columns <= columns

        row_counts = {
            table: conn.execute(f"select count(*) from {table}").fetchone()[0]
            for table in (
                "series",
                "task_sessions",
                "short_term_memories",
                "crystals",
                "concepts",
                "concept_facets",
            )
        }

    assert row_counts == {
        "series": 1,
        "task_sessions": 1,
        "short_term_memories": 1,
        "crystals": 1,
        "concepts": 1,
        "concept_facets": 1,
    }


def test_global_schema_removes_old_concept_name_unique_constraint(tmp_path):
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_previous_schema(conn)
        _insert_previous_schema_rows(conn)
        apply_migration(conn, "global.sql")

    store = ConceptStore(config)
    talent = store.create_concept(
        "Sense",
        description="A game-like aptitude category.",
        semantic_tags=("talent",),
    )
    subskill = store.create_concept(
        "Sense",
        description="A subordinate skill concept.",
        semantic_tags=("subskill",),
    )

    with connect(config.database_path) as conn:
        same_name_rows = conn.execute(
            """
            select id
            from concepts
            where canonical_name = 'Sense'
            order by id
            """
        ).fetchall()
        old_facet = conn.execute(
            """
            select value
            from concept_facets
            where id = 1
            """
        ).fetchone()
        fts_rows = conn.execute(
            """
            select rowid
            from concepts_fts
            where concepts_fts match 'aptitude'
            """
        ).fetchall()
        foreign_key_errors = conn.execute("pragma foreign_key_check").fetchall()

    assert [row["id"] for row in same_name_rows] == [talent.id, subskill.id]
    assert store.list_concepts(semantic_tag="talent") == [talent]
    assert store.list_concepts(semantic_tag="subskill") == [subskill]
    assert old_facet["value"] == "Approved Name"
    assert [row["rowid"] for row in fts_rows] == [talent.id]
    assert foreign_key_errors == []


def test_global_schema_rebuilds_facet_fts_for_previous_schema_rows(tmp_path):
    config = HieronymusConfig(data_root=tmp_path / "memory")
    with connect(config.database_path) as conn:
        _create_previous_schema(conn)
        _insert_previous_schema_rows(conn)
        conn.execute(
            """
            update concept_facets
            set value = 'Legacy Alias'
            where id = 1
            """
        )
        conn.commit()

        assert not _sqlite_table_exists(conn, "concept_facet_fts")
        apply_migration(conn, "global.sql")

    assert [concept.canonical_name for concept in ConceptStore(config).search("Legacy")] == [
        "Approved Name"
    ]


def _create_previous_schema(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        create table series (
          id integer primary key,
          slug text not null unique,
          title text not null,
          default_source_language text not null,
          default_target_language text not null,
          created_at text not null,
          updated_at text not null
        );

        create table task_sessions (
          id integer primary key,
          series_slug text not null references series(slug),
          source_language text not null,
          target_language text not null,
          task_type text not null,
          volume text not null default '',
          chapter text not null default '',
          status text not null,
          cycle_id integer,
          created_at text not null,
          completed_at text
        );

        create table short_term_memories (
          id integer primary key,
          session_id integer not null references task_sessions(id) on delete cascade,
          source_role text not null,
          kind text not null,
          text text not null,
          source_ref text not null default '',
          metadata_json text not null default '{}',
          created_at text not null,
          archived_at text
        );

        create table crystals (
          id integer primary key,
          crystal_type text not null,
          text text not null,
          title text not null default '',
          scope_type text not null,
          scope_key text not null default '',
          series_slug text not null default '',
          source_language text not null default '',
          target_language text not null default '',
          tags_json text not null default '[]',
          strength real not null,
          confidence real not null,
          source_credibility text not null default 'observation',
          rule_intent text not null default '',
          malformed_penalty real not null default 0.0,
          supersedes_crystal_id integer references crystals(id) on delete set null,
          status text not null,
          created_cycle integer not null default 0,
          last_activated_cycle integer,
          last_reinforced_cycle integer,
          created_at text not null,
          updated_at text not null
        );

        create table concepts (
          id integer primary key,
          canonical_name text not null,
          description text not null default '',
          scope_type text not null default 'global',
          scope_key text not null default '',
          status text not null default 'vague',
          confidence real not null default 0.2,
          created_at text not null,
          updated_at text not null,
          check (
            (scope_type = 'global' and scope_key = '')
            or (scope_type != 'global' and scope_key != '')
          ),
          unique(scope_type, scope_key, canonical_name)
        );

        create table concept_facets (
          id integer primary key,
          concept_id integer not null references concepts(id) on delete cascade,
          language text not null default '',
          facet_type text not null,
          value text not null,
          source_crystal_id integer references crystals(id) on delete set null,
          confidence real not null default 0.2,
          created_at text not null,
          updated_at text not null
        );
        """
    )
    conn.commit()


def _sqlite_table_exists(conn: sqlite3.Connection, table: str) -> bool:
    row = conn.execute(
        """
        select 1
        from sqlite_master
        where type = 'table'
          and name = ?
        """,
        (table,),
    ).fetchone()
    return row is not None


def _insert_previous_schema_rows(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        insert into series (
          id, slug, title, default_source_language, default_target_language, created_at, updated_at
        )
        values (1, 'book', 'Book', 'ja', 'en', '2026-06-10T00:00:00', '2026-06-10T00:00:00');

        insert into task_sessions (
          id, series_slug, source_language, target_language, task_type, status, created_at
        )
        values (1, 'book', 'ja', 'en', 'translate', 'active', '2026-06-10T00:00:00');

        insert into short_term_memories (
          id, session_id, source_role, kind, text, created_at
        )
        values (1, 1, 'user', 'correction', 'Use the approved name.', '2026-06-10T00:00:00');

        insert into crystals (
          id, crystal_type, text, scope_type, strength, confidence, status, created_at, updated_at
        )
        values (
          1, 'rule', 'Use the approved name.', 'series', 1.0, 0.9, 'active',
          '2026-06-10T00:00:00', '2026-06-10T00:00:00'
        );

        insert into concepts (
          id, canonical_name, created_at, updated_at
        )
        values (1, 'Approved Name', '2026-06-10T00:00:00', '2026-06-10T00:00:00');

        insert into concept_facets (
          id, concept_id, facet_type, value, created_at, updated_at
        )
        values (1, 1, 'name', 'Approved Name', '2026-06-10T00:00:00', '2026-06-10T00:00:00');
        """
    )
    conn.commit()

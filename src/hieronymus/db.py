from __future__ import annotations

import sqlite3
from importlib.resources import files
from pathlib import Path

GLOBAL_COMPATIBILITY_COLUMNS = {
    "concept_facets": {
        "is_canonical": "integer not null default 0",
        "superseded_at": "text",
    },
    "concepts": {
        "merged_into_concept_id": "integer references concepts(id)",
    },
    "short_term_memories": {
        "source_credibility": "text",
        "rule_intent": "text",
        "soft_origin": "text",
    },
    "crystals": {
        "soft_origin": "text",
        "is_inferred": "integer not null default 0",
    },
}


def connect(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    conn.execute("pragma foreign_keys = on")
    conn.execute("pragma journal_mode = wal")
    return conn


def apply_migration(conn: sqlite3.Connection, name: str) -> None:
    sql = files("hieronymus.migrations").joinpath(name).read_text(encoding="utf-8")
    conn.executescript(sql)
    if name == "global.sql":
        conn.commit()
        ensure_global_compatibility_columns(conn)
    conn.commit()


def ensure_column(conn: sqlite3.Connection, table: str, column: str, definition: str) -> None:
    rows = conn.execute(f"pragma table_info({table})").fetchall()
    names = {row["name"] if isinstance(row, sqlite3.Row) else row[1] for row in rows}
    if column not in names:
        conn.execute(f"alter table {table} add column {column} {definition}")


def ensure_global_compatibility_columns(conn: sqlite3.Connection) -> None:
    for table, columns in GLOBAL_COMPATIBILITY_COLUMNS.items():
        for column, definition in columns.items():
            ensure_column(conn, table, column, definition)
    conn.commit()
    ensure_concepts_allow_duplicate_names(conn)
    ensure_concept_facet_compatibility(conn)


def ensure_concept_facet_compatibility(conn: sqlite3.Connection) -> None:
    if _table_exists(conn, "concept_facets") and _table_exists(
        conn,
        "concept_facet_language_tags",
    ):
        conn.execute(
            """
            insert or ignore into concept_facet_language_tags(facet_id, language_tag)
            select id, language
            from concept_facets
            where language != ''
            """
        )

    if _concept_facet_fts_needs_rebuild(conn):
        conn.execute("insert into concept_facet_fts(concept_facet_fts) values ('rebuild')")
    conn.commit()


def ensure_concepts_allow_duplicate_names(conn: sqlite3.Connection) -> None:
    if not _concepts_has_unique_name_scope_constraint(conn):
        return

    if conn.in_transaction:
        raise RuntimeError("concepts compatibility rebuild requires no active transaction")

    foreign_keys_enabled = bool(conn.execute("pragma foreign_keys").fetchone()[0])
    conn.execute("pragma foreign_keys = off")
    try:
        conn.execute("begin")
        conn.execute("drop trigger if exists concepts_ai")
        conn.execute("drop trigger if exists concepts_ad")
        conn.execute("drop trigger if exists concepts_au")
        conn.execute(
            """
            create table concepts_new (
              id integer primary key,
              canonical_name text not null,
              description text not null default '',
              scope_type text not null default 'global',
              scope_key text not null default '',
              status text not null default 'candidate',
              confidence real not null default 0.2,
              merged_into_concept_id integer references concepts_new(id),
              created_at text not null,
              updated_at text not null,
              check (
                (scope_type = 'global' and scope_key = '')
                or (scope_type != 'global' and scope_key != '')
              )
            )
            """
        )
        conn.execute(
            """
            insert into concepts_new(
              id,
              canonical_name,
              description,
              scope_type,
              scope_key,
              status,
              confidence,
              merged_into_concept_id,
              created_at,
              updated_at
            )
            select
              id,
              canonical_name,
              description,
              scope_type,
              scope_key,
              status,
              confidence,
              merged_into_concept_id,
              created_at,
              updated_at
            from concepts
            """
        )
        conn.execute("drop table concepts")
        conn.execute("alter table concepts_new rename to concepts")
        conn.execute(
            """
            create trigger concepts_ai
            after insert on concepts
            begin
              insert into concepts_fts(rowid, canonical_name, description)
              values (new.id, new.canonical_name, new.description);
            end
            """
        )
        conn.execute(
            """
            create trigger concepts_ad
            after delete on concepts
            begin
              insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
              values ('delete', old.id, old.canonical_name, old.description);
            end
            """
        )
        conn.execute(
            """
            create trigger concepts_au
            after update on concepts
            begin
              insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
              values ('delete', old.id, old.canonical_name, old.description);
              insert into concepts_fts(rowid, canonical_name, description)
              values (new.id, new.canonical_name, new.description);
            end
            """
        )
        if _table_exists(conn, "concepts_fts"):
            conn.execute("insert into concepts_fts(concepts_fts) values ('rebuild')")
        conn.execute("commit")
    except Exception:
        if conn.in_transaction:
            conn.execute("rollback")
        raise
    finally:
        conn.execute(f"pragma foreign_keys = {int(foreign_keys_enabled)}")


def _concepts_has_unique_name_scope_constraint(conn: sqlite3.Connection) -> bool:
    for index_row in conn.execute("pragma index_list(concepts)").fetchall():
        is_unique = bool(
            index_row["unique"] if isinstance(index_row, sqlite3.Row) else index_row[2]
        )
        if not is_unique:
            continue
        index_name = index_row["name"] if isinstance(index_row, sqlite3.Row) else index_row[1]
        columns = tuple(
            info_row["name"] if isinstance(info_row, sqlite3.Row) else info_row[2]
            for info_row in conn.execute(f"pragma index_info({index_name})").fetchall()
        )
        if columns == ("scope_type", "scope_key", "canonical_name"):
            return True
    return False


def _table_exists(conn: sqlite3.Connection, table: str) -> bool:
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


def _table_row_count(conn: sqlite3.Connection, table: str) -> int:
    row = conn.execute(f"select count(*) from {table}").fetchone()
    return int(row[0])


def _concept_facet_fts_needs_rebuild(conn: sqlite3.Connection) -> bool:
    if not (
        _table_exists(conn, "concept_facets")
        and _table_exists(conn, "concept_facet_fts")
        and _table_exists(conn, "concept_facet_fts_idx")
    ):
        return False
    if _table_row_count(conn, "concept_facets") == 0:
        return False
    return _table_row_count(conn, "concept_facet_fts_idx") == 0

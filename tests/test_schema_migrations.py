from __future__ import annotations

import sqlite3
from datetime import datetime
from pathlib import Path

import pytest

import hieronymus.db as db


def migration(version: str, name: str, sql: str) -> db.SchemaMigration:
    return db.SchemaMigration(version=version, name=name, sql=sql)


def ledger_rows(conn: sqlite3.Connection) -> list[tuple[str, str]]:
    return [
        (row["version"], row["name"])
        for row in conn.execute(
            "select version, name from schema_migrations order by version"
        ).fetchall()
    ]


def test_fresh_database_baselines_verified_packaged_migrations_without_replay(
    tmp_path: Path,
) -> None:
    with db.connect(tmp_path / "hieronymus.sqlite") as conn:
        statements: list[str] = []
        conn.set_trace_callback(statements.append)

        db.ensure_schema(conn)

        row = conn.execute(
            "select version, name, checksum, applied_at from schema_migrations"
        ).fetchone()
        triggers = {
            trigger["name"]
            for trigger in conn.execute(
                "select name from sqlite_master where type = 'trigger'"
            ).fetchall()
        }

    assert (row["version"], row["name"]) == ("0001", "memory_fts_triggers")
    assert len(row["checksum"]) == 64
    assert row["applied_at"].endswith("Z")
    assert datetime.fromisoformat(row["applied_at"].replace("Z", "+00:00")).utcoffset() is not None
    assert {
        "short_term_memories_ai",
        "short_term_memories_ad",
        "short_term_memories_au",
        "crystals_ai",
        "crystals_ad",
        "crystals_au",
    } <= triggers
    assert not any(
        statement.lstrip().lower().startswith("drop trigger") for statement in statements
    )
    assert not any("values ('rebuild')" in statement.lower() for statement in statements)


def test_existing_pre_ledger_database_applies_fts_upgrade_once(tmp_path: Path) -> None:
    with db.connect(tmp_path / "hieronymus.sqlite") as conn:
        db.apply_migration(conn, "global.sql")
        conn.execute("drop table schema_migrations")
        conn.execute(
            """
            insert into series(
              slug, title, default_source_language, default_target_language,
              created_at, updated_at
            ) values ('series', 'Series', 'ja', 'ru', 'now', 'now')
            """
        )
        session_id = conn.execute(
            """
            insert into task_sessions(
              series_slug, source_language, target_language, task_type, status,
              created_at, last_activity_at
            ) values ('series', 'ja', 'ru', 'translate', 'active', 'now', 'now')
            """
        ).lastrowid
        conn.execute("drop trigger short_term_memories_ai")
        memory_id = conn.execute(
            """
            insert into short_term_memories(
              session_id, source_role, kind, text, created_at
            ) values (?, 'user', 'note', 'missing legacy token', 'now')
            """,
            (session_id,),
        ).lastrowid
        conn.commit()

        db.ensure_schema(conn)
        first_applied_at = conn.execute(
            "select applied_at from schema_migrations where version = '0001'"
        ).fetchone()[0]
        db.ensure_schema(conn)

        repaired_row = conn.execute(
            """
            select rowid from short_term_memories_fts
            where short_term_memories_fts match 'missing'
            """
        ).fetchone()
        second_applied_at = conn.execute(
            "select applied_at from schema_migrations where version = '0001'"
        ).fetchone()[0]

    assert repaired_row[0] == memory_id
    assert first_applied_at == second_applied_at


def test_existing_partial_pre_ledger_schema_skips_inapplicable_fts_rebuild(
    tmp_path: Path,
) -> None:
    with db.connect(tmp_path / "hieronymus.sqlite") as conn:
        conn.execute("create table crystals (id integer primary key, crystal_type text, text text)")
        conn.commit()

        db.ensure_schema(conn)

        assert ledger_rows(conn) == [("0001", "memory_fts_triggers")]


def test_migrations_apply_in_lexical_version_order() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    migrations = [
        migration("0010", "tenth", "insert into migration_order values (10);"),
        migration(
            "0002",
            "second",
            "create table migration_order(position integer); "
            "insert into migration_order values (2);",
        ),
    ]

    db.apply_schema_migrations(conn, migrations)

    assert [row[0] for row in conn.execute("select position from migration_order")] == [2, 10]
    assert ledger_rows(conn) == [("0002", "second"), ("0010", "tenth")]


def test_applied_migration_is_not_replayed() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    migrations = [
        migration(
            "0001",
            "once",
            "create table effects(value integer); insert into effects values (1);",
        )
    ]

    db.apply_schema_migrations(conn, migrations)
    db.apply_schema_migrations(conn, migrations)

    assert conn.execute("select count(*) from effects").fetchone()[0] == 1
    assert ledger_rows(conn) == [("0001", "once")]


@pytest.mark.parametrize(
    ("changed_name", "changed_sql", "detail"),
    [
        ("renamed", "create table stable(value integer);", "name"),
        ("stable", "create table changed(value integer);", "checksum"),
    ],
)
def test_recorded_migration_metadata_mismatch_hard_fails(
    changed_name: str,
    changed_sql: str,
    detail: str,
) -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    db.apply_schema_migrations(
        conn,
        [migration("0001", "stable", "create table stable(value integer);")],
    )

    with pytest.raises(db.SchemaMigrationError, match=detail):
        db.apply_schema_migrations(
            conn,
            [migration("0001", changed_name, changed_sql)],
        )


def test_failed_migration_rolls_back_changes_and_ledger_record() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    broken = migration(
        "0001",
        "broken",
        "create table partial(value integer); insert into missing_table values (1);",
    )

    with pytest.raises(sqlite3.OperationalError, match="missing_table"):
        db.apply_schema_migrations(conn, [broken])

    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'partial'"
        ).fetchone()
        is None
    )
    assert ledger_rows(conn) == []


@pytest.mark.parametrize("statement", ["vacuum", "commit"])
def test_non_transactional_statement_fails_without_partial_commit(statement: str) -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    non_transactional = migration(
        "0001",
        "non_transactional",
        f"create table partial(value integer); {statement};",
    )

    with pytest.raises(db.SchemaMigrationError, match="cannot run transactionally"):
        db.apply_schema_migrations(conn, [non_transactional])

    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'partial'"
        ).fetchone()
        is None
    )
    assert ledger_rows(conn) == []


def test_runner_rejects_an_existing_transaction_before_any_schema_change() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("create table existing(value integer)")
    conn.commit()
    conn.execute("insert into existing values (1)")

    with pytest.raises(db.SchemaMigrationError, match="active transaction"):
        db.apply_schema_migrations(
            conn,
            [migration("0001", "new", "create table new_table(value integer);")],
        )

    assert conn.in_transaction
    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'schema_migrations'"
        ).fetchone()
        is None
    )
    conn.rollback()

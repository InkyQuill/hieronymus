from __future__ import annotations

import sqlite3
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path
from threading import Barrier

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


def test_two_concurrent_fresh_initializers_share_one_valid_ledger(tmp_path: Path) -> None:
    database_path = tmp_path / "hieronymus.sqlite"
    ready = Barrier(2)
    connections = [
        sqlite3.connect(database_path, check_same_thread=False),
        sqlite3.connect(database_path, check_same_thread=False),
    ]
    for conn in connections:
        conn.row_factory = sqlite3.Row
        conn.execute("pragma foreign_keys = on")
        conn.execute("pragma journal_mode = wal")

    def initialize(conn: sqlite3.Connection) -> None:
        try:
            ready.wait(timeout=5)
            db.ensure_schema(conn)
        finally:
            conn.close()

    with ThreadPoolExecutor(max_workers=2) as executor:
        futures = [executor.submit(initialize, conn) for conn in connections]
        for future in futures:
            future.result(timeout=10)

    with db.connect(database_path) as conn:
        assert ledger_rows(conn) == [("0001", "memory_fts_triggers")]
        assert conn.execute("pragma integrity_check").fetchone()[0] == "ok"


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


def test_existing_partial_pre_ledger_schema_normalizes_usable_fts_shape(
    tmp_path: Path,
) -> None:
    with db.connect(tmp_path / "hieronymus.sqlite") as conn:
        conn.execute("create table crystals (id integer primary key, crystal_type text, text text)")
        conn.commit()

        db.ensure_schema(conn)

        assert ledger_rows(conn) == [("0001", "memory_fts_triggers")]
        crystal_id = conn.execute(
            "insert into crystals(crystal_type, text) values ('lesson', 'first token')"
        ).lastrowid
        assert (
            conn.execute(
                "select rowid from crystals_fts where crystals_fts match 'first'"
            ).fetchone()[0]
            == crystal_id
        )

        conn.execute("update crystals set text = 'second token' where id = ?", (crystal_id,))
        assert (
            conn.execute(
                "select rowid from crystals_fts where crystals_fts match 'second'"
            ).fetchone()[0]
            == crystal_id
        )

        conn.execute("delete from crystals where id = ?", (crystal_id,))
        assert (
            conn.execute(
                "select rowid from crystals_fts where crystals_fts match 'second'"
            ).fetchone()
            is None
        )


def test_later_migration_failure_rolls_back_global_repairs_and_ledger(
    tmp_path: Path,
    monkeypatch,
) -> None:
    database_path = tmp_path / "hieronymus.sqlite"
    with db.connect(database_path) as conn:
        conn.execute("create table crystals (id integer primary key, text text)")
        conn.execute("insert into crystals(id, text) values (1, 'preserved')")
        conn.commit()

        monkeypatch.setattr(
            db,
            "discover_schema_migrations",
            lambda: [
                migration(
                    "0001",
                    "earlier_success",
                    "create table earlier_effect(value integer);",
                ),
                migration(
                    "0002",
                    "broken_later",
                    "create table transient(value integer); insert into missing_table values (1);",
                ),
            ],
        )

        with pytest.raises(sqlite3.OperationalError, match="missing_table"):
            db.ensure_schema(conn)

        assert {row["name"] for row in conn.execute("pragma table_info(crystals)")} == {
            "id",
            "text",
        }
        assert conn.execute("select text from crystals where id = 1").fetchone()[0] == "preserved"
        assert (
            conn.execute(
                "select 1 from sqlite_master where type = 'table' and name = 'series'"
            ).fetchone()
            is None
        )
        assert (
            conn.execute(
                "select 1 from sqlite_master where type = 'table' and name = 'earlier_effect'"
            ).fetchone()
            is None
        )
        assert (
            conn.execute(
                "select 1 from sqlite_master where type = 'table' and name = 'schema_migrations'"
            ).fetchone()
            is None
        )


def test_migrations_apply_in_numeric_version_order() -> None:
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


@pytest.mark.parametrize(
    ("statement", "keyword"),
    [
        ("-- comment\n  COMMIT", "COMMIT"),
        ("  /* first */ \n -- second\n BeGiN", "BEGIN"),
        ("\t-- first\n /* second */  ROLLBACK", "ROLLBACK"),
        ("/* first */\n\t-- second\n VaCuUm", "VACUUM"),
        ("-- comment\n ATTACH DATABASE ':memory:' AS aux", "ATTACH"),
        ("/* comment */ \n DeTaCh DATABASE aux", "DETACH"),
    ],
)
def test_non_transactional_statement_fails_without_partial_commit(
    statement: str,
    keyword: str,
) -> None:
    assert db._leading_sql_keyword(statement) == keyword.lower()
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("attach database ':memory:' as aux")
    non_transactional = migration(
        "0001",
        "non_transactional",
        f"create table partial(value integer); {statement};",
    )

    with pytest.raises(db.SchemaMigrationError, match=keyword):
        db.apply_schema_migrations(conn, [non_transactional])

    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'partial'"
        ).fetchone()
        is None
    )
    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'schema_migrations'"
        ).fetchone()
        is None
    )


@pytest.mark.parametrize(
    "sql",
    [
        "pragma user_version = 7; create table effect(value integer);",
        "savepoint nested; create table effect(value integer); release nested;",
    ],
)
def test_transactional_sql_statements_are_not_overbroadly_rejected(sql: str) -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row

    db.apply_schema_migrations(conn, [migration("0001", "allowed", sql)])

    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'effect'"
        ).fetchone()
        is not None
    )


def test_detach_cannot_run_under_sqlites_required_write_transaction() -> None:
    conn = sqlite3.connect(":memory:")
    conn.execute("attach database ':memory:' as aux")
    conn.execute("begin immediate")

    with pytest.raises(sqlite3.OperationalError, match="locked"):
        conn.execute("detach database aux")

    conn.rollback()


def test_attach_survives_rollback_and_creates_its_database_file(tmp_path: Path) -> None:
    attached_path = tmp_path / "attached.sqlite"
    conn = sqlite3.connect(":memory:")
    conn.execute("begin immediate")
    conn.execute(f"attach database '{attached_path}' as leaked")
    conn.rollback()

    assert attached_path.exists()
    assert "leaked" in {row[1] for row in conn.execute("pragma database_list")}


def test_runner_rejects_comment_prefixed_attach_without_connection_or_file_leak(
    tmp_path: Path,
) -> None:
    attached_path = tmp_path / "rejected.sqlite"
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    sql = (
        "create table partial(value integer); "
        "-- connection state must not escape rollback\n  "
        f"ATTACH DATABASE '{attached_path}' AS aux; "
        "insert into missing_table values (1);"
    )

    with pytest.raises(db.SchemaMigrationError, match="ATTACH"):
        db.apply_schema_migrations(conn, [migration("0001", "attach", sql)])

    assert not attached_path.exists()
    assert {row[1] for row in conn.execute("pragma database_list")} == {"main"}
    assert (
        conn.execute(
            "select 1 from sqlite_master where type = 'table' and name = 'partial'"
        ).fetchone()
        is None
    )


def test_mixed_comments_before_trigger_body_remain_valid_sql() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    sql = """
    create table source(value integer);
    create table audit(value integer);
    -- trigger comment
      /* another comment */
    create trigger source_ai after insert on source begin
      insert into audit(value) values (new.value);
    end;
    """

    db.apply_schema_migrations(conn, [migration("0001", "commented_trigger", sql)])
    conn.execute("insert into source(value) values (7)")

    assert conn.execute("select value from audit").fetchone()[0] == 7


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


def write_migration_resource(directory: Path, name: str, sql: str = "select 1;") -> None:
    directory.joinpath(name).write_text(sql, encoding="utf-8")


@pytest.mark.parametrize(
    "filename",
    [
        "1_unpadded.sql",
        "001_mixed_width.sql",
        "00001_mixed_width.sql",
        "٠٠٠١_arabic_indic.sql",
        "notes.sql",
        "0002_BadName.sql",
    ],
)
def test_discovery_rejects_malformed_sql_resource_names(
    tmp_path: Path,
    monkeypatch,
    filename: str,
) -> None:
    write_migration_resource(tmp_path, filename)
    monkeypatch.setattr(db, "files", lambda package: tmp_path)

    with pytest.raises(db.SchemaMigrationError, match="four zero-padded digits"):
        db.discover_schema_migrations()


def test_discovery_rejects_duplicate_numeric_versions(tmp_path: Path, monkeypatch) -> None:
    write_migration_resource(tmp_path, "0001_first.sql")
    write_migration_resource(tmp_path, "0001_second.sql")
    monkeypatch.setattr(db, "files", lambda package: tmp_path)

    with pytest.raises(db.SchemaMigrationError, match="duplicate migration version 0001"):
        db.discover_schema_migrations()


def test_runner_rejects_non_ascii_version_digits() -> None:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row

    with pytest.raises(db.SchemaMigrationError, match="four zero-padded digits"):
        db.apply_schema_migrations(
            conn,
            [migration("٠٠٠١", "arabic_indic", "select 1;")],
        )


def test_discovery_sorts_valid_resources_by_numeric_version(tmp_path: Path, monkeypatch) -> None:
    write_migration_resource(tmp_path, "0010_tenth.sql")
    write_migration_resource(tmp_path, "0002_second.sql")
    monkeypatch.setattr(db, "files", lambda package: tmp_path)

    discovered = db.discover_schema_migrations()

    assert [(item.version, item.name) for item in discovered] == [
        ("0002", "second"),
        ("0010", "tenth"),
    ]

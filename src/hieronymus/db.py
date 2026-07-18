from __future__ import annotations

import re
import sqlite3
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime
from hashlib import sha256
from importlib.resources import files
from pathlib import Path

MIGRATION_FILENAME = re.compile(r"^(?P<version>\d+)_(?P<name>[a-z0-9_]+)\.sql$")
NON_TRANSACTIONAL_SQL = {
    "attach",
    "begin",
    "commit",
    "detach",
    "end",
    "pragma",
    "release",
    "rollback",
    "savepoint",
    "vacuum",
}
MIGRATION_LEDGER_SQL = """
create table if not exists schema_migrations (
  version text primary key,
  name text not null,
  checksum text not null,
  applied_at text not null
)
"""

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
    "task_sessions": {
        "last_activity_at": "text not null default ''",
    },
    "crystals": {
        "source_credibility": "text not null default 'observation'",
        "rule_intent": "text not null default ''",
        "malformed_penalty": "real not null default 0",
        "supersedes_crystal_id": "integer references crystals(id)",
        "soft_origin": "text",
        "is_inferred": "integer not null default 0",
    },
}


class SchemaMigrationError(RuntimeError):
    """Raised when ordered schema migration invariants are violated."""


@dataclass(frozen=True)
class SchemaMigration:
    version: str
    name: str
    sql: str

    @property
    def checksum(self) -> str:
        return sha256(self.sql.encode()).hexdigest()


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


def ensure_schema(conn: sqlite3.Connection) -> None:
    """Create or upgrade the global schema through the ordered migration ledger."""
    _require_no_active_transaction(conn)
    migrations = discover_schema_migrations()
    if _table_exists(conn, "schema_migrations"):
        apply_schema_migrations(conn, migrations)
        return

    is_fresh = not _has_application_schema(conn)
    apply_migration(conn, "global.sql")
    if is_fresh:
        _baseline_schema_migrations(conn, migrations)
    else:
        apply_schema_migrations(conn, migrations)


def discover_schema_migrations() -> list[SchemaMigration]:
    """Read numbered SQL migrations from package resources in lexical order."""
    migrations: list[SchemaMigration] = []
    versions = files("hieronymus.migrations.versions")
    for resource in sorted(versions.iterdir(), key=lambda item: item.name):
        match = MIGRATION_FILENAME.fullmatch(resource.name)
        if match is None:
            continue
        migrations.append(
            SchemaMigration(
                version=match.group("version"),
                name=match.group("name"),
                sql=resource.read_text(encoding="utf-8"),
            )
        )
    _validate_migration_order(migrations)
    return migrations


def apply_schema_migrations(
    conn: sqlite3.Connection,
    migrations: Iterable[SchemaMigration],
) -> None:
    """Apply each pending migration atomically and record it after success."""
    _require_no_active_transaction(conn)
    ordered = sorted(migrations, key=lambda migration: migration.version)
    _validate_migration_order(ordered)
    _create_migration_ledger(conn)
    applied = {
        row["version"] if isinstance(row, sqlite3.Row) else row[0]: row
        for row in conn.execute(
            "select version, name, checksum, applied_at from schema_migrations"
        ).fetchall()
    }
    for migration in ordered:
        row = applied.get(migration.version)
        if row is not None:
            _validate_applied_migration(row, migration)

    for migration in ordered:
        if migration.version in applied:
            continue
        statements = _sql_statements(migration.sql)
        _reject_non_transactional_statements(migration, statements)
        try:
            conn.execute("begin immediate")
            for statement in statements:
                conn.execute(statement)
            conn.execute(
                """
                insert into schema_migrations(version, name, checksum, applied_at)
                values (?, ?, ?, ?)
                """,
                (
                    migration.version,
                    migration.name,
                    migration.checksum,
                    _utc_timestamp(),
                ),
            )
            conn.commit()
        except Exception:
            if conn.in_transaction:
                conn.rollback()
            raise


def _require_no_active_transaction(conn: sqlite3.Connection) -> None:
    if conn.in_transaction:
        raise SchemaMigrationError("schema migrations cannot start inside an active transaction")


def _validate_migration_order(migrations: list[SchemaMigration]) -> None:
    versions = [migration.version for migration in migrations]
    if len(versions) != len(set(versions)):
        raise SchemaMigrationError("schema migration versions must be unique")
    if versions != sorted(versions):
        raise SchemaMigrationError("schema migrations must use lexical numeric order")


def _create_migration_ledger(conn: sqlite3.Connection) -> None:
    conn.execute(MIGRATION_LEDGER_SQL)
    conn.commit()


def _validate_applied_migration(row: sqlite3.Row | tuple, migration: SchemaMigration) -> None:
    recorded_name = row["name"] if isinstance(row, sqlite3.Row) else row[1]
    recorded_checksum = row["checksum"] if isinstance(row, sqlite3.Row) else row[2]
    if recorded_name != migration.name:
        raise SchemaMigrationError(
            f"migration {migration.version} name mismatch: "
            f"recorded {recorded_name!r}, packaged {migration.name!r}"
        )
    if recorded_checksum != migration.checksum:
        raise SchemaMigrationError(f"migration {migration.version} checksum mismatch")


def _sql_statements(sql: str) -> list[str]:
    statements: list[str] = []
    statement = ""
    for character in sql:
        statement += character
        if character == ";" and sqlite3.complete_statement(statement):
            statements.append(statement)
            statement = ""
    if statement.strip():
        statements.append(statement)
    return statements


def _reject_non_transactional_statements(
    migration: SchemaMigration,
    statements: list[str],
) -> None:
    for statement in statements:
        keyword = _leading_sql_keyword(statement)
        if keyword in NON_TRANSACTIONAL_SQL:
            raise SchemaMigrationError(
                f"migration {migration.version} contains {keyword.upper()}, "
                "which cannot run transactionally"
            )


def _leading_sql_keyword(statement: str) -> str:
    remainder = statement.lstrip()
    while remainder.startswith(("--", "/*")):
        if remainder.startswith("--"):
            _, separator, remainder = remainder.partition("\n")
            if not separator:
                return ""
        else:
            end = remainder.find("*/", 2)
            if end == -1:
                return ""
            remainder = remainder[end + 2 :].lstrip()
    match = re.match(r"[a-z]+", remainder, re.IGNORECASE)
    return match.group(0).lower() if match is not None else ""


def _baseline_schema_migrations(
    conn: sqlite3.Connection,
    migrations: list[SchemaMigration],
) -> None:
    validators = {"0001": _verify_memory_fts_trigger_state}
    try:
        conn.execute("begin immediate")
        conn.execute(MIGRATION_LEDGER_SQL)
        for migration in migrations:
            validator = validators.get(migration.version)
            if validator is None:
                raise SchemaMigrationError(
                    f"fresh schema has no baseline verifier for migration {migration.version}"
                )
            validator(conn)
            conn.execute(
                """
                insert into schema_migrations(version, name, checksum, applied_at)
                values (?, ?, ?, ?)
                """,
                (migration.version, migration.name, migration.checksum, _utc_timestamp()),
            )
        conn.commit()
    except Exception:
        if conn.in_transaction:
            conn.rollback()
        raise


def _verify_memory_fts_trigger_state(conn: sqlite3.Connection) -> None:
    expected = {
        "short_term_memories_ai",
        "short_term_memories_ad",
        "short_term_memories_au",
        "crystals_ai",
        "crystals_ad",
        "crystals_au",
    }
    present = {
        row["name"] if isinstance(row, sqlite3.Row) else row[0]
        for row in conn.execute("select name from sqlite_master where type = 'trigger'").fetchall()
    }
    missing = sorted(expected - present)
    if missing:
        raise SchemaMigrationError(
            f"fresh schema does not represent migration 0001; missing triggers: {missing}"
        )


def _has_application_schema(conn: sqlite3.Connection) -> bool:
    return (
        conn.execute(
            """
            select 1 from sqlite_master
            where type = 'table'
              and name not like 'sqlite_%'
              and name != 'schema_migrations'
            limit 1
            """
        ).fetchone()
        is not None
    )


def _utc_timestamp() -> str:
    return datetime.now(UTC).isoformat(timespec="seconds").replace("+00:00", "Z")


def ensure_column(conn: sqlite3.Connection, table: str, column: str, definition: str) -> None:
    if column not in _column_names(conn, table):
        conn.execute(f"alter table {table} add column {column} {definition}")


def _column_names(conn: sqlite3.Connection, table: str) -> set[str]:
    rows = conn.execute(f"pragma table_info({table})").fetchall()
    return {row["name"] if isinstance(row, sqlite3.Row) else row[1] for row in rows}


def ensure_global_compatibility_columns(conn: sqlite3.Connection) -> None:
    for table, columns in GLOBAL_COMPATIBILITY_COLUMNS.items():
        for column, definition in columns.items():
            ensure_column(conn, table, column, definition)
    task_session_columns = _column_names(conn, "task_sessions")
    if {"created_at", "last_activity_at"} <= task_session_columns:
        conn.execute(
            """
            update task_sessions
            set last_activity_at = created_at
            where last_activity_at = ''
            """
        )
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
            select id, lower(trim(language))
            from concept_facets
            where trim(language) != ''
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

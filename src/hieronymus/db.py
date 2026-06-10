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

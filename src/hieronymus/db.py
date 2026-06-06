from __future__ import annotations

import sqlite3
from importlib.resources import files
from pathlib import Path


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
    conn.commit()

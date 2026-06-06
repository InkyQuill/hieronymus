from __future__ import annotations

import re
from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import MemoryEntry

_MAX_SEARCH_LIMIT = 50
_FTS_OPERATORS = frozenset({"and", "or", "not", "near"})
_TOKEN_RE = re.compile(r"\w+")


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _search_expression(query: str) -> str:
    tokens = [token for token in _TOKEN_RE.findall(query) if token.casefold() not in _FTS_OPERATORS]
    return " ".join(f'"{token}"' for token in tokens)


class MemoryStore:
    def __init__(self, database_path: Path) -> None:
        self.database_path = database_path

    def add(self, *, kind: str, text: str, source_ref: str = "", importance: int = 3) -> int:
        if not kind.strip():
            raise ValueError("kind must not be empty")
        if not text.strip():
            raise ValueError("text must not be empty")

        now = _now()
        with connect(self.database_path) as conn:
            cursor = conn.execute(
                """
                insert into memories(kind, text, importance, source_ref, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?)
                """,
                (kind, text, importance, source_ref, now, now),
            )
            memory_id = int(cursor.lastrowid)
            conn.execute(
                "insert into memories_fts(rowid, text) values (?, ?)",
                (memory_id, text),
            )
            conn.commit()
        return memory_id

    def search(self, query: str, *, limit: int = 5) -> list[MemoryEntry]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        expression = _search_expression(query)
        if not expression:
            return []

        bounded_limit = min(limit, _MAX_SEARCH_LIMIT)
        with connect(self.database_path) as conn:
            rows = conn.execute(
                """
                select memories.*
                from memories_fts
                join memories on memories.id = memories_fts.rowid
                where memories_fts match ?
                order by memories.importance desc, bm25(memories_fts)
                limit ?
                """,
                (expression, bounded_limit),
            ).fetchall()
        return [
            MemoryEntry(
                id=row["id"],
                kind=row["kind"],
                text=row["text"],
                importance=row["importance"],
                source_ref=row["source_ref"],
            )
            for row in rows
        ]

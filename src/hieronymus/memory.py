from __future__ import annotations

import json
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
    def __init__(self, database_path: Path, *, series_slug: str) -> None:
        self.database_path = database_path
        self.series_slug = series_slug

    def _session_id(self, conn) -> int:
        row = conn.execute(
            """
            select id
            from task_sessions
            where series_slug = ? and task_type = 'memory'
            order by id
            limit 1
            """,
            (self.series_slug,),
        ).fetchone()
        if row is not None:
            return int(row["id"])

        now = _now()
        series = conn.execute(
            """
            select default_source_language, default_target_language
            from series
            where slug = ?
            """,
            (self.series_slug,),
        ).fetchone()
        if series is None:
            raise KeyError(f"unknown series: {self.series_slug}")
        cursor = conn.execute(
            """
            insert into task_sessions(
              series_slug,
              source_language,
              target_language,
              task_type,
              status,
              created_at
            )
            values (?, ?, ?, 'memory', 'active', ?)
            """,
            (
                self.series_slug,
                series["default_source_language"],
                series["default_target_language"],
                now,
            ),
        )
        return int(cursor.lastrowid)

    def add(self, *, kind: str, text: str, source_ref: str = "", importance: int = 3) -> int:
        if not kind.strip():
            raise ValueError("kind must not be empty")
        if not text.strip():
            raise ValueError("text must not be empty")

        now = _now()
        with connect(self.database_path) as conn:
            session_id = self._session_id(conn)
            metadata_json = json.dumps({"importance": importance}, separators=(",", ":"))
            cursor = conn.execute(
                """
                insert into short_term_memories(
                  session_id,
                  source_role,
                  kind,
                  text,
                  source_ref,
                  metadata_json,
                  created_at
                )
                values (?, 'user', ?, ?, ?, ?, ?)
                """,
                (session_id, kind, text, source_ref, metadata_json, now),
            )
            memory_id = int(cursor.lastrowid)
            conn.execute(
                "insert into short_term_memories_fts(rowid, text) values (?, ?)",
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
                select short_term_memories.*
                from short_term_memories_fts
                join short_term_memories on short_term_memories.id = short_term_memories_fts.rowid
                join task_sessions on task_sessions.id = short_term_memories.session_id
                where short_term_memories_fts match ?
                  and task_sessions.series_slug = ?
                  and short_term_memories.archived_at is null
                order by
                  cast(
                    json_extract(short_term_memories.metadata_json, '$.importance')
                    as integer
                  ) desc,
                  bm25(short_term_memories_fts)
                limit ?
                """,
                (expression, self.series_slug, bounded_limit),
            ).fetchall()
        return [
            MemoryEntry(
                id=row["id"],
                kind=row["kind"],
                text=row["text"],
                importance=int(json.loads(row["metadata_json"]).get("importance", 3)),
                source_ref=row["source_ref"],
            )
            for row in rows
        ]

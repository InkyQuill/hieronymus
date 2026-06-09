from __future__ import annotations

import json
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import (
    ShortTermMemoryRecord,
    TaskSessionRecord,
    TranslationContext,
)
from hieronymus.short_memory import validate_short_memory_text

_SOURCE_ROLES = frozenset({"mundane", "mentor", "user", "system"})


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _require_non_empty(value: str, field_name: str) -> None:
    if not value.strip():
        raise ValueError(f"{field_name} must not be empty")


class WorkspaceStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def start_session(self, context: TranslationContext) -> TaskSessionRecord:
        now = _now()
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                insert into task_sessions(
                  series_slug,
                  source_language,
                  target_language,
                  task_type,
                  volume,
                  chapter,
                  status,
                  created_at
                )
                values (?, ?, ?, ?, ?, ?, 'active', ?)
                """,
                (
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                    now,
                ),
            )
            session_id = int(cursor.lastrowid)
            conn.commit()
        return TaskSessionRecord(
            id=session_id,
            context=context,
            status="active",
            cycle_id=None,
        )

    def get_session(self, session_id: int) -> TaskSessionRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                """
                select *
                from task_sessions
                where id = ?
                """,
                (session_id,),
            ).fetchone()
        if row is None:
            raise KeyError(f"unknown session: {session_id}")
        return TaskSessionRecord(
            id=row["id"],
            context=TranslationContext(
                series_slug=row["series_slug"],
                source_language=row["source_language"],
                target_language=row["target_language"],
                task_type=row["task_type"],
                volume=row["volume"],
                chapter=row["chapter"],
            ),
            status=row["status"],
            cycle_id=row["cycle_id"],
        )

    def complete_session(self, session_id: int) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update task_sessions
                set status = 'completed',
                    completed_at = ?
                where id = ?
                """,
                (_now(), session_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown session: {session_id}")
            conn.commit()

    def add_short_term_memory(
        self,
        session_id: int,
        source_role: str,
        kind: str,
        text: str,
        source_ref: str = "",
        metadata: dict[str, object] | None = None,
    ) -> int:
        self._validate_source_role(source_role)
        _require_non_empty(kind, "kind")
        text = text.strip()
        validation = validate_short_memory_text(text)
        memory_metadata = dict(metadata or {})
        memory_metadata["sentence_count"] = validation.sentence_count
        if validation.warning:
            memory_metadata["validation_warning"] = validation.warning

        metadata_json = json.dumps(
            memory_metadata,
            ensure_ascii=False,
            sort_keys=True,
        )
        now = _now()
        with connect(self.config.database_path) as conn:
            self._require_active_session(conn, session_id)
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
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    session_id,
                    source_role,
                    kind,
                    text,
                    source_ref,
                    metadata_json,
                    now,
                ),
            )
            memory_id = int(cursor.lastrowid)
            conn.execute(
                "insert into short_term_memories_fts(rowid, text) values (?, ?)",
                (memory_id, text),
            )
            conn.commit()
        return memory_id

    def list_short_term_memories(
        self,
        session_id: int,
    ) -> list[ShortTermMemoryRecord]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from short_term_memories
                where session_id = ?
                  and archived_at is null
                order by id
                """,
                (session_id,),
            ).fetchall()
        return [
            ShortTermMemoryRecord(
                id=row["id"],
                session_id=row["session_id"],
                source_role=row["source_role"],
                kind=row["kind"],
                text=row["text"],
                source_ref=row["source_ref"],
                metadata=json.loads(row["metadata_json"]),
            )
            for row in rows
        ]

    def _validate_source_role(self, source_role: str) -> None:
        if source_role not in _SOURCE_ROLES:
            raise ValueError(f"unknown source_role: {source_role}")

    def _require_active_session(self, conn, session_id: int) -> None:
        row = conn.execute(
            "select status from task_sessions where id = ?",
            (session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown session: {session_id}")
        if row["status"] != "active":
            raise ValueError("short-term memories require an active session")

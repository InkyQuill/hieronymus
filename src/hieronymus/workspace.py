from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import search_expression
from hieronymus.db import apply_migration, connect
from hieronymus.ingest_config import load_ingest_config
from hieronymus.memory_models import (
    ShortTermMemoryRecord,
    TaskSessionRecord,
    TranslationContext,
    normalize_string_tuple,
)
from hieronymus.short_memory import validate_short_memory_text

_MAX_SHORT_TERM_MEMORIES_PER_BATCH = 500


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _require_non_empty(value: str, field_name: str) -> None:
    if not value.strip():
        raise ValueError(f"{field_name} must not be empty")


def _json_object(raw: str) -> dict[str, object]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def _metadata_string(value: object, default: str = "") -> str:
    return value if isinstance(value, str) else default


def _metadata_strings(value: object, *, lowercase: bool = False) -> tuple[str, ...]:
    if not isinstance(value, list):
        return ()
    return normalize_string_tuple(
        (item for item in value if isinstance(item, str)),
        lowercase=lowercase,
    )


def _insert_metadata_values(
    conn,
    *,
    table: str,
    owner_column: str,
    owner_id: int,
    value_column: str,
    values: Iterable[str],
) -> None:
    conn.executemany(
        f"insert into {table}({owner_column}, {value_column}) values (?, ?)",
        [(owner_id, value) for value in values],
    )


def _load_metadata_values(
    conn,
    *,
    table: str,
    owner_column: str,
    owner_id: int,
    value_column: str,
) -> tuple[str, ...]:
    rows = conn.execute(
        f"""
        select {value_column}
        from {table}
        where {owner_column} = ?
        order by rowid
        """,
        (owner_id,),
    ).fetchall()
    return tuple(row[value_column] for row in rows)


def _batch_item_string(item: Mapping[str, object], field_name: str, default: str = "") -> str:
    value = item.get(field_name, default)
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be a string")
    return value


def _batch_item_strings(item: Mapping[str, object], field_name: str) -> Iterable[str]:
    value = item.get(field_name, ())
    if not isinstance(value, (list, tuple)) or not all(isinstance(entry, str) for entry in value):
        raise ValueError(f"{field_name} must be a list of strings")
    return value


def _batch_item_metadata(item: Mapping[str, object]) -> dict[str, object] | None:
    value = item.get("metadata")
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("metadata must be an object")
    return value


def short_memory_from_row(conn, row) -> ShortTermMemoryRecord:
    metadata = _json_object(row["metadata_json"])
    language_tags = _load_metadata_values(
        conn,
        table="short_term_memory_language_tags",
        owner_column="memory_id",
        owner_id=row["id"],
        value_column="language_tag",
    ) or _metadata_strings(metadata.get("language_tags"), lowercase=True)
    story_scopes = _load_metadata_values(
        conn,
        table="short_term_memory_story_scopes",
        owner_column="memory_id",
        owner_id=row["id"],
        value_column="story_scope",
    ) or _metadata_strings(metadata.get("story_scopes"))
    semantic_tags = _load_metadata_values(
        conn,
        table="short_term_memory_semantic_tags",
        owner_column="memory_id",
        owner_id=row["id"],
        value_column="semantic_tag",
    ) or _metadata_strings(metadata.get("semantic_tags"))
    return ShortTermMemoryRecord(
        id=row["id"],
        session_id=row["session_id"],
        source_role=row["source_role"],
        kind=row["kind"],
        text=row["text"],
        source_ref=row["source_ref"],
        metadata=metadata,
        language_tags=language_tags,
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        source_credibility=row["source_credibility"]
        or _metadata_string(metadata.get("source_credibility"), "observation"),
        rule_intent=row["rule_intent"] or _metadata_string(metadata.get("rule_intent")),
        soft_origin=row["soft_origin"] or _metadata_string(metadata.get("soft_origin")),
    )


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
                  created_at,
                  last_activity_at
                )
                values (?, ?, ?, ?, ?, ?, 'active', ?, ?)
                """,
                (
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    context.task_type,
                    context.volume,
                    context.chapter,
                    now,
                    now,
                ),
            )
            session_id = int(cursor.lastrowid)
            _insert_metadata_values(
                conn,
                table="task_session_language_tags",
                owner_column="session_id",
                owner_id=session_id,
                value_column="language_tag",
                values=context.language_tags,
            )
            _insert_metadata_values(
                conn,
                table="task_session_story_scopes",
                owner_column="session_id",
                owner_id=session_id,
                value_column="story_scope",
                values=context.story_scopes,
            )
            _insert_metadata_values(
                conn,
                table="task_session_semantic_tags",
                owner_column="session_id",
                owner_id=session_id,
                value_column="semantic_tag",
                values=context.semantic_tags,
            )
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
                where id = ? and status = 'active'
                """,
                (session_id,),
            ).fetchone()
            if row is not None:
                language_tags = _load_metadata_values(
                    conn,
                    table="task_session_language_tags",
                    owner_column="session_id",
                    owner_id=session_id,
                    value_column="language_tag",
                )
                story_scopes = _load_metadata_values(
                    conn,
                    table="task_session_story_scopes",
                    owner_column="session_id",
                    owner_id=session_id,
                    value_column="story_scope",
                )
                semantic_tags = _load_metadata_values(
                    conn,
                    table="task_session_semantic_tags",
                    owner_column="session_id",
                    owner_id=session_id,
                    value_column="semantic_tag",
                )
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
                language_tags=language_tags,
                story_scopes=story_scopes,
                semantic_tags=semantic_tags,
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

    def complete_inactive_sessions(self, cutoff) -> tuple[int, ...]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                "select id from task_sessions where status = 'active' and last_activity_at < ?",
                (cutoff.isoformat(),),
            ).fetchall()
            session_ids = tuple(int(row["id"]) for row in rows)
            if session_ids:
                conn.execute(
                    """
                    update task_sessions
                    set status = 'completed', completed_at = ?
                    where status = 'active' and last_activity_at < ?
                    """,
                    (_now(), cutoff.isoformat()),
                )
            conn.commit()
        return session_ids

    def add_short_term_memory(
        self,
        session_id: int,
        source_role: str,
        kind: str,
        text: str,
        source_ref: str = "",
        metadata: dict[str, object] | None = None,
        *,
        language_tags: Iterable[str] = (),
        story_scopes: Iterable[str] = (),
        semantic_tags: Iterable[str] = (),
        source_credibility: str = "observation",
        rule_intent: str = "",
        soft_origin: str = "",
    ) -> int:
        return self.add_short_term_memories_batch(
            session_id,
            [
                {
                    "source_role": source_role,
                    "kind": kind,
                    "text": text,
                    "source_ref": source_ref,
                    "metadata": metadata,
                    "language_tags": list(language_tags),
                    "story_scopes": list(story_scopes),
                    "semantic_tags": list(semantic_tags),
                    "source_credibility": source_credibility,
                    "rule_intent": rule_intent,
                    "soft_origin": soft_origin,
                }
            ],
        )[0]

    def add_short_term_memories_batch(
        self,
        session_id: int,
        items: list[Mapping[str, object]],
    ) -> list[int]:
        if not items:
            raise ValueError("items must not be empty")
        if len(items) > _MAX_SHORT_TERM_MEMORIES_PER_BATCH:
            raise ValueError(
                "a batch may contain at most "
                f"{_MAX_SHORT_TERM_MEMORIES_PER_BATCH} short-term memories"
            )

        short_memory_limits = load_ingest_config(self.config).short_memory
        prepared = [
            self._prepare_short_term_memory(item, short_memory_limits=short_memory_limits)
            for item in items
        ]
        memory_ids: list[int] = []
        with connect(self.config.database_path) as conn:
            self._require_active_session(conn, session_id)
            for memory in prepared:
                cursor = conn.execute(
                    """
                insert into short_term_memories(
                  session_id,
                  source_role,
                  kind,
                  text,
                  source_ref,
                  metadata_json,
                  source_credibility,
                  rule_intent,
                  soft_origin,
                  created_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                    (
                        session_id,
                        memory["source_role"],
                        memory["kind"],
                        memory["text"],
                        memory["source_ref"],
                        memory["metadata_json"],
                        memory["source_credibility"],
                        memory["rule_intent"],
                        memory["soft_origin"],
                        _now(),
                    ),
                )
                memory_id = int(cursor.lastrowid)
                memory_ids.append(memory_id)
                _insert_metadata_values(
                    conn,
                    table="short_term_memory_language_tags",
                    owner_column="memory_id",
                    owner_id=memory_id,
                    value_column="language_tag",
                    values=memory["language_tags"],
                )
                _insert_metadata_values(
                    conn,
                    table="short_term_memory_story_scopes",
                    owner_column="memory_id",
                    owner_id=memory_id,
                    value_column="story_scope",
                    values=memory["story_scopes"],
                )
                _insert_metadata_values(
                    conn,
                    table="short_term_memory_semantic_tags",
                    owner_column="memory_id",
                    owner_id=memory_id,
                    value_column="semantic_tag",
                    values=memory["semantic_tags"],
                )
                conn.execute(
                    "insert into short_term_memories_fts(rowid, text) values (?, ?)",
                    (memory_id, memory["text"]),
                )
            conn.execute(
                "update task_sessions set last_activity_at = ? where id = ?",
                (_now(), session_id),
            )
            conn.commit()
        return memory_ids

    def _prepare_short_term_memory(
        self,
        item: Mapping[str, object],
        *,
        short_memory_limits,
    ) -> dict[str, object]:
        source_role = _batch_item_string(item, "source_role", "agent").strip()
        _require_non_empty(source_role, "source_role")
        kind = _batch_item_string(item, "kind")
        _require_non_empty(kind, "kind")
        text = _batch_item_string(item, "text").strip()
        validation = validate_short_memory_text(text, limits=short_memory_limits)
        normalized_language_tags = normalize_string_tuple(
            _batch_item_strings(item, "language_tags"), lowercase=True
        )
        normalized_story_scopes = normalize_string_tuple(_batch_item_strings(item, "story_scopes"))
        normalized_semantic_tags = normalize_string_tuple(
            _batch_item_strings(item, "semantic_tags")
        )
        source_credibility = _batch_item_string(item, "source_credibility", "observation").strip()
        source_credibility = source_credibility or "observation"
        rule_intent = _batch_item_string(item, "rule_intent").strip()
        soft_origin = _batch_item_string(item, "soft_origin").strip()
        memory_metadata = {
            key: value
            for key, value in (_batch_item_metadata(item) or {}).items()
            if key not in {"sentence_count", "symbol_count", "validation_warning"}
        }
        if normalized_language_tags:
            memory_metadata["language_tags"] = list(normalized_language_tags)
        if normalized_story_scopes:
            memory_metadata["story_scopes"] = list(normalized_story_scopes)
        if normalized_semantic_tags:
            memory_metadata["semantic_tags"] = list(normalized_semantic_tags)
        if source_credibility != "observation":
            memory_metadata["source_credibility"] = source_credibility
        if rule_intent:
            memory_metadata["rule_intent"] = rule_intent
        if soft_origin:
            memory_metadata["soft_origin"] = soft_origin
        memory_metadata["sentence_count"] = validation.sentence_count
        memory_metadata["symbol_count"] = validation.symbol_count
        if validation.warning:
            memory_metadata["validation_warning"] = validation.warning
        return {
            "source_role": source_role,
            "kind": kind,
            "text": text,
            "source_ref": _batch_item_string(item, "source_ref"),
            "metadata_json": json.dumps(memory_metadata, ensure_ascii=False, sort_keys=True),
            "language_tags": normalized_language_tags,
            "story_scopes": normalized_story_scopes,
            "semantic_tags": normalized_semantic_tags,
            "source_credibility": source_credibility,
            "rule_intent": rule_intent,
            "soft_origin": soft_origin,
        }

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
            return [short_memory_from_row(conn, row) for row in rows]

    def search_short_term_memories(
        self,
        session_id: int,
        query: str,
        *,
        limit: int = 50,
    ) -> list[ShortTermMemoryRecord]:
        if limit < 1:
            raise ValueError("limit must be at least 1")
        expression = search_expression(query)
        if not expression:
            return []
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select short_term_memories.*
                from short_term_memories_fts
                join short_term_memories
                  on short_term_memories.id = short_term_memories_fts.rowid
                where short_term_memories_fts match ?
                  and short_term_memories.session_id = ?
                  and short_term_memories.archived_at is null
                order by bm25(short_term_memories_fts), short_term_memories.id
                limit ?
                """,
                (expression, session_id, limit),
            ).fetchall()
            return [short_memory_from_row(conn, row) for row in rows]

    def _require_active_session(self, conn, session_id: int) -> None:
        row = conn.execute(
            "select status from task_sessions where id = ?",
            (session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown session: {session_id}")
        if row["status"] != "active":
            raise ValueError("short-term memories require an active session")

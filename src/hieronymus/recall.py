from __future__ import annotations

import json
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore, search_expression
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import RecallResult, ShortTermMemoryRecord, TranslationContext

_RECALL_REASON = "weighted search match"
_SHORT_TERM_REASON = "active session short-term memory match"
_SHORT_TERM_BASE_SCORE = 0.30
_SHORT_TERM_RANK_STEP = 0.01
_STORY_SCOPE_BOOST = 0.25
_MAX_LONG_TERM_CANDIDATE_LIMIT = 50
_MAX_SHORT_TERM_LIMIT = 50


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _short_memory_from_row(row) -> ShortTermMemoryRecord:
    return ShortTermMemoryRecord(
        id=row["id"],
        session_id=row["session_id"],
        source_role=row["source_role"],
        kind=row["kind"],
        text=row["text"],
        source_ref=row["source_ref"],
        metadata=json.loads(row["metadata_json"]),
    )


def _long_term_candidate_limit(limit: int) -> int:
    return min(max(limit * 4, limit + 10), _MAX_LONG_TERM_CANDIDATE_LIMIT)


class RecallService:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.crystals = CrystalStore(config)
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def recall(
        self,
        session_id: int,
        context: TranslationContext,
        query: str,
        *,
        limit: int = 10,
    ) -> list[RecallResult]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        now = _now()
        with connect(self.config.database_path) as conn:
            self._require_active_session(conn, session_id)
            short_term_memories = self._search_short_term_memories(
                conn,
                session_id=session_id,
                query=query,
                limit=limit,
            )

        scored_crystals = self.crystals.search_scored(
            context,
            query,
            limit=_long_term_candidate_limit(limit),
        )
        ranked_items: list[tuple[str, float, int, object]] = []
        context_tags = set(context.tags)
        for crystal, score in scored_crystals:
            ranked_score = score
            if context_tags.intersection(crystal.story_scopes):
                ranked_score += _STORY_SCOPE_BOOST
            ranked_items.append(("long_term", ranked_score, crystal.id, crystal))
        for memory, score in short_term_memories:
            ranked_items.append(("short_term", score, memory.id, memory))

        source_preference = {"long_term": 0, "short_term": 1}
        ranked_items.sort(
            key=lambda item: (
                -item[1],
                source_preference[item[0]],
                item[2],
            )
        )

        results: list[RecallResult] = []
        for rank, (source, score, _item_id, payload) in enumerate(ranked_items[:limit], start=1):
            if source == "long_term":
                results.append(
                    RecallResult.long_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=_RECALL_REASON,
                    )
                )
            else:
                results.append(
                    RecallResult.short_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=_SHORT_TERM_REASON,
                    )
                )

        with connect(self.config.database_path) as conn:
            session = self._require_active_session(conn, session_id)
            for result in results:
                if result.source != "long_term" or result.crystal is None:
                    continue
                conn.execute(
                    """
                    insert into crystal_activations(
                      crystal_id,
                      session_id,
                      recall_query,
                      rank,
                      score,
                      reason,
                      cycle_id,
                      created_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        result.crystal.id,
                        session_id,
                        query,
                        result.rank,
                        result.score,
                        result.reason,
                        session["cycle_id"],
                        now,
                    ),
                )
            conn.commit()

        return results

    def _search_short_term_memories(
        self,
        conn,
        *,
        session_id: int,
        query: str,
        limit: int,
    ) -> list[tuple[ShortTermMemoryRecord, float]]:
        expression = search_expression(query)
        if not expression:
            return []

        bounded_limit = min(limit, _MAX_SHORT_TERM_LIMIT)
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
            (expression, session_id, bounded_limit),
        ).fetchall()
        return [
            (_short_memory_from_row(row), _SHORT_TERM_BASE_SCORE - (index * _SHORT_TERM_RANK_STEP))
            for index, row in enumerate(rows)
        ]

    def _require_active_session(self, conn, session_id: int):
        row = conn.execute(
            "select status, cycle_id from task_sessions where id = ?",
            (session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown session: {session_id}")
        if row["status"] != "active":
            raise ValueError("recall requires an active session")
        return row

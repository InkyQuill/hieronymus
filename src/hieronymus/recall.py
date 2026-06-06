from __future__ import annotations

import json
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import RecallResult, TranslationContext

_RECALL_REASON = "weighted search match"


def _now() -> str:
    return datetime.now(UTC).isoformat()


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

        scored_crystals = self.crystals.search_scored(context, query, limit=limit)
        results = [
            RecallResult(
                crystal=crystal,
                rank=rank,
                score=score,
                reason=_RECALL_REASON,
            )
            for rank, (crystal, score) in enumerate(scored_crystals, start=1)
        ]

        now = _now()
        with connect(self.config.database_path) as conn:
            session = self._require_active_session(conn, session_id)
            for result in results:
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
                    values (?, 'system', 'recalled_crystal', ?, '', ?, ?)
                    """,
                    (
                        session_id,
                        result.crystal.text,
                        json.dumps(
                            {
                                "crystal_id": result.crystal.id,
                                "rank": result.rank,
                                "score": result.score,
                            },
                            ensure_ascii=False,
                            sort_keys=True,
                        ),
                        now,
                    ),
                )
                memory_id = int(cursor.lastrowid)
                conn.execute(
                    "insert into short_term_memories_fts(rowid, text) values (?, ?)",
                    (memory_id, result.crystal.text),
                )
            conn.commit()

        return results

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

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect, ensure_schema
from hieronymus.dream_persistence import append_audit_entry


@dataclass(frozen=True)
class DreamAuditEntry:
    id: int
    dream_run_id: int
    phase_run_id: int | None
    event_type: str
    severity: str
    summary: str
    payload: Any
    created_at: str


class DreamAuditStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            ensure_schema(conn)

    def append(
        self,
        *,
        dream_run_id: int,
        phase_run_id: int | None,
        event_type: str,
        severity: str,
        summary: str,
        payload: Any,
    ) -> int:
        with connect(self.config.database_path) as conn:
            audit_id = append_audit_entry(
                conn,
                dream_run_id=dream_run_id,
                phase_run_id=phase_run_id,
                event_type=event_type,
                severity=severity,
                summary=summary,
                payload=payload,
            )
            conn.commit()
        return audit_id

    def list_for_run(self, dream_run_id: int) -> list[DreamAuditEntry]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_audit_entries
                where dream_run_id = ?
                order by id
                """,
                (dream_run_id,),
            ).fetchall()
        return [
            DreamAuditEntry(
                id=int(row["id"]),
                dream_run_id=int(row["dream_run_id"]),
                phase_run_id=(None if row["phase_run_id"] is None else int(row["phase_run_id"])),
                event_type=row["event_type"],
                severity=row["severity"],
                summary=row["summary"],
                payload=json.loads(row["payload_json"]),
                created_at=row["created_at"],
            )
            for row in rows
        ]

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

_REDACTED = "[REDACTED]"
_SECRET_KEYS = frozenset(
    {
        "apikey",
        "authorization",
        "xapikey",
        "anthropicversion",
        "token",
        "bearer",
    }
)


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
            apply_migration(conn, "global.sql")

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
        payload_json = json.dumps(
            _redact_payload(payload),
            ensure_ascii=False,
            sort_keys=True,
        )
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                insert into dream_audit_entries(
                  dream_run_id,
                  phase_run_id,
                  event_type,
                  severity,
                  summary,
                  payload_json,
                  created_at
                )
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    dream_run_id,
                    phase_run_id,
                    event_type,
                    severity,
                    summary,
                    payload_json,
                    _now(),
                ),
            )
            conn.commit()
        return int(cursor.lastrowid)

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


def _redact_payload(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: _REDACTED if _is_secret_key(key) else _redact_payload(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [_redact_payload(item) for item in value]
    return value


def _is_secret_key(key: object) -> bool:
    if not isinstance(key, str):
        return False
    normalized = key.replace("-", "").replace("_", "").lower()
    return normalized in _SECRET_KEYS


def _now() -> str:
    return datetime.now(UTC).isoformat()

from __future__ import annotations

from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

IMMEDIATE_EVENT_DELTAS = {
    "confirmed_by_user": (0.15, 0.20),
    "contradicted_by_user": (-0.20, -0.25),
    "deleted_by_user": (-0.50, -0.35),
}
PASSIVE_EVENT_DELTAS = {
    "cited": (0.03, 0.02),
    "used_in_translation": (0.05, 0.02),
    "passed_review": (0.07, 0.05),
    "caused_correction": (-0.10, -0.12),
    "superseded": (-0.12, -0.05),
}

_ARCHIVE_STRENGTH_THRESHOLD = 0.05


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _clamp_score(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def apply_score_delta(
    *,
    strength: float,
    confidence: float,
    status: str,
    crystal_type: str,
    strength_delta: float,
    confidence_delta: float,
) -> tuple[float, float, str]:
    updated_strength = _clamp_score(strength + strength_delta)
    updated_confidence = _clamp_score(confidence + confidence_delta)
    updated_status = status
    if updated_confidence == 0.0 and not (crystal_type == "rule" and status == "active"):
        updated_status = "archived"
    return (updated_strength, updated_confidence, updated_status)


class FeedbackStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def record(
        self,
        crystal_id: int,
        event_type: str,
        source_role: str,
        evidence: str = "",
        session_id: int | None = None,
    ) -> int:
        strength_delta, confidence_delta = self._event_delta(event_type)
        is_immediate = event_type in IMMEDIATE_EVENT_DELTAS
        now = _now()

        with connect(self.config.database_path) as conn:
            crystal = conn.execute(
                """
                select series_slug, source_language, target_language,
                       crystal_type, strength, confidence, status
                from crystals
                where id = ?
                """,
                (crystal_id,),
            ).fetchone()
            if crystal is None:
                raise KeyError(f"unknown crystal: {crystal_id}")

            if session_id is not None:
                session = conn.execute(
                    """
                    select series_slug, source_language, target_language
                    from task_sessions
                    where id = ?
                    """,
                    (session_id,),
                ).fetchone()
                if session is None:
                    raise KeyError(f"unknown session: {session_id}")
                self._validate_session_context(crystal, session)

            cursor = conn.execute(
                """
                insert into memory_events(
                  crystal_id,
                  session_id,
                  event_type,
                  source_role,
                  evidence,
                  strength_delta,
                  confidence_delta,
                  applied,
                  created_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    crystal_id,
                    session_id,
                    event_type,
                    source_role,
                    evidence,
                    strength_delta,
                    confidence_delta,
                    int(is_immediate),
                    now,
                ),
            )
            event_id = int(cursor.lastrowid)

            if is_immediate:
                strength, confidence, status = apply_score_delta(
                    strength=float(crystal["strength"]),
                    confidence=float(crystal["confidence"]),
                    status=crystal["status"],
                    crystal_type=crystal["crystal_type"],
                    strength_delta=strength_delta,
                    confidence_delta=confidence_delta,
                )
                if event_type == "deleted_by_user" and strength < _ARCHIVE_STRENGTH_THRESHOLD:
                    status = "archived"
                conn.execute(
                    """
                    update crystals
                    set strength = ?,
                        confidence = ?,
                        status = ?,
                        updated_at = ?
                    where id = ?
                    """,
                    (strength, confidence, status, now, crystal_id),
                )

            conn.commit()

        return event_id

    def _event_delta(self, event_type: str) -> tuple[float, float]:
        if event_type in IMMEDIATE_EVENT_DELTAS:
            return IMMEDIATE_EVENT_DELTAS[event_type]
        if event_type in PASSIVE_EVENT_DELTAS:
            return PASSIVE_EVENT_DELTAS[event_type]
        raise ValueError(f"unknown event_type: {event_type}")

    def _validate_session_context(self, crystal, session) -> None:
        if session["series_slug"] != crystal["series_slug"]:
            raise ValueError("feedback session series_slug does not match crystal context")
        if crystal["source_language"] and session["source_language"] != crystal["source_language"]:
            raise ValueError("feedback session source_language does not match crystal context")
        if crystal["target_language"] and session["target_language"] != crystal["target_language"]:
            raise ValueError("feedback session target_language does not match crystal context")

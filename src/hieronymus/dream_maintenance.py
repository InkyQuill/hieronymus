from __future__ import annotations

import sqlite3
from dataclasses import dataclass, field

from hieronymus.scoring import PASSIVE_EVENT_DELTAS, apply_score_delta
from hieronymus.values import clamp_score, utc_now

STRENGTH_DECAY_PER_CYCLE = 0.03
CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW = 0.20
CONFIDENCE_DECAY_PER_CYCLE = 0.01
MAX_SKIPPED_CANDIDATE_RECORDS = 2
DREAM_MAINTENANCE_INDEX = "idx_crystals_dream_maintenance"

_INVALID_PASSIVE_EVENT_WHERE = """
memory_events.applied = 0
and (
  memory_events.crystal_id is null
  or not exists (
    select 1
    from crystals
    where crystals.id = memory_events.crystal_id
  )
)
"""

_DECAY_CANDIDATE_SQL = f"""
select id, crystal_type, strength, confidence, status
from crystals indexed by {DREAM_MAINTENANCE_INDEX}
where (status = 'candidate' or (status = 'active' and crystal_type != 'rule'))
  and max(
    coalesce(last_reinforced_cycle, -1),
    coalesce(last_activated_cycle, -1),
    coalesce(created_cycle, -1)
  ) < ?
order by max(
  coalesce(last_reinforced_cycle, -1),
  coalesce(last_activated_cycle, -1),
  coalesce(created_cycle, -1)
), id
limit ?
"""


@dataclass(frozen=True)
class DecayCandidate:
    id: int
    crystal_type: str
    strength: float
    confidence: float
    status: str


@dataclass(frozen=True)
class DecaySelection:
    candidates: tuple[DecayCandidate, ...]
    sentinel: DecayCandidate | None

    @property
    def has_more(self) -> bool:
        return self.sentinel is not None


@dataclass(frozen=True)
class MaintenanceSummary:
    created_crystal_ids: tuple[int, ...] = ()
    created_concept_ids: tuple[int, ...] = ()
    created_facet_ids: tuple[int, ...] = ()
    created_links: tuple[dict[str, int | str], ...] = ()
    superseded_crystal_ids: tuple[int, ...] = ()
    reinforced_crystal_ids: tuple[int, ...] = ()
    decayed_crystal_ids: tuple[int, ...] = ()
    rejected_entries: tuple[dict[str, object], ...] = ()
    skipped_candidates: tuple[dict[str, object], ...] = ()
    searched_related_candidates: dict[str, object] = field(default_factory=dict)
    affected_memory_set: dict[str, object] = field(default_factory=dict)


def _unique_ints(values: list[int]) -> tuple[int, ...]:
    return tuple(dict.fromkeys(values))


class DreamMaintenance:
    """Apply bounded dream maintenance inside a caller-owned transaction."""

    def __init__(self, conn: sqlite3.Connection) -> None:
        self.conn = conn

    def has_candidates(self, cycle_id: int) -> bool:
        invalid = self.conn.execute(
            f"select 1 from memory_events where {_INVALID_PASSIVE_EVENT_WHERE} limit 1"
        ).fetchone()
        if invalid is not None:
            return True
        passive = self.conn.execute(
            """
            select 1 from memory_events
            where applied = 0 and crystal_id is not null
            limit 1
            """
        ).fetchone()
        if passive is not None:
            return True
        return self.select_decay_candidates(cycle_id=cycle_id, limit=0).has_more

    def select_decay_candidates(self, *, cycle_id: int, limit: int) -> DecaySelection:
        cap = max(limit, 0)
        rows = self.conn.execute(
            _DECAY_CANDIDATE_SQL,
            (cycle_id, cap + 1),
        ).fetchall()
        candidates = tuple(self._decay_candidate(row) for row in rows[:cap])
        sentinel = self._decay_candidate(rows[cap]) if len(rows) > cap else None
        return DecaySelection(candidates=candidates, sentinel=sentinel)

    def explain_decay_candidate_query(self, *, cycle_id: int, limit: int) -> tuple[str, ...]:
        rows = self.conn.execute(
            f"explain query plan {_DECAY_CANDIDATE_SQL}",
            (cycle_id, max(limit, 0) + 1),
        ).fetchall()
        return tuple(str(row[3]) for row in rows)

    @staticmethod
    def _decay_candidate(row: sqlite3.Row) -> DecayCandidate:
        return DecayCandidate(
            id=int(row["id"]),
            crystal_type=str(row["crystal_type"]),
            strength=float(row["strength"]),
            confidence=float(row["confidence"]),
            status=str(row["status"]),
        )

    def apply_passive_events(
        self,
        cycle_id: int,
        *,
        max_changed_crystals: int,
    ) -> MaintenanceSummary:
        now = utc_now()
        skipped_candidates = self._apply_invalid_passive_events(cycle_id)
        selected_crystal_ids = self._select_passive_event_crystal_ids(
            max_changed_crystals=max_changed_crystals
        )
        rows = self._pending_passive_event_rows(selected_crystal_ids)
        reinforced_ids: list[int] = []
        decayed_ids: list[int] = []
        for event in rows:
            crystal_id = event["crystal_id"]
            event_id = int(event["id"])
            crystal = self.conn.execute(
                "select strength, confidence from crystals where id = ?",
                (crystal_id,),
            ).fetchone()
            if crystal is None:
                skipped_candidates.append(
                    {
                        "entry_path": f"passive_events[{event_id}]",
                        "reason": "unknown_crystal",
                        "event_id": event_id,
                        "crystal_id": int(crystal_id),
                    }
                )
                self._mark_memory_event_applied(event_id, cycle_id)
                continue

            strength_delta = float(event["strength_delta"])
            confidence_delta = float(event["confidence_delta"])
            strength = clamp_score(float(crystal["strength"]) + strength_delta)
            confidence = clamp_score(float(crystal["confidence"]) + confidence_delta)
            last_reinforced_cycle = (
                cycle_id
                if strength_delta > 0 and event["event_type"] in PASSIVE_EVENT_DELTAS
                else None
            )
            if last_reinforced_cycle is None:
                self.conn.execute(
                    """
                    update crystals set strength = ?, confidence = ?, updated_at = ?
                    where id = ?
                    """,
                    (strength, confidence, now, crystal_id),
                )
            else:
                self.conn.execute(
                    """
                    update crystals
                    set strength = ?, confidence = ?, last_reinforced_cycle = ?, updated_at = ?
                    where id = ?
                    """,
                    (strength, confidence, last_reinforced_cycle, now, crystal_id),
                )
            if strength_delta > 0 or confidence_delta > 0:
                reinforced_ids.append(int(crystal_id))
            if strength_delta < 0 or confidence_delta < 0:
                decayed_ids.append(int(crystal_id))
            self._mark_memory_event_applied(event_id, cycle_id)
        skipped_candidates.extend(self._passive_event_cap_skips(selected_crystal_ids))
        return MaintenanceSummary(
            reinforced_crystal_ids=_unique_ints(reinforced_ids),
            decayed_crystal_ids=_unique_ints(decayed_ids),
            skipped_candidates=tuple(skipped_candidates),
        )

    def _apply_invalid_passive_events(self, cycle_id: int) -> list[dict[str, object]]:
        rows = self.conn.execute(
            f"""
            select id, crystal_id from memory_events
            where {_INVALID_PASSIVE_EVENT_WHERE}
            order by id limit ?
            """,
            (MAX_SKIPPED_CANDIDATE_RECORDS + 1,),
        ).fetchall()
        if not rows:
            return []
        skipped: list[dict[str, object]] = []
        for row in rows[:MAX_SKIPPED_CANDIDATE_RECORDS]:
            event_id = int(row["id"])
            crystal_id = row["crystal_id"]
            item: dict[str, object] = {
                "entry_path": f"passive_events[{event_id}]",
                "reason": "missing_crystal_id" if crystal_id is None else "unknown_crystal",
                "event_id": event_id,
            }
            if crystal_id is not None:
                item["crystal_id"] = int(crystal_id)
            skipped.append(item)
        if len(rows) > MAX_SKIPPED_CANDIDATE_RECORDS:
            skipped.append(
                {
                    "entry_path": "passive_events",
                    "reason": "invalid_passive_event",
                    "skipped_count_at_least": 1,
                }
            )
        self.conn.execute(
            f"""
            update memory_events set applied = 1, cycle_id = ?
            where {_INVALID_PASSIVE_EVENT_WHERE}
            """,
            (cycle_id,),
        )
        return skipped

    def _select_passive_event_crystal_ids(self, *, max_changed_crystals: int) -> tuple[int, ...]:
        if max_changed_crystals < 1:
            return ()
        rows = self.conn.execute(
            """
            select memory_events.crystal_id
            from memory_events
            join crystals on crystals.id = memory_events.crystal_id
            where memory_events.applied = 0 and memory_events.crystal_id is not null
            group by memory_events.crystal_id
            order by min(memory_events.id)
            limit ?
            """,
            (max_changed_crystals,),
        ).fetchall()
        return tuple(int(row["crystal_id"]) for row in rows)

    def _pending_passive_event_rows(self, selected_crystal_ids: tuple[int, ...]):
        if not selected_crystal_ids:
            return []
        placeholders = ", ".join("?" for _ in selected_crystal_ids)
        return self.conn.execute(
            f"""
            select * from memory_events
            where applied = 0 and crystal_id in ({placeholders})
            order by id
            """,
            selected_crystal_ids,
        ).fetchall()

    def _passive_event_cap_skips(
        self, selected_crystal_ids: tuple[int, ...]
    ) -> list[dict[str, object]]:
        where = "memory_events.applied = 0 and memory_events.crystal_id is not null"
        params: list[object] = []
        if selected_crystal_ids:
            placeholders = ", ".join("?" for _ in selected_crystal_ids)
            where += f" and memory_events.crystal_id not in ({placeholders})"
            params.extend(selected_crystal_ids)
        rows = self.conn.execute(
            f"""
            select memory_events.id, memory_events.crystal_id
            from memory_events
            join crystals on crystals.id = memory_events.crystal_id
            where {where}
            group by memory_events.crystal_id
            order by min(memory_events.id)
            limit ?
            """,
            (*params, MAX_SKIPPED_CANDIDATE_RECORDS + 1),
        ).fetchall()
        skipped = [
            {
                "entry_path": f"passive_events[{int(row['id'])}]",
                "reason": "changed_crystal_cap",
                "event_id": int(row["id"]),
                "crystal_id": int(row["crystal_id"]),
            }
            for row in rows[:MAX_SKIPPED_CANDIDATE_RECORDS]
        ]
        if len(rows) > MAX_SKIPPED_CANDIDATE_RECORDS:
            skipped.append(
                {
                    "entry_path": "passive_events",
                    "reason": "changed_crystal_cap",
                    "skipped_count_at_least": 1,
                }
            )
        return skipped

    def _mark_memory_event_applied(self, event_id: int, cycle_id: int) -> None:
        self.conn.execute(
            "update memory_events set applied = 1, cycle_id = ? where id = ?",
            (cycle_id, event_id),
        )

    def mark_activations_for_cycle(self, cycle_id: int, session_ids: list[int]) -> None:
        if not session_ids:
            return
        now = utc_now()
        placeholders = ", ".join("?" for _ in session_ids)
        self.conn.execute(
            f"update crystal_activations set cycle_id = ? where session_id in ({placeholders})",
            (cycle_id, *session_ids),
        )
        self.conn.execute(
            f"""
            update crystals set last_activated_cycle = ?, updated_at = ?
            where id in (
              select distinct crystal_id from crystal_activations
              where session_id in ({placeholders})
            )
            """,
            (cycle_id, now, *session_ids),
        )

    def apply_cycle_decay(
        self,
        cycle_id: int,
        *,
        max_changed_crystals: int,
    ) -> MaintenanceSummary:
        now = utc_now()
        selection = self.select_decay_candidates(
            cycle_id=cycle_id,
            limit=max_changed_crystals,
        )
        skipped_candidates: list[dict[str, object]] = []
        if selection.sentinel is not None:
            skipped_candidates.extend(
                (
                    {
                        "entry_path": f"cycle_decay.crystals[{selection.sentinel.id}]",
                        "reason": "changed_crystal_cap",
                        "crystal_id": selection.sentinel.id,
                    },
                    {
                        "entry_path": "cycle_decay.crystals",
                        "reason": "changed_crystal_cap",
                        "skipped_count_at_least": 1,
                    },
                )
            )
        decayed_ids: list[int] = []
        for crystal in selection.candidates:
            confidence_delta = (
                CONFIDENCE_DECAY_PER_CYCLE
                if clamp_score(crystal.strength - STRENGTH_DECAY_PER_CYCLE)
                < CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW
                else 0.0
            )
            strength, confidence, status = apply_score_delta(
                strength=crystal.strength,
                confidence=crystal.confidence,
                status=crystal.status,
                crystal_type=crystal.crystal_type,
                strength_delta=-STRENGTH_DECAY_PER_CYCLE,
                confidence_delta=-confidence_delta,
            )
            self.conn.execute(
                """
                update crystals set strength = ?, confidence = ?, status = ?, updated_at = ?
                where id = ?
                """,
                (strength, confidence, status, now, crystal.id),
            )
            strength_delta = strength - crystal.strength
            applied_confidence_delta = confidence - crystal.confidence
            if strength_delta == 0.0 and applied_confidence_delta == 0.0:
                continue
            decayed_ids.append(crystal.id)
            self.conn.execute(
                """
                insert into memory_events(
                  crystal_id, session_id, event_type, source_role, evidence,
                  strength_delta, confidence_delta, applied, cycle_id, created_at
                ) values (?, null, 'cycle_decay', 'system', 'cycle decay', ?, ?, 1, ?, ?)
                """,
                (crystal.id, strength_delta, applied_confidence_delta, cycle_id, now),
            )
        return MaintenanceSummary(
            decayed_crystal_ids=tuple(decayed_ids),
            skipped_candidates=tuple(skipped_candidates),
        )

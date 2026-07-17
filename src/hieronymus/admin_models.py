from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class AdminRow:
    id: int | str
    kind: str
    label: str
    status: str
    scope: str
    language_pair: str
    quality_label: str = ""
    tags: tuple[str, ...] = ()


@dataclass(frozen=True)
class AdminDetail:
    title: str
    subtitle: str
    body: str
    fields: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class AdminCrystalEditPayload:
    title: str
    text: str


@dataclass(frozen=True)
class AdminSnapshot:
    view: str
    rows: list[AdminRow]
    selected: AdminRow | None
    detail: AdminDetail
    filters: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class AdminStats:
    series: int
    crystals: int
    lessons: int
    short_term_memories: int
    sessions: int
    dream_runs: int
    pending_proposals: int
    audit_events: int

    def as_dict(self) -> dict[str, int]:
        return {
            "series": self.series,
            "crystals": self.crystals,
            "lessons": self.lessons,
            "short_term_memories": self.short_term_memories,
            "sessions": self.sessions,
            "dream_runs": self.dream_runs,
            "pending_proposals": self.pending_proposals,
            "audit_events": self.audit_events,
        }


@dataclass(frozen=True)
class AdminShortTermStatus:
    pending_count: int
    min_pending_short_term_memories: int
    max_pending_short_term_memories: int
    urgent: bool
    drain_in_progress: bool = False
    drain_completed: int = 0
    drain_remaining: int = 0
    drain_total: int = 0
    drain_progress: float = 0.0

    def as_dict(self) -> dict[str, int | bool | float]:
        return {
            "pending_count": self.pending_count,
            "min_pending_short_term_memories": self.min_pending_short_term_memories,
            "max_pending_short_term_memories": self.max_pending_short_term_memories,
            "urgent": self.urgent,
            "drain_in_progress": self.drain_in_progress,
            "drain_completed": self.drain_completed,
            "drain_remaining": self.drain_remaining,
            "drain_total": self.drain_total,
            "drain_progress": self.drain_progress,
        }


@dataclass(frozen=True)
class AdminDreamStatus:
    state: str
    current_phase: str
    progress: float
    run_id: int | None = None
    cycle_id: int | None = None
    owner: str = ""
    started_at: str = ""

    def as_dict(self) -> dict[str, str | float | int | None]:
        return {
            "state": self.state,
            "current_phase": self.current_phase,
            "progress": self.progress,
            "run_id": self.run_id,
            "cycle_id": self.cycle_id,
            "owner": self.owner,
            "started_at": self.started_at,
        }


@dataclass(frozen=True)
class ActionResult:
    entity_type: str
    entity_id: int | str
    action: str
    message: str


@dataclass(frozen=True)
class ProvenanceDetail:
    title: str
    sources: list[dict[str, str]]


@dataclass(frozen=True)
class DreamReview:
    run_id: int
    source_sessions: list[int]
    consumed_memories: list[str]
    created_crystals: list[str]
    updated_crystals: list[str]
    decayed_crystals: list[str]
    strict_proposals: list[str]
    failed_outputs: list[str]
    validation_errors: list[str]
    passes: list[dict[str, int | str]]

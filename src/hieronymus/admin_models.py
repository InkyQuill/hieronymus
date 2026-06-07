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
class ActionResult:
    entity_type: str
    entity_id: int | str
    action: str
    message: str

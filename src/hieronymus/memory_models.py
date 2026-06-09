from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class TranslationContext:
    series_slug: str
    source_language: str
    target_language: str
    task_type: str
    volume: str = ""
    chapter: str = ""
    tags: tuple[str, ...] = ()

    @property
    def scope_key(self) -> str:
        return f"series:{self.series_slug}"


@dataclass(frozen=True)
class TaskSessionRecord:
    id: int
    context: TranslationContext
    status: str
    cycle_id: int | None


@dataclass(frozen=True)
class ShortTermMemoryRecord:
    id: int
    session_id: int
    source_role: str
    kind: str
    text: str
    source_ref: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class CrystalRecord:
    id: int
    crystal_type: str
    text: str
    title: str
    scope_type: str
    scope_key: str
    series_slug: str
    source_language: str
    target_language: str
    strength: float
    confidence: float
    status: str
    source_credibility: str = "observation"
    rule_intent: str = ""
    malformed_penalty: float = 0.0
    supersedes_crystal_id: int | None = None
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    concept_ids: tuple[int, ...] = ()


@dataclass(frozen=True)
class RecallResult:
    rank: int
    score: float
    reason: str
    source: str = "long_term"
    crystal: CrystalRecord | None = None
    short_term_memory: ShortTermMemoryRecord | None = None

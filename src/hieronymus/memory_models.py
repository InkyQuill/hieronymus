from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal


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
    # Side-table-backed fields stay empty until CrystalStore hydration is added.
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    concept_ids: tuple[int, ...] = ()


@dataclass(frozen=True)
class RecallResult:
    source: Literal["long_term", "short_term"]
    rank: int
    score: float
    reason: str
    crystal: CrystalRecord | None = None
    short_term_memory: ShortTermMemoryRecord | None = None

    def __post_init__(self) -> None:
        if self.source == "long_term":
            if self.crystal is None:
                raise ValueError("long_term recall results require a crystal")
            if self.short_term_memory is not None:
                raise ValueError("long_term recall results must not include short-term memory")
            return

        if self.source == "short_term":
            if self.crystal is not None:
                raise ValueError("short_term recall results must not include a crystal")
            if self.short_term_memory is None:
                raise ValueError("short_term recall results require short-term memory")
            return

        raise ValueError(f"unknown recall source: {self.source}")

    @classmethod
    def long_term(
        cls,
        crystal: CrystalRecord,
        *,
        rank: int,
        score: float,
        reason: str,
    ) -> RecallResult:
        return cls(
            source="long_term",
            rank=rank,
            score=score,
            reason=reason,
            crystal=crystal,
        )

    @classmethod
    def short_term(
        cls,
        short_term_memory: ShortTermMemoryRecord,
        *,
        rank: int,
        score: float,
        reason: str,
    ) -> RecallResult:
        return cls(
            source="short_term",
            rank=rank,
            score=score,
            reason=reason,
            short_term_memory=short_term_memory,
        )

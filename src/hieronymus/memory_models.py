from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Literal


def normalize_string_tuple(
    values: Iterable[str],
    *,
    lowercase: bool = False,
) -> tuple[str, ...]:
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        item = value.strip()
        if lowercase:
            item = item.lower()
        if not item or item in seen:
            continue
        seen.add(item)
        normalized.append(item)
    return tuple(normalized)


@dataclass(frozen=True)
class TranslationContext:
    series_slug: str
    source_language: str
    target_language: str
    task_type: str
    volume: str = ""
    chapter: str = ""
    tags: tuple[str, ...] = ()
    language_tags: tuple[str, ...] | None = None
    story_scopes: tuple[str, ...] | None = None
    semantic_tags: tuple[str, ...] | None = None

    def __post_init__(self) -> None:
        story_scope_seeds: list[str] = []
        if self.volume.strip():
            story_scope_seeds.append(f"volume:{self.volume.strip()}")
        if self.chapter.strip():
            story_scope_seeds.append(f"chapter:{self.chapter.strip()}")
        language_tag_seeds = (
            (self.source_language, self.target_language)
            if self.language_tags is None
            else self.language_tags
        )
        story_scope_values = story_scope_seeds if self.story_scopes is None else self.story_scopes
        semantic_tag_values = self.tags if self.semantic_tags is None else self.semantic_tags

        object.__setattr__(
            self,
            "language_tags",
            normalize_string_tuple(language_tag_seeds, lowercase=True),
        )
        object.__setattr__(
            self,
            "story_scopes",
            normalize_string_tuple(story_scope_values),
        )
        object.__setattr__(
            self,
            "semantic_tags",
            normalize_string_tuple(semantic_tag_values),
        )

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
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    source_credibility: str = "observation"
    rule_intent: str = ""
    soft_origin: str = ""


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
    # Side-table-backed language tags stay empty until CrystalStore hydration is added.
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    soft_origin: str = ""
    is_inferred: bool = False
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

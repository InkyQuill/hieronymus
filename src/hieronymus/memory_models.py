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
    concept_labels: tuple[str, ...] = ()

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
        concept_labels: tuple[str, ...] = (),
    ) -> RecallResult:
        return cls(
            source="long_term",
            rank=rank,
            score=score,
            reason=reason,
            crystal=crystal,
            concept_labels=concept_labels,
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

    @property
    def tier(self) -> Literal["short_term", "long_term"]:
        return self.source

    @property
    def id(self) -> int:
        if self.crystal is not None:
            return self.crystal.id
        assert self.short_term_memory is not None
        return self.short_term_memory.id

    @property
    def title(self) -> str:
        if self.crystal is not None:
            return self.crystal.title
        assert self.short_term_memory is not None
        return self.short_term_memory.kind

    @property
    def kind(self) -> str:
        if self.crystal is not None:
            return self.crystal.crystal_type
        assert self.short_term_memory is not None
        return self.short_term_memory.kind

    @property
    def text(self) -> str:
        if self.crystal is not None:
            return self.crystal.text
        assert self.short_term_memory is not None
        return self.short_term_memory.text

    @property
    def crystal_type(self) -> str | None:
        return self.crystal.crystal_type if self.crystal is not None else None

    @property
    def concept_ids(self) -> tuple[int, ...]:
        return self.crystal.concept_ids if self.crystal is not None else ()

    @property
    def language_tags(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.language_tags
        assert self.short_term_memory is not None
        return self.short_term_memory.language_tags

    @property
    def story_scopes(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.story_scopes
        assert self.short_term_memory is not None
        return self.short_term_memory.story_scopes

    @property
    def semantic_tags(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.semantic_tags
        assert self.short_term_memory is not None
        return self.short_term_memory.semantic_tags

    @property
    def source_credibility(self) -> str:
        if self.crystal is not None:
            return self.crystal.source_credibility
        assert self.short_term_memory is not None
        return self.short_term_memory.source_credibility

    @property
    def confidence(self) -> float:
        return self.crystal.confidence if self.crystal is not None else 0.0

    @property
    def strength(self) -> float:
        return self.crystal.strength if self.crystal is not None else 0.0

    @property
    def soft_origin(self) -> str:
        if self.crystal is not None:
            return self.crystal.soft_origin
        assert self.short_term_memory is not None
        return self.short_term_memory.soft_origin

    @property
    def is_rule(self) -> bool:
        if self.crystal is not None:
            return self.crystal.crystal_type == "rule" and self.crystal.status == "active"
        assert self.short_term_memory is not None
        return bool(self.short_term_memory.rule_intent)

    @property
    def is_thought(self) -> bool:
        if self.crystal is not None:
            return self.crystal.crystal_type == "thought" or self.crystal.is_inferred
        assert self.short_term_memory is not None
        return self.short_term_memory.source_credibility == "thought"

    @property
    def rank_reason(self) -> str:
        return self.reason

    def enriched_payload(self) -> dict[str, object]:
        return {
            "tier": self.tier,
            "id": self.id,
            "title": self.title,
            "kind": self.kind,
            "text": self.text,
            "crystal_type": self.crystal_type,
            "concept_ids": self.concept_ids,
            "concept_labels": self.concept_labels,
            "language_tags": self.language_tags,
            "story_scopes": self.story_scopes,
            "semantic_tags": self.semantic_tags,
            "source_credibility": self.source_credibility,
            "confidence": self.confidence,
            "strength": self.strength,
            "soft_origin": self.soft_origin,
            "is_rule": self.is_rule,
            "is_thought": self.is_thought,
            "score": self.score,
            "rank_reason": self.rank_reason,
        }

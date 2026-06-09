from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ConceptRecord:
    id: int
    canonical_name: str
    description: str
    status: str
    confidence: float
    tags: tuple[str, ...] = ()


@dataclass(frozen=True)
class ConceptFacetRecord:
    id: int
    concept_id: int
    language: str
    facet_type: str
    value: str
    confidence: float
    source_crystal_id: int | None = None


@dataclass(frozen=True)
class ConceptLinkRecord:
    crystal_id: int
    concept_id: int
    link_type: str
    confidence: float


@dataclass(frozen=True)
class ConceptCandidate:
    name: str
    description: str = ""
    tags: tuple[str, ...] = field(default_factory=tuple)
    confidence: float = 0.2

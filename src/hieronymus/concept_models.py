from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ConceptRecord:
    id: int
    canonical_name: str
    description: str
    status: str
    confidence: float
    scope_type: str = "global"
    scope_key: str = ""
    tags: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if self.scope_type == "global":
            if self.scope_key != "":
                raise ValueError("global concept scope requires an empty key")
            return

        if self.scope_key == "":
            raise ValueError("non-global concept scope requires a key")


@dataclass(frozen=True)
class ConceptFacetRecord:
    id: int
    concept_id: int
    language: str
    facet_type: str
    value: str
    confidence: float
    source_crystal_id: int | None = None
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    is_canonical: bool = False


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

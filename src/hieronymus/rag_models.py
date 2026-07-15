from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal

RagSourceType = Literal["text", "markdown", "glossary"]
RagChunkKind = Literal["text", "markdown_section", "glossary_entry"]


@dataclass(frozen=True)
class RagSourceRecord:
    id: int
    series_slug: str
    source_ref: str
    source_type: RagSourceType
    content_type: str
    checksum: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class RagChunkRecord:
    id: int
    source_id: int
    series_slug: str
    source_ref: str
    chunk_kind: RagChunkKind
    text: str
    display_text: str
    location: str
    metadata: dict[str, object] = field(default_factory=dict)
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()

    @property
    def title(self) -> str:
        if self.location:
            return f"{self.source_ref} {self.location}"
        return self.source_ref

    @property
    def kind(self) -> str:
        return self.chunk_kind


@dataclass(frozen=True)
class RagSearchResult:
    chunk: RagChunkRecord
    score: float
    reason: str


@dataclass(frozen=True)
class RagImportResult:
    source: RagSourceRecord
    chunk_count: int
    skipped: bool

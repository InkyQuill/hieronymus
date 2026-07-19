from __future__ import annotations

import hashlib
import json
import math
import threading
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Protocol

from hieronymus.embeddings import EmbeddingIdentity


class SemanticIndexError(RuntimeError):
    """Base class for semantic index failures."""


class GenerationNotFoundError(SemanticIndexError):
    """Raised when a requested generation does not exist."""


class GenerationNotActiveError(SemanticIndexError):
    """Raised when a search targets a generation that is not active."""


class GenerationConflictError(SemanticIndexError):
    """Raised when generation state changed concurrently or conflicts."""


class IndexManifestError(SemanticIndexError):
    """Raised when a generation does not match its rebuild manifest."""


class IndexBatchSizeError(SemanticIndexError):
    """Raised when a mutation exceeds the configured batch bound."""


class IndexClosedError(SemanticIndexError):
    """Raised when an operation targets a closed index."""


class DuplicateChunkIdError(SemanticIndexError):
    """Raised when one mutation or manifest contains a chunk ID more than once."""


@dataclass(frozen=True)
class IndexHealth:
    healthy: bool
    detail: str


@dataclass(frozen=True)
class IndexRow:
    chunk_id: str
    generation: str
    series_slug: str
    source_language: str
    target_language: str
    story_scope: str
    semantic_scope: str
    checksum: str
    identity: EmbeddingIdentity
    vector: tuple[float, ...]

    def __post_init__(self) -> None:
        for name in (
            "chunk_id",
            "generation",
            "series_slug",
            "source_language",
            "target_language",
            "story_scope",
            "semantic_scope",
            "checksum",
        ):
            if not getattr(self, name).strip():
                raise ValueError(f"{name} must not be blank")
        vector = tuple(float(value) for value in self.vector)
        if len(vector) != self.identity.dimensions:
            raise ValueError(
                f"expected {self.identity.dimensions} vector dimensions, received {len(vector)}"
            )
        if any(not math.isfinite(value) for value in vector):
            raise ValueError("vector components must be finite")
        object.__setattr__(self, "vector", vector)


@dataclass(frozen=True)
class SearchFilters:
    series_slug: str | None = None
    source_language: str | None = None
    target_language: str | None = None
    story_scope: str | None = None
    semantic_scope: str | None = None

    def matches(self, row: IndexRow) -> bool:
        return all(
            expected is None or getattr(row, name) == expected
            for name, expected in (
                ("series_slug", self.series_slug),
                ("source_language", self.source_language),
                ("target_language", self.target_language),
                ("story_scope", self.story_scope),
                ("semantic_scope", self.semantic_scope),
            )
        )


@dataclass(frozen=True)
class SearchResult:
    chunk_id: str
    distance: float
    generation: str
    identity: EmbeddingIdentity


class GenerationPointer(Protocol):
    """Authoritative generation pointer; persistent implementations may use SQLite."""

    def read(self) -> str | None: ...

    def compare_and_set(self, expected: str | None, generation: str) -> bool: ...


class InMemoryGenerationPointer:
    """Thread-safe pointer for tests and non-persistent callers."""

    def __init__(self, generation: str | None = None) -> None:
        self._generation = generation
        self._lock = threading.Lock()

    def read(self) -> str | None:
        with self._lock:
            return self._generation

    def compare_and_set(self, expected: str | None, generation: str) -> bool:
        with self._lock:
            if self._generation != expected:
                return False
            self._generation = generation
            return True


class SemanticIndex(Protocol):
    @property
    def active_generation(self) -> str | None: ...

    def health(self) -> IndexHealth: ...

    def begin_rebuild(
        self,
        generation: str,
        *,
        identity: EmbeddingIdentity,
        expected_count: int,
        manifest_checksum: str,
    ) -> None: ...

    def upsert(self, generation: str, rows: Sequence[IndexRow]) -> None: ...

    def delete_chunk_ids(self, generation: str, chunk_ids: Sequence[str]) -> None: ...

    def search(
        self,
        generation: str,
        vector: Sequence[float],
        *,
        limit: int,
        filters: SearchFilters | None = None,
    ) -> list[SearchResult]: ...

    def activate_generation(
        self, generation: str, *, expected_active_generation: str | None
    ) -> None: ...

    def cancel_generation(self, generation: str) -> None: ...

    def close(self) -> None: ...


def compute_manifest_checksum(rows: Sequence[IndexRow]) -> str:
    """Hash authoritative row metadata independently of storage serialization."""
    chunk_ids = [row.chunk_id for row in rows]
    reject_duplicate_chunk_ids(chunk_ids)
    records = [
        {
            "checksum": row.checksum,
            "chunk_id": row.chunk_id,
            "generation": row.generation,
            "model": row.identity.model,
            "provider": row.identity.provider,
            "revision": row.identity.revision,
            "dimensions": row.identity.dimensions,
            "semantic_scope": row.semantic_scope,
            "series_slug": row.series_slug,
            "source_language": row.source_language,
            "story_scope": row.story_scope,
            "target_language": row.target_language,
        }
        for row in sorted(rows, key=lambda item: item.chunk_id)
    ]
    payload = json.dumps(records, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def reject_duplicate_chunk_ids(chunk_ids: Sequence[str]) -> None:
    if len(set(chunk_ids)) != len(chunk_ids):
        raise DuplicateChunkIdError("chunk IDs must be unique within a batch")

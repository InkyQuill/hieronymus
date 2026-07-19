from __future__ import annotations

import math
import threading
from collections.abc import Callable, Sequence

import pytest

from hieronymus.embeddings import EmbeddingIdentity
from hieronymus.semantic_index import (
    GenerationConflictError,
    GenerationNotActiveError,
    GenerationNotFoundError,
    IndexBatchSizeError,
    IndexClosedError,
    IndexManifestError,
    IndexRow,
    InMemoryGenerationPointer,
    SearchFilters,
    SearchResult,
    SemanticIndex,
    compute_manifest_checksum,
)

IDENTITY = EmbeddingIdentity(
    provider="local",
    model="sentence-transformers/all-MiniLM-L6-v2",
    revision=None,
    dimensions=2,
)


def row(
    chunk_id: str,
    vector: tuple[float, float],
    *,
    generation: str = "generation-1",
    series_slug: str = "earthsea",
    source_language: str = "en",
    target_language: str = "ru",
    story_scope: str = "book-1",
    semantic_scope: str = "prose",
    checksum: str | None = None,
) -> IndexRow:
    return IndexRow(
        chunk_id=chunk_id,
        generation=generation,
        series_slug=series_slug,
        source_language=source_language,
        target_language=target_language,
        story_scope=story_scope,
        semantic_scope=semantic_scope,
        checksum=checksum or f"checksum-{chunk_id}",
        identity=IDENTITY,
        vector=vector,
    )


class MemorySemanticIndex:
    """Small independent fake used to exercise the reusable contract."""

    def __init__(self, *, max_batch_size: int = 2) -> None:
        self._pointer = InMemoryGenerationPointer()
        self._max_batch_size = max_batch_size
        self._generations: dict[str, dict[str, IndexRow]] = {}
        self._manifests: dict[str, tuple[int, str]] = {}
        self._lock = threading.RLock()
        self._closed = False

    @property
    def active_generation(self) -> str | None:
        self._ensure_open()
        return self._pointer.read()

    def health(self):
        from hieronymus.semantic_index import IndexHealth

        return IndexHealth(healthy=not self._closed, detail="closed" if self._closed else "ok")

    def begin_rebuild(
        self,
        generation: str,
        *,
        identity: EmbeddingIdentity,
        expected_count: int,
        manifest_checksum: str,
    ) -> None:
        self._ensure_open()
        del identity
        if generation in self._generations:
            raise GenerationConflictError(generation)
        self._generations[generation] = {}
        self._manifests[generation] = (expected_count, manifest_checksum)

    def upsert(self, generation: str, rows: Sequence[IndexRow]) -> None:
        self._ensure_open()
        if self._pointer.read() == generation:
            raise GenerationConflictError("cannot mutate active generation")
        if len(rows) > self._max_batch_size:
            raise IndexBatchSizeError(str(len(rows)))
        generation_rows = self._get_generation(generation)
        for item in rows:
            if item.generation != generation:
                raise ValueError("row generation does not match target generation")
            generation_rows[item.chunk_id] = item

    def delete_chunk_ids(self, generation: str, chunk_ids: Sequence[str]) -> None:
        self._ensure_open()
        if self._pointer.read() == generation:
            raise GenerationConflictError("cannot mutate active generation")
        if len(chunk_ids) > self._max_batch_size:
            raise IndexBatchSizeError(str(len(chunk_ids)))
        generation_rows = self._get_generation(generation)
        for chunk_id in chunk_ids:
            generation_rows.pop(chunk_id, None)

    def search(
        self,
        generation: str,
        vector: Sequence[float],
        *,
        limit: int,
        filters: SearchFilters | None = None,
    ) -> list[SearchResult]:
        self._ensure_open()
        if self._pointer.read() != generation:
            raise GenerationNotActiveError(generation)
        rows = list(self._get_generation(generation).values())
        filters = filters or SearchFilters()
        rows = [item for item in rows if filters.matches(item)]
        results = [
            SearchResult(
                chunk_id=item.chunk_id,
                distance=math.dist(vector, item.vector) ** 2,
                generation=generation,
                identity=item.identity,
            )
            for item in rows
        ]
        return sorted(results, key=lambda item: (item.distance, item.chunk_id))[:limit]

    def activate_generation(
        self, generation: str, *, expected_active_generation: str | None
    ) -> None:
        self._ensure_open()
        rows = list(self._get_generation(generation).values())
        expected_count, expected_checksum = self._manifests[generation]
        if len(rows) != expected_count or compute_manifest_checksum(rows) != expected_checksum:
            raise IndexManifestError(generation)
        if not self._pointer.compare_and_set(expected_active_generation, generation):
            raise GenerationConflictError("active generation changed")

    def cancel_generation(self, generation: str) -> None:
        self._ensure_open()
        if self._pointer.read() == generation:
            raise GenerationConflictError("cannot cancel active generation")
        if generation not in self._generations:
            raise GenerationNotFoundError(generation)
        del self._generations[generation]
        del self._manifests[generation]

    def close(self) -> None:
        self._closed = True

    def _get_generation(self, generation: str) -> dict[str, IndexRow]:
        try:
            return self._generations[generation]
        except KeyError as error:
            raise GenerationNotFoundError(generation) from error

    def _ensure_open(self) -> None:
        if self._closed:
            raise IndexClosedError("index is closed")


type IndexFactory = Callable[[], SemanticIndex]


def run_semantic_index_contract(factory: IndexFactory) -> None:
    index = factory()
    assert index.health().healthy
    assert index.active_generation is None

    old_alpha = row("alpha", (9.0, 9.0))
    alpha = row("alpha", (1.0, 0.0))
    beta = row("beta", (-1.0, 0.0), semantic_scope="dialogue")
    gamma = row("gamma", (0.0, 2.0), series_slug="discworld")
    final_rows = [alpha, beta, gamma]
    index.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=3,
        manifest_checksum=compute_manifest_checksum(final_rows),
    )

    index.upsert("generation-1", [old_alpha])
    index.upsert("generation-1", [alpha])
    index.upsert("generation-1", [beta, gamma])
    index.upsert("generation-1", [beta])  # idempotent
    index.delete_chunk_ids("generation-1", ["gamma", "missing"])
    index.upsert("generation-1", [gamma])
    with pytest.raises(IndexBatchSizeError):
        index.upsert("generation-1", final_rows)
    with pytest.raises(ValueError, match="generation"):
        index.upsert("generation-1", [row("wrong", (0.0, 0.0), generation="other")])
    with pytest.raises(IndexBatchSizeError):
        index.delete_chunk_ids("generation-1", ["alpha", "beta", "gamma"])

    with pytest.raises(GenerationNotActiveError):
        index.search("generation-1", (0.0, 0.0), limit=3)
    index.activate_generation("generation-1", expected_active_generation=None)
    assert index.active_generation == "generation-1"
    with pytest.raises(GenerationConflictError):
        index.upsert("generation-1", [alpha])
    with pytest.raises(GenerationConflictError):
        index.delete_chunk_ids("generation-1", ["alpha"])

    results = index.search("generation-1", (0.0, 0.0), limit=2)
    assert [item.chunk_id for item in results] == ["alpha", "beta"]
    assert all(item.generation == "generation-1" for item in results)
    assert all(item.identity == IDENTITY for item in results)
    assert [item.distance for item in results] == pytest.approx([1.0, 1.0])
    assert [
        item.chunk_id
        for item in index.search(
            "generation-1",
            (0.0, 0.0),
            limit=5,
            filters=SearchFilters(series_slug="earthsea", semantic_scope="dialogue"),
        )
    ] == ["beta"]

    second = row("delta", (0.0, 0.0), generation="generation-2")
    index.begin_rebuild(
        "generation-2",
        identity=IDENTITY,
        expected_count=1,
        manifest_checksum=compute_manifest_checksum([second]),
    )
    index.upsert("generation-2", [second])
    with pytest.raises(GenerationNotActiveError):
        index.search("generation-2", (0.0, 0.0), limit=1)
    with pytest.raises(GenerationConflictError):
        index.activate_generation("generation-2", expected_active_generation=None)
    index.cancel_generation("generation-2")
    with pytest.raises(GenerationNotFoundError):
        index.cancel_generation("generation-2")

    with pytest.raises(GenerationConflictError):
        index.cancel_generation("generation-1")
    index.close()
    assert not index.health().healthy
    with pytest.raises(IndexClosedError):
        _ = index.active_generation


def test_in_memory_fake_satisfies_contract() -> None:
    run_semantic_index_contract(MemorySemanticIndex)


def test_activation_rejects_wrong_count_or_checksum() -> None:
    index = MemorySemanticIndex()
    item = row("alpha", (1.0, 0.0))
    index.begin_rebuild(
        "generation-1",
        identity=IDENTITY,
        expected_count=2,
        manifest_checksum=compute_manifest_checksum([item]),
    )
    index.upsert("generation-1", [item])
    with pytest.raises(IndexManifestError):
        index.activate_generation("generation-1", expected_active_generation=None)

from __future__ import annotations

from hieronymus.rag_models import (
    RagChunkRecord,
    RagImportResult,
    RagSearchResult,
)


def rag_chunk_payload(chunk: RagChunkRecord | None) -> dict[str, object] | None:
    if chunk is None:
        return None
    return {
        "id": chunk.id,
        "source_id": chunk.source_id,
        "series_slug": chunk.series_slug,
        "source_ref": chunk.source_ref,
        "chunk_kind": chunk.chunk_kind,
        "text": chunk.text,
        "display_text": chunk.display_text,
        "location": chunk.location,
        "metadata": chunk.metadata,
        "language_tags": list(chunk.language_tags),
        "story_scopes": list(chunk.story_scopes),
        "semantic_tags": list(chunk.semantic_tags),
    }


def rag_hit_payload(hit: RagSearchResult) -> dict[str, object]:
    chunk = hit.chunk
    return {
        "source": "rag",
        "id": chunk.id,
        "title": chunk.title,
        "kind": chunk.kind,
        "text": chunk.text,
        "display_text": chunk.display_text,
        "source_ref": chunk.source_ref,
        "chunk_kind": chunk.chunk_kind,
        "location": chunk.location,
        "metadata": chunk.metadata,
        "language_tags": list(chunk.language_tags),
        "story_scopes": list(chunk.story_scopes),
        "semantic_tags": list(chunk.semantic_tags),
        "score": hit.score,
        "rank_reason": hit.reason,
    }


def rag_import_payload(result: RagImportResult) -> dict[str, object]:
    return {
        "source_id": result.source.id,
        "series_slug": result.source.series_slug,
        "source_ref": result.source.source_ref,
        "source_type": result.source.source_type,
        "content_type": result.source.content_type,
        "checksum": result.source.checksum,
        "metadata": result.source.metadata,
        "chunk_count": result.chunk_count,
        "skipped": result.skipped,
    }

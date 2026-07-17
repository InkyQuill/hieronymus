from hieronymus.rag_models import (
    RagChunkRecord,
    RagImportResult,
    RagSearchResult,
    RagSourceRecord,
)
from hieronymus.rag_payloads import rag_chunk_payload, rag_hit_payload, rag_import_payload


def _source() -> RagSourceRecord:
    return RagSourceRecord(
        id=1,
        series_slug="only-sense-online",
        source_ref="glossary.csv",
        source_type="glossary",
        content_type="csv",
        checksum="abc",
        metadata={"scope": "main"},
    )


def _chunk() -> RagChunkRecord:
    return RagChunkRecord(
        id=2,
        source_id=1,
        series_slug="only-sense-online",
        source_ref="glossary.csv",
        chunk_kind="glossary_entry",
        text="source: Sense\ntarget: Сенс",
        display_text="source: Sense\ntarget: Сенс",
        location="row 2",
        metadata={"source": "Sense", "target": "Сенс"},
        language_tags=("ja", "ru"),
        story_scopes=("book:5",),
        semantic_tags=("skill:name",),
    )


def test_rag_payloads_share_flat_serialization_contract() -> None:
    source = _source()
    chunk = _chunk()

    imported = rag_import_payload(RagImportResult(source=source, chunk_count=1, skipped=False))
    hit = rag_hit_payload(RagSearchResult(chunk=chunk, score=0.75, reason="rag glossary match"))

    assert "source" not in imported
    assert imported == {
        "source_id": 1,
        "series_slug": "only-sense-online",
        "source_ref": "glossary.csv",
        "source_type": "glossary",
        "content_type": "csv",
        "checksum": "abc",
        "metadata": {"scope": "main"},
        "normalized_path": "",
        "normalized_format": "",
        "chunk_count": 1,
        "skipped": False,
    }
    assert hit["source"] == "rag"
    assert hit["source_ref"] == "glossary.csv"
    assert hit["score"] == 0.75
    assert hit["rank_reason"] == "rag glossary match"
    assert rag_chunk_payload(None) is None
    assert rag_chunk_payload(chunk)["semantic_tags"] == ["skill:name"]

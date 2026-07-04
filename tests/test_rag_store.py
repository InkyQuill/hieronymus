from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.rag_models import RagChunkRecord, RagImportResult, RagSourceRecord


def test_rag_schema_is_created_idempotently(config: HieronymusConfig) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        apply_migration(conn, "global.sql")
        table_names = {
            row["name"]
            for row in conn.execute(
                """
                select name
                from sqlite_master
                where type in ('table', 'virtual')
                  and name like 'rag_%'
                """
            ).fetchall()
        }

    assert {
        "rag_sources",
        "rag_chunks",
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
        "rag_chunks_fts",
    } <= table_names


def test_rag_dataclasses_expose_payload_fields() -> None:
    source = RagSourceRecord(
        id=1,
        series_slug="oso",
        source_ref="glossary.csv",
        source_type="glossary",
        content_type="csv",
        checksum="abc",
        metadata={},
    )
    chunk = RagChunkRecord(
        id=2,
        source_id=source.id,
        series_slug="oso",
        source_ref=source.source_ref,
        chunk_kind="glossary_entry",
        text="Sense -> Сенс",
        display_text="Sense -> Сенс",
        location="row 2",
        metadata={"source": "Sense", "target": "Сенс"},
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )
    result = RagImportResult(source=source, chunk_count=1, skipped=False)

    assert result.source.source_ref == "glossary.csv"
    assert chunk.title == "glossary.csv row 2"
    assert chunk.kind == "glossary_entry"

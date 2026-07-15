import hashlib
from pathlib import Path

import pytest

from hieronymus.rag import load_rag_file
from hieronymus.rag_models import RagChunkRecord, RagImportResult, RagSourceRecord


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


def test_text_file_is_chunked_by_paragraph(tmp_path: Path) -> None:
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.\n\nCooking Talent appears here.", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.checksum == hashlib.sha256(path.read_bytes()).hexdigest()
    assert parsed.content_type == "txt"
    assert parsed.source_type == "text"
    assert [chunk.text for chunk in parsed.chunks] == [
        "Sense menu note.",
        "Cooking Talent appears here.",
    ]
    assert [chunk.location for chunk in parsed.chunks] == ["paragraph 1", "paragraph 2"]


def test_text_file_splits_oversized_paragraphs(tmp_path: Path) -> None:
    path = tmp_path / "chapter.txt"
    path.write_text(("Sense " * 230).strip(), encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert len(parsed.chunks) > 1
    assert all(len(chunk.text) <= 1200 for chunk in parsed.chunks)


def test_markdown_file_preserves_heading_location(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text(
        "# Glossary\n\nSense stays untranslated.\n\n## Skills\n\nEnchant is a skill.",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.content_type == "md"
    assert parsed.source_type == "markdown"
    assert [(chunk.location, chunk.text) for chunk in parsed.chunks] == [
        ("Glossary paragraph 1", "Sense stays untranslated."),
        ("Glossary > Skills paragraph 2", "Enchant is a skill."),
    ]


def test_markdown_file_splits_oversized_paragraphs(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text("# Glossary\n\n" + ("Sense " * 230).strip(), encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert len(parsed.chunks) > 1
    assert all(len(chunk.text) <= 1200 for chunk in parsed.chunks)


def test_markdown_split_paragraphs_do_not_shift_following_locations(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text(
        "# Glossary\n\n" + ("Sense " * 230).strip() + "\n\nNext paragraph.",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.chunks[-1].location == "Glossary paragraph 2"


def test_csv_file_turns_rows_into_glossary_chunks(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text(
        "source,target,category\nSense,Сенс,skill\nEnchant,Зачарование,skill\n",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.content_type == "csv"
    assert parsed.chunks[0].chunk_kind == "glossary_entry"
    assert parsed.chunks[0].location == "row 2"
    assert parsed.chunks[0].metadata == {
        "source": "Sense",
        "target": "Сенс",
        "category": "skill",
    }
    assert "Sense" in parsed.chunks[0].text
    assert "Сенс" in parsed.chunks[0].text


def test_csv_file_rejects_rows_with_extra_fields(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс,extra\n", encoding="utf-8")

    with pytest.raises(ValueError):
        load_rag_file(path, source_type="auto")


@pytest.mark.parametrize(
    ("header", "message"),
    [
        ("source,,target", "non-empty headers"),
        ("source, source ,target", "duplicate headers"),
    ],
)
def test_csv_file_rejects_invalid_headers(
    tmp_path: Path,
    header: str,
    message: str,
) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text(f"{header}\nSense,alias,Сенс\n", encoding="utf-8")

    with pytest.raises(ValueError, match=message):
        load_rag_file(path, source_type="auto")


def test_glossary_file_rejects_invalid_source_type(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс\n", encoding="utf-8")

    with pytest.raises(ValueError):
        load_rag_file(path, source_type="bad")


def test_json_file_accepts_list_of_glossary_entries(tmp_path: Path) -> None:
    path = tmp_path / "glossary.json"
    path.write_text(
        '[{"source": "Sense", "target": "Сенс", "note": "menu term"}]',
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.chunks[0].location == "entry 1"
    assert parsed.chunks[0].metadata["note"] == "menu term"


def test_yaml_file_accepts_mapping_entries(tmp_path: Path) -> None:
    path = tmp_path / "glossary.yaml"
    path.write_text("Sense:\n  target: Сенс\n  category: skill\n", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.source_type == "glossary"
    assert parsed.chunks[0].metadata == {
        "key": "Sense",
        "target": "Сенс",
        "category": "skill",
    }


def test_yaml_mapping_outer_key_overrides_nested_key(tmp_path: Path) -> None:
    path = tmp_path / "glossary.yaml"
    path.write_text(
        "Sense:\n  key: Wrong\n  target: Сенс\n",
        encoding="utf-8",
    )

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.chunks[0].metadata == {
        "key": "Sense",
        "target": "Сенс",
    }

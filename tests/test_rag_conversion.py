from __future__ import annotations

from pathlib import Path
from zipfile import ZipFile

import pytest
from pypdf import PdfWriter

from hieronymus.rag_conversion import normalize_rag_source


def test_text_and_markdown_sources_are_not_transformed(tmp_path: Path) -> None:
    source = tmp_path / "chapter.md"
    source.write_text("# Chapter\n\nText", encoding="utf-8")

    normalized = normalize_rag_source(source, tmp_path / "managed")

    assert normalized.path == source
    assert normalized.format == "markdown"
    assert normalized.original_path == source


def test_epub_is_rejected_with_actionable_error(tmp_path: Path) -> None:
    source = tmp_path / "book.epub"
    source.write_bytes(b"epub")

    with pytest.raises(ValueError, match="split EPUB"):
        normalize_rag_source(source, tmp_path / "managed")


def test_html_is_converted_to_managed_markdown(tmp_path: Path) -> None:
    source = tmp_path / "chapter.html"
    source.write_text("<h1>Chapter</h1><p>Important detail.</p>", encoding="utf-8")

    normalized = normalize_rag_source(source, tmp_path / "managed")

    assert normalized.path != source
    assert normalized.path.suffix == ".md"
    assert normalized.path.read_text(encoding="utf-8") == "# Chapter\n\nImportant detail.\n"
    assert normalized.original_path == source


def test_docx_is_converted_to_managed_markdown(tmp_path: Path) -> None:
    source = tmp_path / "chapter.docx"
    with ZipFile(source, "w") as archive:
        archive.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0"?><Types '
            'xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" '
            'ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/word/document.xml" '
            'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
            "</Types>",
        )
        archive.writestr(
            "_rels/.rels",
            '<?xml version="1.0"?><Relationships '
            'xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/'
            'officeDocument" '
            'Target="word/document.xml"/></Relationships>',
        )
        archive.writestr(
            "word/document.xml",
            '<?xml version="1.0"?><w:document '
            'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
            "<w:body><w:p><w:r><w:t>Important detail.</w:t></w:r></w:p></w:body></w:document>",
        )

    normalized = normalize_rag_source(source, tmp_path / "managed")

    assert "Important detail" in normalized.path.read_text(encoding="utf-8")


def test_pdf_without_extractable_text_is_rejected(tmp_path: Path) -> None:
    source = tmp_path / "scan.pdf"
    writer = PdfWriter()
    writer.add_blank_page(width=100, height=100)
    with source.open("wb") as file:
        writer.write(file)

    with pytest.raises(ValueError, match="no extractable text"):
        normalize_rag_source(source, tmp_path / "managed")

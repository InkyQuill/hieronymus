from __future__ import annotations

import hashlib
from dataclasses import dataclass
from pathlib import Path

import mammoth
from markdownify import markdownify
from pypdf import PdfReader


@dataclass(frozen=True)
class NormalizedRagSource:
    path: Path
    format: str
    original_path: Path


def normalize_rag_source(path: Path, managed_root: Path) -> NormalizedRagSource:
    source = Path(path)
    suffix = source.suffix.casefold()
    if suffix == ".epub":
        raise ValueError("EPUB is unsupported; split EPUB into chapters before RAG import")
    if suffix == ".txt":
        return NormalizedRagSource(path=source, format="text", original_path=source)
    if suffix in {".md", ".markdown"}:
        return NormalizedRagSource(path=source, format="markdown", original_path=source)
    if suffix in {".csv", ".tsv", ".json", ".yaml", ".yml"}:
        return NormalizedRagSource(path=source, format="glossary", original_path=source)
    if suffix == ".html":
        markdown = markdownify(source.read_text(encoding="utf-8"), heading_style="ATX")
        return _write_managed_markdown(source, managed_root, markdown)
    if suffix == ".docx":
        with source.open("rb") as file:
            result = mammoth.convert_to_markdown(file)
        return _write_managed_markdown(source, managed_root, result.value)
    if suffix == ".pdf":
        reader = PdfReader(source)
        markdown = "\n\n".join(
            text.strip() for page in reader.pages if (text := page.extract_text()).strip()
        )
        return _write_managed_markdown(source, managed_root, markdown)
    raise ValueError(f"unsupported RAG source extension: {source.suffix}")


def _write_managed_markdown(
    source: Path,
    managed_root: Path,
    markdown: str,
) -> NormalizedRagSource:
    normalized_text = markdown.strip() + "\n"
    if not normalized_text.strip():
        raise ValueError(f"source produced no extractable text: {source}")
    checksum = hashlib.sha256(source.read_bytes()).hexdigest()
    destination = managed_root / f"{checksum}.md"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(normalized_text, encoding="utf-8")
    return NormalizedRagSource(path=destination, format="markdown", original_path=source)

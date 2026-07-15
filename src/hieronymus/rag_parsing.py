from __future__ import annotations

import csv
import hashlib
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Literal

import yaml

from hieronymus.rag_models import RagChunkKind, RagSourceType

RagLoadSourceType = Literal["auto", "text", "glossary"]

MAX_RAG_CHUNK_CHARS = 1200

_MARKDOWN_HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
_SENTENCE_BOUNDARY_RE = re.compile(r"(?<=[.!?])\s+")
_GLOSSARY_EXTENSIONS = {".csv", ".tsv", ".json", ".yaml", ".yml"}
_TEXT_EXTENSIONS = {".txt", ".md"}


@dataclass(frozen=True)
class ParsedRagChunk:
    chunk_kind: RagChunkKind
    text: str
    display_text: str
    location: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class ParsedRagFile:
    path: Path
    source_type: RagSourceType
    content_type: str
    checksum: str
    chunks: tuple[ParsedRagChunk, ...]
    metadata: dict[str, object] = field(default_factory=dict)


def load_rag_file(path: Path, *, source_type: RagLoadSourceType | str) -> ParsedRagFile:
    if not path.exists():
        raise FileNotFoundError(path)
    if not path.is_file():
        raise ValueError(f"RAG source is not a file: {path}")

    raw_bytes = path.read_bytes()
    checksum = hashlib.sha256(raw_bytes).hexdigest()
    content_type = path.suffix.lower().removeprefix(".")
    if not content_type:
        raise ValueError(f"Unsupported RAG source extension: {path}")

    suffix = f".{content_type}"
    resolved_source_type = _resolve_source_type(suffix, source_type)

    match suffix:
        case ".txt":
            chunks = _parse_text(path)
        case ".md":
            chunks = _parse_markdown(path)
        case ".csv":
            chunks = _parse_delimited_glossary(path, delimiter=",")
        case ".tsv":
            chunks = _parse_delimited_glossary(path, delimiter="\t")
        case ".json":
            chunks = _parse_json_glossary(path)
        case ".yaml" | ".yml":
            chunks = _parse_yaml_glossary(path)
        case _:
            raise ValueError(f"Unsupported RAG source extension: {path.suffix}")

    if not chunks:
        raise ValueError(f"RAG source produced no chunks: {path}")

    return ParsedRagFile(
        path=path,
        source_type=resolved_source_type,
        content_type=content_type,
        checksum=checksum,
        chunks=tuple(chunks),
    )


def _resolve_source_type(suffix: str, source_type: RagLoadSourceType | str) -> RagSourceType:
    if suffix not in _TEXT_EXTENSIONS | _GLOSSARY_EXTENSIONS:
        raise ValueError(f"Unsupported RAG source extension: {suffix}")

    if source_type == "auto":
        if suffix == ".txt":
            return "text"
        if suffix == ".md":
            return "markdown"
        return "glossary"

    if source_type == "text":
        if suffix == ".txt":
            return "text"
        if suffix == ".md":
            return "markdown"
        raise ValueError(f"Text RAG sources do not support {suffix} files")

    if source_type == "glossary":
        if suffix in _GLOSSARY_EXTENSIONS:
            return "glossary"
        raise ValueError(f"Glossary RAG sources do not support {suffix} files")

    raise ValueError(f"Unsupported RAG source type: {source_type}")


def _parse_text(path: Path) -> list[ParsedRagChunk]:
    paragraphs = _paragraphs(path.read_text(encoding="utf-8"))
    chunks: list[ParsedRagChunk] = []
    for paragraph_index, paragraph in enumerate(paragraphs, start=1):
        for location, text in _located_chunk_texts(
            paragraph,
            base_location=f"paragraph {paragraph_index}",
        ):
            chunks.append(
                ParsedRagChunk(
                    chunk_kind="text",
                    text=text,
                    display_text=text,
                    location=location,
                )
            )
    return chunks


def _parse_markdown(path: Path) -> list[ParsedRagChunk]:
    chunks: list[ParsedRagChunk] = []
    heading_stack: list[tuple[int, str]] = []
    paragraph_lines: list[str] = []
    paragraph_count = 0

    def flush_paragraph() -> None:
        nonlocal paragraph_count, paragraph_lines
        paragraph = " ".join(line.strip() for line in paragraph_lines if line.strip()).strip()
        paragraph_lines = []
        if not paragraph:
            return

        heading_path = " > ".join(heading for _, heading in heading_stack)
        paragraph_count += 1
        if heading_path:
            location = f"{heading_path} paragraph {paragraph_count}"
        else:
            location = f"paragraph {paragraph_count}"
        for chunk_location, text in _located_chunk_texts(paragraph, base_location=location):
            chunks.append(
                ParsedRagChunk(
                    chunk_kind="markdown_section",
                    text=text,
                    display_text=text,
                    location=chunk_location,
                )
            )

    for line in path.read_text(encoding="utf-8").splitlines():
        heading_match = _MARKDOWN_HEADING_RE.match(line.strip())
        if heading_match:
            flush_paragraph()
            level = len(heading_match.group(1))
            heading = heading_match.group(2).strip()
            heading_stack = [
                (old_level, old_heading)
                for old_level, old_heading in heading_stack
                if old_level < level
            ]
            heading_stack.append((level, heading))
            continue

        if not line.strip():
            flush_paragraph()
            continue

        paragraph_lines.append(line)

    flush_paragraph()
    return chunks


def _parse_delimited_glossary(path: Path, *, delimiter: str) -> list[ParsedRagChunk]:
    chunks: list[ParsedRagChunk] = []
    with path.open(encoding="utf-8", newline="") as file:
        reader = csv.DictReader(file, delimiter=delimiter)
        headers = [header.strip() for header in reader.fieldnames or ()]
        if not headers or any(not header for header in headers):
            raise ValueError("Delimited glossary requires non-empty headers")
        if len(headers) != len(set(headers)):
            raise ValueError("Delimited glossary contains duplicate headers")
        reader.fieldnames = headers
        for row_number, row in enumerate(reader, start=2):
            if None in row:
                raise ValueError(f"Malformed delimited row {row_number}: extra fields")
            metadata = {
                str(key).strip(): value.strip()
                for key, value in row.items()
                if key is not None and value is not None and value.strip()
            }
            if not metadata:
                continue
            chunks.append(_glossary_chunk(metadata, location=f"row {row_number}"))
    return chunks


def _parse_json_glossary(path: Path) -> list[ParsedRagChunk]:
    data = json.loads(path.read_text(encoding="utf-8"))
    return _parse_structured_glossary(data)


def _parse_yaml_glossary(path: Path) -> list[ParsedRagChunk]:
    data = yaml.safe_load(path.read_text(encoding="utf-8"))
    return _parse_structured_glossary(data)


def _parse_structured_glossary(data: Any) -> list[ParsedRagChunk]:
    chunks: list[ParsedRagChunk] = []
    for index, metadata in enumerate(_glossary_entries(data), start=1):
        chunks.append(_glossary_chunk(metadata, location=f"entry {index}"))
    return chunks


def _glossary_entries(data: Any) -> list[dict[str, object]]:
    if isinstance(data, list):
        return [_metadata_from_value(item) for item in data]

    if isinstance(data, dict):
        entries: list[dict[str, object]] = []
        for key, value in data.items():
            metadata = _metadata_from_value(value)
            metadata = {**metadata, "key": str(key)}
            entries.append(metadata)
        return entries

    if data is None:
        return []

    raise ValueError("Glossary data must be a list or mapping")


def _metadata_from_value(value: Any) -> dict[str, object]:
    if isinstance(value, dict):
        return {
            str(key): nested_value
            for key, nested_value in value.items()
            if nested_value is not None
        }
    return {"value": value}


def _glossary_chunk(metadata: dict[str, object], *, location: str) -> ParsedRagChunk:
    text = _glossary_text(metadata)
    return ParsedRagChunk(
        chunk_kind="glossary_entry",
        text=text,
        display_text=text,
        location=location,
        metadata=metadata,
    )


def _glossary_text(metadata: dict[str, object]) -> str:
    return "\n".join(f"{key}: {value}" for key, value in metadata.items())


def _paragraphs(text: str) -> tuple[str, ...]:
    return tuple(
        " ".join(line.strip() for line in block.splitlines() if line.strip())
        for block in re.split(r"\n\s*\n", text)
        if block.strip()
    )


def _located_chunk_texts(text: str, *, base_location: str) -> tuple[tuple[str, str], ...]:
    parts = _split_chunk_text(text)
    if len(parts) == 1:
        return ((base_location, parts[0]),)
    return tuple(
        (f"{base_location} part {index}", part) for index, part in enumerate(parts, start=1)
    )


def _split_chunk_text(text: str) -> tuple[str, ...]:
    stripped = text.strip()
    if len(stripped) <= MAX_RAG_CHUNK_CHARS:
        return (stripped,)

    chunks: list[str] = []
    current = ""
    for segment in _sentence_segments(stripped):
        if len(segment) > MAX_RAG_CHUNK_CHARS:
            if current:
                chunks.append(current)
                current = ""
            chunks.extend(_word_chunks(segment))
            continue

        candidate = f"{current} {segment}".strip()
        if len(candidate) <= MAX_RAG_CHUNK_CHARS:
            current = candidate
        else:
            if current:
                chunks.append(current)
            current = segment

    if current:
        chunks.append(current)
    return tuple(chunks)


def _sentence_segments(text: str) -> tuple[str, ...]:
    return tuple(
        segment.strip() for segment in _SENTENCE_BOUNDARY_RE.split(text) if segment.strip()
    )


def _word_chunks(text: str) -> tuple[str, ...]:
    chunks: list[str] = []
    current = ""
    for word in text.split():
        if len(word) > MAX_RAG_CHUNK_CHARS:
            if current:
                chunks.append(current)
                current = ""
            chunks.extend(_hard_chunks(word))
            continue

        candidate = f"{current} {word}".strip()
        if len(candidate) <= MAX_RAG_CHUNK_CHARS:
            current = candidate
        else:
            if current:
                chunks.append(current)
            current = word

    if current:
        chunks.append(current)
    return tuple(chunks)


def _hard_chunks(text: str) -> tuple[str, ...]:
    return tuple(
        text[index : index + MAX_RAG_CHUNK_CHARS]
        for index in range(0, len(text), MAX_RAG_CHUNK_CHARS)
    )

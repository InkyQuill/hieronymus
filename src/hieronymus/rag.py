from __future__ import annotations

import csv
import hashlib
import json
import re
import sqlite3
from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

import yaml

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import search_expression
from hieronymus.db import apply_migration, connect
from hieronymus.rag_models import (
    RagChunkKind,
    RagChunkRecord,
    RagImportResult,
    RagSearchResult,
    RagSourceRecord,
    RagSourceType,
)

RagLoadSourceType = Literal["auto", "text", "glossary"]

MAX_RAG_CHUNK_CHARS = 1200
_MAX_RAG_SEARCH_LIMIT = 50

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


class RagStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def import_file(
        self,
        series_slug: str,
        path: Path,
        *,
        source_ref: str | None = None,
        source_type: RagLoadSourceType | str = "auto",
        language_tags: Iterable[str] = (),
        story_scopes: Iterable[str] = (),
        semantic_tags: Iterable[str] = (),
    ) -> RagImportResult:
        source_path = Path(path)
        clean_source_ref = source_ref or str(source_path)
        try:
            parsed = load_rag_file(source_path, source_type=source_type)
        except (ValueError, UnicodeDecodeError, yaml.YAMLError) as exc:
            raise ValueError(f"invalid RAG source: {exc}") from exc

        clean_language_tags = _clean_text_tuple(language_tags)
        clean_story_scopes = _clean_text_tuple(story_scopes)
        clean_semantic_tags = _clean_text_tuple(semantic_tags)

        with connect(self.config.database_path) as conn:
            existing = self._source_row(conn, series_slug, clean_source_ref)
            if existing is not None and existing["checksum"] == parsed.checksum:
                chunk_count = self._refresh_source_chunk_tags(
                    conn,
                    source_id=int(existing["id"]),
                    series_slug=series_slug,
                    language_tags=clean_language_tags,
                    story_scopes=clean_story_scopes,
                    semantic_tags=clean_semantic_tags,
                )
                conn.commit()
                return RagImportResult(
                    source=_source_from_row(existing),
                    chunk_count=chunk_count,
                    skipped=True,
                )

            if existing is not None:
                conn.execute(
                    """
                    delete from rag_sources
                    where id = ?
                      and series_slug = ?
                    """,
                    (int(existing["id"]), series_slug),
                )

            now = _now()
            cursor = conn.execute(
                """
                insert into rag_sources(
                  series_slug,
                  source_ref,
                  source_type,
                  content_type,
                  checksum,
                  metadata_json,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    series_slug,
                    clean_source_ref,
                    parsed.source_type,
                    parsed.content_type,
                    parsed.checksum,
                    _json_dumps(parsed.metadata),
                    now,
                    now,
                ),
            )
            source_id = int(cursor.lastrowid)
            for chunk in parsed.chunks:
                chunk_cursor = conn.execute(
                    """
                    insert into rag_chunks(
                      source_id,
                      series_slug,
                      chunk_kind,
                      text,
                      display_text,
                      location,
                      metadata_json,
                      created_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        source_id,
                        series_slug,
                        chunk.chunk_kind,
                        chunk.text,
                        chunk.display_text,
                        chunk.location,
                        _json_dumps(chunk.metadata),
                        now,
                    ),
                )
                chunk_id = int(chunk_cursor.lastrowid)
                _insert_text_values(
                    conn,
                    table="rag_chunk_language_tags",
                    value_column="language_tag",
                    chunk_id=chunk_id,
                    values=clean_language_tags,
                )
                _insert_text_values(
                    conn,
                    table="rag_chunk_story_scopes",
                    value_column="story_scope",
                    chunk_id=chunk_id,
                    values=clean_story_scopes,
                )
                _insert_text_values(
                    conn,
                    table="rag_chunk_semantic_tags",
                    value_column="semantic_tag",
                    chunk_id=chunk_id,
                    values=clean_semantic_tags,
                )
            conn.commit()

        return RagImportResult(
            source=RagSourceRecord(
                id=source_id,
                series_slug=series_slug,
                source_ref=clean_source_ref,
                source_type=parsed.source_type,
                content_type=parsed.content_type,
                checksum=parsed.checksum,
                metadata=parsed.metadata,
            ),
            chunk_count=len(parsed.chunks),
            skipped=False,
        )

    def search(
        self,
        series_slug: str,
        query: str,
        *,
        limit: int = 10,
    ) -> list[RagSearchResult]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        expression = search_expression(query)
        if not expression:
            return []

        bounded_limit = min(limit, _MAX_RAG_SEARCH_LIMIT)
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select
                  rag_chunks.*,
                  rag_sources.source_ref,
                  bm25(rag_chunks_fts) as rank
                from rag_chunks_fts
                join rag_chunks
                  on rag_chunks.id = rag_chunks_fts.rowid
                join rag_sources
                  on rag_sources.id = rag_chunks.source_id
                 and rag_sources.series_slug = rag_chunks.series_slug
                where rag_chunks_fts match ?
                  and rag_chunks.series_slug = ?
                order by rank, rag_chunks.id
                limit ?
                """,
                (expression, series_slug, bounded_limit),
            ).fetchall()

            return [
                RagSearchResult(
                    chunk=self._chunk_from_row(conn, row),
                    score=max(-float(row["rank"]), 0.000001),
                    reason=_reason_for_chunk_kind(row["chunk_kind"]),
                )
                for row in rows
            ]

    def _source_row(
        self,
        conn: sqlite3.Connection,
        series_slug: str,
        source_ref: str,
    ) -> sqlite3.Row | None:
        return conn.execute(
            """
            select *
            from rag_sources
            where series_slug = ?
              and source_ref = ?
            """,
            (series_slug, source_ref),
        ).fetchone()

    def _source_chunk_count(
        self,
        conn: sqlite3.Connection,
        *,
        source_id: int,
        series_slug: str,
    ) -> int:
        row = conn.execute(
            """
            select count(*) as count
            from rag_chunks
            where source_id = ?
              and series_slug = ?
            """,
            (source_id, series_slug),
        ).fetchone()
        return int(row["count"])

    def _refresh_source_chunk_tags(
        self,
        conn: sqlite3.Connection,
        *,
        source_id: int,
        series_slug: str,
        language_tags: Iterable[str],
        story_scopes: Iterable[str],
        semantic_tags: Iterable[str],
    ) -> int:
        rows = conn.execute(
            """
            select id
            from rag_chunks
            where source_id = ?
              and series_slug = ?
            order by id
            """,
            (source_id, series_slug),
        ).fetchall()
        chunk_ids = tuple(int(row["id"]) for row in rows)
        for chunk_id in chunk_ids:
            _replace_text_values(
                conn,
                table="rag_chunk_language_tags",
                value_column="language_tag",
                chunk_id=chunk_id,
                values=language_tags,
            )
            _replace_text_values(
                conn,
                table="rag_chunk_story_scopes",
                value_column="story_scope",
                chunk_id=chunk_id,
                values=story_scopes,
            )
            _replace_text_values(
                conn,
                table="rag_chunk_semantic_tags",
                value_column="semantic_tag",
                chunk_id=chunk_id,
                values=semantic_tags,
            )
        return len(chunk_ids)

    def _chunk_from_row(self, conn: sqlite3.Connection, row: sqlite3.Row) -> RagChunkRecord:
        chunk_id = int(row["id"])
        return RagChunkRecord(
            id=chunk_id,
            source_id=int(row["source_id"]),
            series_slug=row["series_slug"],
            source_ref=row["source_ref"],
            chunk_kind=row["chunk_kind"],
            text=row["text"],
            display_text=row["display_text"],
            location=row["location"],
            metadata=_json_loads_object(row["metadata_json"]),
            language_tags=_text_values(
                conn,
                table="rag_chunk_language_tags",
                value_column="language_tag",
                chunk_id=chunk_id,
            ),
            story_scopes=_text_values(
                conn,
                table="rag_chunk_story_scopes",
                value_column="story_scope",
                chunk_id=chunk_id,
            ),
            semantic_tags=_text_values(
                conn,
                table="rag_chunk_semantic_tags",
                value_column="semantic_tag",
                chunk_id=chunk_id,
            ),
        )


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
            metadata = {"key": str(key), **metadata}
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


def _source_from_row(row: sqlite3.Row) -> RagSourceRecord:
    return RagSourceRecord(
        id=int(row["id"]),
        series_slug=row["series_slug"],
        source_ref=row["source_ref"],
        source_type=row["source_type"],
        content_type=row["content_type"],
        checksum=row["checksum"],
        metadata=_json_loads_object(row["metadata_json"]),
    )


def _json_dumps(value: dict[str, object]) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True)


def _json_loads_object(value: str) -> dict[str, object]:
    data = json.loads(value)
    if not isinstance(data, dict):
        return {}
    return data


def _insert_text_values(
    conn: sqlite3.Connection,
    *,
    table: str,
    value_column: str,
    chunk_id: int,
    values: Iterable[str],
) -> None:
    if table not in {
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
    }:
        raise ValueError(f"unsupported RAG tag table: {table}")
    if value_column not in {"language_tag", "story_scope", "semantic_tag"}:
        raise ValueError(f"unsupported RAG tag column: {value_column}")

    conn.executemany(
        f"""
        insert into {table}(chunk_id, {value_column})
        values (?, ?)
        """,
        [(chunk_id, value) for value in values],
    )


def _replace_text_values(
    conn: sqlite3.Connection,
    *,
    table: str,
    value_column: str,
    chunk_id: int,
    values: Iterable[str],
) -> None:
    if table not in {
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
    }:
        raise ValueError(f"unsupported RAG tag table: {table}")

    conn.execute(
        f"""
        delete from {table}
        where chunk_id = ?
        """,
        (chunk_id,),
    )
    _insert_text_values(
        conn,
        table=table,
        value_column=value_column,
        chunk_id=chunk_id,
        values=values,
    )


def _text_values(
    conn: sqlite3.Connection,
    *,
    table: str,
    value_column: str,
    chunk_id: int,
) -> tuple[str, ...]:
    if table not in {
        "rag_chunk_language_tags",
        "rag_chunk_story_scopes",
        "rag_chunk_semantic_tags",
    }:
        raise ValueError(f"unsupported RAG tag table: {table}")
    if value_column not in {"language_tag", "story_scope", "semantic_tag"}:
        raise ValueError(f"unsupported RAG tag column: {value_column}")

    rows = conn.execute(
        f"""
        select {value_column}
        from {table}
        where chunk_id = ?
        order by {value_column}
        """,
        (chunk_id,),
    ).fetchall()
    return tuple(row[value_column] for row in rows)


def _reason_for_chunk_kind(chunk_kind: str) -> str:
    if chunk_kind == "glossary_entry":
        return "rag glossary match"
    if chunk_kind == "markdown_section":
        return "rag markdown section match"
    return "rag project text match"


def _now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def _clean_text_tuple(values: Iterable[str]) -> tuple[str, ...]:
    clean_values: list[str] = []
    seen: set[str] = set()
    for value in values:
        clean_value = value.strip()
        if not clean_value or clean_value in seen:
            continue
        seen.add(clean_value)
        clean_values.append(clean_value)
    return tuple(clean_values)

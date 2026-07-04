# RAG Pipelines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a first RAG retrieval lane for explicit project text and glossary files, merge it into recall, and expose CLI/MCP import and search commands.

**Architecture:** Add SQLite-backed RAG sources and chunks with FTS5 search, owned by a new `RagStore`. Extend `RecallResult` with a third `rag` source and make `RecallService` merge protected active rules, memory results, and RAG evidence through a budgeted dual-lane strategy.

**Tech Stack:** Python 3.12, SQLite FTS5, Click, FastMCP, pytest, Ruff, PyYAML for YAML glossary parsing.

---

## File Structure

- Create `src/hieronymus/rag_models.py`: dataclasses for source records, chunk records, import results, and search results.
- Create `src/hieronymus/rag.py`: file parsing, chunking, checksum handling, `RagStore.import_file`, `RagStore.search`, and payload helpers.
- Modify `src/hieronymus/migrations/global.sql`: add RAG source/chunk tables, side tables, and FTS table.
- Modify `src/hieronymus/memory_models.py`: allow `RecallResult.source == "rag"` and add RAG payload properties.
- Modify `src/hieronymus/recall.py`: call `RagStore.search`, protect active rule crystals, and budget/interleave memory plus RAG results.
- Modify `src/hieronymus/cli.py`: add `hiero rag import` and `hiero rag search`.
- Modify `src/hieronymus/mcp_server.py`: add `hieronymus_rag_import` and `hieronymus_rag_search`, and return RAG payloads from recall.
- Modify `pyproject.toml` and `uv.lock`: add `PyYAML`.
- Create `tests/test_rag_store.py`: schema, import, chunking, idempotency, replacement, and search coverage.
- Create `tests/test_rag_cli.py`: CLI import/search coverage.
- Create `tests/test_mcp_rag.py`: MCP import/search payload coverage.
- Modify `tests/test_recall_enriched_memory.py`: enriched RAG payload coverage.
- Create `tests/test_recall_rag.py`: mixed recall, protected rule ordering, and quota fallback coverage.

## Task 1: Add RAG Schema And Models

**Files:**
- Modify: `src/hieronymus/migrations/global.sql`
- Create: `src/hieronymus/rag_models.py`
- Test: `tests/test_rag_store.py`

- [x] **Step 1: Write the failing schema/model test**

Add this test file:

```python
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
```

- [x] **Step 2: Run the schema/model test and verify it fails**

Run: `uv run pytest tests/test_rag_store.py -q`

Expected: FAIL because `hieronymus.rag_models` does not exist and the schema tables do not exist.

- [x] **Step 3: Add the RAG tables to the global migration**

Append this SQL to `src/hieronymus/migrations/global.sql`:

```sql
create table if not exists rag_sources (
  id integer primary key,
  series_slug text not null references series(slug) on delete cascade,
  source_ref text not null,
  source_type text not null,
  content_type text not null,
  checksum text not null,
  metadata_json text not null default '{}',
  created_at text not null,
  updated_at text not null,
  unique(series_slug, source_ref)
);

create table if not exists rag_chunks (
  id integer primary key,
  source_id integer not null references rag_sources(id) on delete cascade,
  series_slug text not null references series(slug) on delete cascade,
  chunk_kind text not null,
  text text not null,
  display_text text not null,
  location text not null default '',
  metadata_json text not null default '{}',
  created_at text not null
);

create table if not exists rag_chunk_language_tags (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  language_tag text not null,
  primary key (chunk_id, language_tag)
);

create table if not exists rag_chunk_story_scopes (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  story_scope text not null,
  primary key (chunk_id, story_scope)
);

create table if not exists rag_chunk_semantic_tags (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  semantic_tag text not null,
  primary key (chunk_id, semantic_tag)
);

create virtual table if not exists rag_chunks_fts using fts5(
  text,
  display_text,
  location,
  content='rag_chunks',
  content_rowid='id'
);

create trigger if not exists rag_chunks_ai
after insert on rag_chunks
begin
  insert into rag_chunks_fts(rowid, text, display_text, location)
  values (new.id, new.text, new.display_text, new.location);
end;

create trigger if not exists rag_chunks_ad
after delete on rag_chunks
begin
  insert into rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  values ('delete', old.id, old.text, old.display_text, old.location);
end;

create trigger if not exists rag_chunks_au
after update on rag_chunks
begin
  insert into rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  values ('delete', old.id, old.text, old.display_text, old.location);
  insert into rag_chunks_fts(rowid, text, display_text, location)
  values (new.id, new.text, new.display_text, new.location);
end;
```

- [x] **Step 4: Add the model dataclasses**

Create `src/hieronymus/rag_models.py`:

```python
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal

RagSourceType = Literal["text", "markdown", "glossary"]
RagChunkKind = Literal["text", "markdown_section", "glossary_entry"]


@dataclass(frozen=True)
class RagSourceRecord:
    id: int
    series_slug: str
    source_ref: str
    source_type: RagSourceType
    content_type: str
    checksum: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class RagChunkRecord:
    id: int
    source_id: int
    series_slug: str
    source_ref: str
    chunk_kind: RagChunkKind
    text: str
    display_text: str
    location: str
    metadata: dict[str, object] = field(default_factory=dict)
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()

    @property
    def title(self) -> str:
        if self.location:
            return f"{self.source_ref} {self.location}"
        return self.source_ref

    @property
    def kind(self) -> str:
        return self.chunk_kind


@dataclass(frozen=True)
class RagSearchResult:
    chunk: RagChunkRecord
    score: float
    reason: str


@dataclass(frozen=True)
class RagImportResult:
    source: RagSourceRecord
    chunk_count: int
    skipped: bool
```

- [x] **Step 5: Run the task test and verify it passes**

Run: `uv run pytest tests/test_rag_store.py::test_rag_schema_is_created_idempotently tests/test_rag_store.py::test_rag_dataclasses_expose_payload_fields -q`

Expected: PASS.

- [x] **Step 6: Commit Task 1**

```bash
git add src/hieronymus/migrations/global.sql src/hieronymus/rag_models.py tests/test_rag_store.py
git commit -m "feat: add rag schema and models"
```

## Task 2: Add File Parsing And Chunking

**Files:**
- Create: `src/hieronymus/rag.py`
- Modify: `tests/test_rag_store.py`
- Modify: `pyproject.toml`
- Modify: `uv.lock`

- [x] **Step 1: Add the YAML dependency**

Run: `uv add pyyaml`

Expected: `pyproject.toml` contains `pyyaml` in `[project].dependencies`, and `uv.lock` is updated.

- [x] **Step 2: Write failing parser/chunker tests**

Append these tests to `tests/test_rag_store.py`:

```python
from pathlib import Path

from hieronymus.rag import load_rag_file


def test_text_file_is_chunked_by_paragraph(tmp_path: Path) -> None:
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.\n\nCooking Talent appears here.", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.content_type == "txt"
    assert parsed.source_type == "text"
    assert [chunk.text for chunk in parsed.chunks] == [
        "Sense menu note.",
        "Cooking Talent appears here.",
    ]
    assert [chunk.location for chunk in parsed.chunks] == ["paragraph 1", "paragraph 2"]


def test_markdown_file_preserves_heading_location(tmp_path: Path) -> None:
    path = tmp_path / "notes.md"
    path.write_text("# Glossary\n\nSense stays untranslated.\n\n## Skills\n\nEnchant is a skill.", encoding="utf-8")

    parsed = load_rag_file(path, source_type="auto")

    assert parsed.content_type == "md"
    assert parsed.source_type == "markdown"
    assert [(chunk.location, chunk.text) for chunk in parsed.chunks] == [
        ("Glossary paragraph 1", "Sense stays untranslated."),
        ("Glossary > Skills paragraph 2", "Enchant is a skill."),
    ]


def test_csv_file_turns_rows_into_glossary_chunks(tmp_path: Path) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text("source,target,category\nSense,Сенс,skill\nEnchant,Зачарование,skill\n", encoding="utf-8")

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


def test_json_file_accepts_list_of_glossary_entries(tmp_path: Path) -> None:
    path = tmp_path / "glossary.json"
    path.write_text('[{"source": "Sense", "target": "Сенс", "note": "menu term"}]', encoding="utf-8")

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
```

- [x] **Step 3: Run parser tests and verify they fail**

Run: `uv run pytest tests/test_rag_store.py -q`

Expected: FAIL because `hieronymus.rag` and `load_rag_file` do not exist.

- [x] **Step 4: Add parser and chunker code**

Create `src/hieronymus/rag.py` with these definitions first:

```python
from __future__ import annotations

import csv
import hashlib
import json
import re
from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

import yaml

from hieronymus.rag_models import RagChunkKind, RagSourceType

_SUPPORTED_CONTENT_TYPES = frozenset({"txt", "md", "csv", "tsv", "json", "yaml", "yml"})
_TEXT_CONTENT_TYPES = frozenset({"txt"})
_MARKDOWN_CONTENT_TYPES = frozenset({"md"})
_GLOSSARY_CONTENT_TYPES = frozenset({"csv", "tsv", "json", "yaml", "yml"})
_SENTENCE_BOUNDARY_PATTERN = re.compile(r"(?<=[.!?。！？])\s+")
_MAX_TEXT_CHARS = 1200


@dataclass(frozen=True)
class ParsedRagChunk:
    chunk_kind: RagChunkKind
    text: str
    display_text: str
    location: str
    metadata: dict[str, object] = field(default_factory=dict)


@dataclass(frozen=True)
class ParsedRagFile:
    source_type: RagSourceType
    content_type: str
    checksum: str
    chunks: tuple[ParsedRagChunk, ...]


def load_rag_file(path: Path, *, source_type: Literal["auto", "text", "glossary"]) -> ParsedRagFile:
    if not path.exists():
        raise FileNotFoundError(f"RAG source file not found: {path}")
    if not path.is_file():
        raise ValueError(f"RAG source path is not a file: {path}")

    content_type = _content_type(path)
    raw = path.read_bytes()
    checksum = hashlib.sha256(raw).hexdigest()
    text = raw.decode("utf-8")
    resolved_source_type = _resolve_source_type(content_type, source_type)

    if resolved_source_type == "text":
        chunks = _text_chunks(text)
    elif resolved_source_type == "markdown":
        chunks = _markdown_chunks(text)
    else:
        chunks = _glossary_chunks(content_type, text)

    if not chunks:
        raise ValueError(f"RAG source produced no chunks: {path}")

    return ParsedRagFile(
        source_type=resolved_source_type,
        content_type=content_type,
        checksum=checksum,
        chunks=tuple(chunks),
    )


def _content_type(path: Path) -> str:
    extension = path.suffix.lower().lstrip(".")
    if extension not in _SUPPORTED_CONTENT_TYPES:
        raise ValueError(f"unsupported RAG source extension: {path.suffix}")
    return extension


def _resolve_source_type(
    content_type: str,
    source_type: Literal["auto", "text", "glossary"],
) -> RagSourceType:
    if source_type == "text":
        if content_type in _GLOSSARY_CONTENT_TYPES:
            raise ValueError(f"{content_type} files must be imported as glossary sources")
        return "markdown" if content_type in _MARKDOWN_CONTENT_TYPES else "text"
    if source_type == "glossary":
        if content_type not in _GLOSSARY_CONTENT_TYPES:
            raise ValueError(f"{content_type} files cannot be imported as glossary sources")
        return "glossary"
    if content_type in _TEXT_CONTENT_TYPES:
        return "text"
    if content_type in _MARKDOWN_CONTENT_TYPES:
        return "markdown"
    return "glossary"


def _text_chunks(text: str) -> list[ParsedRagChunk]:
    paragraphs = [part.strip() for part in re.split(r"\n\s*\n", text) if part.strip()]
    chunks: list[ParsedRagChunk] = []
    paragraph_index = 1
    for paragraph in paragraphs:
        for piece in _bounded_text(paragraph):
            chunks.append(
                ParsedRagChunk(
                    chunk_kind="text",
                    text=piece,
                    display_text=piece,
                    location=f"paragraph {paragraph_index}",
                )
            )
        paragraph_index += 1
    return chunks


def _markdown_chunks(text: str) -> list[ParsedRagChunk]:
    chunks: list[ParsedRagChunk] = []
    headings: list[str] = []
    paragraph_lines: list[str] = []
    paragraph_index = 1

    def flush() -> None:
        nonlocal paragraph_lines, paragraph_index
        paragraph = " ".join(line.strip() for line in paragraph_lines if line.strip()).strip()
        paragraph_lines = []
        if not paragraph:
            return
        prefix = " > ".join(headings)
        location_prefix = f"{prefix} " if prefix else ""
        for piece in _bounded_text(paragraph):
            chunks.append(
                ParsedRagChunk(
                    chunk_kind="markdown_section",
                    text=piece,
                    display_text=piece,
                    location=f"{location_prefix}paragraph {paragraph_index}",
                )
            )
        paragraph_index += 1

    for line in text.splitlines():
        stripped = line.strip()
        if not stripped:
            flush()
            continue
        if stripped.startswith("#"):
            flush()
            level = len(stripped) - len(stripped.lstrip("#"))
            heading = stripped[level:].strip()
            if heading:
                headings = headings[: max(level - 1, 0)]
                headings.append(heading)
            continue
        paragraph_lines.append(stripped)
    flush()
    return chunks


def _bounded_text(text: str) -> list[str]:
    if len(text) <= _MAX_TEXT_CHARS:
        return [text]
    pieces: list[str] = []
    current = ""
    for sentence in _SENTENCE_BOUNDARY_PATTERN.split(text):
        sentence = sentence.strip()
        if not sentence:
            continue
        candidate = sentence if not current else f"{current} {sentence}"
        if len(candidate) > _MAX_TEXT_CHARS and current:
            pieces.append(current)
            current = sentence
        else:
            current = candidate
    if current:
        pieces.append(current)
    return pieces or [text[:_MAX_TEXT_CHARS]]


def _glossary_chunks(content_type: str, text: str) -> list[ParsedRagChunk]:
    if content_type in {"csv", "tsv"}:
        delimiter = "\t" if content_type == "tsv" else ","
        rows = csv.DictReader(text.splitlines(), delimiter=delimiter)
        return [
            _glossary_chunk({key: value for key, value in row.items() if key and value}, f"row {index}")
            for index, row in enumerate(rows, start=2)
        ]

    if content_type == "json":
        payload = json.loads(text)
    else:
        payload = yaml.safe_load(text)
    entries = _glossary_entries(payload)
    return [_glossary_chunk(entry, f"entry {index}") for index, entry in enumerate(entries, start=1)]


def _glossary_entries(payload: object) -> list[dict[str, object]]:
    if isinstance(payload, list):
        return [_dict_entry(item) for item in payload]
    if isinstance(payload, dict):
        entries: list[dict[str, object]] = []
        for key, value in payload.items():
            entry = _dict_entry(value)
            entry = {"key": str(key), **entry}
            entries.append(entry)
        return entries
    raise ValueError("glossary payload must be a list or mapping")


def _dict_entry(value: object) -> dict[str, object]:
    if isinstance(value, dict):
        return {str(key): item for key, item in value.items() if item not in (None, "")}
    return {"value": str(value)}


def _glossary_chunk(entry: dict[str, object], location: str) -> ParsedRagChunk:
    parts = [f"{key}: {value}" for key, value in entry.items()]
    text = "\n".join(parts)
    return ParsedRagChunk(
        chunk_kind="glossary_entry",
        text=text,
        display_text=text,
        location=location,
        metadata=entry,
    )


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _clean_text_tuple(values: Iterable[str]) -> tuple[str, ...]:
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        item = value.strip()
        if not item or item in seen:
            continue
        seen.add(item)
        normalized.append(item)
    return tuple(normalized)
```

- [x] **Step 5: Run parser tests and verify they pass**

Run: `uv run pytest tests/test_rag_store.py -q`

Expected: PASS for schema/model/parser tests.

- [x] **Step 6: Commit Task 2**

```bash
git add pyproject.toml uv.lock src/hieronymus/rag.py tests/test_rag_store.py
git commit -m "feat: parse rag source files"
```

## Task 3: Implement RagStore Import, Replacement, And Search

**Files:**
- Modify: `src/hieronymus/rag.py`
- Modify: `tests/test_rag_store.py`

- [x] **Step 1: Write failing RagStore tests**

Append these tests to `tests/test_rag_store.py`:

```python
import pytest

from hieronymus.rag import RagStore
from hieronymus.registry import Registry


def _series(config: HieronymusConfig) -> str:
    return Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    ).slug


def test_import_file_indexes_chunks_and_searches(config: HieronymusConfig, tmp_path: Path) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.\n\nCooking Talent appears here.", encoding="utf-8")

    result = RagStore(config).import_file(
        series_slug,
        path,
        source_ref="chapter-5.txt",
        source_type="auto",
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("chapter:source",),
    )
    hits = RagStore(config).search(series_slug, "Cooking Talent", limit=5)

    assert result.chunk_count == 2
    assert result.skipped is False
    assert [hit.chunk.text for hit in hits] == ["Cooking Talent appears here."]
    assert hits[0].reason == "rag project text match"
    assert hits[0].chunk.source_ref == "chapter-5.txt"
    assert hits[0].chunk.language_tags == ("ja", "ru")
    assert hits[0].chunk.story_scopes == ("book:5/chapter:5",)
    assert hits[0].chunk.semantic_tags == ("chapter:source",)


def test_import_file_skips_unchanged_checksum(config: HieronymusConfig, tmp_path: Path) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    path.write_text("Sense menu note.", encoding="utf-8")
    store = RagStore(config)

    first = store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")
    second = store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    assert first.skipped is False
    assert second.skipped is True
    assert second.chunk_count == first.chunk_count


def test_changed_import_replaces_old_chunks(config: HieronymusConfig, tmp_path: Path) -> None:
    series_slug = _series(config)
    path = tmp_path / "chapter.txt"
    store = RagStore(config)
    path.write_text("Old Sense note.", encoding="utf-8")
    store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    path.write_text("New Cooking Talent note.", encoding="utf-8")
    store.import_file(series_slug, path, source_ref="chapter.txt", source_type="auto")

    assert store.search(series_slug, "Old", limit=5) == []
    assert [hit.chunk.text for hit in store.search(series_slug, "Cooking Talent", limit=5)] == [
        "New Cooking Talent note."
    ]


def test_failed_changed_import_preserves_old_chunks(config: HieronymusConfig, tmp_path: Path) -> None:
    series_slug = _series(config)
    good_path = tmp_path / "glossary.json"
    store = RagStore(config)
    good_path.write_text('[{"source": "Sense", "target": "Сенс"}]', encoding="utf-8")
    store.import_file(series_slug, good_path, source_ref="glossary.json", source_type="auto")

    good_path.write_text("{broken json", encoding="utf-8")
    with pytest.raises(ValueError, match="invalid RAG source"):
        store.import_file(series_slug, good_path, source_ref="glossary.json", source_type="auto")

    assert [hit.chunk.metadata["source"] for hit in store.search(series_slug, "Sense", limit=5)] == [
        "Sense"
    ]


def test_glossary_hits_get_glossary_reason(config: HieronymusConfig, tmp_path: Path) -> None:
    series_slug = _series(config)
    path = tmp_path / "glossary.csv"
    path.write_text("source,target\nSense,Сенс\n", encoding="utf-8")

    RagStore(config).import_file(series_slug, path, source_ref="glossary.csv", source_type="auto")
    hits = RagStore(config).search(series_slug, "Sense", limit=5)

    assert hits[0].reason == "rag glossary match"
    assert hits[0].chunk.chunk_kind == "glossary_entry"
    assert hits[0].score > 0
```

- [x] **Step 2: Run RagStore tests and verify they fail**

Run: `uv run pytest tests/test_rag_store.py -q`

Expected: FAIL because `RagStore` is not implemented.

- [x] **Step 3: Add `RagStore` row hydration and import/search methods**

Append this implementation to `src/hieronymus/rag.py`:

```python
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.rag_models import RagChunkRecord, RagImportResult, RagSearchResult, RagSourceRecord

_RAG_METADATA_BOOST = 0.10
_RAG_GLOSSARY_BOOST = 0.25
_MAX_RAG_SEARCH_LIMIT = 50


def _rag_search_score(raw_bm25: float, glossary_boost: float) -> float:
    return max(-raw_bm25, 0.0) + glossary_boost


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
        source_type: Literal["auto", "text", "glossary"] = "auto",
        language_tags: Iterable[str] = (),
        story_scopes: Iterable[str] = (),
        semantic_tags: Iterable[str] = (),
    ) -> RagImportResult:
        ref = source_ref or str(path)
        try:
            parsed = load_rag_file(path, source_type=source_type)
        except (OSError, UnicodeDecodeError, json.JSONDecodeError, yaml.YAMLError, ValueError) as error:
            raise ValueError(f"invalid RAG source {ref}: {error}") from error

        clean_language_tags = _clean_text_tuple(language_tags)
        clean_story_scopes = _clean_text_tuple(story_scopes)
        clean_semantic_tags = _clean_text_tuple(semantic_tags)
        now = _now()

        with connect(self.config.database_path) as conn:
            existing = conn.execute(
                """
                select *
                from rag_sources
                where series_slug = ?
                  and source_ref = ?
                """,
                (series_slug, ref),
            ).fetchone()
            if existing is not None and existing["checksum"] == parsed.checksum:
                source = _source_from_row(existing)
                return RagImportResult(
                    source=source,
                    chunk_count=_chunk_count(conn, source.id),
                    skipped=True,
                )

            if existing is None:
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
                    values (?, ?, ?, ?, ?, '{}', ?, ?)
                    """,
                    (series_slug, ref, parsed.source_type, parsed.content_type, parsed.checksum, now, now),
                )
                source_id = int(cursor.lastrowid)
            else:
                source_id = int(existing["id"])
                conn.execute("delete from rag_chunks where source_id = ?", (source_id,))
                conn.execute(
                    """
                    update rag_sources
                    set source_type = ?,
                        content_type = ?,
                        checksum = ?,
                        updated_at = ?
                    where id = ?
                    """,
                    (parsed.source_type, parsed.content_type, parsed.checksum, now, source_id),
                )

            for chunk in parsed.chunks:
                cursor = conn.execute(
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
                        json.dumps(chunk.metadata, ensure_ascii=False, sort_keys=True),
                        now,
                    ),
                )
                chunk_id = int(cursor.lastrowid)
                _insert_text_values(conn, "rag_chunk_language_tags", "language_tag", chunk_id, clean_language_tags)
                _insert_text_values(conn, "rag_chunk_story_scopes", "story_scope", chunk_id, clean_story_scopes)
                _insert_text_values(conn, "rag_chunk_semantic_tags", "semantic_tag", chunk_id, clean_semantic_tags)

            source_row = conn.execute("select * from rag_sources where id = ?", (source_id,)).fetchone()
            conn.commit()

        assert source_row is not None
        return RagImportResult(
            source=_source_from_row(source_row),
            chunk_count=len(parsed.chunks),
            skipped=False,
        )

    def search(self, series_slug: str, query: str, *, limit: int = 10) -> list[RagSearchResult]:
        if limit < 1:
            raise ValueError("limit must be at least 1")
        expression = _search_expression(query)
        if not expression:
            return []
        bounded_limit = min(limit, _MAX_RAG_SEARCH_LIMIT)
        with connect(self.config.database_path) as conn:
            conn.create_function("rag_search_score", 2, _rag_search_score, deterministic=True)
            rows = conn.execute(
                """
                select
                  rag_chunks.*,
                  rag_sources.source_ref,
                  rag_search_score(
                    bm25(rag_chunks_fts),
                    case when rag_chunks.chunk_kind = 'glossary_entry' then ? else 0 end
                  ) as search_score
                from rag_chunks_fts
                join rag_chunks on rag_chunks.id = rag_chunks_fts.rowid
                join rag_sources on rag_sources.id = rag_chunks.source_id
                where rag_chunks_fts match ?
                  and rag_chunks.series_slug = ?
                order by search_score desc, rag_chunks.id
                limit ?
                """,
                (_RAG_GLOSSARY_BOOST, expression, series_slug, bounded_limit),
            ).fetchall()
            chunks = _hydrate_chunks(conn, rows)
        return [
            RagSearchResult(
                chunk=chunk,
                score=float(row["search_score"]),
                reason=_rank_reason(chunk),
            )
            for chunk, row in zip(chunks, rows, strict=True)
        ]


def _search_expression(query: str) -> str:
    tokens = [token for token in re.findall(r"\w+", query) if token.casefold() not in {"and", "or", "not", "near"}]
    return " ".join(f'"{token}"' for token in tokens)


def _source_from_row(row) -> RagSourceRecord:
    return RagSourceRecord(
        id=int(row["id"]),
        series_slug=row["series_slug"],
        source_ref=row["source_ref"],
        source_type=row["source_type"],
        content_type=row["content_type"],
        checksum=row["checksum"],
        metadata=json.loads(row["metadata_json"] or "{}"),
    )


def _chunk_count(conn, source_id: int) -> int:
    return int(conn.execute("select count(*) from rag_chunks where source_id = ?", (source_id,)).fetchone()[0])


def _insert_text_values(conn, table: str, column: str, chunk_id: int, values: tuple[str, ...]) -> None:
    for value in values:
        conn.execute(
            f"insert into {table}(chunk_id, {column}) values (?, ?)",
            (chunk_id, value),
        )


def _hydrate_chunks(conn, rows) -> list[RagChunkRecord]:
    chunk_ids = [int(row["id"]) for row in rows]
    language_tags = _chunk_text_map(conn, chunk_ids, "rag_chunk_language_tags", "language_tag")
    story_scopes = _chunk_text_map(conn, chunk_ids, "rag_chunk_story_scopes", "story_scope")
    semantic_tags = _chunk_text_map(conn, chunk_ids, "rag_chunk_semantic_tags", "semantic_tag")
    return [
        RagChunkRecord(
            id=int(row["id"]),
            source_id=int(row["source_id"]),
            series_slug=row["series_slug"],
            source_ref=row["source_ref"],
            chunk_kind=row["chunk_kind"],
            text=row["text"],
            display_text=row["display_text"],
            location=row["location"],
            metadata=json.loads(row["metadata_json"] or "{}"),
            language_tags=language_tags.get(int(row["id"]), ()),
            story_scopes=story_scopes.get(int(row["id"]), ()),
            semantic_tags=semantic_tags.get(int(row["id"]), ()),
        )
        for row in rows
    ]


def _chunk_text_map(conn, chunk_ids: list[int], table: str, value_column: str) -> dict[int, tuple[str, ...]]:
    if not chunk_ids:
        return {}
    placeholders = ", ".join("?" for _ in chunk_ids)
    rows = conn.execute(
        f"""
        select chunk_id, {value_column} as value
        from {table}
        where chunk_id in ({placeholders})
        order by chunk_id, value
        """,
        chunk_ids,
    ).fetchall()
    values: dict[int, list[str]] = {}
    for row in rows:
        values.setdefault(int(row["chunk_id"]), []).append(row["value"])
    return {key: tuple(items) for key, items in values.items()}


def _rank_reason(chunk: RagChunkRecord) -> str:
    if chunk.chunk_kind == "glossary_entry":
        return "rag glossary match"
    if chunk.chunk_kind == "markdown_section":
        return "rag markdown section match"
    return "rag project text match"
```

- [x] **Step 4: Run RagStore tests and verify import replacement is atomic**

Run: `uv run pytest tests/test_rag_store.py -q`

Expected: PASS. The failed-import test proves parsing happens before any replacement write and the previous indexed version remains available.

- [x] **Step 5: Commit Task 3**

```bash
git add src/hieronymus/rag.py tests/test_rag_store.py
git commit -m "feat: index and search rag sources"
```

## Task 4: Add CLI And MCP RAG Interfaces

**Files:**
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/mcp_server.py`
- Create: `tests/test_rag_cli.py`
- Create: `tests/test_mcp_rag.py`

- [x] **Step 1: Write failing CLI tests**

Create `tests/test_rag_cli.py`:

```python
import json
from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus.config import HieronymusConfig
from hieronymus.registry import Registry


def _series(data_root: Path) -> None:
    Registry(HieronymusConfig(data_root=data_root)).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )


def test_rag_import_and_search_json(tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    _series(data_root)
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Сенс\n", encoding="utf-8")
    runner = CliRunner()

    import_result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "rag",
            "import",
            "only-sense-online",
            str(source),
            "--source-ref",
            "glossary.csv",
            "--json",
        ],
    )
    search_result = runner.invoke(
        main,
        [
            "--data-root",
            str(data_root),
            "rag",
            "search",
            "only-sense-online",
            "Sense",
            "--json",
        ],
    )

    assert import_result.exit_code == 0
    assert json.loads(import_result.output)["chunk_count"] == 1
    assert search_result.exit_code == 0
    payload = json.loads(search_result.output)
    assert payload[0]["source_ref"] == "glossary.csv"
    assert payload[0]["rank_reason"] == "rag glossary match"
    assert payload[0]["metadata"]["target"] == "Сенс"


def test_rag_import_rejects_unknown_series(tmp_path: Path) -> None:
    source = tmp_path / "chapter.txt"
    source.write_text("Sense note.", encoding="utf-8")

    result = CliRunner().invoke(
        main,
        [
            "--data-root",
            str(tmp_path / "hieronymus"),
            "rag",
            "import",
            "missing",
            str(source),
        ],
    )

    assert result.exit_code == 1
    assert "unknown series: missing" in result.output
```

- [x] **Step 2: Write failing MCP tests**

Create `tests/test_mcp_rag.py`:

```python
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus import mcp_server
from hieronymus.registry import Registry


def test_mcp_rag_import_and_search(monkeypatch, tmp_path: Path) -> None:
    data_root = tmp_path / "hieronymus"
    config = HieronymusConfig(data_root=data_root)
    Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    source = tmp_path / "chapter.txt"
    source.write_text("Cooking Talent appears here.", encoding="utf-8")
    monkeypatch.setattr(mcp_server, "load_config", lambda: config)

    imported = mcp_server.hieronymus_rag_import(
        "only-sense-online",
        str(source),
        source_ref="chapter.txt",
    )
    hits = mcp_server.hieronymus_rag_search("only-sense-online", "Cooking Talent")

    assert imported["chunk_count"] == 1
    assert imported["skipped"] is False
    assert hits[0]["source_ref"] == "chapter.txt"
    assert hits[0]["rank_reason"] == "rag project text match"
```

- [x] **Step 3: Run interface tests and verify they fail**

Run: `uv run pytest tests/test_rag_cli.py tests/test_mcp_rag.py -q`

Expected: FAIL because CLI/MCP RAG interfaces do not exist.

- [x] **Step 4: Add payload helper and CLI group**

In `src/hieronymus/cli.py`, import `RagStore`:

```python
from hieronymus.rag import RagStore
```

Add this helper near other payload helpers:

```python
def _rag_hit_payload(hit) -> dict[str, object]:
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
```

Add this group before `init-series`:

```python
@main.group("rag")
def rag_group() -> None:
    """Import and search project RAG sources."""


@rag_group.command("import")
@click.argument("series_slug")
@click.argument("path", type=click.Path(exists=True, dir_okay=False, readable=True, path_type=Path))
@click.option("--type", "source_type", type=click.Choice(["auto", "text", "glossary"]), default="auto")
@click.option("--source-ref", default=None)
@click.option("--language-tag", "language_tags", multiple=True)
@click.option("--story-scope", "story_scopes", multiple=True)
@click.option("--semantic-tag", "semantic_tags", multiple=True)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def rag_import(
    ctx: click.Context,
    series_slug: str,
    path: Path,
    source_type: str,
    source_ref: str | None,
    language_tags: tuple[str, ...],
    story_scopes: tuple[str, ...],
    semantic_tags: tuple[str, ...],
    json_output: bool,
) -> None:
    try:
        Registry(ctx.obj["config"]).get_series(series_slug)
        result = RagStore(ctx.obj["config"]).import_file(
            series_slug,
            path,
            source_ref=source_ref,
            source_type=source_type,
            language_tags=language_tags,
            story_scopes=story_scopes,
            semantic_tags=semantic_tags,
        )
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    payload = {
        "source_id": result.source.id,
        "series_slug": result.source.series_slug,
        "source_ref": result.source.source_ref,
        "source_type": result.source.source_type,
        "content_type": result.source.content_type,
        "chunk_count": result.chunk_count,
        "skipped": result.skipped,
    }
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line=(
            f"RAG source {result.source.source_ref} unchanged"
            if result.skipped
            else f"RAG source {result.source.source_ref} imported with {result.chunk_count} chunk(s)"
        ),
    )


@rag_group.command("search")
@click.argument("series_slug")
@click.argument("query")
@click.option("--limit", default=10, type=int)
@click.option("--json", "json_output", is_flag=True)
@click.pass_context
def rag_search(
    ctx: click.Context,
    series_slug: str,
    query: str,
    limit: int,
    json_output: bool,
) -> None:
    try:
        Registry(ctx.obj["config"]).get_series(series_slug)
        hits = RagStore(ctx.obj["config"]).search(series_slug, query, limit=limit)
    except (KeyError, ValueError) as error:
        _raise_click_error(error)
    payload = [_rag_hit_payload(hit) for hit in hits]
    _echo_json_or_line(
        payload,
        json_output=json_output,
        line="No RAG matches." if not payload else f"{len(payload)} RAG match(es).",
    )
```

- [x] **Step 5: Add MCP tools**

In `src/hieronymus/mcp_server.py`, import `Path` and `RagStore`:

```python
from pathlib import Path
from hieronymus.rag import RagStore
```

Add this helper near `_recall_payload`:

```python
def _rag_hit_payload(hit) -> dict[str, Any]:
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
```

Add these tools before `hieronymus_recall`:

```python
@server.tool()
def hieronymus_rag_import(
    series_slug: str,
    path: str,
    source_ref: str | None = None,
    source_type: str = "auto",
    language_tags: list[str] | None = None,
    story_scopes: list[str] | None = None,
    semantic_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Import an explicit project text or glossary file into the RAG corpus."""
    config, _series = _series_context(series_slug)
    result = RagStore(config).import_file(
        series_slug,
        Path(path),
        source_ref=source_ref,
        source_type=source_type,
        language_tags=language_tags or (),
        story_scopes=story_scopes or (),
        semantic_tags=semantic_tags or (),
    )
    return {
        "source_id": result.source.id,
        "series_slug": result.source.series_slug,
        "source_ref": result.source.source_ref,
        "source_type": result.source.source_type,
        "content_type": result.source.content_type,
        "chunk_count": result.chunk_count,
        "skipped": result.skipped,
    }


@server.tool()
def hieronymus_rag_search(
    series_slug: str,
    query: str,
    limit: int = 10,
) -> list[dict[str, Any]]:
    """Search project text and glossary RAG evidence for a series."""
    config, _series = _series_context(series_slug)
    return [_rag_hit_payload(hit) for hit in RagStore(config).search(series_slug, query, limit=limit)]
```

- [x] **Step 6: Run interface tests**

Run: `uv run pytest tests/test_rag_cli.py tests/test_mcp_rag.py -q`

Expected: PASS.

- [x] **Step 7: Commit Task 4**

```bash
git add src/hieronymus/cli.py src/hieronymus/mcp_server.py tests/test_rag_cli.py tests/test_mcp_rag.py
git commit -m "feat: expose rag import and search"
```

## Task 5: Extend RecallResult For RAG Payloads

**Files:**
- Modify: `src/hieronymus/memory_models.py`
- Modify: `tests/test_recall_enriched_memory.py`

- [x] **Step 1: Write failing enriched payload test**

Append this test to `tests/test_recall_enriched_memory.py`:

```python
from hieronymus.rag_models import RagChunkRecord
from hieronymus.memory_models import RecallResult


def test_rag_recall_result_enriched_payload_contains_citation_fields() -> None:
    chunk = RagChunkRecord(
        id=12,
        source_id=3,
        series_slug="only-sense-online",
        source_ref="glossary.csv",
        chunk_kind="glossary_entry",
        text="source: Sense\ntarget: Сенс",
        display_text="source: Sense\ntarget: Сенс",
        location="row 2",
        metadata={"source": "Sense", "target": "Сенс"},
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )

    result = RecallResult.rag(chunk, rank=1, score=0.75, reason="rag glossary match")
    payload = result.enriched_payload()

    assert payload["tier"] == "rag"
    assert payload["source_ref"] == "glossary.csv"
    assert payload["chunk_kind"] == "glossary_entry"
    assert payload["location"] == "row 2"
    assert payload["metadata"] == {"source": "Sense", "target": "Сенс"}
    assert payload["language_tags"] == ("ja", "ru")
    assert payload["rank_reason"] == "rag glossary match"
```

- [x] **Step 2: Run the enriched payload test and verify it fails**

Run: `uv run pytest tests/test_recall_enriched_memory.py::test_rag_recall_result_enriched_payload_contains_citation_fields -q`

Expected: FAIL because `RecallResult.rag` does not exist.

- [x] **Step 3: Modify `RecallResult` to support RAG**

In `src/hieronymus/memory_models.py`, import `RagChunkRecord`:

```python
from hieronymus.rag_models import RagChunkRecord
```

Change the dataclass fields and validation:

```python
@dataclass(frozen=True)
class RecallResult:
    source: Literal["long_term", "short_term", "rag"]
    rank: int
    score: float
    reason: str
    crystal: CrystalRecord | None = None
    short_term_memory: ShortTermMemoryRecord | None = None
    rag_chunk: RagChunkRecord | None = None
    concept_labels: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if self.source == "long_term":
            if self.crystal is None:
                raise ValueError("long_term recall results require a crystal")
            if self.short_term_memory is not None or self.rag_chunk is not None:
                raise ValueError("long_term recall results must not include other payloads")
            return
        if self.source == "short_term":
            if self.short_term_memory is None:
                raise ValueError("short_term recall results require short-term memory")
            if self.crystal is not None or self.rag_chunk is not None:
                raise ValueError("short_term recall results must not include other payloads")
            return
        if self.source == "rag":
            if self.rag_chunk is None:
                raise ValueError("rag recall results require a RAG chunk")
            if self.crystal is not None or self.short_term_memory is not None:
                raise ValueError("rag recall results must not include memory payloads")
            return
        raise ValueError(f"unknown recall source: {self.source}")
```

Add the constructor:

```python
    @classmethod
    def rag(
        cls,
        chunk: RagChunkRecord,
        *,
        rank: int,
        score: float,
        reason: str,
    ) -> RecallResult:
        return cls(
            source="rag",
            rank=rank,
            score=score,
            reason=reason,
            rag_chunk=chunk,
        )
```

Update properties so `rag_chunk` is handled:

```python
    @property
    def tier(self) -> Literal["short_term", "long_term", "rag"]:
        return self.source

    @property
    def id(self) -> int:
        if self.crystal is not None:
            return self.crystal.id
        if self.short_term_memory is not None:
            return self.short_term_memory.id
        assert self.rag_chunk is not None
        return self.rag_chunk.id

    @property
    def title(self) -> str:
        if self.crystal is not None:
            return self.crystal.title
        if self.short_term_memory is not None:
            return self.short_term_memory.kind
        assert self.rag_chunk is not None
        return self.rag_chunk.title

    @property
    def kind(self) -> str:
        if self.crystal is not None:
            return self.crystal.crystal_type
        if self.short_term_memory is not None:
            return self.short_term_memory.kind
        assert self.rag_chunk is not None
        return self.rag_chunk.kind

    @property
    def text(self) -> str:
        if self.crystal is not None:
            return self.crystal.text
        if self.short_term_memory is not None:
            return self.short_term_memory.text
        assert self.rag_chunk is not None
        return self.rag_chunk.text
```

Replace the `language_tags`, `story_scopes`, and `semantic_tags` properties with
these definitions:

```python
    @property
    def language_tags(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.language_tags
        if self.short_term_memory is not None:
            return self.short_term_memory.language_tags
        assert self.rag_chunk is not None
        return self.rag_chunk.language_tags

    @property
    def story_scopes(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.story_scopes
        if self.short_term_memory is not None:
            return self.short_term_memory.story_scopes
        assert self.rag_chunk is not None
        return self.rag_chunk.story_scopes

    @property
    def semantic_tags(self) -> tuple[str, ...]:
        if self.crystal is not None:
            return self.crystal.semantic_tags
        if self.short_term_memory is not None:
            return self.short_term_memory.semantic_tags
        assert self.rag_chunk is not None
        return self.rag_chunk.semantic_tags
```

Add RAG citation fields to `enriched_payload`:

```python
            "source_ref": self.rag_chunk.source_ref if self.rag_chunk is not None else "",
            "chunk_kind": self.rag_chunk.chunk_kind if self.rag_chunk is not None else "",
            "location": self.rag_chunk.location if self.rag_chunk is not None else "",
            "metadata": self.rag_chunk.metadata if self.rag_chunk is not None else {},
```

- [x] **Step 4: Run enriched payload tests**

Run: `uv run pytest tests/test_recall_enriched_memory.py -q`

Expected: PASS.

- [x] **Step 5: Commit Task 5**

```bash
git add src/hieronymus/memory_models.py tests/test_recall_enriched_memory.py
git commit -m "feat: add rag recall payloads"
```

## Task 6: Merge RAG Into Recall

**Files:**
- Modify: `src/hieronymus/recall.py`
- Modify: `src/hieronymus/cli.py`
- Modify: `src/hieronymus/mcp_server.py`
- Create: `tests/test_recall_rag.py`

- [x] **Step 1: Write failing mixed recall tests**

Create `tests/test_recall_rag.py`:

```python
from pathlib import Path

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.rag import RagStore
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )


def test_recall_returns_memory_and_rag_results(config: HieronymusConfig, tmp_path: Path) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        title="Sense Memory",
        text="Render Sense consistently in menu contexts.",
        strength=0.7,
        confidence=0.7,
    )
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Сенс\n", encoding="utf-8")
    RagStore(config).import_file(context.series_slug, source, source_ref="glossary.csv")

    results = RecallService(config).recall(session.id, context, "Sense", limit=4)

    assert {"long_term", "rag"} <= {result.source for result in results}
    rag = next(result for result in results if result.source == "rag")
    assert rag.rag_chunk is not None
    assert rag.rag_chunk.source_ref == "glossary.csv"
    assert rag.reason == "rag glossary match"


def test_active_rule_crystal_ranks_above_rag(config: HieronymusConfig, tmp_path: Path) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    rule_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        title="Sense Rule",
        text="Sense is translated as Сенс.",
        strength=0.95,
        confidence=0.95,
        rule_intent="terminology",
        status="active",
    )
    source = tmp_path / "glossary.csv"
    source.write_text("source,target\nSense,Wrong\n", encoding="utf-8")
    RagStore(config).import_file(context.series_slug, source, source_ref="glossary.csv")

    results = RecallService(config).recall(session.id, context, "Sense", limit=4)

    assert results[0].source == "long_term"
    assert results[0].crystal is not None
    assert results[0].crystal.id == rule_id
    assert any(result.source == "rag" for result in results[1:])


def test_recall_fills_limit_from_memory_when_rag_is_empty(config: HieronymusConfig) -> None:
    context = _context(config)
    session = WorkspaceStore(config).start_session(context)
    for index in range(3):
        CrystalStore(config).add_crystal(
            context,
            crystal_type="lesson",
            text=f"Sense memory note {index}.",
            strength=0.8,
            confidence=0.8,
        )

    results = RecallService(config).recall(session.id, context, "Sense", limit=3)

    assert len(results) == 3
    assert {result.source for result in results} == {"long_term"}
```

- [x] **Step 2: Run mixed recall tests and verify they fail**

Run: `uv run pytest tests/test_recall_rag.py -q`

Expected: FAIL because `RecallService` does not include RAG results.

- [x] **Step 3: Add RAG search and merge helpers to `RecallService`**

In `src/hieronymus/recall.py`, import `RagStore`:

```python
from hieronymus.rag import RagStore
```

Initialize it in `RecallService.__init__`:

```python
        self.rag = RagStore(config)
```

Add these helpers below `_metadata_search_score` or as private functions:

```python
def _split_memory_rag_budget(limit: int, protected_count: int) -> tuple[int, int]:
    remaining = max(limit - protected_count, 0)
    rag_budget = remaining // 2
    memory_budget = remaining - rag_budget
    return memory_budget, rag_budget


def _is_protected_rule_item(item: tuple[str, float, int, object, str, tuple[str, ...]]) -> bool:
    source, _score, _item_id, payload, _reason, _labels = item
    return (
        source == "long_term"
        and getattr(payload, "crystal_type", "") == "rule"
        and getattr(payload, "status", "") == "active"
    )
```

Refactor the end of `recall()`:

1. Keep current memory candidate collection and sorting.
2. Build `protected_items = [item for item in ranked_items if _is_protected_rule_item(item)]`.
3. Build `memory_items = [item for item in ranked_items if item not in protected_items]`.
4. Fetch `rag_hits = self.rag.search(context.series_slug, query, limit=limit)`.
5. Fill the final candidate list with protected rules first, then budgeted interleaving.

Use this merge helper inside the class:

```python
    def _merge_memory_and_rag(
        self,
        memory_items: list[tuple[str, float, int, object, str, tuple[str, ...]]],
        rag_hits,
        *,
        limit: int,
    ) -> list[tuple[str, float, int, object, str, tuple[str, ...]]]:
        protected = [item for item in memory_items if _is_protected_rule_item(item)]
        other_memory = [item for item in memory_items if not _is_protected_rule_item(item)]
        selected: list[tuple[str, float, int, object, str, tuple[str, ...]]] = protected[:limit]
        memory_budget, rag_budget = _split_memory_rag_budget(limit, len(selected))
        selected_memory = other_memory[:memory_budget]
        selected_rag = [
            ("rag", hit.score, hit.chunk.id, hit.chunk, hit.reason, ())
            for hit in rag_hits[:rag_budget]
        ]
        memory_overflow = other_memory[memory_budget:]
        rag_overflow = [
            ("rag", hit.score, hit.chunk.id, hit.chunk, hit.reason, ())
            for hit in rag_hits[rag_budget:]
        ]

        while len(selected) < limit and (selected_memory or selected_rag):
            if selected_memory:
                selected.append(selected_memory.pop(0))
            if len(selected) >= limit:
                break
            if selected_rag:
                selected.append(selected_rag.pop(0))

        overflow = memory_overflow + rag_overflow
        overflow.sort(key=lambda item: (-item[1], item[0], item[2]))
        selected.extend(overflow[: max(limit - len(selected), 0)])
        return selected[:limit]
```

Replace the result-building loop with this version:

```python
        merged_items = self._merge_memory_and_rag(
            ranked_items,
            self.rag.search(context.series_slug, query, limit=limit),
            limit=limit,
        )

        results: list[RecallResult] = []
        for rank, (source, score, _item_id, payload, reason, labels) in enumerate(
            merged_items,
            start=1,
        ):
            if source == "long_term":
                results.append(
                    RecallResult.long_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=reason,
                        concept_labels=labels,
                    )
                )
            elif source == "short_term":
                results.append(
                    RecallResult.short_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=_SHORT_TERM_REASON,
                    )
                )
            else:
                results.append(
                    RecallResult.rag(
                        payload,
                        rank=rank,
                        score=score,
                        reason=reason,
                    )
                )
```

- [x] **Step 4: Update CLI recall payload**

In `src/hieronymus/cli.py`, change the recall JSON payload loop to use enriched payloads, matching MCP:

```python
    payload = []
    for result in results:
        enriched = result.enriched_payload()
        for key in ("concept_ids", "concept_labels", "language_tags", "story_scopes", "semantic_tags"):
            enriched[key] = list(enriched[key])
        payload.append(
            {
                **enriched,
                "source": result.source,
                "rank": result.rank,
                "reason": result.reason,
                "crystal": _crystal_payload(result.crystal),
                "short_term_memory": _short_term_memory_payload(result.short_term_memory),
            }
        )
```

- [x] **Step 5: Update MCP recall payload for RAG**

In `src/hieronymus/mcp_server.py`, update `_recall_payload` to include a RAG chunk payload:

```python
    return {
        **payload,
        "source": result.source,
        "rank": result.rank,
        "reason": result.reason,
        "crystal": _crystal_payload(result.crystal),
        "short_term_memory": _short_term_memory_payload(result.short_term_memory),
        "rag_chunk": (
            {
                "id": result.rag_chunk.id,
                "source_ref": result.rag_chunk.source_ref,
                "chunk_kind": result.rag_chunk.chunk_kind,
                "location": result.rag_chunk.location,
                "metadata": result.rag_chunk.metadata,
            }
            if result.rag_chunk is not None
            else None
        ),
    }
```

- [x] **Step 6: Run recall tests**

Run: `uv run pytest tests/test_recall_rag.py tests/test_recall_enriched_memory.py tests/test_combined_recall.py -q`

Expected: PASS.

- [x] **Step 7: Commit Task 6**

```bash
git add src/hieronymus/recall.py src/hieronymus/cli.py src/hieronymus/mcp_server.py tests/test_recall_rag.py tests/test_recall_enriched_memory.py
git commit -m "feat: merge rag evidence into recall"
```

## Task 7: Documentation And Full Verification

**Files:**
- Modify: `docs/usage.md`
- Modify: `docs/memory-dreaming.md`

- [ ] **Step 1: Add user-facing RAG usage docs**

In `docs/usage.md`, add this section after "Store a Concept With Facets":

```markdown
## Import Project RAG Sources

RAG sources are explicit project text and glossary files. They are advisory
evidence for recall; active rule crystals remain mandatory.

Import a text or Markdown file:

```bash
hiero rag import only-sense-online ./chapter-005.txt --source-ref book:5/chapter:5/source.txt
```

Import a glossary:

```bash
hiero rag import only-sense-online ./glossary.csv --type glossary --source-ref glossary/main.csv
```

Search RAG evidence directly:

```bash
hiero rag search only-sense-online "Cooking Talent" --json
```

Ordinary recall includes both memory results and RAG evidence. RAG entries include
their source reference, chunk kind, location, score, and rank reason so agents can
cite where evidence came from.
```

- [ ] **Step 2: Document memory/RAG priority**

In `docs/memory-dreaming.md`, add this paragraph near the recall explanation:

```markdown
Recall uses a dual-lane strategy. The memory lane searches active short-term
memories, crystals, concepts, facets, metadata, and protected rule crystals. The
RAG lane searches imported project text and glossary chunks. Active rule crystals
rank above RAG evidence, while ordinary memory and RAG results share the remaining
limit through a budgeted merge. RAG evidence is advisory and does not create or
activate rule crystals by itself.
```

- [ ] **Step 3: Run focused tests**

Run:

```bash
uv run pytest tests/test_rag_store.py tests/test_rag_cli.py tests/test_mcp_rag.py tests/test_recall_rag.py tests/test_recall_enriched_memory.py tests/test_combined_recall.py
```

Expected: PASS.

- [ ] **Step 4: Run project verification**

Run:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: all commands PASS.

- [ ] **Step 5: Commit Task 7**

```bash
git add docs/usage.md docs/memory-dreaming.md
git commit -m "docs: document rag source retrieval"
```

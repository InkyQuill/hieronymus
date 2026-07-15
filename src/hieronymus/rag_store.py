from __future__ import annotations

import json
import sqlite3
from collections.abc import Iterable
from datetime import UTC, datetime
from pathlib import Path

import yaml

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import search_expression
from hieronymus.db import apply_migration, connect
from hieronymus.rag_models import (
    RagChunkRecord,
    RagImportResult,
    RagSearchResult,
    RagSourceRecord,
)
from hieronymus.rag_parsing import RagLoadSourceType, load_rag_file

_MAX_RAG_SEARCH_LIMIT = 50
_RAG_LANGUAGE_TAG_BOOST = 0.05
_RAG_STORY_SCOPE_BOOST = 0.10
_RAG_SEMANTIC_TAG_BOOST = 0.10
_RAG_GLOSSARY_BOOST = 0.25


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
            if (
                existing is not None
                and existing["checksum"] == parsed.checksum
                and existing["source_type"] == parsed.source_type
                and existing["content_type"] == parsed.content_type
            ):
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
        language_tags: Iterable[str] = (),
        story_scopes: Iterable[str] = (),
        semantic_tags: Iterable[str] = (),
    ) -> list[RagSearchResult]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        expression = search_expression(query)
        if not expression:
            return []

        bounded_limit = min(limit, _MAX_RAG_SEARCH_LIMIT)
        clean_language_tags = _clean_text_tuple(language_tags)
        clean_story_scopes = _clean_text_tuple(story_scopes)
        clean_semantic_tags = _clean_text_tuple(semantic_tags)
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
                """,
                (expression, series_slug),
            ).fetchall()

            results = [
                RagSearchResult(
                    chunk=chunk,
                    score=_rag_search_score(
                        float(row["rank"]),
                        chunk,
                        language_tags=clean_language_tags,
                        story_scopes=clean_story_scopes,
                        semantic_tags=clean_semantic_tags,
                    ),
                    reason=_reason_for_chunk_kind(row["chunk_kind"]),
                )
                for row in rows
                for chunk in (self._chunk_from_row(conn, row),)
            ]
            return sorted(results, key=lambda hit: (-hit.score, hit.chunk.id))[:bounded_limit]

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


def _rag_search_score(
    rank: float,
    chunk: RagChunkRecord,
    *,
    language_tags: Iterable[str],
    story_scopes: Iterable[str],
    semantic_tags: Iterable[str],
) -> float:
    score = max(-rank, 0.000001)
    if chunk.chunk_kind == "glossary_entry":
        score += _RAG_GLOSSARY_BOOST
    if _has_casefold_intersection(language_tags, chunk.language_tags):
        score += _RAG_LANGUAGE_TAG_BOOST
    if _has_casefold_intersection(story_scopes, chunk.story_scopes):
        score += _RAG_STORY_SCOPE_BOOST
    if _has_casefold_intersection(semantic_tags, chunk.semantic_tags):
        score += _RAG_SEMANTIC_TAG_BOOST
    return score


def _has_casefold_intersection(
    left: Iterable[str],
    right: Iterable[str],
) -> bool:
    left_values = {value.casefold() for value in left}
    return any(value.casefold() in left_values for value in right)


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

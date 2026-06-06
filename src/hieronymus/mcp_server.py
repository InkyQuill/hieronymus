from __future__ import annotations

from dataclasses import asdict
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

from hieronymus.config import load_config
from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase

server = FastMCP("hieronymus")


def _series_database_path(series_slug: str) -> Path:
    series = Registry(load_config()).get_series(series_slug)
    return series.database_path


@server.tool()
def hieronymus_termbase_contract(series_slug: str, raw_text: str) -> list[dict[str, Any]]:
    database_path = _series_database_path(series_slug)
    terms = Termbase(database_path).contract(raw_text)
    return [asdict(term) for term in terms]


@server.tool()
def hieronymus_termbase_validate(
    series_slug: str, raw_text: str, translated_text: str
) -> list[dict[str, Any]]:
    database_path = _series_database_path(series_slug)
    findings = Termbase(database_path).validate(
        raw_text=raw_text,
        translated_text=translated_text,
    )
    return [asdict(finding) for finding in findings]


@server.tool()
def hieronymus_termbase_propose(
    series_slug: str,
    category: str,
    source_text: str,
    canonical_translation: str,
    tags: list[str] | None = None,
    notes: str = "",
) -> dict[str, int]:
    database_path = _series_database_path(series_slug)
    term_id = Termbase(database_path).propose(
        category=category,
        source_text=source_text,
        canonical_translation=canonical_translation,
        tags=tags,
        notes=notes,
    )
    return {"term_id": term_id}


@server.tool()
def hieronymus_termbase_approve(series_slug: str, term_id: int) -> dict[str, int | bool]:
    database_path = _series_database_path(series_slug)
    Termbase(database_path).approve(term_id)
    return {"term_id": term_id, "approved": True}


@server.tool()
def hieronymus_memory_search(series_slug: str, query: str, limit: int = 5) -> list[dict[str, Any]]:
    database_path = _series_database_path(series_slug)
    memories = MemoryStore(database_path).search(query, limit=limit)
    return [asdict(memory) for memory in memories]


@server.tool()
def hieronymus_memory_add(
    series_slug: str,
    kind: str,
    text: str,
    source_ref: str = "",
    importance: int = 3,
) -> dict[str, int]:
    database_path = _series_database_path(series_slug)
    memory_id = MemoryStore(database_path).add(
        kind=kind,
        text=text,
        source_ref=source_ref,
        importance=importance,
    )
    return {"memory_id": memory_id}


def main() -> None:
    server.run(transport="stdio")

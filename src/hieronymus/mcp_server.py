from __future__ import annotations

from dataclasses import asdict
from typing import Any

from mcp.server.fastmcp import FastMCP

from hieronymus.config import HieronymusConfig, load_config
from hieronymus.memory import MemoryStore
from hieronymus.registry import Registry, Series
from hieronymus.termbase import Termbase

server = FastMCP("hieronymus")


def _load_validated_config() -> HieronymusConfig:
    config = load_config()
    if config.data_root.exists() and not config.data_root.is_dir():
        raise ValueError(f"data root is not a directory: {config.data_root}")
    return config


def _series_context(series_slug: str) -> tuple[HieronymusConfig, Series]:
    config = _load_validated_config()
    series = Registry(config).get_series(series_slug)
    return config, series


def _termbase(config: HieronymusConfig, series: Series) -> Termbase:
    return Termbase(
        config.database_path,
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
    )


@server.tool()
def hieronymus_termbase_contract(series_slug: str, raw_text: str) -> list[dict[str, Any]]:
    """Return approved termbase entries required by raw source text."""
    config, series = _series_context(series_slug)
    terms = _termbase(config, series).contract(raw_text)
    return [asdict(term) for term in terms]


@server.tool()
def hieronymus_termbase_validate(
    series_slug: str, raw_text: str, translated_text: str
) -> list[dict[str, Any]]:
    """Validate translated text against approved termbase entries."""
    config, series = _series_context(series_slug)
    findings = _termbase(config, series).validate(
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
    """Propose a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    term_id = _termbase(config, series).propose(
        category=category,
        source_text=source_text,
        canonical_translation=canonical_translation,
        tags=tags,
        notes=notes,
    )
    return {"term_id": term_id}


@server.tool()
def hieronymus_termbase_approve(series_slug: str, term_id: int) -> dict[str, int | bool]:
    """Approve a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    _termbase(config, series).approve(term_id)
    return {"term_id": term_id, "approved": True}


@server.tool()
def hieronymus_memory_search(series_slug: str, query: str, limit: int = 5) -> list[dict[str, Any]]:
    """Search translation memory entries for a series."""
    config, series = _series_context(series_slug)
    memories = MemoryStore(config.database_path, series_slug=series.slug).search(query, limit=limit)
    return [asdict(memory) for memory in memories]


@server.tool()
def hieronymus_memory_add(
    series_slug: str,
    kind: str,
    text: str,
    source_ref: str = "",
    importance: int = 3,
) -> dict[str, int]:
    """Add a translation memory entry for a series."""
    config, series = _series_context(series_slug)
    memory_id = MemoryStore(config.database_path, series_slug=series.slug).add(
        kind=kind,
        text=text,
        source_ref=source_ref,
        importance=importance,
    )
    return {"memory_id": memory_id}


def main() -> None:
    server.run(transport="stdio")

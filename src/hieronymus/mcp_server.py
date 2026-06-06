from __future__ import annotations

from dataclasses import asdict
from typing import Any

from mcp.server.fastmcp import FastMCP

from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig, load_config
from hieronymus.dreaming import DeterministicDreamProvider, DreamService
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry, Series
from hieronymus.scoring import FeedbackStore
from hieronymus.termbase import Termbase
from hieronymus.workspace import WorkspaceStore

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


def _termbase(config: HieronymusConfig, series: Series, context: TranslationContext) -> Termbase:
    return Termbase(config, context)


def _memory(config: HieronymusConfig, series: Series, context: TranslationContext) -> MemoryStore:
    return MemoryStore(config, context)


def _translation_context(
    series: Series,
    *,
    source_language: str | None = None,
    target_language: str | None = None,
    task_type: str = "translation",
    volume: str = "",
    chapter: str = "",
) -> TranslationContext:
    source = source_language or series.source_language
    target = target_language or series.target_language
    if source != series.source_language:
        raise ValueError(
            f"source_language {source!r} does not match registry default "
            f"{series.source_language!r} for series {series.slug!r}"
        )
    if target != series.target_language:
        raise ValueError(
            f"target_language {target!r} does not match registry default "
            f"{series.target_language!r} for series {series.slug!r}"
        )
    return TranslationContext(
        series_slug=series.slug,
        source_language=source,
        target_language=target,
        task_type=task_type,
        volume=volume,
        chapter=chapter,
    )


@server.tool()
def hieronymus_termbase_contract(
    series_slug: str,
    raw_text: str,
    source_language: str | None = None,
    target_language: str | None = None,
) -> list[dict[str, Any]]:
    """Return approved termbase entries required by raw source text."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    terms = _termbase(config, series, context).contract(raw_text)
    return [asdict(term) for term in terms]


@server.tool()
def hieronymus_termbase_validate(
    series_slug: str,
    raw_text: str,
    translated_text: str,
    source_language: str | None = None,
    target_language: str | None = None,
) -> list[dict[str, Any]]:
    """Validate translated text against approved termbase entries."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    findings = _termbase(config, series, context).validate(
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
    source_language: str | None = None,
    target_language: str | None = None,
) -> dict[str, int]:
    """Propose a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    term_id = _termbase(config, series, context).propose(
        category=category,
        source_text=source_text,
        canonical_translation=canonical_translation,
        tags=tags,
        notes=notes,
    )
    return {"term_id": term_id}


@server.tool()
def hieronymus_termbase_approve(
    series_slug: str,
    term_id: int,
    source_language: str | None = None,
    target_language: str | None = None,
) -> dict[str, int | bool]:
    """Approve a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    _termbase(config, series, context).approve(term_id)
    return {"term_id": term_id, "approved": True}


@server.tool()
def hieronymus_memory_search(
    series_slug: str,
    query: str,
    limit: int = 5,
    source_language: str | None = None,
    target_language: str | None = None,
) -> list[dict[str, Any]]:
    """Search translation memory entries for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    memories = _memory(config, series, context).search(query, limit=limit)
    return [asdict(memory) for memory in memories]


@server.tool()
def hieronymus_memory_add(
    series_slug: str,
    kind: str,
    text: str,
    source_ref: str = "",
    importance: int = 3,
    source_language: str | None = None,
    target_language: str | None = None,
) -> dict[str, int]:
    """Add a translation memory entry for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    memory_id = _memory(config, series, context).add(
        kind=kind,
        text=text,
        source_ref=source_ref,
        importance=importance,
    )
    return {"memory_id": memory_id}


@server.tool()
def hieronymus_session_start(
    series_slug: str,
    source_language: str = "ja",
    target_language: str = "en",
    task_type: str = "translation",
    volume: str = "",
    chapter: str = "",
) -> dict[str, int]:
    """Start an agent workflow session for a translation context."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        task_type=task_type,
        volume=volume,
        chapter=chapter,
    )
    session = WorkspaceStore(config).start_session(context)
    return {"session_id": session.id}


@server.tool()
def hieronymus_session_complete(session_id: int) -> dict[str, int | bool]:
    """Complete an agent workflow session so it can be dreamed."""
    config = _load_validated_config()
    WorkspaceStore(config).complete_session(session_id)
    return {"session_id": session_id, "completed": True}


@server.tool()
def hieronymus_short_term_add(
    session_id: int,
    source_role: str,
    kind: str,
    text: str,
    source_ref: str = "",
    metadata: dict[str, object] | None = None,
) -> dict[str, int]:
    """Add a short-term memory to an active session."""
    config = _load_validated_config()
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session_id=session_id,
        source_role=source_role,
        kind=kind,
        text=text,
        source_ref=source_ref,
        metadata=metadata,
    )
    return {"memory_id": memory_id}


@server.tool()
def hieronymus_recall(
    session_id: int,
    series_slug: str,
    query: str,
    source_language: str = "ja",
    target_language: str = "en",
    task_type: str = "translation",
    volume: str = "",
    chapter: str = "",
    limit: int = 10,
) -> list[dict[str, Any]]:
    """Recall long-term crystals for the exact stored session context."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        task_type=task_type,
        volume=volume,
        chapter=chapter,
    )
    session = WorkspaceStore(config).get_session(session_id)
    if session.context != context:
        raise ValueError("session context mismatch")

    results = RecallService(config).recall(session_id, context, query, limit=limit)
    return [
        {
            "crystal_id": result.crystal.id,
            "text": result.crystal.text,
            "rank": result.rank,
            "score": result.score,
            "reason": result.reason,
        }
        for result in results
    ]


@server.tool()
def hieronymus_feedback(
    crystal_id: int,
    event_type: str,
    source_role: str,
    evidence: str = "",
    session_id: int | None = None,
) -> dict[str, int]:
    """Record feedback for a crystal."""
    config = _load_validated_config()
    event_id = FeedbackStore(config).record(
        crystal_id=crystal_id,
        event_type=event_type,
        source_role=source_role,
        evidence=evidence,
        session_id=session_id,
    )
    return {"event_id": event_id}


@server.tool()
def hieronymus_dream(provider: str = "deterministic") -> dict[str, int | str]:
    """Run a dream cycle over completed sessions."""
    config = _load_validated_config()
    if provider != "deterministic":
        raise ValueError(f"unsupported dream provider: {provider}")
    run = DreamService(config, DeterministicDreamProvider()).run_cycle()
    return {"cycle_id": run.cycle_id, "status": run.status}


@server.tool()
def hieronymus_concept_proposals_list() -> list[dict[str, Any]]:
    """List pending strict concept proposals."""
    config = _load_validated_config()
    return [asdict(proposal) for proposal in ConceptProposalStore(config).list_pending()]


def main() -> None:
    server.run(transport="stdio")

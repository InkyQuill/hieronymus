from __future__ import annotations

import json
from dataclasses import asdict
from typing import Any

from mcp.server.fastmcp import FastMCP

from hieronymus.agent_ingestion import IngestionService
from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig, load_config
from hieronymus.db import connect
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import DreamService
from hieronymus.memory import MemoryStore
from hieronymus.memory_models import (
    CrystalRecord,
    ShortTermMemoryRecord,
    TaskSessionRecord,
    TranslationContext,
)
from hieronymus.recall import RecallService
from hieronymus.registry import Registry, Series
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


def _crystal_payload(crystal: CrystalRecord | None) -> dict[str, Any] | None:
    if crystal is None:
        return None
    return {
        "id": crystal.id,
        "crystal_type": crystal.crystal_type,
        "text": crystal.text,
        "title": crystal.title,
        "confidence": crystal.confidence,
        "strength": crystal.strength,
        "status": crystal.status,
        "source_credibility": crystal.source_credibility,
        "rule_intent": crystal.rule_intent,
        "story_scopes": list(crystal.story_scopes),
        "semantic_tags": list(crystal.semantic_tags),
        "concept_ids": list(crystal.concept_ids),
    }


def _short_term_memory_payload(memory: ShortTermMemoryRecord | None) -> dict[str, Any] | None:
    if memory is None:
        return None
    return {
        "id": memory.id,
        "source_role": memory.source_role,
        "kind": memory.kind,
        "text": memory.text,
        "metadata": memory.metadata,
    }


def _translation_context(
    series: Series,
    *,
    source_language: str | None = None,
    target_language: str | None = None,
    task_type: str = "translation",
    volume: str = "",
    chapter: str = "",
) -> TranslationContext:
    source = series.source_language if source_language is None else source_language
    target = series.target_language if target_language is None else target_language
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


def _ensure_default_session(
    config: HieronymusConfig,
    series: Series,
    *,
    source_language: str | None = None,
    target_language: str | None = None,
) -> TaskSessionRecord:
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
    )
    workspace = WorkspaceStore(config)
    with connect(config.database_path) as conn:
        row = conn.execute(
            """
            select *
            from task_sessions
            where series_slug = ?
              and source_language = ?
              and target_language = ?
              and task_type = ?
              and volume = ?
              and chapter = ?
              and status = 'active'
            order by id desc
            limit 1
            """,
            (
                context.series_slug,
                context.source_language,
                context.target_language,
                context.task_type,
                context.volume,
                context.chapter,
            ),
        ).fetchone()
    if row is not None:
        return TaskSessionRecord(
            id=int(row["id"]),
            context=context,
            status=row["status"],
            cycle_id=row["cycle_id"],
        )
    return workspace.start_session(context)


def _strict_concept_proposal_payload(proposal: object) -> dict[str, Any]:
    return asdict(proposal)


def _optional_string(value: object) -> str:
    return value if isinstance(value, str) else ""


def _required_string(value: object) -> str | None:
    if isinstance(value, str) and value.strip():
        return value
    return None


def _string_list(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item for item in value if isinstance(item, str)]


def _recent_dream_audit_proposal_payloads(config: HieronymusConfig) -> list[dict[str, Any]]:
    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select id, payload_json
            from dream_audit_entries
            order by id desc
            limit 50
            """
        ).fetchall()

    payloads: list[dict[str, Any]] = []
    for row in rows:
        try:
            payload = json.loads(row["payload_json"])
        except json.JSONDecodeError:
            continue
        proposals = payload.get("concept_proposals") if isinstance(payload, dict) else None
        if not isinstance(proposals, list):
            continue
        for proposal in proposals:
            if not isinstance(proposal, dict):
                continue
            concept_text = _required_string(proposal.get("concept_text"))
            if concept_text is None:
                continue
            source_form = _required_string(proposal.get("source_form", concept_text))
            if source_form is None:
                continue
            canonical_rendering = _required_string(
                proposal.get("canonical_rendering", concept_text)
            )
            if canonical_rendering is None:
                canonical_rendering = concept_text
            payloads.append(
                {
                    "id": int(row["id"]),
                    "series_slug": _optional_string(proposal.get("series_slug")),
                    "source_language": _optional_string(proposal.get("source_language")),
                    "target_language": _optional_string(proposal.get("target_language")),
                    "concept_text": concept_text,
                    "source_form": source_form,
                    "canonical_rendering": canonical_rendering,
                    "approved_variants": _string_list(proposal.get("approved_variants")),
                    "forbidden_variants": _string_list(proposal.get("forbidden_variants")),
                    "rationale": _optional_string(proposal.get("rationale")),
                    "status": "audit",
                }
            )
    return payloads


@server.tool()
def hieronymus_termbase_contract(
    series_slug: str,
    raw_text: str,
    source_language: str | None = None,
    target_language: str | None = None,
    volume: str = "",
    chapter: str = "",
) -> list[dict[str, Any]]:
    """Return approved termbase entries required by raw source text."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        volume=volume,
        chapter=chapter,
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
    volume: str = "",
    chapter: str = "",
) -> list[dict[str, Any]]:
    """Validate translated text against approved termbase entries."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        volume=volume,
        chapter=chapter,
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
    volume: str = "",
    chapter: str = "",
) -> dict[str, int]:
    """Propose a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        volume=volume,
        chapter=chapter,
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
    volume: str = "",
    chapter: str = "",
) -> dict[str, int | bool]:
    """Approve a pending termbase entry for a series."""
    config, series = _series_context(series_slug)
    context = _translation_context(
        series,
        source_language=source_language,
        target_language=target_language,
        volume=volume,
        chapter=chapter,
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
) -> dict[str, int | str]:
    """Add user memory as short-term learning material for a series."""
    if not kind.strip():
        raise ValueError("kind must not be empty")
    config, series = _series_context(series_slug)
    session = _ensure_default_session(
        config,
        series,
        source_language=source_language,
        target_language=target_language,
    )
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction" if kind in {"rule", "correction"} else "note",
        text=text,
        source_ref=source_ref,
        metadata={"legacy_kind": kind, "importance": importance},
    )
    return {"memory_id": memory_id, "storage": "short_term"}


@server.tool()
def hieronymus_session_start(
    series_slug: str,
    source_language: str | None = None,
    target_language: str | None = None,
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
def hieronymus_learn(
    session_id: int,
    text: str,
    source_role: str,
    source_ref: str = "",
    kind: str = "learned_block",
) -> dict[str, object]:
    """Split material into short-term memories eligible for dreaming."""
    config = _load_validated_config()
    result = IngestionService(config).learn(
        session_id=session_id,
        text=text,
        source_role=source_role,
        source_ref=source_ref,
        kind=kind,
    )
    return {
        "session_id": result.session_id,
        "block_count": result.block_count,
        "memory_ids": result.memory_ids,
    }


@server.tool()
def hieronymus_read(
    session_id: int,
    text: str,
    source_ref: str = "",
    store_observation: bool = False,
) -> dict[str, object]:
    """Extract temporary concepts/terms without committing the full source by default."""
    config = _load_validated_config()
    result = IngestionService(config).read(
        session_id=session_id,
        text=text,
        source_ref=source_ref,
        store_observation=store_observation,
    )
    return {
        "session_id": result.session_id,
        "candidate_terms": result.candidate_terms,
        "findings": result.findings,
        "stored_memory_ids": result.stored_memory_ids,
    }


@server.tool()
def hieronymus_recall(
    session_id: int,
    series_slug: str,
    query: str,
    source_language: str | None = None,
    target_language: str | None = None,
    task_type: str | None = None,
    volume: str | None = None,
    chapter: str | None = None,
    limit: int = 10,
) -> list[dict[str, Any]]:
    """Recall long-term crystals and active short-term memories for a stored session."""
    config, series = _series_context(series_slug)
    session = WorkspaceStore(config).get_session(session_id)
    context = _translation_context(
        series,
        source_language=(
            session.context.source_language if source_language is None else source_language
        ),
        target_language=(
            session.context.target_language if target_language is None else target_language
        ),
        task_type=session.context.task_type if task_type is None else task_type,
        volume=session.context.volume if volume is None else volume,
        chapter=session.context.chapter if chapter is None else chapter,
    )
    if session.context != context:
        raise ValueError("session context mismatch")

    results = RecallService(config).recall(session_id, context, query, limit=limit)
    return [
        {
            "source": result.source,
            "rank": result.rank,
            "score": result.score,
            "reason": result.reason,
            "crystal": _crystal_payload(result.crystal),
            "short_term_memory": _short_term_memory_payload(result.short_term_memory),
        }
        for result in results
    ]


@server.tool()
def hieronymus_feedback(
    session_id: int,
    correction_text: str,
) -> dict[str, int]:
    """Record user correction feedback as short-term memory."""
    config = _load_validated_config()
    memory_id = WorkspaceStore(config).add_short_term_memory(
        session_id=session_id,
        source_role="user",
        kind="correction",
        text=correction_text,
    )
    return {"memory_id": memory_id}


@server.tool()
def hieronymus_dream(
    provider: str | None = None,
    wait: bool = False,
) -> dict[str, int | str]:
    """Run dreaming over all pending completed-session memories."""
    config = _load_validated_config()
    run = DreamService(config, resolve_provider(config, provider)).run_all(
        owner="mcp",
        wait=wait,
    )
    return {
        "cycle_id": run.cycle_id,
        "status": run.status,
        "provider": run.provider,
        "input_count": run.input_count,
        "created_crystal_count": run.created_crystal_count,
        "proposal_count": run.proposal_count,
    }


@server.tool()
def hieronymus_concept_proposals_list() -> list[dict[str, Any]]:
    """List strict proposals and legacy-compatible vague concept suggestions."""
    config = _load_validated_config()
    return [
        *[
            _strict_concept_proposal_payload(proposal)
            for proposal in ConceptProposalStore(config).list_pending()
        ],
        *_recent_dream_audit_proposal_payloads(config),
    ]


def main() -> None:
    server.run(transport="stdio")

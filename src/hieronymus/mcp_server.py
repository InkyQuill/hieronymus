from __future__ import annotations

import inspect
import json
from contextvars import ContextVar
from dataclasses import asdict
from functools import wraps
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

from hieronymus.concept_models import ConceptFacetRecord, ConceptRecord
from hieronymus.concepts import CONCEPT_CANDIDATE, ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig, load_config
from hieronymus.crystals import CrystalStore
from hieronymus.daemon_mcp_client import DaemonMcpClient
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
from hieronymus.rag import RagStore
from hieronymus.rag_payloads import (
    rag_chunk_payload as _rag_chunk_payload,
)
from hieronymus.rag_payloads import (
    rag_hit_payload as _rag_hit_payload,
)
from hieronymus.rag_payloads import (
    rag_import_payload as _rag_import_payload,
)
from hieronymus.recall import RecallService
from hieronymus.registry import Registry, Series
from hieronymus.termbase import Termbase
from hieronymus.workspace import WorkspaceStore

_daemon_config: ContextVar[HieronymusConfig | None] = ContextVar("daemon_config", default=None)
_DIRECT_MCP_OPERATIONS: dict[str, Any] = {}


class DaemonMcpServer(FastMCP):
    def tool(self, *args: Any, **kwargs: Any):
        register = super().tool(*args, **kwargs)

        def decorate(function: Any) -> Any:
            operation = function.__name__.removeprefix("hieronymus_")
            _DIRECT_MCP_OPERATIONS[operation] = function
            signature = inspect.signature(function)

            @wraps(function)
            def invoke_via_daemon(*function_args: Any, **function_kwargs: Any) -> Any:
                if _daemon_config.get() is not None:
                    return function(*function_args, **function_kwargs)
                bound = signature.bind(*function_args, **function_kwargs)
                bound.apply_defaults()
                return _daemon_client().invoke(operation, dict(bound.arguments))

            return register(invoke_via_daemon)

        return decorate


server = DaemonMcpServer("hieronymus")
_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION = (
    "Compatibility wrapper. New workflows should use concept, facet, short-term memory, "
    "and rule-crystal primitives."
)


def _load_validated_config() -> HieronymusConfig:
    config = _daemon_config.get() or load_config()
    if config.data_root.exists() and not config.data_root.is_dir():
        raise ValueError(f"data root is not a directory: {config.data_root}")
    return config


def _daemon_client() -> DaemonMcpClient:
    return DaemonMcpClient(_load_validated_config())


def invoke_daemon_operation(
    config: HieronymusConfig,
    operation: str,
    params: dict[str, object],
) -> object:
    function = _DIRECT_MCP_OPERATIONS.get(operation)
    if function is None:
        raise KeyError(operation)
    try:
        bound = inspect.signature(function).bind(**params)
    except TypeError as error:
        raise ValueError(str(error)) from error
    bound.apply_defaults()
    token = _daemon_config.set(config)
    try:
        return function(*bound.args, **bound.kwargs)
    finally:
        _daemon_config.reset(token)


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


def _concept_payload(concept: ConceptRecord) -> dict[str, Any]:
    return {
        "id": concept.id,
        "canonical_name": concept.canonical_name,
        "description": concept.description,
        "status": concept.status,
        "confidence": concept.confidence,
        "scope_type": concept.scope_type,
        "scope_key": concept.scope_key,
        "semantic_tags": list(concept.tags),
        "merged_into_concept_id": concept.merged_into_concept_id,
    }


def _facet_payload(facet: ConceptFacetRecord) -> dict[str, Any]:
    return {
        "id": facet.id,
        "concept_id": facet.concept_id,
        "language": facet.language,
        "facet_type": facet.facet_type,
        "kind": facet.kind,
        "value": facet.value,
        "confidence": facet.confidence,
        "source_crystal_id": facet.source_crystal_id,
        "language_tags": list(facet.language_tags),
        "story_scopes": list(facet.story_scopes),
        "semantic_tags": list(facet.semantic_tags),
        "is_canonical": facet.is_canonical,
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
        "language_tags": list(memory.language_tags),
        "story_scopes": list(memory.story_scopes),
        "semantic_tags": list(memory.semantic_tags),
        "source_credibility": memory.source_credibility,
        "rule_intent": memory.rule_intent,
        "soft_origin": memory.soft_origin,
    }


def _recall_payload(result) -> dict[str, Any]:
    payload = result.enriched_payload()
    for key in (
        "concept_ids",
        "concept_labels",
        "language_tags",
        "story_scopes",
        "semantic_tags",
    ):
        payload[key] = list(payload[key])
    return {
        **payload,
        "source": result.source,
        "rank": result.rank,
        "reason": result.reason,
        "crystal": _crystal_payload(result.crystal),
        "short_term_memory": _short_term_memory_payload(result.short_term_memory),
        "rag_chunk": _rag_chunk_payload(result.rag_chunk),
    }


def _series_payload(series: Series) -> dict[str, Any]:
    payload = {
        "slug": series.slug,
        "title": series.title,
        "source_language": series.source_language,
        "target_language": series.target_language,
        "language_tags": list(series.language_tags),
    }
    if series.id is not None:
        payload["id"] = series.id
    return payload


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
        row_id = int(row["id"])
        for index, proposal in enumerate(proposals):
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
                    "id": f"{row_id}-{index}",
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
def hieronymus_status() -> dict[str, Any]:
    """Report MCP adapter mode and discovered local service status."""
    return _daemon_client().invoke("status", {})


@server.tool()
def hieronymus_series_create(
    slug: str,
    title: str,
    source_language: str = "",
    target_language: str = "",
    language_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Create or update a language-neutral series."""
    config = _load_validated_config()
    series = Registry(config).create_series(
        slug=slug,
        title=title,
        source_language=source_language,
        target_language=target_language,
        language_tags=language_tags,
    )
    return _series_payload(series)


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
def hieronymus_series_init(
    slug: str,
    title: str,
    source_language: str = "",
    target_language: str = "",
    language_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Compatibility wrapper for creating or updating a series."""
    return hieronymus_series_create(
        slug=slug,
        title=title,
        source_language=source_language,
        target_language=target_language,
        language_tags=language_tags,
    )


@server.tool()
def hieronymus_series_list() -> list[dict[str, Any]]:
    """List registered series with language tags."""
    config = _load_validated_config()
    return [_series_payload(series) for series in Registry(config).list_series()]


@server.tool()
def hieronymus_series_set_language_tags(
    series_id: int,
    language_tags: list[str],
) -> dict[str, Any]:
    """Replace language tags for a series without changing compatibility fields."""
    config = _load_validated_config()
    registry = Registry(config)
    registry.set_series_language_tags(series_id, language_tags)
    for series in registry.list_series():
        if series.id == series_id:
            return _series_payload(series)
    raise KeyError(f"unknown series id: {series_id}")


@server.tool()
def hieronymus_concept_list(
    status: str | None = None,
    semantic_tag: str | None = None,
    series_slug: str | None = None,
    include_global: bool = True,
) -> list[dict[str, Any]]:
    """List concepts with optional status, tag, and series-scope filters."""
    config = _load_validated_config()
    concepts = ConceptStore(config).list_concepts(status=status, semantic_tag=semantic_tag)
    if series_slug is not None:
        scope_key = f"series:{series_slug}"
        concepts = [
            concept
            for concept in concepts
            if concept.scope_key == scope_key or (include_global and concept.scope_type == "global")
        ]
    return [_concept_payload(concept) for concept in concepts]


@server.tool()
def hieronymus_concept_get(concept_id: int) -> dict[str, Any]:
    """Get one concept by id."""
    config = _load_validated_config()
    return _concept_payload(ConceptStore(config).get(concept_id))


@server.tool()
def hieronymus_concept_create(
    canonical_name: str,
    description: str = "",
    status: str = CONCEPT_CANDIDATE,
    confidence: float = 0.2,
    semantic_tags: list[str] | None = None,
    series_slug: str = "",
    scope_type: str = "global",
    scope_key: str = "",
) -> dict[str, Any]:
    """Create a concept primitive."""
    config = _load_validated_config()
    if series_slug:
        Registry(config).get_series(series_slug)
        scope_type = "series"
        scope_key = f"series:{series_slug}"
    concept = ConceptStore(config).create_concept(
        canonical_name,
        description=description,
        status=status,
        confidence=confidence,
        scope_type=scope_type,
        scope_key=scope_key,
        semantic_tags=semantic_tags or (),
    )
    return _concept_payload(concept)


@server.tool()
def hieronymus_concept_update(
    concept_id: int,
    description: str | None = None,
    status: str | None = None,
    confidence: float | None = None,
) -> dict[str, Any]:
    """Update concept mutable metadata."""
    config = _load_validated_config()
    concept = ConceptStore(config).update_concept(
        concept_id,
        description=description,
        status=status,
        confidence=confidence,
    )
    return _concept_payload(concept)


@server.tool()
def hieronymus_concept_archive(concept_id: int, reason: str = "") -> dict[str, Any]:
    """Archive a concept so recall and strict rule logic stop using it."""
    config = _load_validated_config()
    store = ConceptStore(config)
    store.archive_concept(concept_id, reason)
    return _concept_payload(store.get(concept_id))


@server.tool()
def hieronymus_concept_merge(
    source_concept_id: int,
    target_concept_id: int,
    reason: str = "",
) -> dict[str, Any]:
    """Merge one concept into another active concept."""
    config = _load_validated_config()
    store = ConceptStore(config)
    store.merge_concepts(source_concept_id, target_concept_id, reason)
    return {
        "source": _concept_payload(store.get(source_concept_id)),
        "target": _concept_payload(store.get(target_concept_id)),
    }


@server.tool()
def hieronymus_concept_rename(
    concept_id: int,
    new_label: str,
    source_crystal_id: int | None = None,
) -> dict[str, Any]:
    """Rename a concept while retaining the old name as a former-label facet."""
    config = _load_validated_config()
    concept = ConceptStore(config).rename_concept(
        concept_id,
        new_label,
        source_crystal_id=source_crystal_id,
    )
    return _concept_payload(concept)


@server.tool()
def hieronymus_concept_facet_add(
    concept_id: int,
    value: str,
    language: str = "",
    language_tags: list[str] | None = None,
    kind: str | None = "name",
    facet_type: str | None = None,
    confidence: float = 0.2,
    source_crystal_id: int | None = None,
    is_canonical: bool = False,
    story_scopes: list[str] | None = None,
    semantic_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Add a multilingual concept facet."""
    config = _load_validated_config()
    storage_kind = None if facet_type and kind == "name" else kind
    facet = ConceptStore(config).add_facet(
        concept_id,
        value,
        language=language,
        language_tags=language_tags or (),
        kind=storage_kind,
        facet_type=facet_type,
        confidence=confidence,
        source_crystal_id=source_crystal_id,
        is_canonical=is_canonical,
        story_scopes=story_scopes or (),
        semantic_tags=semantic_tags or (),
    )
    return _facet_payload(facet)


@server.tool()
def hieronymus_concept_facet_update(
    facet_id: int,
    value: str | None = None,
    language: str | None = None,
    language_tags: list[str] | None = None,
    kind: str | None = None,
    facet_type: str | None = None,
    confidence: float | None = None,
    source_crystal_id: int | None = None,
    is_canonical: bool | None = None,
    story_scopes: list[str] | None = None,
    semantic_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Update a concept facet primitive."""
    config = _load_validated_config()
    facet = ConceptStore(config).update_facet(
        facet_id,
        value=value,
        language=language,
        language_tags=language_tags,
        kind=kind,
        facet_type=facet_type,
        confidence=confidence,
        source_crystal_id=source_crystal_id,
        is_canonical=is_canonical,
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
    )
    return _facet_payload(facet)


@server.tool()
def hieronymus_concept_facet_list(concept_id: int) -> list[dict[str, Any]]:
    """List active facets for a concept."""
    config = _load_validated_config()
    return [_facet_payload(facet) for facet in ConceptStore(config).list_facets(concept_id)]


@server.tool()
def hieronymus_concept_facet_set_canonical(
    concept_id: int,
    facet_id: int,
) -> dict[str, Any]:
    """Set one concept facet as canonical for its concept."""
    config = _load_validated_config()
    store = ConceptStore(config)
    store.set_canonical_facet(concept_id, facet_id)
    return _facet_payload(store.get_facet(facet_id))


@server.tool()
def hieronymus_concept_semantic_tags_set(
    concept_id: int,
    semantic_tags: list[str],
) -> dict[str, Any]:
    """Replace semantic tags for a concept."""
    config = _load_validated_config()
    store = ConceptStore(config)
    store.set_semantic_tags(concept_id, semantic_tags)
    return _concept_payload(store.get(concept_id))


@server.tool()
def hieronymus_crystal_link_concept(
    crystal_id: int,
    concept_id: int,
    link_type: str = "mentions",
    confidence: float = 0.2,
) -> dict[str, Any]:
    """Link a long-term crystal to a concept."""
    config = _load_validated_config()
    ConceptStore(config).link_crystal(
        crystal_id,
        concept_id,
        link_type=link_type,
        confidence=confidence,
    )
    crystal = CrystalStore(config).get(crystal_id)
    return _crystal_payload(crystal) or {}


@server.tool()
def hieronymus_crystal_story_scopes_set(
    crystal_id: int,
    story_scopes: list[str],
    confidence: float = 0.2,
) -> dict[str, Any]:
    """Replace story scopes for a crystal."""
    config = _load_validated_config()
    crystal = CrystalStore(config).set_story_scopes(
        crystal_id,
        tuple(story_scopes),
        confidence=confidence,
    )
    return _crystal_payload(crystal) or {}


@server.tool()
def hieronymus_crystal_semantic_tags_set(
    crystal_id: int,
    semantic_tags: list[str],
    confidence: float = 0.2,
) -> dict[str, Any]:
    """Replace semantic tags for a crystal."""
    config = _load_validated_config()
    crystal = CrystalStore(config).set_semantic_tags(
        crystal_id,
        tuple(semantic_tags),
        confidence=confidence,
    )
    return _crystal_payload(crystal) or {}


@server.tool()
def hieronymus_rule_crystals_list(
    status: str | None = None,
    series_slug: str | None = None,
    limit: int = 50,
) -> list[dict[str, Any]]:
    """List rule crystals for review."""
    config = _load_validated_config()
    return [
        _crystal_payload(crystal) or {}
        for crystal in CrystalStore(config).list_rule_crystals(
            status=status,
            series_slug=series_slug,
            limit=limit,
        )
    ]


@server.tool()
def hieronymus_rule_crystal_archive(crystal_id: int) -> dict[str, Any]:
    """Archive a rule crystal."""
    config = _load_validated_config()
    crystal = CrystalStore(config).archive_rule_crystal(crystal_id)
    return _crystal_payload(crystal) or {}


@server.tool()
def hieronymus_rule_crystal_validate(crystal_id: int) -> dict[str, Any]:
    """Validate rule-crystal shape and deterministic enforceability."""
    config = _load_validated_config()
    return CrystalStore(config).validate_rule_crystal(crystal_id)


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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
def hieronymus_rag_import(
    series_slug: str,
    path: str,
    source_ref: str | None = None,
    source_type: str = "auto",
    language_tags: list[str] | None = None,
    story_scopes: list[str] | None = None,
    semantic_tags: list[str] | None = None,
) -> dict[str, Any]:
    """Import a text, markdown, or glossary file into the project RAG store."""
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
    return _rag_import_payload(result)


@server.tool()
def hieronymus_rag_search(
    series_slug: str,
    query: str,
    limit: int = 10,
) -> list[dict[str, Any]]:
    """Search project RAG chunks for a series."""
    config, _series = _series_context(series_slug)
    hits = RagStore(config).search(series_slug, query, limit=limit)
    return [_rag_hit_payload(hit) for hit in hits]


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
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
    language_tags: list[str] | None = None,
    story_scopes: list[str] | None = None,
    semantic_tags: list[str] | None = None,
    source_credibility: str = "observation",
    rule_intent: str = "",
    soft_origin: str = "",
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
        language_tags=language_tags or (),
        story_scopes=story_scopes or (),
        semantic_tags=semantic_tags or (),
        source_credibility=source_credibility,
        rule_intent=rule_intent,
        soft_origin=soft_origin,
    )
    return {"memory_id": memory_id}


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
    context = session.context
    if context.series_slug != series.slug:
        raise ValueError("session context mismatch")
    override_values = {
        "source_language": source_language,
        "target_language": target_language,
        "task_type": task_type,
        "volume": volume,
        "chapter": chapter,
    }
    for field_name, override in override_values.items():
        if override is not None and override != getattr(context, field_name):
            raise ValueError(f"session context mismatch: {field_name}")

    results = RecallService(config).recall(session_id, context, query, limit=limit)
    return [_recall_payload(result) for result in results]


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


@server.tool(description=_MEMORY_PRIMITIVES_COMPATIBILITY_DESCRIPTION)
def hieronymus_concept_proposals_list() -> list[dict[str, Any]]:
    """List strict proposals and legacy-compatible candidate concept suggestions."""
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

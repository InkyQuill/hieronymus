from __future__ import annotations

import json
from typing import Any, Protocol

from hieronymus.dream_validation import DreamConceptProposal, NormalizedDreamCrystal
from hieronymus.memory_models import TranslationContext
from hieronymus.values import utc_now

_REDACTED = "[REDACTED]"
_SECRET_KEYS = frozenset(
    {
        "apikey",
        "authorization",
        "xapikey",
        "xgoogapikey",
        "anthropicversion",
        "token",
        "bearer",
    }
)


class ConceptProposalWriter(Protocol):
    def _create_with_connection(
        self,
        conn,
        *,
        dream_run_id: int,
        series_slug: str,
        source_language: str,
        target_language: str,
        concept_text: str,
        source_form: str,
        canonical_rendering: str,
        approved_variants: list[str],
        forbidden_variants: list[str],
        rationale: str,
    ) -> int: ...


def insert_crystal(
    conn,
    context: TranslationContext,
    candidate: NormalizedDreamCrystal,
) -> int:
    """Insert a crystal and its graph edges in the caller's transaction."""
    now = utc_now()
    tags_json = candidate.semantic_tags if candidate.semantic_tags else context.tags
    cursor = conn.execute(
        """
        insert into crystals(
          crystal_type,
          text,
          title,
          scope_type,
          scope_key,
          series_slug,
          source_language,
          target_language,
          tags_json,
          strength,
          confidence,
          source_credibility,
          rule_intent,
          is_inferred,
          malformed_penalty,
          supersedes_crystal_id,
          status,
          created_at,
          updated_at
        )
        values (?, ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
        """,
        (
            candidate.crystal_type,
            candidate.text,
            candidate.title,
            context.scope_key,
            context.series_slug,
            context.source_language,
            context.target_language,
            json.dumps(list(tags_json), ensure_ascii=False, sort_keys=True),
            candidate.strength,
            candidate.confidence,
            candidate.source_credibility,
            candidate.rule_intent,
            int(candidate.is_inferred),
            candidate.malformed_penalty,
            candidate.supersedes_crystal_id,
            now,
            now,
        ),
    )
    crystal_id = int(cursor.lastrowid)
    for memory_id in candidate.source_memory_ids:
        conn.execute(
            """
            insert into crystal_sources(crystal_id, short_term_memory_id)
            values (?, ?)
            """,
            (crystal_id, memory_id),
        )
    for story_scope in candidate.story_scopes:
        conn.execute(
            """
            insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
            values (?, ?, ?, ?)
            on conflict(crystal_id, scope) do update set
              confidence = max(crystal_story_scopes.confidence, excluded.confidence)
            """,
            (crystal_id, story_scope, candidate.confidence, now),
        )
    for semantic_tag in candidate.semantic_tags:
        conn.execute(
            """
            insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
            values (?, ?, ?, ?)
            on conflict(crystal_id, tag) do update set
              confidence = max(crystal_semantic_tags.confidence, excluded.confidence)
            """,
            (crystal_id, semantic_tag, candidate.confidence, now),
        )
    for concept_id in candidate.concept_ids:
        conn.execute(
            """
            insert into crystal_concepts(
              crystal_id,
              concept_id,
              link_type,
              confidence,
              created_at
            )
            values (?, ?, 'mentions', ?, ?)
            on conflict(crystal_id, concept_id, link_type) do update set
              confidence = max(crystal_concepts.confidence, excluded.confidence)
            """,
            (crystal_id, concept_id, candidate.confidence, now),
        )
    return crystal_id


def insert_concept_proposals(
    conn,
    writer: ConceptProposalWriter,
    run_id: int,
    proposals: list[DreamConceptProposal],
) -> None:
    """Insert concept proposals without owning the surrounding transaction."""
    for proposal in proposals:
        writer._create_with_connection(
            conn,
            dream_run_id=run_id,
            series_slug=proposal.series_slug,
            source_language=proposal.source_language,
            target_language=proposal.target_language,
            concept_text=proposal.concept_text,
            source_form=proposal.source_form,
            canonical_rendering=proposal.canonical_rendering,
            approved_variants=proposal.approved_variants,
            forbidden_variants=proposal.forbidden_variants,
            rationale=proposal.rationale,
        )


def append_audit_entry(
    conn,
    *,
    dream_run_id: int,
    phase_run_id: int | None,
    event_type: str,
    severity: str,
    summary: str,
    payload: Any,
) -> int:
    """Append a redacted audit entry in the caller's transaction."""
    cursor = conn.execute(
        """
        insert into dream_audit_entries(
          dream_run_id,
          phase_run_id,
          event_type,
          severity,
          summary,
          payload_json,
          created_at
        )
        values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            dream_run_id,
            phase_run_id,
            event_type,
            severity,
            summary,
            json.dumps(_redact_payload(payload), ensure_ascii=False, sort_keys=True),
            utc_now(),
        ),
    )
    return int(cursor.lastrowid)


def _redact_payload(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: _REDACTED if _is_secret_key(key) else _redact_payload(item)
            for key, item in value.items()
        }
    if isinstance(value, list | tuple):
        return [_redact_payload(item) for item in value]
    return value


def _is_secret_key(key: object) -> bool:
    if not isinstance(key, str):
        return False
    normalized = key.replace("-", "").replace("_", "").lower()
    return normalized in _SECRET_KEYS or "token" in normalized or "bearer" in normalized

from __future__ import annotations

import json
import re
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import CrystalRecord, TranslationContext

_ALLOWED_CRYSTAL_TYPES = frozenset(
    {"lesson", "rule", "thought", "observation", "concept_note", "concept", "erudition"}
)
_ALLOWED_STATUSES = frozenset({"active", "candidate", "archived", "rejected"})
_FTS_OPERATORS = frozenset({"and", "or", "not", "near"})
_MAX_SEARCH_LIMIT = 50
_TOKEN_RE = re.compile(r"\w+")


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _clamp_score(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def _clean_text_tuple(values: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(sorted({value.strip() for value in values if value.strip()}))


def _clean_int_tuple(values: tuple[int, ...]) -> tuple[int, ...]:
    return tuple(sorted(set(values)))


def _search_expression(query: str) -> str:
    tokens = [token for token in _TOKEN_RE.findall(query) if token.casefold() not in _FTS_OPERATORS]
    return " ".join(f'"{token}"' for token in tokens)


def _row_to_crystal(
    row,
    *,
    story_scopes: tuple[str, ...] = (),
    semantic_tags: tuple[str, ...] = (),
    concept_ids: tuple[int, ...] = (),
) -> CrystalRecord:
    return CrystalRecord(
        id=row["id"],
        crystal_type=row["crystal_type"],
        text=row["text"],
        title=row["title"],
        scope_type=row["scope_type"],
        scope_key=row["scope_key"],
        series_slug=row["series_slug"],
        source_language=row["source_language"],
        target_language=row["target_language"],
        strength=float(row["strength"]),
        confidence=float(row["confidence"]),
        source_credibility=row["source_credibility"],
        rule_intent=row["rule_intent"],
        malformed_penalty=float(row["malformed_penalty"]),
        supersedes_crystal_id=row["supersedes_crystal_id"],
        status=row["status"],
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        concept_ids=concept_ids,
    )


def _weighted_search_score(
    raw_bm25: float,
    strength: float,
    confidence: float,
    scope_type: str,
) -> float:
    # SQLite FTS5 bm25 is lower-is-better and commonly negative; negating it
    # produces a higher-is-better relevance component for a descending sort.
    fts_component = max(-raw_bm25, 0.0)
    scope_bonus = 0.05 if scope_type == "series" else 0.0
    return fts_component + (strength * 0.35) + (confidence * 0.20) + scope_bonus


class CrystalStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def add_crystal(
        self,
        context: TranslationContext,
        *,
        crystal_type: str,
        text: str,
        title: str = "",
        strength: float = 0.5,
        confidence: float = 0.5,
        source_credibility: str = "observation",
        rule_intent: str = "",
        malformed_penalty: float = 0.0,
        supersedes_crystal_id: int | None = None,
        story_scopes: tuple[str, ...] = (),
        semantic_tags: tuple[str, ...] = (),
        concept_ids: tuple[int, ...] = (),
        status: str = "active",
        source_memory_ids: list[int] | None = None,
    ) -> int:
        self._validate_crystal_type(crystal_type)
        self._validate_status(status)
        if not text.strip():
            raise ValueError("text must not be empty")

        clamped_strength = _clamp_score(strength)
        clamped_confidence = _clamp_score(confidence)
        clean_story_scopes = _clean_text_tuple(story_scopes)
        clean_semantic_tags = _clean_text_tuple(semantic_tags)
        clean_concept_ids = _clean_int_tuple(concept_ids)
        legacy_tags = list(clean_semantic_tags) if clean_semantic_tags else list(context.tags)
        tags_json = json.dumps(legacy_tags, ensure_ascii=False, sort_keys=True)
        now = _now()

        with connect(self.config.database_path) as conn:
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
                  malformed_penalty,
                  supersedes_crystal_id,
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    crystal_type,
                    text,
                    title,
                    context.scope_key,
                    context.series_slug,
                    context.source_language,
                    context.target_language,
                    tags_json,
                    clamped_strength,
                    clamped_confidence,
                    source_credibility,
                    rule_intent,
                    malformed_penalty,
                    supersedes_crystal_id,
                    status,
                    now,
                    now,
                ),
            )
            crystal_id = int(cursor.lastrowid)
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (crystal_id, title, text),
            )
            for story_scope in clean_story_scopes:
                conn.execute(
                    """
                    insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
                    values (?, ?, ?, ?)
                    """,
                    (crystal_id, story_scope, clamped_confidence, now),
                )
            for semantic_tag in clean_semantic_tags:
                conn.execute(
                    """
                    insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                    values (?, ?, ?, ?)
                    """,
                    (crystal_id, semantic_tag, clamped_confidence, now),
                )
            for concept_id in clean_concept_ids:
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
                    """,
                    (crystal_id, concept_id, clamped_confidence, now),
                )
            for memory_id in source_memory_ids or []:
                conn.execute(
                    """
                    insert into crystal_sources(crystal_id, short_term_memory_id)
                    values (?, ?)
                    """,
                    (crystal_id, memory_id),
                )
            conn.commit()
        return crystal_id

    def get(self, crystal_id: int) -> CrystalRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from crystals where id = ?",
                (crystal_id,),
            ).fetchone()
            if row is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            crystal = self._hydrate_crystal(conn, row)
        return crystal

    def search(
        self,
        context: TranslationContext,
        query: str,
        *,
        limit: int = 10,
    ) -> list[CrystalRecord]:
        return [crystal for crystal, _score in self.search_scored(context, query, limit=limit)]

    def search_scored(
        self,
        context: TranslationContext,
        query: str,
        *,
        limit: int = 10,
    ) -> list[tuple[CrystalRecord, float]]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        expression = _search_expression(query)
        if not expression:
            return []

        bounded_limit = min(limit, _MAX_SEARCH_LIMIT)
        with connect(self.config.database_path) as conn:
            conn.create_function(
                "weighted_search_score",
                4,
                _weighted_search_score,
                deterministic=True,
            )
            rows = conn.execute(
                """
                select
                  crystals.*,
                  weighted_search_score(
                    bm25(crystals_fts),
                    crystals.strength,
                    crystals.confidence,
                    crystals.scope_type
                  ) as search_score
                from crystals_fts
                join crystals on crystals.id = crystals_fts.rowid
                where crystals_fts match ?
                  and crystals.status in ('active', 'candidate')
                  and (
                    (
                      crystals.scope_type = 'series'
                      and crystals.scope_key = ?
                    )
                    or crystals.scope_type = 'global'
                  )
                  and (
                    crystals.source_language = ?
                    or crystals.source_language = ''
                  )
                  and (
                    crystals.target_language = ?
                    or crystals.target_language = ''
                  )
                order by
                  search_score desc,
                  crystals.id
                limit ?
                """,
                (
                    expression,
                    context.scope_key,
                    context.source_language,
                    context.target_language,
                    bounded_limit,
                ),
            ).fetchall()
            return [(self._hydrate_crystal(conn, row), float(row["search_score"])) for row in rows]

    def _hydrate_crystal(self, conn, row) -> CrystalRecord:
        story_scopes = tuple(
            scope_row["scope"]
            for scope_row in conn.execute(
                """
                select scope
                from crystal_story_scopes
                where crystal_id = ?
                order by scope
                """,
                (row["id"],),
            )
        )
        semantic_tags = tuple(
            tag_row["tag"]
            for tag_row in conn.execute(
                """
                select tag
                from crystal_semantic_tags
                where crystal_id = ?
                order by tag
                """,
                (row["id"],),
            )
        )
        concept_ids = tuple(
            int(concept_row["concept_id"])
            for concept_row in conn.execute(
                """
                select distinct concept_id
                from crystal_concepts
                where crystal_id = ?
                order by concept_id
                """,
                (row["id"],),
            )
        )
        return _row_to_crystal(
            row,
            story_scopes=story_scopes,
            semantic_tags=semantic_tags,
            concept_ids=concept_ids,
        )

    def _validate_crystal_type(self, crystal_type: str) -> None:
        if crystal_type not in _ALLOWED_CRYSTAL_TYPES:
            raise ValueError(f"unknown crystal_type: {crystal_type}")

    def _validate_status(self, status: str) -> None:
        if status not in _ALLOWED_STATUSES:
            raise ValueError(f"unknown status: {status}")

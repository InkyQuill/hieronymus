from __future__ import annotations

import json
import re
from datetime import UTC, datetime
from typing import Any

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import CrystalRecord, TranslationContext
from hieronymus.rule_crystals import (
    DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD,
    DETERMINISTIC_RULE_STRENGTH_THRESHOLD,
    active_concept_ids_for_rule,
    parse_rule_crystal,
)

_ALLOWED_CRYSTAL_TYPES = frozenset(
    {"lesson", "rule", "thought", "observation", "concept_note", "concept", "erudition"}
)
_ALLOWED_STATUSES = frozenset({"active", "candidate", "archived", "rejected", "superseded"})
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


def search_expression(query: str) -> str:
    tokens = [token for token in _TOKEN_RE.findall(query) if token.casefold() not in _FTS_OPERATORS]
    return " ".join(f'"{token}"' for token in tokens)


def _search_expression(query: str) -> str:
    return search_expression(query)


def _row_to_crystal(
    row,
    *,
    language_tags: tuple[str, ...] = (),
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
        language_tags=language_tags,
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        soft_origin=row["soft_origin"] or "",
        is_inferred=bool(row["is_inferred"]),
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
        language_tags: tuple[str, ...] = (),
        soft_origin: str = "",
        is_inferred: bool = False,
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
        clean_language_tags = _clean_text_tuple(language_tags)
        clean_concept_ids = _clean_int_tuple(concept_ids)
        soft_origin = soft_origin.strip()
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
                  soft_origin,
                  is_inferred,
                  malformed_penalty,
                  supersedes_crystal_id,
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    soft_origin,
                    int(is_inferred),
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
            for language_tag in clean_language_tags:
                conn.execute(
                    """
                    insert into crystal_language_tags(crystal_id, language_tag)
                    values (?, ?)
                    """,
                    (crystal_id, language_tag),
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
            crystal = self._hydrate_crystals(conn, [row])[0]
        return crystal

    def list_rule_crystals(
        self,
        *,
        status: str | None = None,
        series_slug: str | None = None,
        limit: int = 50,
    ) -> list[CrystalRecord]:
        if limit < 1:
            raise ValueError("limit must be at least 1")
        query = ["select * from crystals where crystal_type = 'rule'"]
        params: list[object] = []
        if status is not None:
            self._validate_status(status)
            query.append("and status = ?")
            params.append(status)
        if series_slug is not None:
            query.append("and series_slug = ?")
            params.append(series_slug)
        query.append("order by id desc limit ?")
        params.append(min(limit, _MAX_SEARCH_LIMIT))

        with connect(self.config.database_path) as conn:
            rows = conn.execute("\n".join(query), params).fetchall()
            return self._hydrate_crystals(conn, rows)

    def archive_rule_crystal(self, crystal_id: int) -> CrystalRecord:
        now = _now()
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from crystals where id = ?",
                (crystal_id,),
            ).fetchone()
            if row is None:
                raise KeyError(f"unknown crystal: {crystal_id}")
            if row["crystal_type"] != "rule":
                raise ValueError("crystal is not a rule crystal")
            conn.execute(
                """
                update crystals
                set status = 'archived',
                    updated_at = ?
                where id = ?
                """,
                (now, crystal_id),
            )
            conn.commit()
        return self.get(crystal_id)

    def validate_rule_crystal(self, crystal_id: int) -> dict[str, Any]:
        crystal = self.get(crystal_id)
        with connect(self.config.database_path) as conn:
            active_concept_ids = active_concept_ids_for_rule(conn, crystal_id)
        errors: list[str] = []
        warnings: list[str] = []
        parsed_payload: dict[str, Any] | None = None

        if crystal.crystal_type != "rule":
            errors.append("crystal_type must be 'rule'")
        parsed = parse_rule_crystal(crystal.text)
        if parsed is None:
            errors.append(
                "rule text must match '<source> is translated as <target>[, not <forbidden>]'."
            )
        else:
            parsed_payload = {
                "source_text": parsed.source_text,
                "canonical_translation": parsed.canonical_translation,
                "forbidden_variants": list(parsed.forbidden_variants),
            }

        if crystal.status != "active":
            warnings.append("rule crystal is not active")
        if crystal.confidence < DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD:
            warnings.append("confidence is below deterministic validation threshold")
        if crystal.strength < DETERMINISTIC_RULE_STRENGTH_THRESHOLD:
            warnings.append("strength is below deterministic validation threshold")
        if not crystal.concept_ids:
            warnings.append("rule crystal is not linked to a concept")
        elif not active_concept_ids:
            warnings.append("rule crystal is not linked to an active concept")

        valid = not errors
        enforceable = (
            valid
            and crystal.status == "active"
            and crystal.confidence >= DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD
            and crystal.strength >= DETERMINISTIC_RULE_STRENGTH_THRESHOLD
            and bool(active_concept_ids)
        )
        return {
            "crystal_id": crystal.id,
            "valid": valid,
            "enforceable": enforceable,
            "errors": errors,
            "warnings": warnings,
            "parsed_rule": parsed_payload,
        }

    def set_story_scopes(
        self,
        crystal_id: int,
        story_scopes: tuple[str, ...],
        *,
        confidence: float = 0.2,
    ) -> CrystalRecord:
        self.get(crystal_id)
        clean_story_scopes = _clean_text_tuple(story_scopes)
        clean_confidence = _clamp_score(confidence)
        now = _now()
        with connect(self.config.database_path) as conn:
            conn.execute("delete from crystal_story_scopes where crystal_id = ?", (crystal_id,))
            for story_scope in clean_story_scopes:
                conn.execute(
                    """
                    insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
                    values (?, ?, ?, ?)
                    """,
                    (crystal_id, story_scope, clean_confidence, now),
                )
            conn.execute("update crystals set updated_at = ? where id = ?", (now, crystal_id))
            conn.commit()
        return self.get(crystal_id)

    def set_semantic_tags(
        self,
        crystal_id: int,
        semantic_tags: tuple[str, ...],
        *,
        confidence: float = 0.2,
    ) -> CrystalRecord:
        self.get(crystal_id)
        clean_semantic_tags = _clean_text_tuple(semantic_tags)
        clean_confidence = _clamp_score(confidence)
        now = _now()
        with connect(self.config.database_path) as conn:
            conn.execute("delete from crystal_semantic_tags where crystal_id = ?", (crystal_id,))
            for semantic_tag in clean_semantic_tags:
                conn.execute(
                    """
                    insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                    values (?, ?, ?, ?)
                    """,
                    (crystal_id, semantic_tag, clean_confidence, now),
                )
            conn.execute("update crystals set updated_at = ? where id = ?", (now, crystal_id))
            conn.commit()
        return self.get(crystal_id)

    def low_confidence_first(
        self,
        crystal_ids: tuple[int, ...],
        *,
        limit: int = 5,
    ) -> tuple[int, ...]:
        if limit < 1 or not crystal_ids:
            return ()

        unique_ids = tuple(sorted(set(crystal_ids)))
        placeholders = ", ".join("?" for _ in unique_ids)
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select id
                from crystals
                where id in ({placeholders})
                  and not (crystal_type = 'rule' and status = 'active')
                order by confidence asc, strength asc, id asc
                limit ?
                """,
                (*unique_ids, limit),
            ).fetchall()
        return tuple(int(row["id"]) for row in rows)

    def _supersede_with_connection(
        self,
        conn,
        *,
        old_crystal_id: int,
        new_crystal_id: int,
        reason: str = "",
        cycle_id: int,
    ) -> None:
        if old_crystal_id == new_crystal_id:
            raise ValueError("crystal cannot supersede itself")

        now = _now()
        old_row = conn.execute(
            "select * from crystals where id = ?",
            (old_crystal_id,),
        ).fetchone()
        if old_row is None:
            raise KeyError(f"unknown crystal: {old_crystal_id}")
        new_row = conn.execute(
            "select * from crystals where id = ?",
            (new_crystal_id,),
        ).fetchone()
        if new_row is None:
            raise KeyError(f"unknown crystal: {new_crystal_id}")
        self._validate_supersede_rows(old_row, new_row)

        conn.execute(
            """
            update crystals
            set status = 'superseded',
                updated_at = ?
            where id = ?
            """,
            (now, old_crystal_id),
        )
        conn.execute(
            """
            update crystals
            set supersedes_crystal_id = ?,
                updated_at = ?
            where id = ?
            """,
            (old_crystal_id, now, new_crystal_id),
        )
        conn.execute(
            """
            insert into memory_events(
              crystal_id,
              session_id,
              event_type,
              source_role,
              evidence,
              strength_delta,
              confidence_delta,
              applied,
              cycle_id,
              created_at
            )
            values (?, null, 'supersede', 'system', ?, 0, 0, 1, ?, ?)
            """,
            (old_crystal_id, reason, cycle_id, now),
        )

    def _validate_supersede_rows(self, old_row, new_row) -> None:
        for row in (old_row, new_row):
            if row["status"] not in {"active", "candidate"}:
                raise ValueError("supersede crystals must be active or candidate")
        for column in (
            "series_slug",
            "source_language",
            "target_language",
            "crystal_type",
            "scope_type",
            "scope_key",
        ):
            if old_row[column] != new_row[column]:
                raise ValueError(f"supersede crystal {column} does not match")

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
            crystals = self._hydrate_crystals(conn, rows)
            return list(zip(crystals, (float(row["search_score"]) for row in rows), strict=True))

    def _hydrate_crystals(self, conn, rows) -> list[CrystalRecord]:
        crystal_ids = [int(row["id"]) for row in rows]
        language_tags = self._crystal_text_map(
            conn,
            crystal_ids,
            table="crystal_language_tags",
            value_column="language_tag",
        )
        story_scopes = self._crystal_text_map(
            conn,
            crystal_ids,
            table="crystal_story_scopes",
            value_column="scope",
        )
        semantic_tags = self._crystal_text_map(
            conn,
            crystal_ids,
            table="crystal_semantic_tags",
            value_column="tag",
        )
        concept_ids = self._crystal_concept_map(conn, crystal_ids)
        return [
            _row_to_crystal(
                row,
                language_tags=language_tags.get(int(row["id"]), ()),
                story_scopes=story_scopes.get(int(row["id"]), ()),
                semantic_tags=semantic_tags.get(int(row["id"]), ()),
                concept_ids=concept_ids.get(int(row["id"]), ()),
            )
            for row in rows
        ]

    def _crystal_text_map(
        self,
        conn,
        crystal_ids: list[int],
        *,
        table: str,
        value_column: str,
    ) -> dict[int, tuple[str, ...]]:
        if not crystal_ids:
            return {}
        placeholders = ", ".join("?" for _ in crystal_ids)
        rows = conn.execute(
            f"""
            select crystal_id, {value_column} as value
            from {table}
            where crystal_id in ({placeholders})
            order by crystal_id, value
            """,
            crystal_ids,
        ).fetchall()
        values: dict[int, list[str]] = {crystal_id: [] for crystal_id in crystal_ids}
        for row in rows:
            values[int(row["crystal_id"])].append(row["value"])
        return {crystal_id: tuple(items) for crystal_id, items in values.items()}

    def _crystal_concept_map(self, conn, crystal_ids: list[int]) -> dict[int, tuple[int, ...]]:
        if not crystal_ids:
            return {}
        placeholders = ", ".join("?" for _ in crystal_ids)
        rows = conn.execute(
            f"""
            select distinct crystal_id, concept_id
            from crystal_concepts
            where crystal_id in ({placeholders})
            order by crystal_id, concept_id
            """,
            crystal_ids,
        ).fetchall()
        values: dict[int, list[int]] = {crystal_id: [] for crystal_id in crystal_ids}
        for row in rows:
            values[int(row["crystal_id"])].append(int(row["concept_id"]))
        return {crystal_id: tuple(items) for crystal_id, items in values.items()}

    def _validate_crystal_type(self, crystal_type: str) -> None:
        if crystal_type not in _ALLOWED_CRYSTAL_TYPES:
            raise ValueError(f"unknown crystal_type: {crystal_type}")

    def _validate_status(self, status: str) -> None:
        if status not in _ALLOWED_STATUSES:
            raise ValueError(f"unknown status: {status}")

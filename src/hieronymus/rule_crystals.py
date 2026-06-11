from __future__ import annotations

import re
import sqlite3
from dataclasses import dataclass

from hieronymus.memory_models import TranslationContext

DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD = 0.8
DETERMINISTIC_RULE_STRENGTH_THRESHOLD = 0.8
_INACTIVE_CONCEPT_STATUSES = frozenset({"archived", "merged"})
_SOURCE_FACET_TYPES = frozenset({"name", "alias", "former_label"})
_RULE_RE = re.compile(
    r"^(?P<source>.+?) is translated as (?P<target>.+?)(?:, not (?P<forbidden>.+?))?\.?$"
)


@dataclass(frozen=True)
class ParsedRule:
    source_text: str
    canonical_translation: str
    forbidden_variants: list[str]


@dataclass(frozen=True)
class ActiveRuleCrystal:
    crystal_id: int
    concept_ids: tuple[int, ...]
    source_forms: tuple[str, ...]
    required_renderings: tuple[str, ...]
    forbidden_renderings: tuple[str, ...]
    language_tags: tuple[str, ...]
    story_scopes: tuple[str, ...]
    semantic_tags: tuple[str, ...]
    confidence: float
    strength: float


def parse_rule_crystal(text: str) -> ParsedRule | None:
    match = _RULE_RE.fullmatch(text.strip())
    if match is None:
        return None

    source = match.group("source").strip()
    target = match.group("target").strip()
    forbidden = match.group("forbidden")
    if not source or not target:
        return None

    forbidden_variants: list[str] = []
    if forbidden is not None:
        cleaned_forbidden = forbidden.strip()
        if not cleaned_forbidden:
            return None
        forbidden_variants.append(cleaned_forbidden)

    return ParsedRule(
        source_text=source,
        canonical_translation=target,
        forbidden_variants=forbidden_variants,
    )


def load_active_rule_crystals(
    conn: sqlite3.Connection,
    context: TranslationContext,
) -> list[ActiveRuleCrystal]:
    rows = conn.execute(
        """
        select *
        from crystals
        where crystal_type = 'rule'
          and status = 'active'
          and confidence >= ?
          and strength >= ?
          and (
            (
              scope_type = 'series'
              and scope_key = ?
            )
            or scope_type = 'global'
          )
          and (source_language = ? or source_language = '')
          and (target_language = ? or target_language = '')
          and exists (
            select 1
            from crystal_concepts cc
            join concepts c on c.id = cc.concept_id
            where cc.crystal_id = crystals.id
              and c.status not in (?, ?)
          )
          and not exists (
            select 1
            from crystals successor
            where successor.supersedes_crystal_id = crystals.id
              and successor.crystal_type = 'rule'
              and successor.status = 'active'
          )
        order by id
        """,
        (
            DETERMINISTIC_RULE_CONFIDENCE_THRESHOLD,
            DETERMINISTIC_RULE_STRENGTH_THRESHOLD,
            context.scope_key,
            context.source_language,
            context.target_language,
            *_INACTIVE_CONCEPT_STATUSES,
        ),
    ).fetchall()

    rules: list[ActiveRuleCrystal] = []
    for row in rows:
        parsed = parse_rule_crystal(row["text"])
        if parsed is None:
            continue

        concept_ids = _concept_ids_for_rule(conn, int(row["id"]))
        if not concept_ids:
            continue

        source_forms = [parsed.source_text]
        language_tags = {tag for tag in (row["source_language"], row["target_language"]) if tag}
        language_tags.update(_crystal_language_tags(conn, int(row["id"])))
        story_scopes = set(_crystal_story_scopes(conn, int(row["id"])))
        semantic_tags = set(_crystal_semantic_tags(conn, int(row["id"])))

        for concept_id in concept_ids:
            concept_row = conn.execute(
                "select canonical_name from concepts where id = ?",
                (concept_id,),
            ).fetchone()
            if concept_row is not None:
                source_forms.append(concept_row["canonical_name"])
            semantic_tags.update(_concept_semantic_tags(conn, concept_id))

            for facet in _concept_facets(conn, concept_id):
                facet_language_tags = _facet_language_tags(conn, int(facet["id"]))
                facet_story_scopes = _facet_story_scopes(conn, int(facet["id"]))
                facet_semantic_tags = _facet_semantic_tags(conn, int(facet["id"]))
                language_tags.update(facet_language_tags)
                story_scopes.update(facet_story_scopes)
                semantic_tags.update(facet_semantic_tags)
                if _is_source_facet(
                    facet,
                    facet_language_tags=facet_language_tags,
                    source_language=context.source_language,
                    target_language=context.target_language,
                ):
                    source_forms.append(facet["value"])

        rules.append(
            ActiveRuleCrystal(
                crystal_id=int(row["id"]),
                concept_ids=concept_ids,
                source_forms=_dedupe_text(source_forms),
                required_renderings=(parsed.canonical_translation,),
                forbidden_renderings=tuple(parsed.forbidden_variants),
                language_tags=tuple(sorted(language_tags)),
                story_scopes=tuple(sorted(story_scopes)),
                semantic_tags=tuple(sorted(semantic_tags)),
                confidence=float(row["confidence"]),
                strength=float(row["strength"]),
            )
        )
    return rules


def _concept_ids_for_rule(conn: sqlite3.Connection, crystal_id: int) -> tuple[int, ...]:
    rows = conn.execute(
        """
        select distinct cc.concept_id
        from crystal_concepts cc
        join concepts c on c.id = cc.concept_id
        where cc.crystal_id = ?
          and c.status not in (?, ?)
        order by cc.concept_id
        """,
        (crystal_id, *_INACTIVE_CONCEPT_STATUSES),
    ).fetchall()
    return tuple(int(row["concept_id"]) for row in rows)


def active_concept_ids_for_rule(conn: sqlite3.Connection, crystal_id: int) -> tuple[int, ...]:
    return _concept_ids_for_rule(conn, crystal_id)


def _crystal_language_tags(conn: sqlite3.Connection, crystal_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select language_tag
        from crystal_language_tags
        where crystal_id = ?
        order by language_tag
        """,
        (crystal_id,),
    ).fetchall()
    return tuple(row["language_tag"] for row in rows)


def _crystal_story_scopes(conn: sqlite3.Connection, crystal_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select scope
        from crystal_story_scopes
        where crystal_id = ?
        order by scope
        """,
        (crystal_id,),
    ).fetchall()
    return tuple(row["scope"] for row in rows)


def _crystal_semantic_tags(conn: sqlite3.Connection, crystal_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select tag
        from crystal_semantic_tags
        where crystal_id = ?
        order by tag
        """,
        (crystal_id,),
    ).fetchall()
    return tuple(row["tag"] for row in rows)


def _concept_semantic_tags(conn: sqlite3.Connection, concept_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select tag
        from concept_semantic_tags
        where concept_id = ?
        order by tag
        """,
        (concept_id,),
    ).fetchall()
    return tuple(row["tag"] for row in rows)


def _concept_facets(conn: sqlite3.Connection, concept_id: int) -> list[sqlite3.Row]:
    return conn.execute(
        """
        select *
        from concept_facets
        where concept_id = ?
          and superseded_at is null
        order by is_canonical desc, id
        """,
        (concept_id,),
    ).fetchall()


def _facet_language_tags(conn: sqlite3.Connection, facet_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select language_tag
        from concept_facet_language_tags
        where facet_id = ?
        order by language_tag
        """,
        (facet_id,),
    ).fetchall()
    return tuple(row["language_tag"] for row in rows)


def _facet_story_scopes(conn: sqlite3.Connection, facet_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select story_scope
        from concept_facet_story_scopes
        where facet_id = ?
        order by story_scope
        """,
        (facet_id,),
    ).fetchall()
    return tuple(row["story_scope"] for row in rows)


def _facet_semantic_tags(conn: sqlite3.Connection, facet_id: int) -> tuple[str, ...]:
    rows = conn.execute(
        """
        select semantic_tag
        from concept_facet_semantic_tags
        where facet_id = ?
        order by semantic_tag
        """,
        (facet_id,),
    ).fetchall()
    return tuple(row["semantic_tag"] for row in rows)


def _is_source_facet(
    facet: sqlite3.Row,
    *,
    facet_language_tags: tuple[str, ...],
    source_language: str,
    target_language: str,
) -> bool:
    if facet["facet_type"] not in _SOURCE_FACET_TYPES:
        return False

    tags = set(facet_language_tags)
    if not tags:
        return True
    if source_language in tags:
        return True
    return target_language not in tags


def _dedupe_text(values: list[str]) -> tuple[str, ...]:
    result: list[str] = []
    seen: set[str] = set()
    for value in values:
        clean = value.strip()
        key = clean.casefold()
        if not clean or key in seen:
            continue
        result.append(clean)
        seen.add(key)
    return tuple(result)

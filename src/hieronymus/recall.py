from __future__ import annotations

import re
from collections.abc import Iterable
from datetime import UTC, datetime

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore, search_expression
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import RecallResult, ShortTermMemoryRecord, TranslationContext
from hieronymus.workspace import short_memory_from_row

_RECALL_REASON = "weighted search match"
_LONG_TERM_METADATA_REASON = "metadata recall match"
_SHORT_TERM_REASON = "active session short-term memory match"
_SHORT_TERM_BASE_SCORE = 0.30
_SHORT_TERM_RANK_STEP = 0.01
_STORY_SCOPE_BOOST = 0.25
_SEMANTIC_TAG_BOOST = 0.18
_EXACT_CONCEPT_BOOST = 0.35
_CONCEPT_LINK_BASE_BOOST = 0.18
_ACTIVE_RULE_BOOST = 0.20
_LOW_CONFIDENCE_THOUGHT_PENALTY = 0.12
_SHORT_TERM_METADATA_BASE_SCORE = 0.20
_SHORT_TERM_LANGUAGE_TAG_BOOST = 0.05
_SHORT_TERM_STORY_SCOPE_BOOST = 0.10
_SHORT_TERM_SEMANTIC_TAG_BOOST = 0.10
_SHORT_TERM_SOURCE_CREDIBILITY_BOOST = 0.08
_SHORT_TERM_RULE_INTENT_BOOST = 0.12
_MAX_LONG_TERM_CANDIDATE_LIMIT = 50
_MAX_SHORT_TERM_LIMIT = 50
_TOKEN_RE = re.compile(r"\w+")
_STRUCTURED_DELIMITERS = frozenset({":", "/"})


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _long_term_candidate_limit(limit: int) -> int:
    return min(max(limit * 4, limit + 10), _MAX_LONG_TERM_CANDIDATE_LIMIT)


def _query_terms(query: str) -> frozenset[str]:
    return frozenset(token.casefold() for token in _TOKEN_RE.findall(query))


def _normalized_metadata_value(value: str) -> str:
    return " ".join(value.strip().casefold().split())


def _is_structured_value(value: str) -> bool:
    return any(delimiter in value for delimiter in _STRUCTURED_DELIMITERS)


def _metadata_lookup_values(query: str) -> tuple[str, ...]:
    clean_query = query.strip()
    if not clean_query:
        return ()
    if _is_structured_value(clean_query):
        return (clean_query,)
    values = [clean_query, *_query_terms(clean_query)]
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        normalized_value = _normalized_metadata_value(value)
        if not normalized_value or normalized_value in seen:
            continue
        seen.add(normalized_value)
        normalized.append(value)
    return tuple(normalized)


def _metadata_search_score(strength: float, confidence: float, scope_type: str) -> float:
    scope_bonus = 0.05 if scope_type == "series" else 0.0
    return (strength * 0.35) + (confidence * 0.20) + scope_bonus


class RecallService:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.crystals = CrystalStore(config)
        self.concepts = ConceptStore(config)
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def recall(
        self,
        session_id: int,
        context: TranslationContext,
        query: str,
        *,
        limit: int = 10,
    ) -> list[RecallResult]:
        if limit < 1:
            raise ValueError("limit must be at least 1")

        now = _now()
        with connect(self.config.database_path) as conn:
            self._require_active_session(conn, session_id)
            short_term_memories = self._search_short_term_memories(
                conn,
                session_id=session_id,
                query=query,
                limit=limit,
            )

        scored_crystals = self.crystals.search_scored(
            context,
            query,
            limit=_long_term_candidate_limit(limit),
        )
        ranked_items: list[tuple[str, float, int, object, str, tuple[str, ...]]] = []
        context_story_scopes = set(context.story_scopes).union(context.tags)
        context_semantic_tags = set(context.semantic_tags).union(context.tags)
        long_term_candidates = {
            crystal.id: [crystal, score, _RECALL_REASON] for crystal, score in scored_crystals
        }
        metadata_scores = self._long_term_metadata_candidate_scores(
            context,
            query,
            limit=_long_term_candidate_limit(limit),
        )
        for crystal_id, score in metadata_scores.items():
            if crystal_id in long_term_candidates:
                long_term_candidates[crystal_id][1] += score
                continue
            long_term_candidates[crystal_id] = [
                self.crystals.get(crystal_id),
                score,
                _LONG_TERM_METADATA_REASON,
            ]
        concept_boosts = self.concepts.recall_boosts_for_crystals(
            long_term_candidates,
            query,
            story_scopes=context_story_scopes,
        )
        concept_labels = self._concept_labels_for_crystals(long_term_candidates)

        for crystal, score, reason in long_term_candidates.values():
            ranked_score = float(score)
            if context_story_scopes.intersection(crystal.story_scopes):
                ranked_score += _STORY_SCOPE_BOOST
            if context_semantic_tags.intersection(crystal.semantic_tags):
                ranked_score += _SEMANTIC_TAG_BOOST
            ranked_score += concept_boosts.get(crystal.id, 0.0)
            if crystal.crystal_type == "rule" and crystal.status == "active":
                ranked_score += _ACTIVE_RULE_BOOST
            if (
                crystal.crystal_type == "thought" or crystal.is_inferred
            ) and crystal.confidence < 0.5:
                ranked_score = max(0.0, ranked_score - _LOW_CONFIDENCE_THOUGHT_PENALTY)
            ranked_items.append(
                (
                    "long_term",
                    ranked_score,
                    crystal.id,
                    crystal,
                    str(reason),
                    concept_labels.get(crystal.id, ()),
                )
            )
        for memory, score in short_term_memories:
            ranked_items.append(("short_term", score, memory.id, memory, _SHORT_TERM_REASON, ()))

        source_preference = {"long_term": 0, "short_term": 1}
        ranked_items.sort(
            key=lambda item: (
                -item[1],
                source_preference[item[0]],
                item[2],
            )
        )

        results: list[RecallResult] = []
        for rank, (source, score, _item_id, payload, reason, labels) in enumerate(
            ranked_items[:limit],
            start=1,
        ):
            if source == "long_term":
                results.append(
                    RecallResult.long_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=reason,
                        concept_labels=labels,
                    )
                )
            else:
                results.append(
                    RecallResult.short_term(
                        payload,
                        rank=rank,
                        score=score,
                        reason=_SHORT_TERM_REASON,
                    )
                )

        with connect(self.config.database_path) as conn:
            session = self._require_active_session(conn, session_id)
            for result in results:
                if result.source != "long_term" or result.crystal is None:
                    # crystal_activations has no tier or short-term memory column, so
                    # short-term hits cannot be recorded without losing their tier.
                    continue
                conn.execute(
                    """
                    insert into crystal_activations(
                      crystal_id,
                      session_id,
                      recall_query,
                      rank,
                      score,
                      reason,
                      cycle_id,
                      created_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        result.crystal.id,
                        session_id,
                        query,
                        result.rank,
                        result.score,
                        result.reason,
                        session["cycle_id"],
                        now,
                    ),
                )
            conn.commit()

        return results

    def _search_short_term_memories(
        self,
        conn,
        *,
        session_id: int,
        query: str,
        limit: int,
    ) -> list[tuple[ShortTermMemoryRecord, float]]:
        expression = search_expression(query)
        lookup_values = _metadata_lookup_values(query)
        if not expression and not lookup_values:
            return []

        bounded_limit = min(limit, _MAX_SHORT_TERM_LIMIT)
        scored: dict[int, tuple[ShortTermMemoryRecord, float]] = {}
        if expression:
            rows = conn.execute(
                """
                select short_term_memories.*
                from short_term_memories_fts
                join short_term_memories
                  on short_term_memories.id = short_term_memories_fts.rowid
                where short_term_memories_fts match ?
                  and short_term_memories.session_id = ?
                  and short_term_memories.archived_at is null
                order by bm25(short_term_memories_fts), short_term_memories.id
                limit ?
                """,
                (expression, session_id, bounded_limit),
            ).fetchall()
            for index, row in enumerate(rows):
                memory = short_memory_from_row(conn, row)
                scored[memory.id] = (
                    memory,
                    _SHORT_TERM_BASE_SCORE - (index * _SHORT_TERM_RANK_STEP),
                )

        metadata_boosts = self._short_term_metadata_candidate_boosts(
            conn,
            session_id=session_id,
            lookup_values=lookup_values,
            limit=_MAX_SHORT_TERM_LIMIT,
        )
        if metadata_boosts:
            placeholders = ", ".join("?" for _ in metadata_boosts)
            rows = conn.execute(
                f"""
                select *
                from short_term_memories
                where id in ({placeholders})
                  and session_id = ?
                  and archived_at is null
                """,
                (*metadata_boosts, session_id),
            ).fetchall()
            for row in rows:
                memory = short_memory_from_row(conn, row)
                boost = metadata_boosts[memory.id]
                if boost <= 0:
                    continue
                _existing_memory, score = scored.get(
                    memory.id,
                    (memory, _SHORT_TERM_METADATA_BASE_SCORE),
                )
                scored[memory.id] = (memory, score + boost)

        return sorted(
            scored.values(),
            key=lambda item: (-item[1], item[0].id),
        )[:bounded_limit]

    def _short_term_metadata_candidate_boosts(
        self,
        conn,
        *,
        session_id: int,
        lookup_values: tuple[str, ...],
        limit: int,
    ) -> dict[int, float]:
        if not lookup_values:
            return {}

        boosts: dict[int, float] = {}
        self._add_short_term_side_table_boosts(
            conn,
            boosts,
            session_id=session_id,
            table="short_term_memory_language_tags",
            value_column="language_tag",
            lookup_values=lookup_values,
            boost=_SHORT_TERM_LANGUAGE_TAG_BOOST,
            limit=limit,
        )
        self._add_short_term_side_table_boosts(
            conn,
            boosts,
            session_id=session_id,
            table="short_term_memory_story_scopes",
            value_column="story_scope",
            lookup_values=lookup_values,
            boost=_SHORT_TERM_STORY_SCOPE_BOOST,
            limit=limit,
        )
        self._add_short_term_side_table_boosts(
            conn,
            boosts,
            session_id=session_id,
            table="short_term_memory_semantic_tags",
            value_column="semantic_tag",
            lookup_values=lookup_values,
            boost=_SHORT_TERM_SEMANTIC_TAG_BOOST,
            limit=limit,
        )
        self._add_short_term_scalar_boosts(
            conn,
            boosts,
            session_id=session_id,
            column="source_credibility",
            lookup_values=lookup_values,
            boost=_SHORT_TERM_SOURCE_CREDIBILITY_BOOST,
            limit=limit,
        )
        self._add_short_term_scalar_boosts(
            conn,
            boosts,
            session_id=session_id,
            column="rule_intent",
            lookup_values=lookup_values,
            boost=_SHORT_TERM_RULE_INTENT_BOOST,
            limit=limit,
        )
        return dict(sorted(boosts.items(), key=lambda item: (-item[1], item[0]))[:limit])

    def _add_short_term_side_table_boosts(
        self,
        conn,
        boosts: dict[int, float],
        *,
        session_id: int,
        table: str,
        value_column: str,
        lookup_values: tuple[str, ...],
        boost: float,
        limit: int,
    ) -> None:
        placeholders = ", ".join("?" for _ in lookup_values)
        rows = conn.execute(
            f"""
            select distinct m.id
            from {table} t
            join short_term_memories m on m.id = t.memory_id
            where m.session_id = ?
              and m.archived_at is null
              and t.{value_column} collate nocase in ({placeholders})
            order by m.id
            limit ?
            """,
            (session_id, *lookup_values, limit),
        ).fetchall()
        for row in rows:
            memory_id = int(row["id"])
            boosts[memory_id] = boosts.get(memory_id, 0.0) + boost

    def _add_short_term_scalar_boosts(
        self,
        conn,
        boosts: dict[int, float],
        *,
        session_id: int,
        column: str,
        lookup_values: tuple[str, ...],
        boost: float,
        limit: int,
    ) -> None:
        placeholders = ", ".join("?" for _ in lookup_values)
        rows = conn.execute(
            f"""
            select id
            from short_term_memories
            where session_id = ?
              and archived_at is null
              and {column} collate nocase in ({placeholders})
            order by id
            limit ?
            """,
            (session_id, *lookup_values, limit),
        ).fetchall()
        for row in rows:
            memory_id = int(row["id"])
            boosts[memory_id] = boosts.get(memory_id, 0.0) + boost

    def _long_term_metadata_candidate_scores(
        self,
        context: TranslationContext,
        query: str,
        *,
        limit: int,
    ) -> dict[int, float]:
        lookup_values = _metadata_lookup_values(query)
        if not lookup_values:
            return {}

        scores: dict[int, float] = {}
        with connect(self.config.database_path) as conn:
            self._add_long_term_side_table_scores(
                conn,
                scores,
                context=context,
                table="crystal_semantic_tags",
                value_column="tag",
                lookup_values=lookup_values,
                boost=_SEMANTIC_TAG_BOOST,
                limit=limit,
            )
            self._add_long_term_side_table_scores(
                conn,
                scores,
                context=context,
                table="crystal_story_scopes",
                value_column="scope",
                lookup_values=lookup_values,
                boost=_STORY_SCOPE_BOOST,
                limit=limit,
            )

            concept_scores = self._concept_candidate_scores(conn, query, lookup_values, limit=limit)
            if concept_scores:
                placeholders = ", ".join("?" for _ in concept_scores)
                rows = conn.execute(
                    f"""
                    select distinct
                      c.id,
                      c.strength,
                      c.confidence,
                      c.scope_type,
                      cc.concept_id
                    from crystal_concepts cc
                    join crystals c on c.id = cc.crystal_id
                    where cc.concept_id in ({placeholders})
                      and c.status in ('active', 'candidate')
                      and (
                        (c.scope_type = 'series' and c.scope_key = ?)
                        or c.scope_type = 'global'
                      )
                      and (c.source_language = ? or c.source_language = '')
                      and (c.target_language = ? or c.target_language = '')
                    order by c.id, cc.concept_id
                    limit ?
                    """,
                    (
                        *concept_scores,
                        context.scope_key,
                        context.source_language,
                        context.target_language,
                        limit,
                    ),
                ).fetchall()
                for row in rows:
                    self._add_long_term_metadata_score(
                        scores,
                        row,
                        concept_scores[int(row["concept_id"])],
                    )

        return dict(
            sorted(scores.items(), key=lambda item: (-item[1], item[0]))[:limit],
        )

    def _add_long_term_side_table_scores(
        self,
        conn,
        scores: dict[int, float],
        *,
        context: TranslationContext,
        table: str,
        value_column: str,
        lookup_values: tuple[str, ...],
        boost: float,
        limit: int,
    ) -> None:
        placeholders = ", ".join("?" for _ in lookup_values)
        rows = conn.execute(
            f"""
            select distinct c.id, c.strength, c.confidence, c.scope_type
            from {table} t
            join crystals c on c.id = t.crystal_id
            where c.status in ('active', 'candidate')
              and (
                (c.scope_type = 'series' and c.scope_key = ?)
                or c.scope_type = 'global'
              )
              and (c.source_language = ? or c.source_language = '')
              and (c.target_language = ? or c.target_language = '')
              and t.{value_column} collate nocase in ({placeholders})
            order by c.id
            limit ?
            """,
            (
                context.scope_key,
                context.source_language,
                context.target_language,
                *lookup_values,
                limit,
            ),
        ).fetchall()
        for row in rows:
            self._add_long_term_metadata_score(scores, row, boost)

    def _add_long_term_metadata_score(self, scores: dict[int, float], row, boost: float) -> None:
        score = (
            _metadata_search_score(
                float(row["strength"]),
                float(row["confidence"]),
                row["scope_type"],
            )
            + boost
        )
        scores[int(row["id"])] = max(scores.get(int(row["id"]), 0.0), score)

    def _concept_candidate_scores(
        self,
        conn,
        query: str,
        lookup_values: tuple[str, ...],
        *,
        limit: int,
    ) -> dict[int, float]:
        scores: dict[int, float] = {}
        expression = search_expression(query)
        exact_query = query.strip()

        rows = []
        if exact_query:
            rows = conn.execute(
                """
                select id
                from concepts
                where canonical_name = ? collate nocase
                  and status not in ('archived', 'merged')
                order by id
                limit ?
                """,
                (exact_query, limit),
            ).fetchall()
        for row in rows:
            concept_id = int(row["id"])
            scores[concept_id] = max(scores.get(concept_id, 0.0), _EXACT_CONCEPT_BOOST)

        if expression:
            rows = conn.execute(
                """
                select c.id
                from concepts_fts
                join concepts c on c.id = concepts_fts.rowid
                where concepts_fts match ?
                  and c.status not in ('archived', 'merged')
                order by c.id
                limit ?
                """,
                (expression, limit),
            ).fetchall()
            for row in rows:
                concept_id = int(row["id"])
                scores[concept_id] = max(scores.get(concept_id, 0.0), _CONCEPT_LINK_BASE_BOOST)

            rows = conn.execute(
                """
                select distinct c.id
                from concept_facet_fts
                join concept_facets f on f.id = concept_facet_fts.rowid
                join concepts c on c.id = f.concept_id
                where concept_facet_fts match ?
                  and f.superseded_at is null
                  and c.status not in ('archived', 'merged')
                order by c.id
                limit ?
                """,
                (expression, limit),
            ).fetchall()
            for row in rows:
                concept_id = int(row["id"])
                scores[concept_id] = max(scores.get(concept_id, 0.0), _EXACT_CONCEPT_BOOST)

        if lookup_values:
            placeholders = ", ".join("?" for _ in lookup_values)
            rows = conn.execute(
                f"""
                select distinct c.id
                from concept_semantic_tags t
                join concepts c on c.id = t.concept_id
                where c.status not in ('archived', 'merged')
                  and t.tag collate nocase in ({placeholders})
                order by c.id
                limit ?
                """,
                (*lookup_values, limit),
            ).fetchall()
            for row in rows:
                concept_id = int(row["id"])
                scores[concept_id] = max(scores.get(concept_id, 0.0), _SEMANTIC_TAG_BOOST)

            rows = conn.execute(
                f"""
                select distinct c.id
                from concept_facet_semantic_tags t
                join concept_facets f on f.id = t.facet_id
                join concepts c on c.id = f.concept_id
                where f.superseded_at is null
                  and c.status not in ('archived', 'merged')
                  and t.semantic_tag collate nocase in ({placeholders})
                order by c.id
                limit ?
                """,
                (*lookup_values, limit),
            ).fetchall()
            for row in rows:
                concept_id = int(row["id"])
                scores[concept_id] = max(scores.get(concept_id, 0.0), _SEMANTIC_TAG_BOOST)

            rows = conn.execute(
                f"""
                select distinct c.id
                from concept_facet_story_scopes s
                join concept_facets f on f.id = s.facet_id
                join concepts c on c.id = f.concept_id
                where f.superseded_at is null
                  and c.status not in ('archived', 'merged')
                  and s.story_scope collate nocase in ({placeholders})
                order by c.id
                limit ?
                """,
                (*lookup_values, limit),
            ).fetchall()
            for row in rows:
                concept_id = int(row["id"])
                scores[concept_id] = max(scores.get(concept_id, 0.0), _CONCEPT_LINK_BASE_BOOST)

        return dict(sorted(scores.items(), key=lambda item: (-item[1], item[0]))[:limit])

    def _concept_labels_for_crystals(
        self,
        crystal_ids: Iterable[int],
    ) -> dict[int, tuple[str, ...]]:
        clean_ids = tuple(sorted(set(crystal_ids)))
        if not clean_ids:
            return {}

        placeholders = ", ".join("?" for _ in clean_ids)
        labels: dict[int, list[str]] = {crystal_id: [] for crystal_id in clean_ids}
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select cc.crystal_id, c.canonical_name
                from crystal_concepts cc
                join concepts c on c.id = cc.concept_id
                where cc.crystal_id in ({placeholders})
                order by cc.crystal_id, cc.concept_id
                """,
                clean_ids,
            ).fetchall()
        for row in rows:
            labels[int(row["crystal_id"])].append(row["canonical_name"])
        return {
            crystal_id: tuple(crystal_labels)
            for crystal_id, crystal_labels in labels.items()
            if crystal_labels
        }

    def _require_active_session(self, conn, session_id: int):
        row = conn.execute(
            "select status, cycle_id from task_sessions where id = ?",
            (session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown session: {session_id}")
        if row["status"] != "active":
            raise ValueError("recall requires an active session")
        return row

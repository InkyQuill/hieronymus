from __future__ import annotations

import json
import sqlite3
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime

from hieronymus.concept_models import ConceptFacetRecord, ConceptRecord
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect

CONCEPT_CANDIDATE = "candidate"
CONCEPT_ESTABLISHED = "established"
CONCEPT_ARCHIVED = "archived"
CONCEPT_MERGED = "merged"

_LEGACY_PUBLIC_STATUSES = {
    "vague": CONCEPT_CANDIDATE,
    "solid": CONCEPT_ESTABLISHED,
}
_ALLOWED_PUBLIC_STATUSES = frozenset(
    {
        CONCEPT_CANDIDATE,
        CONCEPT_ESTABLISHED,
        CONCEPT_ARCHIVED,
        CONCEPT_MERGED,
    }
)


def _now() -> str:
    return datetime.now(UTC).isoformat()


@dataclass(frozen=True)
class StrictConceptProposal:
    id: int
    series_slug: str
    source_language: str
    target_language: str
    concept_text: str
    source_form: str
    canonical_rendering: str
    approved_variants: list[str]
    forbidden_variants: list[str]
    rationale: str
    status: str


def _json_array(values: list[str]) -> str:
    return json.dumps(values, ensure_ascii=False, sort_keys=True)


def _clamp_confidence(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def _public_status(status: str) -> str:
    return _LEGACY_PUBLIC_STATUSES.get(status, status)


def _storage_status(status: str) -> str:
    public_status = _public_status(status)
    if public_status not in _ALLOWED_PUBLIC_STATUSES:
        raise ValueError(f"unknown concept status: {status}")
    return public_status


def _status_filter_values(status: str) -> tuple[str, ...]:
    public_status = _storage_status(status)
    values = [public_status]
    if public_status == CONCEPT_CANDIDATE:
        values.append("vague")
    elif public_status == CONCEPT_ESTABLISHED:
        values.append("solid")
    return tuple(values)


def _is_inactive_status(status: str) -> bool:
    return _public_status(status) in {CONCEPT_ARCHIVED, CONCEPT_MERGED}


def _concept_status(
    confidence: float,
    *,
    linked_evidence_count: int = 0,
    current_status: str | None = None,
) -> str:
    if current_status is not None:
        public_current = _public_status(current_status)
        if public_current in {CONCEPT_ESTABLISHED, CONCEPT_ARCHIVED, CONCEPT_MERGED}:
            return public_current

    if (
        confidence >= ConceptStore.ESTABLISHED_CONFIDENCE
        and linked_evidence_count >= ConceptStore.ESTABLISHED_EVIDENCE_COUNT
    ):
        return CONCEPT_ESTABLISHED
    return CONCEPT_CANDIDATE


def _clean_tags(tags: Iterable[str]) -> tuple[str, ...]:
    return tuple(sorted({tag.strip() for tag in tags if tag.strip()}))


def _row_to_concept_record(conn: sqlite3.Connection, row: sqlite3.Row) -> ConceptRecord:
    tags = tuple(
        tag_row["tag"]
        for tag_row in conn.execute(
            """
            select tag
            from concept_semantic_tags
            where concept_id = ?
            order by tag
            """,
            (row["id"],),
        )
    )
    return ConceptRecord(
        id=int(row["id"]),
        canonical_name=row["canonical_name"],
        description=row["description"],
        status=_public_status(row["status"]),
        confidence=float(row["confidence"]),
        scope_type=row["scope_type"],
        scope_key=row["scope_key"],
        tags=tags,
        merged_into_concept_id=row["merged_into_concept_id"],
    )


def _row_to_facet_record(conn: sqlite3.Connection, row: sqlite3.Row) -> ConceptFacetRecord:
    story_scopes = tuple(
        scope_row["story_scope"]
        for scope_row in conn.execute(
            """
            select story_scope
            from concept_facet_story_scopes
            where facet_id = ?
            order by story_scope
            """,
            (row["id"],),
        )
    )
    semantic_tags = tuple(
        tag_row["semantic_tag"]
        for tag_row in conn.execute(
            """
            select semantic_tag
            from concept_facet_semantic_tags
            where facet_id = ?
            order by semantic_tag
            """,
            (row["id"],),
        )
    )
    return ConceptFacetRecord(
        id=int(row["id"]),
        concept_id=int(row["concept_id"]),
        language=row["language"],
        facet_type=row["facet_type"],
        value=row["value"],
        confidence=float(row["confidence"]),
        source_crystal_id=row["source_crystal_id"],
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        is_canonical=bool(row["is_canonical"]),
    )


def _row_to_proposal(row: sqlite3.Row) -> StrictConceptProposal:
    return StrictConceptProposal(
        id=int(row["id"]),
        series_slug=row["series_slug"],
        source_language=row["source_language"],
        target_language=row["target_language"],
        concept_text=row["concept_text"],
        source_form=row["source_form"],
        canonical_rendering=row["canonical_rendering"],
        approved_variants=list(json.loads(row["approved_variants_json"])),
        forbidden_variants=list(json.loads(row["forbidden_variants_json"])),
        rationale=row["rationale"],
        status=row["status"],
    )


def _series_slug_from_scope(scope_type: str, scope_key: str) -> str:
    if scope_type == "global":
        return ""
    return scope_key.removeprefix("series:")


class ConceptStore:
    ESTABLISHED_CONFIDENCE = 0.75
    SOLID_CONFIDENCE = ESTABLISHED_CONFIDENCE
    ESTABLISHED_EVIDENCE_COUNT = 2

    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def create_concept(
        self,
        canonical_name: str,
        *,
        description: str = "",
        status: str = CONCEPT_CANDIDATE,
        confidence: float = 0.2,
        scope_type: str = "global",
        scope_key: str = "",
        semantic_tags: Iterable[str] = (),
    ) -> ConceptRecord:
        name = canonical_name.strip()
        if not name:
            raise ValueError("concept canonical_name must not be empty")
        self._validate_scope(scope_type, scope_key)

        clean_description = description.strip()
        storage_status = _storage_status(status)
        now = _now()
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                insert into concepts(
                  canonical_name,
                  description,
                  scope_type,
                  scope_key,
                  status,
                  confidence,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    name,
                    clean_description,
                    scope_type,
                    scope_key,
                    storage_status,
                    _clamp_confidence(confidence),
                    now,
                    now,
                ),
            )
            concept_id = int(cursor.lastrowid)
            self._set_semantic_tags_with_connection(conn, concept_id, semantic_tags, now=now)
            conn.commit()
        return self.get(concept_id)

    def list_concepts(
        self,
        *,
        status: str | None = None,
        semantic_tag: str | None = None,
    ) -> list[ConceptRecord]:
        query = ["select distinct c.* from concepts c"]
        params: list[object] = []
        where: list[str] = []
        if semantic_tag is not None:
            clean_tag = semantic_tag.strip()
            if clean_tag:
                query.append("join concept_semantic_tags t on t.concept_id = c.id")
                where.append("t.tag = ?")
                params.append(clean_tag)
        if status is not None:
            values = _status_filter_values(status)
            placeholders = ", ".join("?" for _ in values)
            where.append(f"c.status in ({placeholders})")
            params.extend(values)
        if where:
            query.append("where " + " and ".join(where))
        query.append("order by c.id")

        with connect(self.config.database_path) as conn:
            rows = conn.execute("\n".join(query), params).fetchall()
            return [_row_to_concept_record(conn, row) for row in rows]

    def search(self, query: str) -> list[ConceptRecord]:
        clean_query = query.strip()
        if not clean_query:
            return []

        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select distinct c.*
                from concepts c
                left join concept_facets f
                  on f.concept_id = c.id
                 and f.superseded_at is null
                where c.status not in (?, ?)
                  and (
                    c.canonical_name = ? collate nocase
                    or f.value = ? collate nocase
                  )
                order by c.id
                """,
                (CONCEPT_ARCHIVED, CONCEPT_MERGED, clean_query, clean_query),
            ).fetchall()
            return [_row_to_concept_record(conn, row) for row in rows]

    def create_or_reinforce(
        self,
        canonical_name: str,
        *,
        description: str = "",
        tags: tuple[str, ...] = (),
        confidence_delta: float = 0.2,
        scope_type: str = "global",
        scope_key: str = "",
    ) -> int:
        name = canonical_name.strip()
        if not name:
            raise ValueError("concept canonical_name must not be empty")
        self._validate_scope(scope_type, scope_key)

        clean_description = description.strip()
        now = _now()
        with connect(self.config.database_path) as conn:
            concept_id = self._create_or_reinforce_with_connection(
                conn,
                name,
                description=clean_description,
                tags=tags,
                confidence_delta=confidence_delta,
                scope_type=scope_type,
                scope_key=scope_key,
                now=now,
            )
            conn.commit()
        return concept_id

    def _create_or_reinforce_with_connection(
        self,
        conn: sqlite3.Connection,
        canonical_name: str,
        *,
        description: str = "",
        tags: tuple[str, ...] = (),
        confidence_delta: float = 0.2,
        scope_type: str = "global",
        scope_key: str = "",
        now: str | None = None,
    ) -> int:
        name = canonical_name.strip()
        if not name:
            raise ValueError("concept canonical_name must not be empty")
        self._validate_scope(scope_type, scope_key)

        clean_description = description.strip()
        event_time = now or _now()
        clean_tags = _clean_tags(tags)
        row = self._reinforcement_target_with_connection(
            conn,
            name,
            scope_type=scope_type,
            scope_key=scope_key,
            tags=clean_tags,
        )
        if row is None:
            confidence = _clamp_confidence(confidence_delta)
            status = _concept_status(confidence)
            cursor = conn.execute(
                """
                insert into concepts(
                  canonical_name,
                  description,
                  scope_type,
                  scope_key,
                  status,
                  confidence,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    name,
                    clean_description,
                    scope_type,
                    scope_key,
                    status,
                    confidence,
                    event_time,
                    event_time,
                ),
            )
            concept_id = int(cursor.lastrowid)
        else:
            concept_id = int(row["id"])
            confidence = _clamp_confidence(float(row["confidence"]) + confidence_delta)
            linked_evidence_count = self._linked_evidence_count_with_connection(conn, concept_id)
            status = _concept_status(
                confidence,
                linked_evidence_count=linked_evidence_count,
                current_status=row["status"],
            )
            conn.execute(
                """
                update concepts
                set description = case when ? != '' then ? else description end,
                    confidence = ?,
                    status = ?,
                    updated_at = ?
                where id = ?
                """,
                (
                    clean_description,
                    clean_description,
                    confidence,
                    status,
                    event_time,
                    concept_id,
                ),
            )

        for tag in clean_tags:
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                values (?, ?, ?, ?)
                on conflict(concept_id, tag) do update set
                  confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                """,
                (concept_id, tag, confidence, event_time),
            )
        return concept_id

    def get(self, concept_id: int) -> ConceptRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from concepts where id = ?",
                (concept_id,),
            ).fetchone()
            if row is None:
                raise KeyError(f"unknown concept: {concept_id}")
            return _row_to_concept_record(conn, row)

    def add_facet(
        self,
        concept_id: int,
        value: str,
        *,
        language: str = "",
        facet_type: str = "alias",
        confidence: float = 0.2,
        source_crystal_id: int | None = None,
        is_canonical: bool = False,
        story_scopes: Iterable[str] = (),
        semantic_tags: Iterable[str] = (),
    ) -> ConceptFacetRecord:
        clean_value = value.strip()
        if not clean_value:
            raise ValueError("concept facet value must not be empty")
        now = _now()
        with connect(self.config.database_path) as conn:
            self._require_concept_with_connection(conn, concept_id)
            cursor = conn.execute(
                """
                insert into concept_facets(
                  concept_id,
                  language,
                  facet_type,
                  value,
                  source_crystal_id,
                  confidence,
                  is_canonical,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    concept_id,
                    language.strip(),
                    facet_type.strip() or "alias",
                    clean_value,
                    source_crystal_id,
                    _clamp_confidence(confidence),
                    int(is_canonical),
                    now,
                    now,
                ),
            )
            facet_id = int(cursor.lastrowid)
            self._set_facet_story_scopes_with_connection(conn, facet_id, story_scopes)
            self._set_facet_semantic_tags_with_connection(conn, facet_id, semantic_tags)
            if is_canonical:
                self._set_canonical_facet_with_connection(conn, concept_id, facet_id)
            conn.commit()
        return self._get_facet(facet_id)

    def list_facets(self, concept_id: int) -> list[ConceptFacetRecord]:
        with connect(self.config.database_path) as conn:
            self._require_concept_with_connection(conn, concept_id)
            rows = conn.execute(
                """
                select *
                from concept_facets
                where concept_id = ?
                  and superseded_at is null
                order by is_canonical desc, id
                """,
                (concept_id,),
            ).fetchall()
            return [_row_to_facet_record(conn, row) for row in rows]

    def set_canonical_facet(self, concept_id: int, facet_id: int) -> None:
        with connect(self.config.database_path) as conn:
            self._set_canonical_facet_with_connection(conn, concept_id, facet_id)
            conn.commit()

    def rename_concept(
        self,
        concept_id: int,
        new_label: str,
        *,
        source_crystal_id: int | None = None,
    ) -> ConceptRecord:
        clean_label = new_label.strip()
        if not clean_label:
            raise ValueError("concept canonical_name must not be empty")
        now = _now()
        with connect(self.config.database_path) as conn:
            row = self._require_concept_with_connection(conn, concept_id)
            old_label = row["canonical_name"]
            if old_label == clean_label:
                return _row_to_concept_record(conn, row)

            if not self._facet_value_exists_with_connection(conn, concept_id, old_label):
                conn.execute(
                    """
                    insert into concept_facets(
                      concept_id,
                      language,
                      facet_type,
                      value,
                      source_crystal_id,
                      confidence,
                      created_at,
                      updated_at
                    )
                    values (?, '', 'former_label', ?, ?, ?, ?, ?)
                    """,
                    (
                        concept_id,
                        old_label,
                        source_crystal_id,
                        float(row["confidence"]),
                        now,
                        now,
                    ),
                )
            conn.execute(
                """
                insert into concept_renames(concept_id, old_name, new_name, created_at)
                values (?, ?, ?, ?)
                """,
                (concept_id, old_label, clean_label, now),
            )
            conn.execute(
                """
                update concepts
                set canonical_name = ?,
                    updated_at = ?
                where id = ?
                """,
                (clean_label, now, concept_id),
            )
            conn.commit()
        return self.get(concept_id)

    def archive_concept(self, concept_id: int, reason: str) -> None:
        _ = reason.strip()
        now = _now()
        with connect(self.config.database_path) as conn:
            self._require_concept_with_connection(conn, concept_id)
            conn.execute(
                """
                update concepts
                set status = ?,
                    updated_at = ?
                where id = ?
                """,
                (CONCEPT_ARCHIVED, now, concept_id),
            )
            conn.commit()

    def merge_concepts(
        self,
        source_concept_id: int,
        target_concept_id: int,
        reason: str,
    ) -> None:
        if source_concept_id == target_concept_id:
            raise ValueError("source and target concepts must differ")
        _ = reason.strip()
        now = _now()
        with connect(self.config.database_path) as conn:
            source_row = self._require_concept_with_connection(conn, source_concept_id)
            target_row = self._require_concept_with_connection(conn, target_concept_id)
            if _is_inactive_status(target_row["status"]):
                raise ValueError("merge target concept must be active")
            if not self._facet_value_exists_with_connection(
                conn, source_concept_id, source_row["canonical_name"]
            ):
                self._ensure_facet_with_connection(
                    conn,
                    target_concept_id,
                    source_row["canonical_name"],
                    facet_type="former_label",
                    confidence=float(source_row["confidence"]),
                    now=now,
                )

            for row in conn.execute(
                """
                select crystal_id, link_type, confidence, created_at
                from crystal_concepts
                where concept_id = ?
                """,
                (source_concept_id,),
            ).fetchall():
                conn.execute(
                    """
                    insert into crystal_concepts(
                      crystal_id,
                      concept_id,
                      link_type,
                      confidence,
                      created_at
                    )
                    values (?, ?, ?, ?, ?)
                    on conflict(crystal_id, concept_id, link_type) do update set
                      confidence = max(crystal_concepts.confidence, excluded.confidence)
                    """,
                    (
                        row["crystal_id"],
                        target_concept_id,
                        row["link_type"],
                        row["confidence"],
                        row["created_at"],
                    ),
                )
            conn.execute("delete from crystal_concepts where concept_id = ?", (source_concept_id,))
            self._move_facets_to_target_with_connection(
                conn,
                source_concept_id,
                target_concept_id,
                now=now,
            )

            for row in conn.execute(
                """
                select tag, confidence, created_at
                from concept_semantic_tags
                where concept_id = ?
                """,
                (source_concept_id,),
            ).fetchall():
                conn.execute(
                    """
                    insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                    values (?, ?, ?, ?)
                    on conflict(concept_id, tag) do update set
                      confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                    """,
                    (target_concept_id, row["tag"], row["confidence"], row["created_at"]),
                )
            conn.execute(
                "delete from concept_semantic_tags where concept_id = ?",
                (source_concept_id,),
            )
            conn.execute(
                """
                update concepts
                set status = ?,
                    merged_into_concept_id = ?,
                    updated_at = ?
                where id = ?
                """,
                (CONCEPT_MERGED, target_concept_id, now, source_concept_id),
            )
            self._refresh_concept_status_with_connection(conn, target_concept_id, now=now)
            conn.commit()

    def set_semantic_tags(self, concept_id: int, tags: Iterable[str]) -> None:
        with connect(self.config.database_path) as conn:
            self._require_concept_with_connection(conn, concept_id)
            self._set_semantic_tags_with_connection(conn, concept_id, tags, now=_now())
            conn.commit()

    def link_crystal(
        self,
        crystal_id: int,
        concept_id: int,
        *,
        link_type: str,
        confidence: float,
    ) -> None:
        now = _now()
        with connect(self.config.database_path) as conn:
            concept_row = self._require_concept_with_connection(conn, concept_id)
            if _is_inactive_status(concept_row["status"]):
                raise ValueError("cannot link crystal to inactive concept")
            conn.execute(
                """
                insert into crystal_concepts(
                  crystal_id,
                  concept_id,
                  link_type,
                  confidence,
                  created_at
                )
                values (?, ?, ?, ?, ?)
                on conflict(crystal_id, concept_id, link_type) do update set
                  confidence = max(crystal_concepts.confidence, excluded.confidence)
                """,
                (crystal_id, concept_id, link_type, _clamp_confidence(confidence), now),
            )
            self._refresh_concept_status_with_connection(conn, concept_id, now=now)
            conn.commit()

    def concept_ids_for_crystal(self, crystal_id: int) -> tuple[int, ...]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select distinct concept_id
                from crystal_concepts
                where crystal_id = ?
                order by concept_id
                """,
                (crystal_id,),
            ).fetchall()
        return tuple(int(row["concept_id"]) for row in rows)

    def _validate_scope(self, scope_type: str, scope_key: str) -> None:
        ConceptRecord(
            id=0,
            canonical_name="scope check",
            description="",
            status=CONCEPT_CANDIDATE,
            confidence=0.0,
            scope_type=scope_type,
            scope_key=scope_key,
        )

    def _require_concept_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
    ) -> sqlite3.Row:
        row = conn.execute("select * from concepts where id = ?", (concept_id,)).fetchone()
        if row is None:
            raise KeyError(f"unknown concept: {concept_id}")
        return row

    def _get_facet(self, facet_id: int) -> ConceptFacetRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from concept_facets where id = ?", (facet_id,)).fetchone()
            if row is None:
                raise KeyError(f"unknown concept facet: {facet_id}")
            return _row_to_facet_record(conn, row)

    def _reinforcement_target_with_connection(
        self,
        conn: sqlite3.Connection,
        canonical_name: str,
        *,
        scope_type: str,
        scope_key: str,
        tags: tuple[str, ...],
    ) -> sqlite3.Row | None:
        rows = conn.execute(
            """
            select id, confidence, status
            from concepts
            where scope_type = ?
              and scope_key = ?
              and canonical_name = ?
              and status not in (?, ?)
            order by id
            """,
            (scope_type, scope_key, canonical_name, CONCEPT_ARCHIVED, CONCEPT_MERGED),
        ).fetchall()
        if len(rows) <= 1:
            return rows[0] if rows else None

        if not tags:
            return None

        requested_tags = set(tags)
        tagged_rows = [
            (row, set(self._semantic_tags_with_connection(conn, int(row["id"])))) for row in rows
        ]
        exact_matches = [row for row, row_tags in tagged_rows if row_tags == requested_tags]
        if len(exact_matches) == 1:
            return exact_matches[0]

        overlap_matches = [row for row, row_tags in tagged_rows if row_tags & requested_tags]
        if len(overlap_matches) == 1:
            return overlap_matches[0]

        return None

    def _semantic_tags_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
    ) -> tuple[str, ...]:
        return tuple(
            row["tag"]
            for row in conn.execute(
                """
                select tag
                from concept_semantic_tags
                where concept_id = ?
                order by tag
                """,
                (concept_id,),
            )
        )

    def _linked_evidence_count_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
    ) -> int:
        row = conn.execute(
            """
            select count(distinct crystal_id) as evidence_count
            from crystal_concepts
            where concept_id = ?
            """,
            (concept_id,),
        ).fetchone()
        return int(row["evidence_count"])

    def _refresh_concept_status_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        *,
        now: str,
    ) -> None:
        row = self._require_concept_with_connection(conn, concept_id)
        status = _concept_status(
            float(row["confidence"]),
            linked_evidence_count=self._linked_evidence_count_with_connection(conn, concept_id),
            current_status=row["status"],
        )
        if status != row["status"]:
            conn.execute(
                """
                update concepts
                set status = ?,
                    updated_at = ?
                where id = ?
                """,
                (status, now, concept_id),
            )

    def _set_semantic_tags_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        tags: Iterable[str],
        *,
        now: str,
    ) -> None:
        conn.execute("delete from concept_semantic_tags where concept_id = ?", (concept_id,))
        for tag in _clean_tags(tags):
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, created_at)
                values (?, ?, ?)
                """,
                (concept_id, tag, now),
            )

    def _set_facet_story_scopes_with_connection(
        self,
        conn: sqlite3.Connection,
        facet_id: int,
        story_scopes: Iterable[str],
    ) -> None:
        for story_scope in _clean_tags(story_scopes):
            conn.execute(
                """
                insert into concept_facet_story_scopes(facet_id, story_scope)
                values (?, ?)
                """,
                (facet_id, story_scope),
            )

    def _set_facet_semantic_tags_with_connection(
        self,
        conn: sqlite3.Connection,
        facet_id: int,
        semantic_tags: Iterable[str],
    ) -> None:
        for semantic_tag in _clean_tags(semantic_tags):
            conn.execute(
                """
                insert into concept_facet_semantic_tags(facet_id, semantic_tag)
                values (?, ?)
                """,
                (facet_id, semantic_tag),
            )

    def _set_canonical_facet_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        facet_id: int,
    ) -> None:
        facet = conn.execute(
            """
            select id
            from concept_facets
            where id = ?
              and concept_id = ?
              and superseded_at is null
            """,
            (facet_id, concept_id),
        ).fetchone()
        if facet is None:
            raise KeyError(f"unknown concept facet: {facet_id}")
        now = _now()
        conn.execute(
            """
            update concept_facets
            set is_canonical = 0,
                updated_at = ?
            where concept_id = ?
            """,
            (now, concept_id),
        )
        conn.execute(
            """
            update concept_facets
            set is_canonical = 1,
                updated_at = ?
            where id = ?
            """,
            (now, facet_id),
        )

    def _facet_value_exists_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        value: str,
    ) -> bool:
        row = conn.execute(
            """
            select 1
            from concept_facets
            where concept_id = ?
              and value = ?
              and superseded_at is null
            limit 1
            """,
            (concept_id, value),
        ).fetchone()
        return row is not None

    def _ensure_facet_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        value: str,
        *,
        facet_type: str,
        language: str = "",
        source_crystal_id: int | None = None,
        confidence: float = 0.2,
        now: str,
    ) -> int:
        clean_value = value.strip()
        clean_type = facet_type.strip() or "alias"
        clean_language = language.strip()
        existing = self._facet_by_identity_with_connection(
            conn,
            concept_id,
            value=clean_value,
            facet_type=clean_type,
            language=clean_language,
        )
        if existing is not None:
            return int(existing["id"])
        if self._facet_value_exists_with_connection(conn, concept_id, clean_value):
            row = conn.execute(
                """
                select id
                from concept_facets
                where concept_id = ?
                  and value = ?
                  and superseded_at is null
                order by id
                limit 1
                """,
                (concept_id, clean_value),
            ).fetchone()
            return int(row["id"])

        cursor = conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              source_crystal_id,
              confidence,
              created_at,
              updated_at
            )
            values (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                concept_id,
                clean_language,
                clean_type,
                clean_value,
                source_crystal_id,
                _clamp_confidence(confidence),
                now,
                now,
            ),
        )
        return int(cursor.lastrowid)

    def _move_facets_to_target_with_connection(
        self,
        conn: sqlite3.Connection,
        source_concept_id: int,
        target_concept_id: int,
        *,
        now: str,
    ) -> None:
        rows = conn.execute(
            """
            select *
            from concept_facets
            where concept_id = ?
              and superseded_at is null
            order by id
            """,
            (source_concept_id,),
        ).fetchall()
        for row in rows:
            existing = self._facet_by_identity_with_connection(
                conn,
                target_concept_id,
                value=row["value"],
                facet_type=row["facet_type"],
                language=row["language"],
            )
            if existing is None:
                conn.execute(
                    """
                    update concept_facets
                    set concept_id = ?,
                        is_canonical = 0,
                        updated_at = ?
                    where id = ?
                    """,
                    (target_concept_id, now, row["id"]),
                )
                continue

            self._copy_facet_metadata_with_connection(
                conn,
                source_facet_id=int(row["id"]),
                target_facet_id=int(existing["id"]),
            )
            conn.execute("delete from concept_facets where id = ?", (row["id"],))

    def _facet_by_identity_with_connection(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
        *,
        value: str,
        facet_type: str,
        language: str,
    ) -> sqlite3.Row | None:
        return conn.execute(
            """
            select *
            from concept_facets
            where concept_id = ?
              and value = ?
              and facet_type = ?
              and language = ?
              and superseded_at is null
            order by id
            limit 1
            """,
            (concept_id, value, facet_type, language),
        ).fetchone()

    def _copy_facet_metadata_with_connection(
        self,
        conn: sqlite3.Connection,
        *,
        source_facet_id: int,
        target_facet_id: int,
    ) -> None:
        conn.execute(
            """
            insert or ignore into concept_facet_story_scopes(facet_id, story_scope)
            select ?, story_scope
            from concept_facet_story_scopes
            where facet_id = ?
            """,
            (target_facet_id, source_facet_id),
        )
        conn.execute(
            """
            insert or ignore into concept_facet_semantic_tags(facet_id, semantic_tag)
            select ?, semantic_tag
            from concept_facet_semantic_tags
            where facet_id = ?
            """,
            (target_facet_id, source_facet_id),
        )


class ConceptProposalStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def create(
        self,
        *,
        dream_run_id: int | None,
        series_slug: str,
        source_language: str,
        target_language: str,
        concept_text: str,
        source_form: str,
        canonical_rendering: str,
        approved_variants: list[str] | None = None,
        forbidden_variants: list[str] | None = None,
        rationale: str = "",
    ) -> int:
        with connect(self.config.database_path) as conn:
            proposal_id = self._create_with_connection(
                conn,
                dream_run_id=dream_run_id,
                series_slug=series_slug,
                source_language=source_language,
                target_language=target_language,
                concept_text=concept_text,
                source_form=source_form,
                canonical_rendering=canonical_rendering,
                approved_variants=approved_variants,
                forbidden_variants=forbidden_variants,
                rationale=rationale,
            )
            conn.commit()
        return proposal_id

    def get(self, proposal_id: int) -> StrictConceptProposal:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (proposal_id,),
            ).fetchone()
        if row is None:
            raise KeyError(f"unknown concept proposal: {proposal_id}")
        return _row_to_proposal(row)

    def list_pending(self) -> list[StrictConceptProposal]:
        with connect(self.config.database_path) as conn:
            strict_rows = conn.execute(
                """
                select *
                from strict_concept_proposals
                where status = 'pending'
                order by id
                """
            ).fetchall()
            candidate_rows = conn.execute(
                """
                select id, canonical_name, description, scope_type, scope_key, status
                from concepts
                where status in ('candidate', 'vague')
                order by updated_at desc, id desc
                limit 50
                """
            ).fetchall()
            candidates = [
                self._candidate_concept_proposal_with_connection(conn, row)
                for row in candidate_rows
            ]
        return [*[_row_to_proposal(row) for row in strict_rows], *candidates]

    def approve(self, proposal_id: int) -> None:
        self._set_status(proposal_id, "approved")

    def reject(self, proposal_id: int) -> None:
        self._set_status(proposal_id, "rejected")

    def _create_with_connection(
        self,
        conn: sqlite3.Connection,
        *,
        dream_run_id: int | None,
        series_slug: str,
        source_language: str,
        target_language: str,
        concept_text: str,
        source_form: str,
        canonical_rendering: str,
        approved_variants: list[str] | None = None,
        forbidden_variants: list[str] | None = None,
        rationale: str = "",
    ) -> int:
        self._validate_required("source_language", source_language)
        self._validate_required("target_language", target_language)
        self._validate_required("concept_text", concept_text)
        self._validate_required("source_form", source_form)
        self._validate_required("canonical_rendering", canonical_rendering)
        approved = self._validate_variants("approved_variants", approved_variants)
        forbidden = self._validate_variants("forbidden_variants", forbidden_variants)
        now = _now()
        cursor = conn.execute(
            """
            insert into strict_concept_proposals(
              dream_run_id,
              series_slug,
              source_language,
              target_language,
              concept_text,
              source_form,
              canonical_rendering,
              approved_variants_json,
              forbidden_variants_json,
              rationale,
              status,
              created_at,
              updated_at
            )
            values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)
            """,
            (
                dream_run_id,
                series_slug,
                source_language,
                target_language,
                concept_text,
                source_form,
                canonical_rendering,
                _json_array(approved),
                _json_array(forbidden),
                rationale,
                now,
                now,
            ),
        )
        return int(cursor.lastrowid)

    def _set_status(self, proposal_id: int, status: str) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update strict_concept_proposals
                set status = ?,
                    updated_at = ?
                where id = ?
                """,
                (status, _now(), proposal_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown concept proposal: {proposal_id}")
            conn.commit()

    def _candidate_concept_proposal_with_connection(
        self,
        conn: sqlite3.Connection,
        row: sqlite3.Row,
    ) -> StrictConceptProposal:
        facet_rows = conn.execute(
            """
            select value
            from concept_facets
            where concept_id = ?
              and value != ''
              and superseded_at is null
            order by confidence desc, id
            """,
            (row["id"],),
        ).fetchall()
        facet_values = [facet["value"] for facet in facet_rows]
        semantic_tags = tuple(
            tag_row["tag"]
            for tag_row in conn.execute(
                """
                select tag
                from concept_semantic_tags
                where concept_id = ?
                order by tag
                """,
                (row["id"],),
            )
        )
        canonical_name = row["canonical_name"]
        canonical_rendering = facet_values[0] if facet_values else canonical_name
        rationale = row["description"]
        if semantic_tags:
            tag_note = f"Semantic tags: {', '.join(semantic_tags)}"
            rationale = f"{rationale}\n{tag_note}" if rationale else tag_note
        return StrictConceptProposal(
            id=_candidate_concept_proposal_id(int(row["id"])),
            series_slug=_series_slug_from_scope(row["scope_type"], row["scope_key"]),
            source_language="",
            target_language="",
            concept_text=canonical_name,
            source_form=canonical_name,
            canonical_rendering=canonical_rendering,
            approved_variants=facet_values,
            forbidden_variants=[],
            rationale=rationale,
            status=_public_status(row["status"]),
        )

    def _validate_required(self, field: str, value: str) -> None:
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"{field} must not be empty")

    def _validate_variants(self, field: str, values: list[str] | None) -> list[str]:
        if values is None:
            return []
        if not isinstance(values, list):
            raise ValueError(f"{field} must be a list")
        if not all(isinstance(value, str) for value in values):
            raise ValueError(f"{field} must contain only strings")
        return values


def _candidate_concept_proposal_id(concept_id: int) -> int:
    return -abs(concept_id)

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from datetime import UTC, datetime

from hieronymus.concept_models import ConceptRecord
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect


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


def _concept_status(confidence: float) -> str:
    return "solid" if confidence >= ConceptStore.SOLID_CONFIDENCE else "vague"


def _clean_tags(tags: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(sorted({tag.strip() for tag in tags if tag.strip()}))


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


class ConceptStore:
    SOLID_CONFIDENCE = 0.75

    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

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
            row = conn.execute(
                """
                select id, confidence
                from concepts
                where scope_type = ? and scope_key = ? and canonical_name = ?
                """,
                (scope_type, scope_key, name),
            ).fetchone()
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
                        now,
                        now,
                    ),
                )
                concept_id = int(cursor.lastrowid)
            else:
                concept_id = int(row["id"])
                confidence = _clamp_confidence(float(row["confidence"]) + confidence_delta)
                status = _concept_status(confidence)
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
                        now,
                        concept_id,
                    ),
                )

            for tag in _clean_tags(tags):
                conn.execute(
                    """
                    insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                    values (?, ?, ?, ?)
                    on conflict(concept_id, tag) do update set
                      confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                    """,
                    (concept_id, tag, confidence, now),
                )
            conn.commit()
        return concept_id

    def get(self, concept_id: int) -> ConceptRecord:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from concepts where id = ?",
                (concept_id,),
            ).fetchone()
            if row is None:
                raise KeyError(f"unknown concept: {concept_id}")
            tags = tuple(
                tag_row["tag"]
                for tag_row in conn.execute(
                    """
                    select tag
                    from concept_semantic_tags
                    where concept_id = ?
                    order by tag
                    """,
                    (concept_id,),
                )
            )

        return ConceptRecord(
            id=int(row["id"]),
            canonical_name=row["canonical_name"],
            description=row["description"],
            status=row["status"],
            confidence=float(row["confidence"]),
            scope_type=row["scope_type"],
            scope_key=row["scope_key"],
            tags=tags,
        )

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
            status="vague",
            confidence=0.0,
            scope_type=scope_type,
            scope_key=scope_key,
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
            rows = conn.execute(
                """
                select *
                from strict_concept_proposals
                where status = 'pending'
                order by id
                """
            ).fetchall()
        return [_row_to_proposal(row) for row in rows]

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

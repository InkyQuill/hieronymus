from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from datetime import UTC, datetime

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

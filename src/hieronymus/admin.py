from __future__ import annotations

import json
import sqlite3
from datetime import UTC, datetime

from hieronymus.admin_models import (
    ActionResult,
    AdminCrystalEditPayload,
    AdminDetail,
    AdminRow,
    AdminSnapshot,
    AdminStats,
    DreamReview,
    ProvenanceDetail,
)
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import DreamRunRecord, DreamService
from hieronymus.memory_models import TranslationContext
from hieronymus.service_manager import ServiceManager

ADMIN_VIEWS = (
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Dream Audits",
    "Audit Log",
)
_ADMIN_IMMEDIATE_EVENT_DELTAS = {
    "confirmed_by_user": (0.15, 0.20),
    "contradicted_by_user": (-0.20, -0.25),
    "deleted_by_user": (-0.50, -0.35),
}


def _rule_crystal_text(
    source_form: str,
    canonical_rendering: str,
    forbidden_variants: list[str],
) -> str:
    if forbidden_variants:
        return f"{source_form} is translated as {canonical_rendering}, not {forbidden_variants[0]}."
    return f"{source_form} is translated as {canonical_rendering}."


def _validate_rule_crystal_shape(
    *,
    canonical_rendering: str,
    approved_variants: list[str],
    forbidden_variants: list[str],
) -> None:
    if len(forbidden_variants) > 1:
        raise ValueError("rule crystals support at most one forbidden variant")
    if any(variant != canonical_rendering for variant in approved_variants):
        raise ValueError("approved variants that differ from canonical rendering are unsupported")


_ARCHIVE_STRENGTH_THRESHOLD = 0.05


class AdminStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def status_payload(self) -> dict[str, object]:
        return {
            "tui": "available",
            "views": list(ADMIN_VIEWS),
            "counts": self.stats().as_dict(),
            "service": ServiceManager(self.config).status(),
        }

    def provenance_for_crystal(self, crystal_id: int) -> ProvenanceDetail:
        with connect(self.config.database_path) as conn:
            crystal = self._get_crystal(conn, crystal_id)
            rows = conn.execute(
                """
                select
                  m.id,
                  m.session_id,
                  m.source_role,
                  m.kind,
                  m.text,
                  m.source_ref
                from crystal_sources as s
                join short_term_memories as m
                  on m.id = s.short_term_memory_id
                where s.crystal_id = ?
                order by m.id
                """,
                (crystal_id,),
            ).fetchall()
        return ProvenanceDetail(
            title=crystal["title"] or _excerpt(crystal["text"]),
            sources=[
                {
                    "id": str(row["id"]),
                    "session_id": str(row["session_id"]),
                    "source_role": row["source_role"],
                    "kind": row["kind"],
                    "text": row["text"],
                    "source_ref": row["source_ref"],
                }
                for row in rows
            ],
        )

    def recall_reasons_for_crystal(self, crystal_id: int) -> list[dict[str, str]]:
        with connect(self.config.database_path) as conn:
            self._get_crystal(conn, crystal_id)
            rows = conn.execute(
                """
                select session_id, recall_query, rank, score, reason
                from crystal_activations
                where crystal_id = ?
                order by id desc
                """,
                (crystal_id,),
            ).fetchall()
        return [
            {
                "session_id": str(row["session_id"]),
                "query": row["recall_query"],
                "rank": str(row["rank"]),
                "score": f"{float(row['score']):.3f}",
                "reason": row["reason"],
            }
            for row in rows
        ]

    def crystal_edit_payload(self, crystal_id: int) -> AdminCrystalEditPayload:
        with connect(self.config.database_path) as conn:
            row = self._get_crystal(conn, crystal_id)
        return AdminCrystalEditPayload(title=row["title"], text=row["text"])

    def add_crystal(
        self,
        *,
        series_slug: str,
        source_language: str,
        target_language: str,
        crystal_type: str,
        title: str,
        text: str,
        tags: tuple[str, ...] = (),
    ) -> int:
        context = TranslationContext(
            series_slug=series_slug,
            source_language=source_language,
            target_language=target_language,
            task_type="admin",
            tags=tags,
        )
        crystal_id = CrystalStore(self.config).add_crystal(
            context,
            crystal_type=crystal_type,
            title=title,
            text=text,
        )
        self._audit("add", "crystal", crystal_id, note="Added from admin TUI")
        return crystal_id

    def run_manual_dreaming(self) -> DreamRunRecord:
        run = DreamService(self.config, resolve_provider(self.config)).run_all(owner="admin")
        self._audit(
            "run",
            "dream",
            run.id,
            note=f"Manual dream run {run.cycle_id} with provider {run.provider}",
        )
        return run

    def dream_review(self, run_id: int) -> DreamReview:
        with connect(self.config.database_path) as conn:
            run = self._get_dream_run(conn, run_id)
            cycle_id = int(run["cycle_id"])
            source_sessions = [
                int(row["id"])
                for row in conn.execute(
                    """
                    select id
                    from task_sessions
                    where cycle_id = ?
                    order by id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            consumed_memories = [
                row["text"]
                for row in conn.execute(
                    """
                    select m.text
                    from short_term_memories as m
                    join task_sessions as s
                      on s.id = m.session_id
                    where s.cycle_id = ?
                    order by m.id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            created_crystals = [
                row["text"]
                for row in conn.execute(
                    """
                    select text
                    from crystals
                    where created_cycle = ?
                    order by id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            strict_proposals = [
                row["concept_text"]
                for row in conn.execute(
                    """
                    select concept_text
                    from strict_concept_proposals
                    where dream_run_id = ?
                    order by id
                    """,
                    (run_id,),
                ).fetchall()
            ]
            decayed_crystals = [
                row["label"]
                for row in conn.execute(
                    """
                    select coalesce(nullif(c.title, ''), c.text, m.crystal_id) as label
                    from memory_events as m
                    left join crystals as c
                      on c.id = m.crystal_id
                    where m.cycle_id = ?
                      and m.applied = 1
                      and m.event_type = 'cycle_decay'
                      and (m.strength_delta < 0 or m.confidence_delta < 0)
                    order by m.id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]

        failed_outputs = [run["error"]] if run["status"] == "failed" and run["error"] else []
        return DreamReview(
            run_id=run_id,
            source_sessions=source_sessions,
            consumed_memories=consumed_memories,
            created_crystals=created_crystals,
            updated_crystals=[],
            decayed_crystals=decayed_crystals,
            strict_proposals=strict_proposals,
            failed_outputs=failed_outputs,
            validation_errors=[],
        )

    def reinforce_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "confirmed_by_user",
                evidence=evidence,
            )
            self._audit_with_connection(conn, "reinforce", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "reinforce", "Crystal reinforced")

    def decay_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "contradicted_by_user",
                evidence=evidence,
            )
            self._audit_with_connection(conn, "decay", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "decay", "Crystal decayed")

    def edit_crystal(self, crystal_id: int, *, title: str, text: str) -> ActionResult:
        if not text.strip():
            raise ValueError("text must not be empty")
        now = self._now()
        with connect(self.config.database_path) as conn:
            before = self._get_crystal(conn, crystal_id)
            conn.execute(
                """
                update crystals
                set title = ?,
                    text = ?,
                    updated_at = ?
                where id = ?
                """,
                (title, text, now, crystal_id),
            )
            self._replace_crystal_fts(
                conn,
                crystal_id,
                old_title=before["title"],
                old_text=before["text"],
                title=title,
                text=text,
            )
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "edit",
                "crystal",
                crystal_id,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "edit", "Crystal edited")

    def deprecate_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(conn, "deprecate", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "deprecate", "Crystal deprecated")

    def delete_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before_json = self._row_json(self._get_crystal(conn, crystal_id))
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "deleted_by_user",
                evidence=evidence,
            )
            conn.execute(
                """
                update crystals
                set status = 'archived',
                    strength = 0,
                    confidence = 0,
                    updated_at = ?
                where id = ?
                """,
                (self._now(), crystal_id),
            )
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "delete",
                "crystal",
                crystal_id,
                note=evidence,
                before_json=before_json,
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "delete", "Crystal deleted")

    def supersede_crystal(
        self,
        crystal_id: int,
        *,
        replacement_id: int,
        evidence: str,
    ) -> ActionResult:
        if crystal_id == replacement_id:
            raise ValueError("crystal cannot supersede itself")
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            replacement = self._get_crystal(conn, replacement_id)
            self._validate_active_crystals([source, replacement])
            self._validate_crystal_contexts([source, replacement])
            conn.execute(
                """
                insert or ignore into crystal_links(
                  source_crystal_id,
                  target_crystal_id,
                  link_type
                )
                values (?, ?, 'supersedes')
                """,
                (replacement_id, crystal_id),
            )
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(conn, "supersede", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "supersede", "Crystal superseded")

    def merge_crystals(self, crystal_ids: list[int], *, title: str, text: str) -> int:
        if len(set(crystal_ids)) != len(crystal_ids):
            raise ValueError("crystal IDs must identify distinct crystals")
        if len(crystal_ids) < 2:
            raise ValueError("at least two distinct crystals are required")
        if not text.strip():
            raise ValueError("text must not be empty")
        now = self._now()
        with connect(self.config.database_path) as conn:
            rows = [self._get_crystal(conn, crystal_id) for crystal_id in crystal_ids]
            self._validate_active_crystals(rows)
            self._validate_crystal_contexts(rows)
            first = rows[0]
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
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                """,
                (
                    first["crystal_type"],
                    text,
                    title,
                    first["scope_type"],
                    first["scope_key"],
                    first["series_slug"],
                    first["source_language"],
                    first["target_language"],
                    first["tags_json"],
                    max(float(row["strength"]) for row in rows),
                    max(float(row["confidence"]) for row in rows),
                    now,
                    now,
                ),
            )
            merged_id = int(cursor.lastrowid)
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (merged_id, title, text),
            )
            for crystal_id in crystal_ids:
                conn.execute(
                    """
                    insert or ignore into crystal_links(
                      source_crystal_id,
                      target_crystal_id,
                      link_type
                    )
                    values (?, ?, 'merged_from')
                    """,
                    (merged_id, crystal_id),
                )
                self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(
                conn,
                "merge",
                "crystal",
                merged_id,
                before_json=json.dumps(crystal_ids),
                after_json=self._row_json(self._get_crystal(conn, merged_id)),
            )
            conn.commit()
        return merged_id

    def split_crystal(
        self,
        crystal_id: int,
        *,
        parts: list[tuple[str, str] | dict[str, str]],
    ) -> list[int]:
        if len(parts) < 2:
            raise ValueError("at least two parts are required")
        now = self._now()
        new_ids: list[int] = []
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            self._validate_active_crystals([source])
            for part in parts:
                title, text = self._split_part_title_text(part)
                if not isinstance(title, str):
                    raise ValueError("part title must be a string")
                if not isinstance(text, str):
                    raise ValueError("part text must be a string")
                if not text.strip():
                    raise ValueError("part text must not be empty")
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
                      status,
                      created_at,
                      updated_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                    """,
                    (
                        source["crystal_type"],
                        text,
                        title,
                        source["scope_type"],
                        source["scope_key"],
                        source["series_slug"],
                        source["source_language"],
                        source["target_language"],
                        source["tags_json"],
                        source["strength"],
                        source["confidence"],
                        now,
                        now,
                    ),
                )
                new_id = int(cursor.lastrowid)
                new_ids.append(new_id)
                conn.execute(
                    "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                    (new_id, title, text),
                )
                conn.execute(
                    """
                    insert or ignore into crystal_links(
                      source_crystal_id,
                      target_crystal_id,
                      link_type
                    )
                    values (?, ?, 'split_from')
                    """,
                    (new_id, crystal_id),
                )
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(
                conn,
                "split",
                "crystal",
                crystal_id,
                before_json=self._row_json(source),
                after_json=json.dumps(new_ids),
            )
            conn.commit()
        return new_ids

    def promote_local_lesson(self, crystal_id: int, *, evidence: str) -> int:
        now = self._now()
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            self._validate_promotable_lesson(source)
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
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, 'global', 'global', '', ?, ?, ?, ?, ?, 'candidate', ?, ?)
                """,
                (
                    source["crystal_type"],
                    source["text"],
                    source["title"],
                    source["source_language"],
                    source["target_language"],
                    source["tags_json"],
                    source["strength"],
                    source["confidence"],
                    now,
                    now,
                ),
            )
            promoted_id = int(cursor.lastrowid)
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (promoted_id, source["title"], source["text"]),
            )
            self._audit_with_connection(
                conn,
                "promote",
                "crystal",
                promoted_id,
                note=evidence,
                before_json=self._row_json(source),
                after_json=self._row_json(self._get_crystal(conn, promoted_id)),
            )
            conn.commit()
        return promoted_id

    def activate_global_lesson(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before = self._get_crystal(conn, crystal_id)
            self._validate_activatable_global_lesson(before)
            self._set_crystal_status(conn, crystal_id, "active")
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "activate",
                "crystal",
                crystal_id,
                note=evidence,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "activate", "Global lesson activated")

    def approve_proposal(self, proposal_id: int) -> int:
        now = self._now()
        with connect(self.config.database_path) as conn:
            proposal = self._get_proposal(conn, proposal_id)
            if proposal["status"] != "pending":
                raise ValueError("proposal must be pending")
            crystal_id = self._insert_rule_crystal_for_proposal(conn, proposal, now=now)
            conn.execute(
                """
                update strict_concept_proposals
                set status = 'approved',
                    updated_at = ?
                where id = ?
                """,
                (now, proposal_id),
            )
            self._audit_with_connection(
                conn,
                "approve",
                "strict_concept_proposal",
                proposal_id,
                before_json=self._row_json(proposal),
                after_json=json.dumps({"crystal_id": crystal_id}, sort_keys=True),
            )
            conn.commit()
        return crystal_id

    def reject_proposal(self, proposal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before = self._get_proposal(conn, proposal_id)
            if before["status"] != "pending":
                raise ValueError("proposal must be pending")
            conn.execute(
                """
                update strict_concept_proposals
                set status = 'rejected',
                    updated_at = ?
                where id = ?
                """,
                (self._now(), proposal_id),
            )
            after = self._get_proposal(conn, proposal_id)
            self._audit_with_connection(
                conn,
                "reject",
                "strict_concept_proposal",
                proposal_id,
                note=evidence,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult(
            "strict_concept_proposal",
            proposal_id,
            "reject",
            "Proposal rejected",
        )

    def stats(self) -> AdminStats:
        with connect(self.config.database_path) as conn:
            return AdminStats(
                series=self._count(conn, "series"),
                crystals=self._count(conn, "crystals"),
                lessons=self._count(
                    conn,
                    "crystals",
                    "where crystal_type = 'lesson'",
                ),
                short_term_memories=self._count(
                    conn,
                    "short_term_memories",
                    "where archived_at is null",
                ),
                sessions=self._count(conn, "task_sessions"),
                dream_runs=self._count(conn, "dream_runs"),
                pending_proposals=self._count(
                    conn,
                    "strict_concept_proposals",
                    "where status = 'pending'",
                ),
                audit_events=self._count_audit_events(conn),
            )

    def list_crystals(
        self,
        *,
        series_slug: str | None = None,
        crystal_type: str | None = None,
        status: str | None = None,
        tags: tuple[str, ...] = (),
        limit: int = 200,
    ) -> list[AdminRow]:
        clauses = []
        params: list[object] = []
        if series_slug is not None:
            clauses.append("series_slug = ?")
            params.append(series_slug)
        if crystal_type is not None:
            clauses.append("crystal_type = ?")
            params.append(crystal_type)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)

        where_sql = f"where {' and '.join(clauses)}" if clauses else ""
        bounded_limit = max(limit, 1)
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select *
                from crystals
                {where_sql}
                order by id
                """,
                params,
            ).fetchall()

        required_tags = set(tags)
        admin_rows = []
        for row in rows:
            row_tags = _json_tuple(row["tags_json"])
            if required_tags and not required_tags.issubset(row_tags):
                continue
            admin_rows.append(self._crystal_row(row, row_tags))
            if len(admin_rows) >= bounded_limit:
                break
        return admin_rows

    def snapshot(self, view: str, selected_id: int | str | None = None) -> AdminSnapshot:
        if view not in ADMIN_VIEWS:
            raise ValueError(f"unknown admin view: {view}")

        rows = self._rows_for_view(view)
        selected = self._select_row(rows, selected_id)
        detail = self._detail_for_view(view, selected)
        return AdminSnapshot(
            view=view,
            rows=rows,
            selected=selected,
            detail=detail,
            filters=[],
        )

    def _rows_for_view(self, view: str) -> list[AdminRow]:
        if view == "Concepts":
            return self._list_strict_terms(label_column="source_text")
        if view == "Renderings":
            return self._list_strict_terms(label_column="canonical_translation")
        if view == "Crystals":
            return self.list_crystals()
        if view == "Lessons":
            return self.list_crystals(crystal_type="lesson")
        if view == "Short-Term Sessions":
            return self._list_sessions()
        if view == "Dream Runs":
            return self._list_dream_runs()
        if view == "Dream Audits":
            return self._list_dream_audits()
        if view == "Proposals":
            return self._list_proposals()
        if view == "Audit Log":
            return self._list_audit_log()
        raise ValueError(f"unknown admin view: {view}")

    def _list_strict_terms(self, *, label_column: str) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_terms
                order by id
                limit 200
                """
            ).fetchall()
            result = []
            for row in rows:
                tag_rows = conn.execute(
                    """
                    select tag
                    from strict_term_tags
                    where term_id = ?
                    order by tag
                    """,
                    (row["id"],),
                ).fetchall()
                result.append(
                    AdminRow(
                        id=int(row["id"]),
                        kind=row["category"],
                        label=row[label_column],
                        status=row["status"],
                        scope=row["series_slug"],
                        language_pair=_language_pair(row),
                        tags=tuple(tag["tag"] for tag in tag_rows),
                    )
                )
        return result

    def _list_sessions(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from task_sessions
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["task_type"],
                label=_session_label(row),
                status=row["status"],
                scope=row["series_slug"],
                language_pair=_language_pair(row),
            )
            for row in rows
        ]

    def _list_dream_runs(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_runs
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["provider"],
                label=f"Cycle {row['cycle_id']}",
                status=row["status"],
                scope="global",
                language_pair="",
                quality_label=(
                    f"{row['created_crystal_count']} crystals / {row['proposal_count']} proposals"
                ),
            )
            for row in rows
        ]

    def _list_dream_audits(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_audit_entries
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind="dream audit",
                label=f"{row['event_type']}: {row['summary']}",
                status=row["severity"],
                scope=f"dream:{row['dream_run_id']}",
                language_pair="",
                quality_label=row["created_at"],
            )
            for row in rows
        ]

    def _list_proposals(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_concept_proposals
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind="strict concept",
                label=row["concept_text"],
                status=row["status"],
                scope=row["series_slug"],
                language_pair=_language_pair(row),
                quality_label=row["canonical_rendering"],
            )
            for row in rows
        ]

    def _list_audit_log(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            audit_table = self._audit_log_table(conn)
            if audit_table != "memory_events":
                rows = conn.execute(
                    f"""
                    select *
                    from {audit_table}
                    order by id desc
                    limit 200
                    """
                ).fetchall()
                return [
                    AdminRow(
                        id=int(row["id"]),
                        kind=_row_value(row, "action", _row_value(row, "event_type", "event")),
                        label=_row_value(row, "note", _row_value(row, "message", "")),
                        status=_row_value(row, "entity_type", _row_value(row, "status", "")),
                        scope=_row_value(
                            row, "entity_id", _row_value(row, "series_slug", "global")
                        ),
                        language_pair="",
                        quality_label=_row_value(row, "created_at", ""),
                    )
                    for row in rows
                ]
            rows = conn.execute(
                """
                select *
                from memory_events
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["event_type"],
                label=row["evidence"] or row["source_role"],
                status="applied" if row["applied"] else "pending",
                scope=f"session:{row['session_id']}" if row["session_id"] is not None else "global",
                language_pair="",
                quality_label=_event_quality(row),
            )
            for row in rows
        ]

    def _crystal_row(self, row: sqlite3.Row, tags: tuple[str, ...]) -> AdminRow:
        return AdminRow(
            id=int(row["id"]),
            kind=row["crystal_type"],
            label=row["title"] or _excerpt(row["text"]),
            status=row["status"],
            scope=row["series_slug"] or row["scope_key"] or row["scope_type"],
            language_pair=_language_pair(row),
            quality_label=(f"{_percent(row['confidence'])} conf / {_percent(row['strength'])} str"),
            tags=tags,
        )

    def _select_row(
        self,
        rows: list[AdminRow],
        selected_id: int | str | None,
    ) -> AdminRow | None:
        if not rows:
            return None
        if selected_id is None:
            return rows[0]
        normalized_id = str(selected_id)
        return next((row for row in rows if str(row.id) == normalized_id), rows[0])

    def _detail_for_view(self, view: str, selected: AdminRow | None) -> AdminDetail:
        if selected is None:
            return AdminDetail(title=view, subtitle="No rows", body="")
        if view in {"Crystals", "Lessons"}:
            return self._crystal_detail(int(selected.id))
        if view in {"Concepts", "Renderings"}:
            return self._strict_term_detail(int(selected.id))
        if view == "Short-Term Sessions":
            return self._session_detail(int(selected.id))
        if view == "Dream Runs":
            return self._dream_run_detail(int(selected.id))
        if view == "Dream Audits":
            return self._dream_audit_detail(int(selected.id))
        if view == "Proposals":
            return self._proposal_detail(int(selected.id))
        if view == "Audit Log":
            return AdminDetail(
                title=selected.label or selected.kind,
                subtitle=selected.status,
                body=selected.quality_label,
                fields=_row_fields(selected),
            )
        raise ValueError(f"unknown admin view: {view}")

    def _crystal_detail(self, crystal_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing crystal", subtitle="", body="")
        label = row["title"] or _excerpt(row["text"])
        body = f"{label}\n\n{row['text']}" if label else row["text"]
        return AdminDetail(
            title=label,
            subtitle=f"{row['crystal_type']} / {row['status']}",
            body=body,
            fields=(
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
                (
                    "Quality",
                    f"{_percent(row['confidence'])} conf / {_percent(row['strength'])} str",
                ),
            ),
        )

    def _strict_term_detail(self, term_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from strict_terms where id = ?", (term_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing term", subtitle="", body="")
        return AdminDetail(
            title=row["source_text"],
            subtitle=f"{row['category']} / {row['status']}",
            body=row["notes"],
            fields=(
                ("Rendering", row["canonical_translation"]),
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
            ),
        )

    def _session_detail(self, session_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from task_sessions where id = ?", (session_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing session", subtitle="", body="")
        return AdminDetail(
            title=_session_label(row),
            subtitle=f"{row['task_type']} / {row['status']}",
            body="",
            fields=(
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
                ("Cycle", str(row["cycle_id"] or "")),
            ),
        )

    def _dream_run_detail(self, run_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from dream_runs where id = ?", (run_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing dream run", subtitle="", body="")
        return AdminDetail(
            title=f"Cycle {row['cycle_id']}",
            subtitle=f"{row['provider']} / {row['status']}",
            body=row["error"],
            fields=(
                ("Inputs", str(row["input_count"])),
                ("Crystals", str(row["created_crystal_count"])),
                ("Proposals", str(row["proposal_count"])),
            ),
        )

    def _dream_audit_detail(self, audit_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from dream_audit_entries where id = ?",
                (audit_id,),
            ).fetchone()
        if row is None:
            return AdminDetail(title="Missing dream audit", subtitle="", body="")
        payload = json.loads(row["payload_json"])
        return AdminDetail(
            title=f"{row['event_type']}: {row['summary']}",
            subtitle=row["severity"],
            body=json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True),
            fields=(
                ("Dream run", str(row["dream_run_id"])),
                ("Phase run", "" if row["phase_run_id"] is None else str(row["phase_run_id"])),
                ("Severity", row["severity"]),
                ("Created", row["created_at"]),
            ),
        )

    def _proposal_detail(self, proposal_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (proposal_id,),
            ).fetchone()
        if row is None:
            return AdminDetail(title="Missing proposal", subtitle="", body="")
        return AdminDetail(
            title=row["concept_text"],
            subtitle=f"strict concept / {row['status']}",
            body=row["rationale"],
            fields=(
                ("Source form", row["source_form"]),
                ("Rendering", row["canonical_rendering"]),
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
            ),
        )

    def _set_crystal_status(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        status: str,
    ) -> None:
        cursor = conn.execute(
            """
            update crystals
            set status = ?,
                updated_at = ?
            where id = ?
            """,
            (status, self._now(), crystal_id),
        )
        if cursor.rowcount == 0:
            raise KeyError(f"unknown crystal: {crystal_id}")

    def _record_immediate_feedback(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        event_type: str,
        *,
        evidence: str,
    ) -> int:
        strength_delta, confidence_delta = _ADMIN_IMMEDIATE_EVENT_DELTAS[event_type]
        now = self._now()
        crystal = self._get_crystal(conn, crystal_id)
        cursor = conn.execute(
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
              created_at
            )
            values (?, null, ?, 'user', ?, ?, ?, 1, ?)
            """,
            (
                crystal_id,
                event_type,
                evidence,
                strength_delta,
                confidence_delta,
                now,
            ),
        )
        strength = _clamp_score(float(crystal["strength"]) + strength_delta)
        confidence = _clamp_score(float(crystal["confidence"]) + confidence_delta)
        status = crystal["status"]
        if event_type == "deleted_by_user" and strength < _ARCHIVE_STRENGTH_THRESHOLD:
            status = "archived"
        conn.execute(
            """
            update crystals
            set strength = ?,
                confidence = ?,
                status = ?,
                updated_at = ?
            where id = ?
            """,
            (strength, confidence, status, now, crystal_id),
        )
        return int(cursor.lastrowid)

    def _validate_crystal_contexts(self, rows: list[sqlite3.Row]) -> None:
        if len(rows) < 2:
            return
        first = rows[0]
        for row in rows[1:]:
            for column in (
                "series_slug",
                "source_language",
                "target_language",
                "crystal_type",
                "scope_type",
                "scope_key",
            ):
                if row[column] != first[column]:
                    raise ValueError(f"crystal {column} does not match")

    def _validate_active_crystals(self, rows: list[sqlite3.Row]) -> None:
        for row in rows:
            if row["status"] not in {"active", "candidate"}:
                raise ValueError("crystals must be active or candidate")

    def _validate_promotable_lesson(self, row: sqlite3.Row) -> None:
        if row["crystal_type"] != "lesson":
            raise ValueError("source crystal must be a lesson")
        if row["scope_type"] != "series":
            raise ValueError("source lesson must be series-scoped")
        if row["status"] in {"archived", "rejected"}:
            raise ValueError("source lesson must not be archived or rejected")

    def _validate_activatable_global_lesson(self, row: sqlite3.Row) -> None:
        if row["crystal_type"] != "lesson":
            raise ValueError("crystal must be a lesson")
        if row["scope_type"] != "global":
            raise ValueError("lesson must be global-scoped")
        if row["status"] != "candidate":
            raise ValueError("global lesson must be candidate")

    def _split_part_title_text(
        self,
        part: tuple[str, str] | dict[str, str],
    ) -> tuple[object, object]:
        if isinstance(part, tuple):
            if len(part) != 2:
                raise ValueError("part tuple must contain title and text")
            return part
        if isinstance(part, dict):
            return part.get("title", ""), part.get("text", "")
        raise ValueError("part must be a title/text tuple or dict")

    def _audit(
        self,
        action: str,
        entity_type: str,
        entity_id: int | str,
        *,
        note: str = "",
        before_json: str = "{}",
        after_json: str = "{}",
    ) -> None:
        with connect(self.config.database_path) as conn:
            self._audit_with_connection(
                conn,
                action,
                entity_type,
                entity_id,
                note=note,
                before_json=before_json,
                after_json=after_json,
            )
            conn.commit()

    def _audit_with_connection(
        self,
        conn: sqlite3.Connection,
        action: str,
        entity_type: str,
        entity_id: int | str,
        *,
        note: str = "",
        before_json: str = "{}",
        after_json: str = "{}",
    ) -> None:
        conn.execute(
            """
            insert into audit_log(
              action,
              entity_type,
              entity_id,
              note,
              before_json,
              after_json,
              created_at
            )
            values (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                action,
                entity_type,
                str(entity_id),
                note,
                before_json,
                after_json,
                self._now(),
            ),
        )

    def _row_json(self, row: sqlite3.Row) -> str:
        return json.dumps(
            {key: row[key] for key in row.keys()},
            ensure_ascii=False,
            sort_keys=True,
        )

    def _get_crystal(self, conn: sqlite3.Connection, crystal_id: int) -> sqlite3.Row:
        row = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        if row is None:
            raise KeyError(f"unknown crystal: {crystal_id}")
        return row

    def _get_proposal(self, conn: sqlite3.Connection, proposal_id: int) -> sqlite3.Row:
        row = conn.execute(
            "select * from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown concept proposal: {proposal_id}")
        return row

    def _get_dream_run(self, conn: sqlite3.Connection, run_id: int) -> sqlite3.Row:
        row = conn.execute("select * from dream_runs where id = ?", (run_id,)).fetchone()
        if row is None:
            raise KeyError(f"unknown dream run: {run_id}")
        return row

    def _replace_crystal_fts(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        *,
        old_title: str,
        old_text: str,
        title: str,
        text: str,
    ) -> None:
        conn.execute(
            "insert into crystals_fts(crystals_fts, rowid, title, text) values ('delete', ?, ?, ?)",
            (crystal_id, old_title, old_text),
        )
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
            (crystal_id, title, text),
        )

    def _insert_rule_crystal_for_proposal(
        self,
        conn: sqlite3.Connection,
        proposal: sqlite3.Row,
        *,
        now: str,
    ) -> int:
        approved_variants = [
            variant.strip()
            for variant in json.loads(proposal["approved_variants_json"])
            if isinstance(variant, str) and variant.strip()
        ]
        forbidden_variants = [
            variant.strip()
            for variant in json.loads(proposal["forbidden_variants_json"])
            if isinstance(variant, str) and variant.strip()
        ]
        _validate_rule_crystal_shape(
            canonical_rendering=proposal["canonical_rendering"],
            approved_variants=approved_variants,
            forbidden_variants=forbidden_variants,
        )
        text = _rule_crystal_text(
            proposal["source_form"].strip() or proposal["concept_text"],
            proposal["canonical_rendering"],
            forbidden_variants,
        )
        semantic_tags = ("strict-concept", "translation-rule")
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
            values ('rule', ?, ?, 'series', ?, ?, ?, ?, ?, 0.8, 0.95,
                    'user_rule', '', 0.0, null, 'active', ?, ?)
            """,
            (
                text,
                proposal["concept_text"],
                f"series:{proposal['series_slug']}",
                proposal["series_slug"],
                proposal["source_language"],
                proposal["target_language"],
                json.dumps(semantic_tags, ensure_ascii=False, sort_keys=True),
                now,
                now,
            ),
        )
        crystal_id = int(cursor.lastrowid)
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
            (crystal_id, proposal["concept_text"], text),
        )
        for tag in semantic_tags:
            conn.execute(
                """
                insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                values (?, ?, 0.95, ?)
                """,
                (crystal_id, tag, now),
            )
        return crystal_id

    def _insert_alias(
        self,
        conn: sqlite3.Connection,
        term_id: int,
        *,
        language: str,
        text: str,
        kind: str,
    ) -> None:
        if not text.strip():
            return
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
            values (?, ?, ?, ?, 1)
            """,
            (term_id, language, text, kind),
        )

    def _now(self) -> str:
        return datetime.now(UTC).isoformat()

    def _count(
        self,
        conn: sqlite3.Connection,
        table: str,
        where_sql: str = "",
    ) -> int:
        if not self._table_exists(conn, table):
            return 0
        row = conn.execute(f"select count(*) from {table} {where_sql}").fetchone()
        return int(row[0])

    def _count_audit_events(self, conn: sqlite3.Connection) -> int:
        return self._count(conn, self._audit_log_table(conn))

    def _audit_log_table(self, conn: sqlite3.Connection) -> str:
        for table in ("audit_log", "audit_events", "memory_events"):
            if self._table_exists(conn, table):
                return table
        return "memory_events"

    def _table_exists(self, conn: sqlite3.Connection, table: str) -> bool:
        row = conn.execute(
            """
            select 1
            from sqlite_master
            where type = 'table'
              and name = ?
            """,
            (table,),
        ).fetchone()
        return row is not None


def _json_tuple(value: str) -> tuple[str, ...]:
    loaded = json.loads(value)
    if not isinstance(loaded, list):
        return ()
    return tuple(str(item) for item in loaded)


def _clamp_score(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def _language_pair(row: sqlite3.Row) -> str:
    return f"{row['source_language']} -> {row['target_language']}"


def _percent(value: object) -> str:
    return f"{round(float(value) * 100):.0f}%"


def _excerpt(text: str, *, limit: int = 80) -> str:
    normalized = " ".join(text.split())
    if len(normalized) <= limit:
        return normalized
    return f"{normalized[: limit - 1]}..."


def _session_label(row: sqlite3.Row) -> str:
    parts = [row["series_slug"]]
    if row["volume"]:
        parts.append(f"v{row['volume']}")
    if row["chapter"]:
        parts.append(f"ch{row['chapter']}")
    return " / ".join(parts)


def _row_value(row: sqlite3.Row, key: str, default: str) -> str:
    if key not in row.keys():
        return default
    value = row[key]
    return default if value is None else str(value)


def _row_fields(row: AdminRow) -> tuple[tuple[str, str], ...]:
    return (
        ("Kind", row.kind),
        ("Scope", row.scope),
        ("Language", row.language_pair),
        ("Quality", row.quality_label),
    )


def _event_quality(row: sqlite3.Row) -> str:
    return f"{row['strength_delta']:+.2f} str / {row['confidence_delta']:+.2f} conf"

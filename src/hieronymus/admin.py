from __future__ import annotations

import json
import sqlite3

from hieronymus.admin_models import AdminDetail, AdminRow, AdminSnapshot, AdminStats
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.service_manager import ServiceManager

ADMIN_VIEWS = (
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Audit Log",
)


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
                        kind=_row_value(row, "event_type", "event"),
                        label=_row_value(row, "message", _row_value(row, "evidence", "")),
                        status=_row_value(row, "status", ""),
                        scope=_row_value(row, "series_slug", "global"),
                        language_pair="",
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

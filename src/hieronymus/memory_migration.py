from __future__ import annotations

import json
import sqlite3
from collections import Counter
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.rule_crystals import parse_rule_crystal


class Database(Protocol):
    database_path: Path


@dataclass(frozen=True)
class MemoryGraphMigrationReport:
    created: Mapping[str, int] = field(default_factory=dict)
    updated: Mapping[str, int] = field(default_factory=dict)
    skipped: Mapping[str, int] = field(default_factory=dict)
    pending: Mapping[str, int] = field(default_factory=dict)
    dry_run: bool = False

    def has_pending_work(self) -> bool:
        return any(count > 0 for count in self.pending.values())


def _now(conn: sqlite3.Connection) -> str:
    return str(conn.execute("select datetime('now')").fetchone()[0])


def _clean_tags(values: Iterable[str], *, lowercase: bool = False) -> tuple[str, ...]:
    cleaned: list[str] = []
    seen: set[str] = set()
    for value in values:
        item = str(value).strip()
        if lowercase:
            item = item.casefold()
        if not item or item in seen:
            continue
        cleaned.append(item)
        seen.add(item)
    return tuple(cleaned)


def _json_string_values(raw: str | None) -> tuple[str, ...]:
    if not raw:
        return ()
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        return _clean_tags(raw.split(","))
    if not isinstance(payload, list):
        return ()
    return _clean_tags(value for value in payload if isinstance(value, str))


def _rule_text(
    source_text: str,
    canonical_translation: str,
    forbidden_variants: tuple[str, ...],
) -> str:
    if forbidden_variants:
        return (
            f"{source_text} is translated as {canonical_translation}, not {forbidden_variants[0]}."
        )
    return f"{source_text} is translated as {canonical_translation}."


def _validate_rule_shape(
    *,
    source_text: str,
    canonical_translation: str,
    approved_variants: tuple[str, ...],
    forbidden_variants: tuple[str, ...],
) -> bool:
    if len(forbidden_variants) > 1:
        return False
    if any(variant != canonical_translation for variant in approved_variants):
        return False
    parsed = parse_rule_crystal(_rule_text(source_text, canonical_translation, forbidden_variants))
    return (
        parsed is not None
        and parsed.source_text == source_text
        and parsed.canonical_translation == canonical_translation
        and tuple(parsed.forbidden_variants) == forbidden_variants
    )


class MemoryGraphMigrator:
    def __init__(self, db: Database | HieronymusConfig | Path | str) -> None:
        self.database_path = _database_path(db)
        with connect(self.database_path) as conn:
            apply_migration(conn, "global.sql")

    def run(self) -> MemoryGraphMigrationReport:
        created: Counter[str] = Counter()
        updated: Counter[str] = Counter()
        skipped: Counter[str] = Counter()
        with connect(self.database_path) as conn:
            self._ensure_ledger(conn)
            self._migrate_series_languages(conn, updated)
            self._migrate_task_sessions(conn, updated)
            self._migrate_crystal_metadata(conn, updated)
            self._migrate_soft_origins(conn, updated)
            self._migrate_legacy_concept_statuses(conn, updated)
            self._migrate_strict_terms(conn, created, skipped)
            self._migrate_strict_concept_proposals(conn, created, skipped)
            conn.commit()

        return MemoryGraphMigrationReport(
            created=dict(created),
            updated=dict(updated),
            skipped=dict(skipped),
        )

    def dry_report(self) -> MemoryGraphMigrationReport:
        return self._dry_report_for_path(self.database_path)

    @classmethod
    def inspect(cls, db: Database | HieronymusConfig | Path | str) -> MemoryGraphMigrationReport:
        return cls._dry_report_for_path(_database_path(db))

    @classmethod
    def _dry_report_for_path(cls, database_path: Path) -> MemoryGraphMigrationReport:
        pending: Counter[str] = Counter()
        with sqlite3.connect(database_path) as conn:
            conn.row_factory = sqlite3.Row
            inspector = cls.__new__(cls)
            inspector.database_path = database_path
            inspector._count_pending_series_languages(conn, pending)
            inspector._count_pending_task_sessions(conn, pending)
            inspector._count_pending_crystal_metadata(conn, pending)
            inspector._count_pending_soft_origins(conn, pending)
            inspector._count_pending_legacy_concept_statuses(conn, pending)
            inspector._count_pending_generated_graph(conn, pending)
        return MemoryGraphMigrationReport(pending=dict(pending), dry_run=True)

    def _migrate_series_languages(
        self,
        conn: sqlite3.Connection,
        updated: Counter[str],
    ) -> None:
        if not _has_table(conn, "series") or not _has_table(conn, "series_language_tags"):
            return
        for row in conn.execute(
            "select id, default_source_language, default_target_language from series"
        ):
            for language_tag in _clean_tags(
                (row["default_source_language"], row["default_target_language"]),
                lowercase=True,
            ):
                updated["series_language_tags"] += _insert_ignore(
                    conn,
                    """
                    insert or ignore into series_language_tags(series_id, language_tag)
                    values (?, ?)
                    """,
                    (row["id"], language_tag),
                )

    def _migrate_task_sessions(
        self,
        conn: sqlite3.Connection,
        updated: Counter[str],
    ) -> None:
        if not _has_table(conn, "task_sessions"):
            return

        columns = _columns(conn, "task_sessions")
        rows = conn.execute("select * from task_sessions order by id").fetchall()
        for row in rows:
            if _has_table(conn, "task_session_language_tags"):
                for language_tag in _clean_tags(
                    (row["source_language"], row["target_language"]),
                    lowercase=True,
                ):
                    updated["task_session_language_tags"] += _insert_ignore(
                        conn,
                        """
                        insert or ignore into task_session_language_tags(session_id, language_tag)
                        values (?, ?)
                        """,
                        (row["id"], language_tag),
                    )

            if _has_table(conn, "task_session_story_scopes"):
                story_scopes = []
                if "volume" in columns and str(row["volume"]).strip():
                    story_scopes.append(f"volume:{str(row['volume']).strip()}")
                if "chapter" in columns and str(row["chapter"]).strip():
                    story_scopes.append(f"chapter:{str(row['chapter']).strip()}")
                for story_scope in _clean_tags(story_scopes):
                    updated["task_session_story_scopes"] += _insert_ignore(
                        conn,
                        """
                        insert or ignore into task_session_story_scopes(session_id, story_scope)
                        values (?, ?)
                        """,
                        (row["id"], story_scope),
                    )

            if _has_table(conn, "task_session_semantic_tags"):
                for semantic_tag in _legacy_session_tags(row, columns):
                    updated["task_session_semantic_tags"] += _insert_ignore(
                        conn,
                        """
                        insert or ignore into task_session_semantic_tags(session_id, semantic_tag)
                        values (?, ?)
                        """,
                        (row["id"], semantic_tag),
                    )

    def _migrate_crystal_metadata(
        self,
        conn: sqlite3.Connection,
        updated: Counter[str],
    ) -> None:
        if not _has_table(conn, "crystals"):
            return
        for row in conn.execute("select * from crystals order by id"):
            if _has_table(conn, "crystal_language_tags"):
                for language_tag in _clean_tags(
                    (row["source_language"], row["target_language"]),
                    lowercase=True,
                ):
                    updated["crystal_language_tags"] += _insert_ignore(
                        conn,
                        """
                        insert or ignore into crystal_language_tags(crystal_id, language_tag)
                        values (?, ?)
                        """,
                        (row["id"], language_tag),
                    )

            if _has_table(conn, "crystal_semantic_tags"):
                for tag in _json_string_values(row["tags_json"]):
                    updated["crystal_semantic_tags"] += _insert_ignore(
                        conn,
                        """
                        insert or ignore into crystal_semantic_tags(
                          crystal_id,
                          tag,
                          confidence,
                          created_at
                        )
                        values (?, ?, ?, ?)
                        """,
                        (row["id"], tag, row["confidence"], row["created_at"]),
                    )

    def _migrate_soft_origins(
        self,
        conn: sqlite3.Connection,
        updated: Counter[str],
    ) -> None:
        if _has_table(conn, "short_term_memories") and _has_columns(
            conn,
            "short_term_memories",
            {"source_ref", "soft_origin"},
        ):
            updated["short_term_memories.soft_origin"] += _execute_rowcount(
                conn,
                """
                update short_term_memories
                set soft_origin = source_ref
                where source_ref != ''
                  and (soft_origin is null or soft_origin = '')
                """,
            )

        if _has_table(conn, "crystals") and _has_columns(
            conn,
            "crystals",
            {"source_ref", "soft_origin"},
        ):
            updated["crystals.soft_origin"] += _execute_rowcount(
                conn,
                """
                update crystals
                set soft_origin = source_ref
                where source_ref != ''
                  and (soft_origin is null or soft_origin = '')
                """,
            )

    def _migrate_legacy_concept_statuses(
        self,
        conn: sqlite3.Connection,
        updated: Counter[str],
    ) -> None:
        if not _has_table(conn, "concepts"):
            return
        updated["concepts.status"] += _execute_rowcount(
            conn,
            "update concepts set status = 'candidate' where status = 'vague'",
        )
        updated["concepts.status"] += _execute_rowcount(
            conn,
            "update concepts set status = 'established' where status = 'solid'",
        )

    def _migrate_strict_terms(
        self,
        conn: sqlite3.Connection,
        created: Counter[str],
        skipped: Counter[str],
    ) -> None:
        if not _has_table(conn, "strict_terms"):
            return
        rows = conn.execute(
            """
            select *
            from strict_terms
            where status in ('approved', 'active')
            order by id
            """
        ).fetchall()
        for term in rows:
            tags = self._strict_term_tags(conn, int(term["id"]))
            forbidden = self._strict_term_forbidden_variants(conn, int(term["id"]))
            approved = self._strict_term_approved_variants(conn, int(term["id"]))
            if not _validate_rule_shape(
                source_text=term["source_text"],
                canonical_translation=term["canonical_translation"],
                approved_variants=approved,
                forbidden_variants=forbidden,
            ):
                skipped["strict_terms.unsupported_rule_shape"] += 1
                continue

            concept_id = self._ensure_concept(
                conn,
                source_table="strict_terms",
                source_id=str(term["id"]),
                canonical_name=term["source_text"],
                description=term["notes"],
                scope_key=f"series:{term['series_slug']}",
                status="established",
                confidence=0.95,
                semantic_tags=tags,
                created=created,
            )
            self._ensure_facet(
                conn,
                source_table="strict_terms",
                source_id=f"{term['id']}:source",
                concept_id=concept_id,
                value=term["source_text"],
                facet_type="name",
                language=term["source_language"],
                confidence=0.95,
                is_canonical=True,
                created=created,
            )
            self._ensure_facet(
                conn,
                source_table="strict_terms",
                source_id=f"{term['id']}:rendering",
                concept_id=concept_id,
                value=term["canonical_translation"],
                facet_type="rendering",
                language=term["target_language"],
                confidence=0.95,
                is_canonical=False,
                created=created,
            )
            crystal_id = self._ensure_rule_crystal(
                conn,
                source_table="strict_terms",
                source_id=str(term["id"]),
                title="",
                text=_rule_text(term["source_text"], term["canonical_translation"], forbidden),
                series_slug=term["series_slug"],
                source_language=term["source_language"],
                target_language=term["target_language"],
                status="active",
                strength=0.8,
                confidence=0.95,
                semantic_tags=tags,
                created=created,
            )
            self._ensure_crystal_concept_link(conn, crystal_id, concept_id, confidence=0.95)

    def _migrate_strict_concept_proposals(
        self,
        conn: sqlite3.Connection,
        created: Counter[str],
        skipped: Counter[str],
    ) -> None:
        if not _has_table(conn, "strict_concept_proposals"):
            return
        for proposal in conn.execute("select * from strict_concept_proposals order by id"):
            status = str(proposal["status"])
            if status == "rejected":
                skipped["strict_concept_proposals.rejected"] += 1
                continue
            source_form = str(proposal["source_form"]).strip() or proposal["concept_text"]
            approved = _json_string_values(proposal["approved_variants_json"])
            forbidden = _json_string_values(proposal["forbidden_variants_json"])
            semantic_tags = ("strict-concept", "translation-rule")
            concept_status = "established" if status == "approved" else "candidate"
            rule_status = "active" if status == "approved" else "candidate"
            confidence = 0.95 if status == "approved" else 0.45
            if not _validate_rule_shape(
                source_text=source_form,
                canonical_translation=proposal["canonical_rendering"],
                approved_variants=approved,
                forbidden_variants=forbidden,
            ):
                skipped["strict_concept_proposals.unsupported_rule_shape"] += 1
                continue

            concept_id = self._ensure_concept(
                conn,
                source_table="strict_concept_proposals",
                source_id=str(proposal["id"]),
                canonical_name=proposal["concept_text"],
                description=proposal["rationale"],
                scope_key=f"series:{proposal['series_slug']}",
                status=concept_status,
                confidence=confidence,
                semantic_tags=semantic_tags,
                created=created,
            )
            self._ensure_facet(
                conn,
                source_table="strict_concept_proposals",
                source_id=f"{proposal['id']}:source",
                concept_id=concept_id,
                value=source_form,
                facet_type="name",
                language=proposal["source_language"],
                confidence=confidence,
                is_canonical=True,
                created=created,
            )
            self._ensure_facet(
                conn,
                source_table="strict_concept_proposals",
                source_id=f"{proposal['id']}:rendering",
                concept_id=concept_id,
                value=proposal["canonical_rendering"],
                facet_type="rendering",
                language=proposal["target_language"],
                confidence=confidence,
                is_canonical=False,
                created=created,
            )
            crystal_id = self._ensure_rule_crystal(
                conn,
                source_table="strict_concept_proposals",
                source_id=str(proposal["id"]),
                title=proposal["concept_text"],
                text=_rule_text(source_form, proposal["canonical_rendering"], forbidden),
                series_slug=proposal["series_slug"],
                source_language=proposal["source_language"],
                target_language=proposal["target_language"],
                status=rule_status,
                strength=confidence,
                confidence=confidence,
                semantic_tags=semantic_tags,
                created=created,
            )
            self._ensure_crystal_concept_link(conn, crystal_id, concept_id, confidence=confidence)

    def _ensure_concept(
        self,
        conn: sqlite3.Connection,
        *,
        source_table: str,
        source_id: str,
        canonical_name: str,
        description: str,
        scope_key: str,
        status: str,
        confidence: float,
        semantic_tags: tuple[str, ...],
        created: Counter[str],
    ) -> int:
        existing = self._ledger_target(conn, source_table, source_id, "concepts")
        if existing is not None and _row_exists(conn, "concepts", existing):
            concept_id = existing
        else:
            natural = self._matching_concept(conn, canonical_name, scope_key, semantic_tags)
            if natural is None:
                now = _now(conn)
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
                    values (?, ?, 'series', ?, ?, ?, ?, ?)
                    """,
                    (canonical_name, description, scope_key, status, confidence, now, now),
                )
                concept_id = int(cursor.lastrowid)
                created["concepts"] += 1
            else:
                concept_id = natural
            self._record_ledger(conn, source_table, source_id, "concepts", concept_id)

        for tag in semantic_tags:
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                values (?, ?, ?, ?)
                on conflict(concept_id, tag) do update set
                  confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                """,
                (concept_id, tag, confidence, _now(conn)),
            )
        return concept_id

    def _ensure_facet(
        self,
        conn: sqlite3.Connection,
        *,
        source_table: str,
        source_id: str,
        concept_id: int,
        value: str,
        facet_type: str,
        language: str,
        confidence: float,
        is_canonical: bool,
        created: Counter[str],
    ) -> int:
        existing = self._ledger_target(conn, source_table, source_id, "concept_facets")
        if existing is not None and _row_exists(conn, "concept_facets", existing):
            facet_id = existing
        else:
            row = conn.execute(
                """
                select id
                from concept_facets
                where concept_id = ?
                  and value = ?
                  and facet_type = ?
                  and superseded_at is null
                order by id
                limit 1
                """,
                (concept_id, value, facet_type),
            ).fetchone()
            if row is None:
                now = _now(conn)
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
                    values (?, ?, ?, ?, null, ?, ?, ?, ?)
                    """,
                    (
                        concept_id,
                        language,
                        facet_type,
                        value,
                        confidence,
                        int(is_canonical),
                        now,
                        now,
                    ),
                )
                facet_id = int(cursor.lastrowid)
                created["concept_facets"] += 1
            else:
                facet_id = int(row["id"])
            self._record_ledger(conn, source_table, source_id, "concept_facets", facet_id)

        for language_tag in _clean_tags((language,), lowercase=True):
            conn.execute(
                """
                insert or ignore into concept_facet_language_tags(facet_id, language_tag)
                values (?, ?)
                """,
                (facet_id, language_tag),
            )
        return facet_id

    def _ensure_rule_crystal(
        self,
        conn: sqlite3.Connection,
        *,
        source_table: str,
        source_id: str,
        title: str,
        text: str,
        series_slug: str,
        source_language: str,
        target_language: str,
        status: str,
        strength: float,
        confidence: float,
        semantic_tags: tuple[str, ...],
        created: Counter[str],
    ) -> int:
        existing = self._ledger_target(conn, source_table, source_id, "crystals")
        if existing is not None and _row_exists(conn, "crystals", existing):
            crystal_id = existing
        else:
            scope_key = f"series:{series_slug}"
            row = conn.execute(
                """
                select id
                from crystals
                where crystal_type = 'rule'
                  and text = ?
                  and scope_type = 'series'
                  and scope_key = ?
                  and series_slug = ?
                  and source_language = ?
                  and target_language = ?
                  and status = ?
                order by id
                limit 1
                """,
                (text, scope_key, series_slug, source_language, target_language, status),
            ).fetchone()
            if row is None:
                now = _now(conn)
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
                    values ('rule', ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, 'user_rule',
                            '', 0.0, null, ?, ?, ?)
                    """,
                    (
                        text,
                        title,
                        scope_key,
                        series_slug,
                        source_language,
                        target_language,
                        json.dumps(semantic_tags, ensure_ascii=False, sort_keys=True),
                        strength,
                        confidence,
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
                created["crystals"] += 1
            else:
                crystal_id = int(row["id"])
            self._record_ledger(conn, source_table, source_id, "crystals", crystal_id)

        for language_tag in _clean_tags((source_language, target_language), lowercase=True):
            conn.execute(
                """
                insert or ignore into crystal_language_tags(crystal_id, language_tag)
                values (?, ?)
                """,
                (crystal_id, language_tag),
            )
        for tag in semantic_tags:
            conn.execute(
                """
                insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                values (?, ?, ?, ?)
                on conflict(crystal_id, tag) do update set
                  confidence = max(crystal_semantic_tags.confidence, excluded.confidence)
                """,
                (crystal_id, tag, confidence, _now(conn)),
            )
        return crystal_id

    def _ensure_crystal_concept_link(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        concept_id: int,
        *,
        confidence: float,
    ) -> None:
        conn.execute(
            """
            insert into crystal_concepts(
              crystal_id,
              concept_id,
              link_type,
              confidence,
              created_at
            )
            values (?, ?, 'defines', ?, ?)
            on conflict(crystal_id, concept_id, link_type) do update set
              confidence = max(crystal_concepts.confidence, excluded.confidence)
            """,
            (crystal_id, concept_id, confidence, _now(conn)),
        )

    def _strict_term_tags(self, conn: sqlite3.Connection, term_id: int) -> tuple[str, ...]:
        if not _has_table(conn, "strict_term_tags"):
            return ()
        return tuple(
            row["tag"]
            for row in conn.execute(
                """
                select tag
                from strict_term_tags
                where term_id = ?
                order by tag
                """,
                (term_id,),
            )
        )

    def _strict_term_forbidden_variants(
        self,
        conn: sqlite3.Connection,
        term_id: int,
    ) -> tuple[str, ...]:
        return self._strict_term_aliases(conn, term_id, "forbidden_variant")

    def _strict_term_approved_variants(
        self,
        conn: sqlite3.Connection,
        term_id: int,
    ) -> tuple[str, ...]:
        return self._strict_term_aliases(conn, term_id, "approved_variant")

    def _strict_term_aliases(
        self,
        conn: sqlite3.Connection,
        term_id: int,
        kind: str,
    ) -> tuple[str, ...]:
        if not _has_table(conn, "strict_term_aliases"):
            return ()
        return tuple(
            row["text"]
            for row in conn.execute(
                """
                select text
                from strict_term_aliases
                where term_id = ? and kind = ?
                order by id
                """,
                (term_id, kind),
            )
            if str(row["text"]).strip()
        )

    def _matching_concept(
        self,
        conn: sqlite3.Connection,
        canonical_name: str,
        scope_key: str,
        semantic_tags: tuple[str, ...],
    ) -> int | None:
        rows = conn.execute(
            """
            select id
            from concepts
            where canonical_name = ?
              and scope_type = 'series'
              and scope_key = ?
              and status not in ('archived', 'merged')
            order by id
            """,
            (canonical_name, scope_key),
        ).fetchall()
        if len(rows) == 1:
            return int(rows[0]["id"])
        if not rows or not semantic_tags:
            return None
        wanted = set(semantic_tags)
        scored = [
            (
                len(wanted.intersection(_concept_semantic_tags(conn, int(row["id"])))),
                int(row["id"]),
            )
            for row in rows
        ]
        best = max(score for score, _ in scored)
        if best <= 0:
            return None
        best_ids = [concept_id for score, concept_id in scored if score == best]
        return best_ids[0] if len(best_ids) == 1 else None

    def _ledger_target(
        self,
        conn: sqlite3.Connection,
        source_table: str,
        source_id: str,
        target_table: str,
    ) -> int | None:
        self._ensure_ledger(conn)
        row = conn.execute(
            """
            select target_id
            from memory_graph_migration_ledger
            where source_table = ?
              and source_id = ?
              and target_table = ?
            """,
            (source_table, source_id, target_table),
        ).fetchone()
        return None if row is None else int(row["target_id"])

    def _record_ledger(
        self,
        conn: sqlite3.Connection,
        source_table: str,
        source_id: str,
        target_table: str,
        target_id: int,
    ) -> None:
        self._ensure_ledger(conn)
        conn.execute(
            """
            insert into memory_graph_migration_ledger(
              source_table,
              source_id,
              target_table,
              target_id
            )
            values (?, ?, ?, ?)
            on conflict(source_table, source_id, target_table) do update set
              target_id = excluded.target_id
            """,
            (source_table, source_id, target_table, target_id),
        )

    def _ensure_ledger(self, conn: sqlite3.Connection) -> None:
        conn.execute(
            """
            create table if not exists memory_graph_migration_ledger (
              source_table text not null,
              source_id text not null,
              target_table text not null,
              target_id integer not null,
              created_at text not null default (datetime('now')),
              primary key (source_table, source_id, target_table)
            )
            """
        )

    def _count_pending_series_languages(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if _has_table(conn, "series") and _has_table(conn, "series_language_tags"):
            pending["series_language_tags"] = _pending_default_language_tags(conn, "series")

    def _count_pending_task_sessions(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if not _has_table(conn, "task_sessions"):
            return
        columns = _columns(conn, "task_sessions")
        for row in conn.execute("select * from task_sessions"):
            pending["task_session_language_tags"] += _missing_values(
                conn,
                "task_session_language_tags",
                "session_id",
                int(row["id"]),
                "language_tag",
                _clean_tags((row["source_language"], row["target_language"]), lowercase=True),
            )
            story_scopes = []
            if "volume" in columns and str(row["volume"]).strip():
                story_scopes.append(f"volume:{str(row['volume']).strip()}")
            if "chapter" in columns and str(row["chapter"]).strip():
                story_scopes.append(f"chapter:{str(row['chapter']).strip()}")
            pending["task_session_story_scopes"] += _missing_values(
                conn,
                "task_session_story_scopes",
                "session_id",
                int(row["id"]),
                "story_scope",
                _clean_tags(story_scopes),
            )
            pending["task_session_semantic_tags"] += _missing_values(
                conn,
                "task_session_semantic_tags",
                "session_id",
                int(row["id"]),
                "semantic_tag",
                _legacy_session_tags(row, columns),
            )

    def _count_pending_crystal_metadata(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if not _has_table(conn, "crystals"):
            return
        for row in conn.execute("select * from crystals"):
            pending["crystal_language_tags"] += _missing_values(
                conn,
                "crystal_language_tags",
                "crystal_id",
                int(row["id"]),
                "language_tag",
                _clean_tags((row["source_language"], row["target_language"]), lowercase=True),
            )
            pending["crystal_semantic_tags"] += _missing_values(
                conn,
                "crystal_semantic_tags",
                "crystal_id",
                int(row["id"]),
                "tag",
                _json_string_values(row["tags_json"]),
            )

    def _count_pending_soft_origins(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if _has_table(conn, "short_term_memories") and _has_columns(
            conn,
            "short_term_memories",
            {"source_ref", "soft_origin"},
        ):
            pending["short_term_memories.soft_origin"] = _scalar_count(
                conn,
                """
                select count(*)
                from short_term_memories
                where source_ref != ''
                  and (soft_origin is null or soft_origin = '')
                """,
            )

    def _count_pending_legacy_concept_statuses(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if _has_table(conn, "concepts"):
            pending["concepts.status"] = _scalar_count(
                conn,
                "select count(*) from concepts where status in ('vague', 'solid')",
            )

    def _count_pending_generated_graph(
        self,
        conn: sqlite3.Connection,
        pending: Counter[str],
    ) -> None:
        if _has_table(conn, "strict_terms"):
            if _has_table(conn, "memory_graph_migration_ledger"):
                pending["strict_terms"] = _scalar_count(
                    conn,
                    """
                    select count(*)
                    from strict_terms
                    where status in ('approved', 'active')
                      and not exists (
                        select 1
                        from memory_graph_migration_ledger ledger
                        where ledger.source_table = 'strict_terms'
                          and ledger.source_id = cast(strict_terms.id as text)
                          and ledger.target_table = 'crystals'
                      )
                    """,
                )
            else:
                pending["strict_terms"] = _scalar_count(
                    conn,
                    "select count(*) from strict_terms where status in ('approved', 'active')",
                )
        if _has_table(conn, "strict_concept_proposals"):
            if _has_table(conn, "memory_graph_migration_ledger"):
                pending["strict_concept_proposals"] = _scalar_count(
                    conn,
                    """
                    select count(*)
                    from strict_concept_proposals
                    where status != 'rejected'
                      and not exists (
                        select 1
                        from memory_graph_migration_ledger ledger
                        where ledger.source_table = 'strict_concept_proposals'
                          and ledger.source_id = cast(strict_concept_proposals.id as text)
                          and ledger.target_table = 'concepts'
                      )
                    """,
                )
            else:
                pending["strict_concept_proposals"] = _scalar_count(
                    conn,
                    "select count(*) from strict_concept_proposals where status != 'rejected'",
                )


def _database_path(db: Database | HieronymusConfig | Path | str) -> Path:
    if isinstance(db, str):
        return Path(db)
    if isinstance(db, Path):
        return db
    return Path(db.database_path)


def _has_table(conn: sqlite3.Connection, table: str) -> bool:
    row = conn.execute(
        "select 1 from sqlite_master where type = 'table' and name = ?",
        (table,),
    ).fetchone()
    return row is not None


def _columns(conn: sqlite3.Connection, table: str) -> set[str]:
    if not _has_table(conn, table):
        return set()
    return {row["name"] for row in conn.execute(f"pragma table_info({table})")}


def _has_columns(conn: sqlite3.Connection, table: str, columns: set[str]) -> bool:
    return columns <= _columns(conn, table)


def _row_exists(conn: sqlite3.Connection, table: str, row_id: int) -> bool:
    row = conn.execute(f"select 1 from {table} where id = ?", (row_id,)).fetchone()
    return row is not None


def _concept_semantic_tags(conn: sqlite3.Connection, concept_id: int) -> tuple[str, ...]:
    return tuple(
        row["tag"]
        for row in conn.execute(
            "select tag from concept_semantic_tags where concept_id = ? order by tag",
            (concept_id,),
        )
    )


def _insert_ignore(conn: sqlite3.Connection, sql: str, params: tuple[object, ...]) -> int:
    cursor = conn.execute(sql, params)
    return max(cursor.rowcount, 0)


def _execute_rowcount(
    conn: sqlite3.Connection,
    sql: str,
    params: tuple[object, ...] = (),
) -> int:
    cursor = conn.execute(sql, params)
    return max(cursor.rowcount, 0)


def _legacy_session_tags(row: sqlite3.Row, columns: set[str]) -> tuple[str, ...]:
    for column in ("tags_json", "semantic_tags_json", "tags"):
        if column in columns:
            return _json_string_values(row[column])
    return ()


def _missing_values(
    conn: sqlite3.Connection,
    table: str,
    owner_column: str,
    owner_id: int,
    value_column: str,
    values: tuple[str, ...],
) -> int:
    if not values or not _has_table(conn, table):
        return 0
    return sum(
        1
        for value in values
        if conn.execute(
            f"select 1 from {table} where {owner_column} = ? and {value_column} = ?",
            (owner_id, value),
        ).fetchone()
        is None
    )


def _pending_default_language_tags(conn: sqlite3.Connection, table: str) -> int:
    total = 0
    for row in conn.execute(
        f"select id, default_source_language, default_target_language from {table}"
    ):
        total += _missing_values(
            conn,
            "series_language_tags",
            "series_id",
            int(row["id"]),
            "language_tag",
            _clean_tags(
                (row["default_source_language"], row["default_target_language"]),
                lowercase=True,
            ),
        )
    return total


def _scalar_count(conn: sqlite3.Connection, sql: str) -> int:
    return int(conn.execute(sql).fetchone()[0])

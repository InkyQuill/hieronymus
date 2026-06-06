from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import ContractTerm


def _now() -> str:
    return datetime.now(UTC).isoformat()


class Termbase:
    def __init__(self, database_path: Path) -> None:
        self.database_path = database_path

    def propose(
        self,
        *,
        category: str,
        source_text: str,
        canonical_translation: str,
        tags: list[str] | None = None,
        notes: str = "",
    ) -> int:
        now = _now()
        with connect(self.database_path) as conn:
            cursor = conn.execute(
                """
                insert into terms(
                  category,
                  source_text,
                  canonical_translation,
                  status,
                  scope,
                  notes,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, 'pending', 'series', ?, ?, ?)
                """,
                (category, source_text, canonical_translation, notes, now, now),
            )
            term_id = int(cursor.lastrowid)
            for tag in tags or []:
                conn.execute("insert into term_tags(term_id, tag) values (?, ?)", (term_id, tag))
            conn.execute(
                "insert into terms_fts(rowid, source_text, canonical_translation, notes) "
                "values (?, ?, ?, ?)",
                (term_id, source_text, canonical_translation, notes),
            )
            conn.commit()
        return term_id

    def approve(self, term_id: int) -> None:
        with connect(self.database_path) as conn:
            conn.execute(
                "update terms set status = 'approved', updated_at = ? where id = ?",
                (_now(), term_id),
            )
            conn.commit()

    def add_alias(
        self,
        term_id: int,
        *,
        kind: str,
        text: str,
        language: str,
        case_sensitive: bool = True,
    ) -> None:
        with connect(self.database_path) as conn:
            conn.execute(
                """
                insert into term_aliases(term_id, language, text, kind, case_sensitive)
                values (?, ?, ?, ?, ?)
                """,
                (term_id, language, text, kind, int(case_sensitive)),
            )
            conn.commit()

    def contract(self, raw_text: str) -> list[ContractTerm]:
        with connect(self.database_path) as conn:
            rows = conn.execute(
                "select * from terms where status = 'approved' order by id"
            ).fetchall()
            result: list[ContractTerm] = []
            for row in rows:
                source_forms = [row["source_text"]]
                alias_rows = conn.execute(
                    "select * from term_aliases where term_id = ?",
                    (row["id"],),
                ).fetchall()
                source_forms.extend(
                    alias["text"] for alias in alias_rows if alias["kind"] == "source_variant"
                )
                if not any(form in raw_text for form in source_forms):
                    continue
                tags = [
                    tag_row["tag"]
                    for tag_row in conn.execute(
                        "select tag from term_tags where term_id = ? order by tag",
                        (row["id"],),
                    )
                ]
                forbidden = [
                    alias["text"] for alias in alias_rows if alias["kind"] == "forbidden_variant"
                ]
                result.append(
                    ContractTerm(
                        id=row["id"],
                        category=row["category"],
                        source_text=row["source_text"],
                        canonical_translation=row["canonical_translation"],
                        forbidden_variants=forbidden,
                        tags=tags,
                        notes=row["notes"],
                    )
                )
        return result

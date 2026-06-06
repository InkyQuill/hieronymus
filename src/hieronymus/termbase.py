from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import ContractTerm

_VALID_ALIAS_KINDS = frozenset(
    {
        "source_variant",
        "approved_variant",
        "forbidden_variant",
        "search_alias",
    }
)


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _contains(raw_text: str, text: str, *, case_sensitive: bool) -> bool:
    if case_sensitive:
        return text in raw_text
    return text.casefold() in raw_text.casefold()


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
        if not source_text.strip():
            raise ValueError("source_text must not be empty")
        if not canonical_translation.strip():
            raise ValueError("canonical_translation must not be empty")

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
            cursor = conn.execute(
                "update terms set status = 'approved', updated_at = ? where id = ?",
                (_now(), term_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown term: {term_id}")
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
        if kind not in _VALID_ALIAS_KINDS:
            raise ValueError(f"unknown alias kind: {kind}")
        if not text.strip():
            raise ValueError("alias text must not be empty")

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
                alias_rows = conn.execute(
                    "select * from term_aliases where term_id = ? order by id",
                    (row["id"],),
                ).fetchall()
                source_variant_aliases = [
                    alias for alias in alias_rows if alias["kind"] == "source_variant"
                ]
                if row["source_text"] not in raw_text and not any(
                    _contains(
                        raw_text,
                        alias["text"],
                        case_sensitive=bool(alias["case_sensitive"]),
                    )
                    for alias in source_variant_aliases
                ):
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

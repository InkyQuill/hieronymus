from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

from hieronymus.db import connect
from hieronymus.models import ContractTerm, ValidationFinding

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
    def __init__(
        self,
        database_path: Path,
        *,
        series_slug: str,
        source_language: str,
        target_language: str,
    ) -> None:
        self.database_path = database_path
        self.series_slug = series_slug
        self.source_language = source_language
        self.target_language = target_language

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
                insert into strict_terms(
                  series_slug,
                  source_language,
                  target_language,
                  category,
                  source_text,
                  canonical_translation,
                  status,
                  notes,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)
                """,
                (
                    self.series_slug,
                    self.source_language,
                    self.target_language,
                    category,
                    source_text,
                    canonical_translation,
                    notes,
                    now,
                    now,
                ),
            )
            term_id = int(cursor.lastrowid)
            for tag in tags or []:
                conn.execute(
                    "insert into strict_term_tags(term_id, tag) values (?, ?)",
                    (term_id, tag),
                )
            conn.execute(
                "insert into strict_terms_fts(rowid, source_text, canonical_translation, notes) "
                "values (?, ?, ?, ?)",
                (term_id, source_text, canonical_translation, notes),
            )
            conn.commit()
        return term_id

    def approve(self, term_id: int) -> None:
        with connect(self.database_path) as conn:
            cursor = conn.execute(
                """
                update strict_terms
                set status = 'approved', updated_at = ?
                where id = ? and series_slug = ?
                """,
                (_now(), term_id, self.series_slug),
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
            term = conn.execute(
                """
                select id
                from strict_terms
                where id = ?
                  and series_slug = ?
                  and source_language = ?
                  and target_language = ?
                """,
                (term_id, self.series_slug, self.source_language, self.target_language),
            ).fetchone()
            if term is None:
                raise KeyError(f"unknown term: {term_id}")
            conn.execute(
                """
                insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
                values (?, ?, ?, ?, ?)
                """,
                (term_id, language, text, kind, int(case_sensitive)),
            )
            conn.commit()

    def contract(self, raw_text: str) -> list[ContractTerm]:
        with connect(self.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_terms
                where status = 'approved' and series_slug = ?
                order by id
                """,
                (self.series_slug,),
            ).fetchall()
            result: list[ContractTerm] = []
            for row in rows:
                alias_rows = conn.execute(
                    "select * from strict_term_aliases where term_id = ? order by id",
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
                        "select tag from strict_term_tags where term_id = ? order by tag",
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

    def validate(self, *, raw_text: str, translated_text: str) -> list[ValidationFinding]:
        findings: list[ValidationFinding] = []
        contracted_terms = self.contract(raw_text)
        with connect(self.database_path) as conn:
            for term in contracted_terms:
                forbidden_rows = conn.execute(
                    """
                    select text, case_sensitive
                    from strict_term_aliases
                    where term_id = ? and kind = 'forbidden_variant'
                    order by id
                    """,
                    (term.id,),
                ).fetchall()
                for forbidden in forbidden_rows:
                    alias_text = forbidden["text"]
                    if not _contains(
                        translated_text,
                        alias_text,
                        case_sensitive=bool(forbidden["case_sensitive"]),
                    ):
                        continue
                    findings.append(
                        ValidationFinding(
                            term_id=term.id,
                            kind="forbidden_variant",
                            severity="high",
                            expected=term.canonical_translation,
                            observed=alias_text,
                            message=(
                                f"Use {term.canonical_translation!r} for {term.source_text!r}; "
                                f"{alias_text!r} is forbidden."
                            ),
                        )
                    )
                approved_variant_rows = conn.execute(
                    """
                    select text, case_sensitive
                    from strict_term_aliases
                    where term_id = ? and kind = 'approved_variant'
                    order by id
                    """,
                    (term.id,),
                ).fetchall()
                has_approved_form = term.canonical_translation in translated_text or any(
                    _contains(
                        translated_text,
                        approved_variant["text"],
                        case_sensitive=bool(approved_variant["case_sensitive"]),
                    )
                    for approved_variant in approved_variant_rows
                )
                if not has_approved_form:
                    findings.append(
                        ValidationFinding(
                            term_id=term.id,
                            kind="missing_canonical",
                            severity="medium",
                            expected=term.canonical_translation,
                            observed="",
                            message=(
                                f"Raw text contains {term.source_text!r}, but translation does "
                                f"not contain approved form {term.canonical_translation!r}."
                            ),
                        )
                    )
        return findings

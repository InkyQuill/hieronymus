from __future__ import annotations

import json
import re
import sqlite3
from dataclasses import dataclass
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.models import ContractTerm, ValidationFinding

_VALID_ALIAS_KINDS = frozenset(
    {
        "source_variant",
        "approved_variant",
        "forbidden_variant",
        "search_alias",
    }
)


@dataclass(frozen=True)
class ParsedRule:
    source_text: str
    canonical_translation: str
    forbidden_variants: list[str]


_RULE_RE = re.compile(
    r"^(?P<source>.+?) is translated as (?P<target>.+?)(?:, not (?P<forbidden>.+?))?\.?$"
)


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _contains(raw_text: str, text: str, *, case_sensitive: bool) -> bool:
    if case_sensitive:
        return text in raw_text
    return text.casefold() in raw_text.casefold()


def _parse_rule_crystal(text: str) -> ParsedRule | None:
    match = _RULE_RE.fullmatch(text.strip())
    if match is None:
        return None

    source = match.group("source").strip()
    target = match.group("target").strip()
    forbidden = match.group("forbidden")
    if not source or not target:
        return None

    forbidden_variants: list[str] = []
    if forbidden is not None:
        cleaned_forbidden = forbidden.strip()
        if not cleaned_forbidden:
            return None
        forbidden_variants.append(cleaned_forbidden)

    return ParsedRule(
        source_text=source,
        canonical_translation=target,
        forbidden_variants=forbidden_variants,
    )


def _rule_text(source_text: str, canonical_translation: str, forbidden_variants: list[str]) -> str:
    if forbidden_variants:
        return (
            f"{source_text} is translated as {canonical_translation}, not {forbidden_variants[0]}."
        )
    return f"{source_text} is translated as {canonical_translation}."


class Termbase:
    def __init__(self, config: HieronymusConfig, context: TranslationContext) -> None:
        self.config = config
        self.context = context

    @property
    def series_slug(self) -> str:
        return self.context.series_slug

    @property
    def source_language(self) -> str:
        return self.context.source_language

    @property
    def target_language(self) -> str:
        return self.context.target_language

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
        with connect(self.config.database_path) as conn:
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
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update strict_terms
                set status = 'approved', updated_at = ?
                where id = ?
                  and series_slug = ?
                  and source_language = ?
                  and target_language = ?
                """,
                (
                    _now(),
                    term_id,
                    self.series_slug,
                    self.source_language,
                    self.target_language,
                ),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown term: {term_id}")
            term = conn.execute(
                """
                select *
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
            self._insert_rule_crystal_for_strict_term(conn, term)
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

        with connect(self.config.database_path) as conn:
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
        result: list[ContractTerm] = []
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from crystals
                where crystal_type = 'rule'
                  and status = 'active'
                  and (
                    (
                      scope_type = 'series'
                      and scope_key = ?
                    )
                    or scope_type = 'global'
                  )
                  and (source_language = ? or source_language = '')
                  and (target_language = ? or target_language = '')
                order by id
                """,
                (self.context.scope_key, self.source_language, self.target_language),
            ).fetchall()
            for row in rows:
                rule = _parse_rule_crystal(row["text"])
                if rule is None:
                    continue
                if not _contains(raw_text, rule.source_text, case_sensitive=False):
                    continue
                tags = [
                    tag_row["tag"]
                    for tag_row in conn.execute(
                        """
                        select tag
                        from crystal_semantic_tags
                        where crystal_id = ?
                        order by tag
                        """,
                        (row["id"],),
                    )
                ]
                result.append(
                    ContractTerm(
                        id=row["id"],
                        category="rule",
                        source_text=rule.source_text,
                        canonical_translation=rule.canonical_translation,
                        forbidden_variants=rule.forbidden_variants,
                        tags=tags,
                        notes=row["text"],
                    )
                )
        return result

    def validate(
        self,
        *,
        translated_text: str,
        raw_text: str | None = None,
        source_text: str | None = None,
    ) -> list[ValidationFinding]:
        if raw_text is not None and source_text is not None:
            raise ValueError("pass either raw_text or source_text, not both")
        source = raw_text if raw_text is not None else source_text
        if source is None:
            raise TypeError("validate requires raw_text or source_text")

        findings: list[ValidationFinding] = []
        for term in self.contract(source):
            for forbidden_variant in term.forbidden_variants:
                if not _contains(
                    translated_text,
                    forbidden_variant,
                    case_sensitive=True,
                ):
                    continue
                findings.append(
                    ValidationFinding(
                        term_id=term.id,
                        kind="forbidden_variant",
                        severity="high",
                        expected=term.canonical_translation,
                        observed=forbidden_variant,
                        message=(
                            f"Use {term.canonical_translation!r} for {term.source_text!r}; "
                            f"{forbidden_variant!r} is forbidden."
                        ),
                    )
                )
            if term.canonical_translation not in translated_text:
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

    def _insert_rule_crystal_for_strict_term(
        self,
        conn: sqlite3.Connection,
        term: sqlite3.Row,
    ) -> int:
        forbidden_rows = conn.execute(
            """
            select text
            from strict_term_aliases
            where term_id = ? and kind = 'forbidden_variant'
            order by id
            """,
            (term["id"],),
        ).fetchall()
        forbidden_variants = [row["text"] for row in forbidden_rows if row["text"].strip()]
        text = _rule_text(
            term["source_text"],
            term["canonical_translation"],
            forbidden_variants,
        )
        now = _now()
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
            values ('rule', ?, '', 'series', ?, ?, ?, ?, ?, 0.8, 0.95,
                    'user_rule', '', 0.0, null, 'active', ?, ?)
            """,
            (
                text,
                self.context.scope_key,
                term["series_slug"],
                term["source_language"],
                term["target_language"],
                json.dumps(self.context.tags, ensure_ascii=False, sort_keys=True),
                now,
                now,
            ),
        )
        crystal_id = int(cursor.lastrowid)
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, '', ?)",
            (crystal_id, text),
        )
        tags = conn.execute(
            """
            select tag
            from strict_term_tags
            where term_id = ?
            order by tag
            """,
            (term["id"],),
        ).fetchall()
        for tag in tags:
            conn.execute(
                """
                insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                values (?, ?, 0.95, ?)
                """,
                (crystal_id, tag["tag"], now),
            )
        return crystal_id

from __future__ import annotations

import json
import sqlite3
from datetime import UTC, datetime

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.models import ContractTerm, ValidationFinding
from hieronymus.rule_crystals import (
    ActiveRuleCrystal,
    load_active_rule_crystals,
    parse_rule_crystal,
)

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


def _rule_text(source_text: str, canonical_translation: str, forbidden_variants: list[str]) -> str:
    if forbidden_variants:
        return (
            f"{source_text} is translated as {canonical_translation}, not {forbidden_variants[0]}."
        )
    return f"{source_text} is translated as {canonical_translation}."


def _validate_rule_shape(
    *,
    source_text: str,
    canonical_translation: str,
    approved_variants: list[str],
    forbidden_variants: list[str],
) -> None:
    if len(forbidden_variants) > 1:
        raise ValueError("rule crystals support at most one forbidden variant")
    if any(variant != canonical_translation for variant in approved_variants):
        raise ValueError("approved variants that differ from canonical rendering are unsupported")
    text = _rule_text(source_text, canonical_translation, forbidden_variants)
    parsed = parse_rule_crystal(text)
    if (
        parsed is None
        or parsed.source_text != source_text
        or parsed.canonical_translation != canonical_translation
        or parsed.forbidden_variants != forbidden_variants
    ):
        raise ValueError("rule crystal text cannot round-trip parsed fields")


_UNSUPPORTED_RULE_ALIAS_KINDS = frozenset({"source_variant", "search_alias"})


def _maximal_matched_surfaces(
    raw_text: str,
    rules: list[ActiveRuleCrystal],
) -> dict[str, list[ActiveRuleCrystal]]:
    raw_folded = raw_text.casefold()
    matches: list[tuple[int, int, str, ActiveRuleCrystal]] = []
    for rule in rules:
        for source_form in rule.source_forms:
            form = source_form.strip()
            if not form:
                continue
            form_folded = form.casefold()
            start = raw_folded.find(form_folded)
            while start >= 0:
                end = start + len(form_folded)
                matches.append((start, end, form, rule))
                start = raw_folded.find(form_folded, start + 1)

    accepted: list[tuple[int, int, str, ActiveRuleCrystal]] = []
    for start, end, form, rule in sorted(matches, key=lambda item: (-(item[1] - item[0]), item[0])):
        if any(
            accepted_start <= start
            and end <= accepted_end
            and (accepted_end - accepted_start) > (end - start)
            for accepted_start, accepted_end, _, _ in accepted
        ):
            continue
        accepted.append((start, end, form, rule))

    grouped: dict[str, list[ActiveRuleCrystal]] = {}
    display_by_key: dict[str, str] = {}
    seen_rule_keys: set[tuple[str, int]] = set()
    for _, _, form, rule in accepted:
        key = form.casefold()
        display_by_key.setdefault(key, form)
        rule_key = (key, rule.crystal_id)
        if rule_key in seen_rule_keys:
            continue
        grouped.setdefault(key, []).append(rule)
        seen_rule_keys.add(rule_key)

    return {display_by_key[key]: rules_for_form for key, rules_for_form in grouped.items()}


def _context_resolved_rules(
    candidates: list[ActiveRuleCrystal],
    *,
    context: TranslationContext,
) -> list[ActiveRuleCrystal]:
    context_semantic_tags = set(context.semantic_tags)
    context_story_scopes = set(context.story_scopes)
    default_language_tags = {context.source_language, context.target_language}
    context_language_tags = set(context.language_tags) - default_language_tags

    scored: list[tuple[int, ActiveRuleCrystal]] = []
    for candidate in candidates:
        score = 0
        if context_semantic_tags.intersection(candidate.semantic_tags):
            score += 1
        if context_story_scopes.intersection(candidate.story_scopes):
            score += 1
        if context_language_tags.intersection(set(candidate.language_tags) - default_language_tags):
            score += 1
        scored.append((score, candidate))

    best_score = max(score for score, _ in scored)
    if best_score <= 0:
        return []
    return [candidate for score, candidate in scored if score == best_score]


def _ambiguity_finding(
    surface: str,
    candidates: list[ActiveRuleCrystal],
) -> ValidationFinding:
    renderings = sorted(
        {
            rendering
            for candidate in candidates
            for rendering in candidate.required_renderings
            if rendering
        }
    )
    return ValidationFinding(
        term_id=min(candidate.crystal_id for candidate in candidates),
        kind="ambiguous_source",
        severity="warning",
        expected=", ".join(renderings),
        observed=surface,
        message=(
            f"Source form {surface!r} maps to multiple active rule concepts; add concept, "
            "semantic tag, or story scope context before enforcing a rendering."
        ),
    )


def _conflicting_rules_finding(
    surface: str,
    candidates: list[ActiveRuleCrystal],
) -> ValidationFinding:
    renderings = sorted(
        {
            rendering
            for candidate in candidates
            for rendering in candidate.required_renderings
            if rendering
        }
    )
    return ValidationFinding(
        term_id=min(candidate.crystal_id for candidate in candidates),
        kind="conflicting_active_rules",
        severity="warning",
        expected=", ".join(renderings),
        observed=surface,
        message=(
            f"Source form {surface!r} has conflicting active rules for the same concept; "
            "supersede, archive, or consolidate one rule before enforcing a rendering."
        ),
    )


def _has_conflicting_required_renderings(candidates: list[ActiveRuleCrystal]) -> bool:
    renderings = {
        candidate.required_renderings for candidate in candidates if candidate.required_renderings
    }
    return len(renderings) > 1


def _tag_score(candidate_tags: tuple[str, ...], wanted_tags: tuple[str, ...]) -> int:
    return len(set(candidate_tags).intersection(wanted_tags))


def _prefer_longer_source_surface(
    current: tuple[ActiveRuleCrystal, str] | None,
    candidate: ActiveRuleCrystal,
    surface: str,
) -> tuple[ActiveRuleCrystal, str]:
    if current is None:
        return (candidate, surface)
    _, current_surface = current
    if len(surface) > len(current_surface):
        return (candidate, surface)
    return current


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
            term = conn.execute(
                """
                select *
                from strict_terms
                where id = ?
                  and series_slug = ?
                  and source_language = ?
                  and target_language = ?
                """,
                (
                    term_id,
                    self.series_slug,
                    self.source_language,
                    self.target_language,
                ),
            ).fetchone()
            if term is None:
                raise KeyError(f"unknown term: {term_id}")
            self._validate_strict_term_rule_shape(conn, term)
            self._insert_rule_crystal_for_strict_term(conn, term)
            conn.execute(
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
                select id, status
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
            if term["status"] == "approved":
                raise ValueError("approved term aliases must be represented as rule crystals")
            conn.execute(
                """
                insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
                values (?, ?, ?, ?, ?)
                """,
                (term_id, language, text, kind, int(case_sensitive)),
            )
            conn.commit()

    def contract(self, raw_text: str) -> list[ContractTerm]:
        active_rules, ambiguity_findings, _ = self._resolve_active_rules(raw_text)
        return [
            self._contract_term_for_active_rule(rule, source_text=source_text)
            for rule, source_text in active_rules
        ]

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
        active_rules, ambiguity_findings, _ = self._resolve_active_rules(source)
        findings.extend(ambiguity_findings)
        terms = [
            self._contract_term_for_active_rule(rule, source_text=source_text)
            for rule, source_text in active_rules
        ]
        for term in terms:
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

    def _resolve_active_rules(
        self,
        raw_text: str,
    ) -> tuple[list[tuple[ActiveRuleCrystal, str]], list[ValidationFinding], bool]:
        with connect(self.config.database_path) as conn:
            self._ensure_approved_strict_terms_migrated(conn)
            conn.commit()
            rules = load_active_rule_crystals(conn, self.context)
        if not rules:
            return [], [], False

        matched_surfaces = _maximal_matched_surfaces(raw_text, rules)
        if not matched_surfaces:
            return [], [], False

        selected: dict[int, tuple[ActiveRuleCrystal, str]] = {}
        warnings: list[ValidationFinding] = []
        for surface, candidates in matched_surfaces.items():
            concept_sets = {candidate.concept_ids for candidate in candidates}
            if len(concept_sets) == 1:
                if _has_conflicting_required_renderings(candidates):
                    warnings.append(_conflicting_rules_finding(surface, candidates))
                    continue
                for candidate in candidates:
                    selected[candidate.crystal_id] = _prefer_longer_source_surface(
                        selected.get(candidate.crystal_id),
                        candidate,
                        surface,
                    )
                continue

            resolved = _context_resolved_rules(
                candidates,
                context=self.context,
            )
            resolved_concept_sets = {rule.concept_ids for rule in resolved}
            if len(resolved_concept_sets) == 1:
                if _has_conflicting_required_renderings(resolved):
                    warnings.append(_conflicting_rules_finding(surface, resolved))
                    continue
                for candidate in resolved:
                    selected[candidate.crystal_id] = _prefer_longer_source_surface(
                        selected.get(candidate.crystal_id),
                        candidate,
                        surface,
                    )
                continue

            warnings.append(_ambiguity_finding(surface, candidates))

        return list(selected.values()), warnings, True

    def _contract_term_for_active_rule(
        self,
        rule: ActiveRuleCrystal,
        *,
        source_text: str,
    ) -> ContractTerm:
        canonical_translation = rule.required_renderings[0] if rule.required_renderings else ""
        return ContractTerm(
            id=rule.crystal_id,
            category="rule",
            source_text=source_text,
            canonical_translation=canonical_translation,
            forbidden_variants=list(rule.forbidden_renderings),
            tags=list(rule.semantic_tags),
            notes=_rule_text(
                source_text,
                canonical_translation,
                list(rule.forbidden_renderings),
            ),
        )

    def _ensure_approved_strict_terms_migrated(self, conn: sqlite3.Connection) -> None:
        rows = conn.execute(
            """
            select *
            from strict_terms
            where status = 'approved'
              and series_slug = ?
              and source_language = ?
              and target_language = ?
            order by id
            """,
            (self.series_slug, self.source_language, self.target_language),
        ).fetchall()
        for term in rows:
            if self._approved_strict_term_rule_is_linked(conn, term):
                continue
            self._validate_strict_term_rule_shape(conn, term)
            self._insert_rule_crystal_for_strict_term(conn, term)

    def _validate_strict_term_rule_shape(
        self,
        conn: sqlite3.Connection,
        term: sqlite3.Row,
    ) -> None:
        alias_rows = conn.execute(
            """
            select text, kind, case_sensitive
            from strict_term_aliases
            where term_id = ?
            order by id
            """,
            (term["id"],),
        ).fetchall()
        for row in alias_rows:
            if row["kind"] in _UNSUPPORTED_RULE_ALIAS_KINDS:
                raise ValueError(f"{row['kind']} aliases are unsupported by rule crystals")
            if not bool(row["case_sensitive"]):
                raise ValueError("case-insensitive aliases are unsupported by rule crystals")

        approved_variants = [
            row["text"].strip()
            for row in alias_rows
            if row["kind"] == "approved_variant" and row["text"].strip()
        ]
        forbidden_variants = [
            row["text"].strip()
            for row in alias_rows
            if row["kind"] == "forbidden_variant" and row["text"].strip()
        ]
        _validate_rule_shape(
            source_text=term["source_text"],
            canonical_translation=term["canonical_translation"],
            approved_variants=approved_variants,
            forbidden_variants=forbidden_variants,
        )

    def _insert_rule_crystal_for_strict_term(
        self,
        conn: sqlite3.Connection,
        term: sqlite3.Row,
    ) -> int:
        now = _now()
        tags = self._strict_term_tags(conn, int(term["id"]))
        forbidden_variants = self._strict_term_forbidden_variants(conn, int(term["id"]))
        concept_id = self._ensure_concept_for_strict_term(
            conn,
            term,
            tags=tags,
            now=now,
        )
        text = _rule_text(
            term["source_text"],
            term["canonical_translation"],
            forbidden_variants,
        )
        existing = conn.execute(
            """
            select id
            from crystals
            where crystal_type = 'rule'
              and status = 'active'
              and text = ?
              and scope_type = 'series'
              and scope_key = ?
              and series_slug = ?
              and source_language = ?
              and target_language = ?
            order by id
            limit 1
            """,
            (
                text,
                self.context.scope_key,
                term["series_slug"],
                term["source_language"],
                term["target_language"],
            ),
        ).fetchone()
        if existing is not None:
            crystal_id = int(existing["id"])
            self._ensure_crystal_semantic_tags(conn, crystal_id, tags=tags, now=now)
            self._link_rule_crystal_to_concept(
                conn,
                crystal_id=crystal_id,
                concept_id=concept_id,
                now=now,
            )
            return crystal_id

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
                json.dumps(tags, ensure_ascii=False, sort_keys=True),
                now,
                now,
            ),
        )
        crystal_id = int(cursor.lastrowid)
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, '', ?)",
            (crystal_id, text),
        )
        self._ensure_crystal_semantic_tags(conn, crystal_id, tags=tags, now=now)
        self._link_rule_crystal_to_concept(
            conn,
            crystal_id=crystal_id,
            concept_id=concept_id,
            now=now,
        )
        return crystal_id

    def _approved_strict_term_rule_is_linked(
        self,
        conn: sqlite3.Connection,
        term: sqlite3.Row,
    ) -> bool:
        text = _rule_text(
            term["source_text"],
            term["canonical_translation"],
            self._strict_term_forbidden_variants(conn, int(term["id"])),
        )
        row = conn.execute(
            """
            select 1
            from crystals crystal
            join crystal_concepts cc on cc.crystal_id = crystal.id
            join concepts c on c.id = cc.concept_id
            where crystal.crystal_type = 'rule'
              and crystal.status = 'active'
              and crystal.text = ?
              and crystal.scope_type = 'series'
              and crystal.scope_key = ?
              and crystal.series_slug = ?
              and crystal.source_language = ?
              and crystal.target_language = ?
              and c.status not in ('archived', 'merged')
            limit 1
            """,
            (
                text,
                self.context.scope_key,
                term["series_slug"],
                term["source_language"],
                term["target_language"],
            ),
        ).fetchone()
        return row is not None

    def _strict_term_tags(self, conn: sqlite3.Connection, term_id: int) -> tuple[str, ...]:
        rows = conn.execute(
            """
            select tag
            from strict_term_tags
            where term_id = ?
            order by tag
            """,
            (term_id,),
        ).fetchall()
        return tuple(row["tag"] for row in rows)

    def _strict_term_forbidden_variants(
        self,
        conn: sqlite3.Connection,
        term_id: int,
    ) -> list[str]:
        rows = conn.execute(
            """
            select text
            from strict_term_aliases
            where term_id = ? and kind = 'forbidden_variant'
            order by id
            """,
            (term_id,),
        ).fetchall()
        return [row["text"] for row in rows if row["text"].strip()]

    def _ensure_concept_for_strict_term(
        self,
        conn: sqlite3.Connection,
        term: sqlite3.Row,
        *,
        tags: tuple[str, ...],
        now: str,
    ) -> int:
        concept_id = self._matching_concept_id_for_strict_term(
            conn,
            term["source_text"],
            tags=tags,
        )
        if concept_id is None:
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
                values (?, ?, 'series', ?, 'established', 0.95, ?, ?)
                """,
                (
                    term["source_text"],
                    term["notes"],
                    self.context.scope_key,
                    now,
                    now,
                ),
            )
            concept_id = int(cursor.lastrowid)

        for tag in tags:
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                values (?, ?, 0.95, ?)
                on conflict(concept_id, tag) do update set
                  confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                """,
                (concept_id, tag, now),
            )

        self._ensure_concept_facet(
            conn,
            concept_id=concept_id,
            value=term["source_text"],
            facet_type="name",
            language_tag=term["source_language"],
            is_canonical=True,
            now=now,
        )
        self._ensure_concept_facet(
            conn,
            concept_id=concept_id,
            value=term["canonical_translation"],
            facet_type="rendering",
            language_tag=term["target_language"],
            is_canonical=False,
            now=now,
        )
        return concept_id

    def _matching_concept_id_for_strict_term(
        self,
        conn: sqlite3.Connection,
        source_text: str,
        *,
        tags: tuple[str, ...],
    ) -> int | None:
        candidates = conn.execute(
            """
            select id
            from concepts
            where canonical_name = ?
              and scope_type = 'series'
              and scope_key = ?
              and status not in ('archived', 'merged')
            order by id
            """,
            (source_text, self.context.scope_key),
        ).fetchall()
        if not candidates:
            return None
        if len(candidates) == 1:
            return int(candidates[0]["id"])
        if not tags:
            return None

        scored = [
            (
                _tag_score(self._concept_semantic_tags(conn, int(row["id"])), tags),
                int(row["id"]),
            )
            for row in candidates
        ]
        best_score = max(score for score, _ in scored)
        if best_score <= 0:
            return None
        best_ids = [concept_id for score, concept_id in scored if score == best_score]
        if len(best_ids) != 1:
            return None
        return best_ids[0]

    def _concept_semantic_tags(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
    ) -> tuple[str, ...]:
        rows = conn.execute(
            """
            select tag
            from concept_semantic_tags
            where concept_id = ?
            order by tag
            """,
            (concept_id,),
        ).fetchall()
        return tuple(row["tag"] for row in rows)

    def _ensure_concept_facet(
        self,
        conn: sqlite3.Connection,
        *,
        concept_id: int,
        value: str,
        facet_type: str,
        language_tag: str,
        is_canonical: bool,
        now: str,
    ) -> None:
        existing = conn.execute(
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
        if existing is None:
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
                values (?, ?, ?, ?, null, 0.95, ?, ?, ?)
                """,
                (
                    concept_id,
                    language_tag,
                    facet_type,
                    value,
                    int(is_canonical),
                    now,
                    now,
                ),
            )
            facet_id = int(cursor.lastrowid)
        else:
            facet_id = int(existing["id"])

        if language_tag:
            conn.execute(
                """
                insert or ignore into concept_facet_language_tags(facet_id, language_tag)
                values (?, ?)
                """,
                (facet_id, language_tag.casefold()),
            )

    def _ensure_crystal_semantic_tags(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        *,
        tags: tuple[str, ...],
        now: str,
    ) -> None:
        for tag in tags:
            conn.execute(
                """
                insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                values (?, ?, 0.95, ?)
                on conflict(crystal_id, tag) do update set
                  confidence = max(crystal_semantic_tags.confidence, excluded.confidence)
                """,
                (crystal_id, tag, now),
            )

    def _link_rule_crystal_to_concept(
        self,
        conn: sqlite3.Connection,
        *,
        crystal_id: int,
        concept_id: int,
        now: str,
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
            values (?, ?, 'defines', 0.95, ?)
            on conflict(crystal_id, concept_id, link_type) do update set
              confidence = max(crystal_concepts.confidence, excluded.confidence)
            """,
            (crystal_id, concept_id, now),
        )

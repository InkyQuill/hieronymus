import pytest

from hieronymus.concepts import CONCEPT_ESTABLISHED, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.rule_crystals import parse_rule_crystal
from hieronymus.termbase import Termbase


def _context_for_series(series, *, target_language: str | None = None) -> TranslationContext:
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=target_language or series.target_language,
        task_type="translation",
    )


def _termbase_for_series(config, series, *, target_language: str | None = None) -> Termbase:
    return Termbase(config, _context_for_series(series, target_language=target_language))


def _add_rule_crystal(
    config: HieronymusConfig,
    context: TranslationContext,
    text: str,
    *,
    semantic_tags: tuple[str, ...] = (),
    confidence: float = 0.95,
    strength: float = 0.8,
    link_concept: bool = True,
) -> int:
    concept_ids: tuple[int, ...] = ()
    parsed = parse_rule_crystal(text)
    if link_concept and parsed is not None:
        concept = ConceptStore(config).create_concept(
            parsed.source_text,
            status=CONCEPT_ESTABLISHED,
            confidence=0.95,
            scope_type="series",
            scope_key=context.scope_key,
            semantic_tags=semantic_tags,
        )
        ConceptStore(config).add_facet(
            concept.id,
            parsed.source_text,
            kind="name",
            language_tags=(context.source_language,),
            semantic_tags=semantic_tags,
        )
        concept_ids = (concept.id,)
    return CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text=text,
        source_credibility="user_rule",
        confidence=confidence,
        strength=strength,
        semantic_tags=semantic_tags,
        concept_ids=concept_ids,
    )


def test_validate_flags_forbidden_variant(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    crystal_id = _add_rule_crystal(
        config,
        termbase.context,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
        semantic_tags=("sense",),
    )

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    finding = findings[0]
    assert finding.term_id == crystal_id
    assert finding.kind == "forbidden_variant"
    assert finding.severity == "high"
    assert finding.expected == "ATK Up"
    assert finding.observed == "Attack Increase"
    assert "is forbidden" in finding.message


def test_validate_flags_missing_canonical_when_source_present(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    crystal_id = _add_rule_crystal(config, termbase.context, "ガンツ is translated as Ganz.")

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    finding = findings[0]
    assert finding.term_id == crystal_id
    assert finding.kind == "missing_canonical"
    assert finding.severity == "medium"
    assert finding.expected == "Ganz"
    assert finding.observed == ""
    assert "approved form 'Ganz'" in finding.message


def test_validate_accepts_approved_variant_as_final_form(config):
    series = Registry(config).create_series(
        slug="gantz",
        title="Gantz",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    _add_rule_crystal(config, termbase.context, "ガンツ is translated as Gantz.")

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    assert findings == []


def test_validate_returns_no_findings_for_clean_translation(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    _add_rule_crystal(
        config,
        termbase.context,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
    )

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up ATK Up.",
    )

    assert findings == []


def test_validate_returns_no_findings_without_contracted_term(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    _add_rule_crystal(
        config,
        termbase.context,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
    )

    findings = termbase.validate(
        raw_text="敏捷性上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert findings == []


def test_validate_isolated_by_target_language(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    english = _termbase_for_series(config, series, target_language="en")
    russian = _termbase_for_series(config, series, target_language="ru")
    _add_rule_crystal(config, english.context, "攻撃力上昇 is translated as ATK Up.")
    _add_rule_crystal(config, russian.context, "攻撃力上昇 is translated as Усиление атаки.")

    findings = english.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up ATK Up.",
    )

    assert findings == []


def test_unlinked_low_confidence_rule_crystal_does_not_validate(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    _add_rule_crystal(
        config,
        termbase.context,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
        confidence=0.2,
        strength=0.2,
        link_concept=False,
    )

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert findings == []


def test_reapproving_strict_term_does_not_duplicate_validation_findings(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")

    termbase.approve(term_id)
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert [finding.kind for finding in findings] == [
        "forbidden_variant",
        "missing_canonical",
    ]


def test_existing_approved_strict_term_rule_is_migrated_before_validation(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    with connect(termbase.config.database_path) as conn:
        conn.execute("update strict_terms set status = 'approved' where id = ?", (term_id,))
        conn.commit()
    crystal_id = _add_rule_crystal(
        config,
        termbase.context,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
        link_concept=False,
    )

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    with connect(termbase.config.database_path) as conn:
        links = conn.execute(
            """
            select concept_id
            from crystal_concepts
            where crystal_id = ?
            """,
            (crystal_id,),
        ).fetchall()

    assert [finding.kind for finding in findings] == [
        "forbidden_variant",
        "missing_canonical",
    ]
    assert [finding.term_id for finding in findings] == [crystal_id, crystal_id]
    assert len(links) == 1


def test_approve_rejects_multiple_forbidden_variants(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Boost", language="en")

    with pytest.raises(ValueError, match="rule crystals support at most one forbidden variant"):
        termbase.approve(term_id)


def test_approve_rejects_noncanonical_approved_variant(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="person_name",
        source_text="ガンツ",
        canonical_translation="Ganz",
    )
    termbase.add_alias(term_id, kind="approved_variant", text="Gantz", language="en")

    with pytest.raises(
        ValueError,
        match="approved variants that differ from canonical rendering are unsupported",
    ):
        termbase.approve(term_id)


def test_rule_crystal_validation_reports_forbidden_old_rendering(
    config: HieronymusConfig,
) -> None:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    context = _context_for_series(series)
    CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Готовка, not Кулинария.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        semantic_tags=("translation-rule", "cooking"),
        concept_ids=(
            ConceptStore(config)
            .create_concept(
                "Cooking Talent",
                status=CONCEPT_ESTABLISHED,
                confidence=0.95,
                scope_type="series",
                scope_key=context.scope_key,
                semantic_tags=("translation-rule", "cooking"),
            )
            .id,
        ),
    )

    findings = Termbase(config, context).validate(
        source_text="Cooking Talent",
        translated_text="Кулинария",
    )

    assert findings[0].expected == "Готовка"
    assert findings[0].observed == "Кулинария"


def test_unparseable_rule_crystal_does_not_validate(
    config: HieronymusConfig,
) -> None:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    context = _context_for_series(series)
    _add_rule_crystal(
        config,
        context,
        "Cooking Talent should usually become Готовка.",
        semantic_tags=("translation-rule", "cooking"),
    )

    findings = Termbase(config, context).validate(
        source_text="Cooking Talent",
        translated_text="Кулинария",
    )

    assert findings == []

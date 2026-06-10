from __future__ import annotations

from dataclasses import dataclass

from hieronymus.concepts import CONCEPT_ESTABLISHED, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


@dataclass(frozen=True)
class CookingRules:
    context: TranslationContext
    talent_concept_id: int
    talent_rule_id: int
    subskill_concept_id: int
    subskill_rule_id: int


def _context(
    config: HieronymusConfig,
    *,
    semantic_tags: tuple[str, ...] = (),
    story_scopes: tuple[str, ...] = (),
) -> TranslationContext:
    series = Registry(config).create_series(
        slug="oso",
        title="Only Sense Online",
        source_language="en",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translation",
        semantic_tags=semantic_tags,
        story_scopes=story_scopes,
    )


def _same_series_context(
    context: TranslationContext,
    *,
    semantic_tags: tuple[str, ...] = (),
    story_scopes: tuple[str, ...] = (),
) -> TranslationContext:
    return TranslationContext(
        series_slug=context.series_slug,
        source_language=context.source_language,
        target_language=context.target_language,
        task_type=context.task_type,
        semantic_tags=semantic_tags,
        story_scopes=story_scopes,
    )


def _create_cooking_rules(config: HieronymusConfig) -> CookingRules:
    context = _context(config)
    concepts = ConceptStore(config)
    crystals = CrystalStore(config)

    talent = concepts.create_concept(
        "Cooking Talent",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:talent",),
    )
    concepts.add_facet(
        talent.id,
        "Cooking talent",
        kind="name",
        language_tags=("en",),
        semantic_tags=("role:talent",),
    )
    concepts.add_facet(
        talent.id,
        "Cooking",
        kind="name",
        language_tags=("en",),
        semantic_tags=("role:talent",),
        story_scopes=("skill-tree:talents",),
    )
    old_rule_id = crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Cooking Talent is translated as Кулинария.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        semantic_tags=("role:talent", "translation-rule"),
        concept_ids=(talent.id,),
    )
    talent_rule_id = crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Cooking talent is translated as Готовка, not Кулинария.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        supersedes_crystal_id=old_rule_id,
        semantic_tags=("role:talent", "translation-rule"),
        concept_ids=(talent.id,),
    )

    subskill = concepts.create_concept(
        "Preparation/Cooking subskill",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:subskill",),
    )
    concepts.add_facet(
        subskill.id,
        "Cooking",
        kind="name",
        language_tags=("en",),
        semantic_tags=("role:subskill",),
        story_scopes=("skill-tree:preparation",),
    )
    subskill_rule_id = crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Cooking is translated as Приготовление.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        story_scopes=("skill-tree:preparation",),
        semantic_tags=("role:subskill", "translation-rule"),
        concept_ids=(subskill.id,),
    )

    return CookingRules(
        context=context,
        talent_concept_id=talent.id,
        talent_rule_id=talent_rule_id,
        subskill_concept_id=subskill.id,
        subskill_rule_id=subskill_rule_id,
    )


def test_talent_rule_applies_to_cooking_talent_without_subskill_leakage(
    config: HieronymusConfig,
) -> None:
    rules = _create_cooking_rules(config)
    context = _same_series_context(rules.context, semantic_tags=("role:talent",))

    findings = Termbase(config, context).validate(
        source_text="Cooking Talent",
        translated_text="Кулинария",
    )

    assert findings
    assert {finding.term_id for finding in findings} == {rules.talent_rule_id}
    assert {finding.expected for finding in findings} == {"Готовка"}
    assert {finding.observed for finding in findings} == {"", "Кулинария"}


def test_cooking_subskill_rule_applies_when_context_resolves_to_subskill(
    config: HieronymusConfig,
) -> None:
    rules = _create_cooking_rules(config)
    context = _same_series_context(rules.context, semantic_tags=("role:subskill",))

    findings = Termbase(config, context).validate(
        source_text="Cooking",
        translated_text="Готовка",
    )

    assert [(finding.term_id, finding.kind, finding.expected) for finding in findings] == [
        (rules.subskill_rule_id, "missing_canonical", "Приготовление")
    ]


def test_ambiguous_cooking_warns_without_enforcing_a_rendering(
    config: HieronymusConfig,
) -> None:
    rules = _create_cooking_rules(config)

    findings = Termbase(config, rules.context).validate(
        source_text="Cooking",
        translated_text="Готовка",
    )

    assert len(findings) == 1
    assert findings[0].kind == "ambiguous_source"
    assert findings[0].severity == "warning"
    assert findings[0].observed == "Cooking"
    assert findings[0].expected == "Готовка, Приготовление"


def test_ambiguous_contract_does_not_fall_back_to_unlinked_raw_rule(
    config: HieronymusConfig,
) -> None:
    rules = _create_cooking_rules(config)
    CrystalStore(config).add_crystal(
        rules.context,
        crystal_type="rule",
        text="Cooking is translated as Сырая готовка.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
    )

    contract = Termbase(config, rules.context).contract("Cooking")

    assert contract == []


def test_contract_returns_unambiguous_terms_when_other_surface_is_ambiguous(
    config: HieronymusConfig,
) -> None:
    rules = _create_cooking_rules(config)
    smithing = ConceptStore(config).create_concept(
        "Smithing Talent",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=rules.context.scope_key,
        semantic_tags=("role:talent",),
    )
    ConceptStore(config).add_facet(
        smithing.id,
        "Smithing",
        kind="name",
        language_tags=("en",),
        semantic_tags=("role:talent",),
    )
    smithing_rule_id = CrystalStore(config).add_crystal(
        rules.context,
        crystal_type="rule",
        text="Smithing is translated as Кузнечное дело.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        semantic_tags=("role:talent", "translation-rule"),
        concept_ids=(smithing.id,),
    )

    contract = Termbase(config, rules.context).contract("Cooking and Smithing")

    assert [term.id for term in contract] == [smithing_rule_id]
    assert [term.canonical_translation for term in contract] == ["Кузнечное дело"]


def test_conflicting_active_rules_for_same_concept_warn_without_enforcing(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    sense = ConceptStore(config).create_concept(
        "Sense",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:talent",),
    )
    ConceptStore(config).add_facet(
        sense.id,
        "Sense",
        kind="name",
        language_tags=("en",),
        semantic_tags=("role:talent",),
    )
    crystals = CrystalStore(config)
    crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Sense is translated as Сенс.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        concept_ids=(sense.id,),
    )
    crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Sense is translated as Чувство.",
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        concept_ids=(sense.id,),
    )

    findings = Termbase(config, context).validate(
        source_text="Sense",
        translated_text="Sense",
    )

    assert len(findings) == 1
    assert findings[0].kind == "conflicting_active_rules"
    assert findings[0].severity == "warning"
    assert findings[0].observed == "Sense"
    assert findings[0].expected == "Сенс, Чувство"


def test_high_confidence_advisory_crystal_does_not_validate(
    config: HieronymusConfig,
) -> None:
    context = _context(config, semantic_tags=("role:talent",))
    concepts = ConceptStore(config)
    talent = concepts.create_concept(
        "Cooking Talent",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:talent",),
    )
    CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Cooking Talent is translated as Готовка, not Кулинария.",
        source_credibility="expert",
        confidence=0.95,
        strength=0.95,
        semantic_tags=("role:talent", "translation-rule"),
        concept_ids=(talent.id,),
    )

    findings = Termbase(config, context).validate(
        source_text="Cooking Talent",
        translated_text="Кулинария",
    )

    assert findings == []

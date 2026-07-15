import pytest

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import RecallResult, TranslationContext
from hieronymus.rag_models import RagChunkRecord
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(
    config: HieronymusConfig,
    *,
    slug: str = "only-sense-online",
    story_scopes: tuple[str, ...] = (),
    semantic_tags: tuple[str, ...] = (),
) -> TranslationContext:
    series = Registry(config).create_series(
        slug=slug,
        title=slug.replace("-", " ").title(),
        source_language="ja",
        target_language="en",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
    )


def _enriched_keys() -> set[str]:
    return {
        "tier",
        "id",
        "title",
        "kind",
        "text",
        "crystal_type",
        "concept_ids",
        "concept_labels",
        "language_tags",
        "story_scopes",
        "semantic_tags",
        "source_credibility",
        "confidence",
        "strength",
        "soft_origin",
        "is_rule",
        "is_thought",
        "score",
        "rank_reason",
    }


def test_yun_enchant_returns_enriched_short_and_long_term_hits(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Yun Enchant should stay in English during menu checks.",
        language_tags=("en", "ja"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
        source_credibility="user_suggestion",
        soft_origin="session-note",
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="rule",
        text="Yun Enchant is an active skill name rule.",
        title="Yun Enchant Rule",
        source_credibility="user_rule",
        rule_intent="terminology",
        strength=0.8,
        confidence=0.9,
        semantic_tags=("skill:name",),
    )

    results = RecallService(config).recall(session.id, context, "Yun Enchant", limit=5)

    tiers = {result.tier for result in results}
    assert tiers == {"short_term", "long_term"}
    assert all(_enriched_keys() <= result.enriched_payload().keys() for result in results)
    short = next(result for result in results if result.tier == "short_term")
    long = next(result for result in results if result.tier == "long_term")
    assert short.id == memory_id
    assert short.title == "note"
    assert short.kind == "note"
    assert short.crystal_type is None
    assert short.language_tags == ("en", "ja")
    assert short.story_scopes == ("book:5/chapter:5",)
    assert short.semantic_tags == ("skill:name",)
    assert short.source_credibility == "user_suggestion"
    assert short.confidence == 0.0
    assert short.strength == 0.0
    assert short.soft_origin == "session-note"
    assert not short.is_rule
    assert not short.is_thought
    assert short.rank_reason == "active session short-term memory match"
    assert long.id == crystal_id
    assert long.title == "Yun Enchant Rule"
    assert long.kind == "rule"
    assert long.crystal_type == "rule"
    assert long.source_credibility == "user_rule"
    assert long.confidence == 0.9
    assert long.strength == 0.8
    assert long.is_rule
    assert not long.is_thought
    assert long.rank_reason == "weighted search match"


def test_concept_facet_finds_linked_crystal_without_exact_prose_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concepts = ConceptStore(config)
    concept = concepts.create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    concepts.add_facet(
        concept.id,
        "Enchant",
        kind="name",
        language_tags=("en",),
        is_canonical=True,
        confidence=0.9,
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Treat the weapon imbue skill as a proper noun.",
        title="Imbue Skill Naming",
        concept_ids=(concept.id,),
        strength=0.7,
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert [result.id for result in results] == [crystal_id]
    assert results[0].tier == "long_term"
    assert results[0].concept_ids == (concept.id,)
    assert results[0].concept_labels == ("Enchant",)
    assert results[0].score > 0


def test_short_term_language_tags_are_searchable_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this transient note available for locale-sensitive checks.",
        language_tags=("ja",),
    )

    results = RecallService(config).recall(session.id, context, "ja", limit=5)

    assert [result.id for result in results] == [memory_id]
    assert results[0].tier == "short_term"
    assert results[0].language_tags == ("ja",)
    assert results[0].score > 0


def test_short_term_story_scopes_are_searchable_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this scene-local note available for later consistency checks.",
        story_scopes=("book:5/chapter:5",),
    )

    results = RecallService(config).recall(session.id, context, "book:5/chapter:5", limit=5)

    assert [result.id for result in results] == [memory_id]
    assert results[0].tier == "short_term"
    assert results[0].story_scopes == ("book:5/chapter:5",)
    assert results[0].score > 0


def test_short_term_semantic_tags_are_searchable_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this naming note available for later consistency checks.",
        semantic_tags=("skill:name",),
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [memory_id]
    assert results[0].tier == "short_term"
    assert results[0].semantic_tags == ("skill:name",)
    assert results[0].score > 0


def test_short_term_structured_semantic_tags_do_not_match_sibling_tags(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    character_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this character naming note available for later consistency checks.",
        semantic_tags=("character:name",),
    )
    skill_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this skill naming note available for later consistency checks.",
        semantic_tags=("skill:name",),
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [skill_id]
    assert character_id not in {result.id for result in results}


def test_short_term_structured_story_scopes_do_not_match_partial_sibling_scopes(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    sibling_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this sibling scene note available for later consistency checks.",
        story_scopes=("book:1/chapter:5",),
    )
    target_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this target scene note available for later consistency checks.",
        story_scopes=("book:5/chapter:5",),
    )

    results = RecallService(config).recall(session.id, context, "book:5/chapter:5", limit=5)

    assert [result.id for result in results] == [target_id]
    assert sibling_id not in {result.id for result in results}


def test_short_term_metadata_search_reaches_beyond_first_fifty_memories(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    for index in range(55):
        workspace.add_short_term_memory(
            session.id,
            source_role="mentor",
            kind="note",
            text=f"Unrelated transient note {index}.",
        )
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="Keep this late naming note available for later consistency checks.",
        semantic_tags=("skill:name",),
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [memory_id]


def test_short_term_source_credibility_and_rule_intent_boost_ranking(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    fts_only_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="note",
        text="terminology user_rule appears in this ordinary note.",
    )
    rule_memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="user",
        kind="correction",
        text="Keep this correction available even when prose does not repeat metadata.",
        source_credibility="user_rule",
        rule_intent="terminology",
    )

    results = RecallService(config).recall(session.id, context, "terminology user_rule", limit=5)

    assert [result.id for result in results] == [rule_memory_id, fts_only_id]
    assert results[0].tier == "short_term"
    assert results[0].source_credibility == "user_rule"
    assert results[0].is_rule
    assert results[0].score > results[1].score


def test_crystal_semantic_tags_are_searchable_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the status display terse in menus.",
        title="Menu Status Style",
        semantic_tags=("skill:name",),
        strength=0.6,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [crystal_id]
    assert results[0].tier == "long_term"
    assert results[0].semantic_tags == ("skill:name",)
    assert results[0].reason == "metadata recall match"


def test_crystal_semantic_tag_candidate_limit_prefers_stronger_later_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    low_ids = [
        crystals.add_crystal(
            context,
            crystal_type="lesson",
            text=f"Keep low confidence skill naming note {index}.",
            title=f"Low Skill Naming {index}",
            semantic_tags=("skill:name",),
            strength=0.1,
            confidence=0.1,
        )
        for index in range(20)
    ]
    high_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep high confidence skill naming note.",
        title="High Skill Naming",
        semantic_tags=("skill:name",),
        strength=0.95,
        confidence=0.95,
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert results[0].id == high_id
    assert high_id in {result.id for result in results}
    assert results[0].score > next(result.score for result in results if result.id == low_ids[0])


def test_crystal_structured_semantic_tags_do_not_match_sibling_tags(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    character_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the character naming display terse in menus.",
        title="Character Naming",
        semantic_tags=("character:name",),
        strength=0.6,
        confidence=0.7,
    )
    skill_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the skill naming display terse in menus.",
        title="Skill Naming",
        semantic_tags=("skill:name",),
        strength=0.6,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [skill_id]
    assert character_id not in {result.id for result in results}


def test_crystal_story_scopes_are_searchable_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the status display terse in menus.",
        title="Scoped Menu Status Style",
        story_scopes=("book:5/chapter:5",),
        strength=0.6,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "book:5/chapter:5", limit=5)

    assert [result.id for result in results] == [crystal_id]
    assert results[0].tier == "long_term"
    assert results[0].story_scopes == ("book:5/chapter:5",)
    assert results[0].reason == "metadata recall match"


def test_crystal_structured_story_scopes_do_not_match_partial_sibling_scopes(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    sibling_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the sibling scene display terse in menus.",
        title="Sibling Scene",
        story_scopes=("book:1/chapter:5",),
        strength=0.6,
        confidence=0.7,
    )
    target_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep the target scene display terse in menus.",
        title="Target Scene",
        story_scopes=("book:5/chapter:5",),
        strength=0.6,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "book:5/chapter:5", limit=5)

    assert [result.id for result in results] == [target_id]
    assert sibling_id not in {result.id for result in results}


def test_concept_label_finds_linked_crystal_without_text_or_facet_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concept = ConceptStore(config).create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Treat the weapon imbue skill as a proper noun.",
        title="Imbue Skill Naming",
        concept_ids=(concept.id,),
        strength=0.7,
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert [result.id for result in results] == [crystal_id]
    assert results[0].concept_ids == (concept.id,)
    assert results[0].concept_labels == ("Enchant",)
    assert results[0].reason == "metadata recall match"


def test_concept_label_link_expansion_is_bounded_and_deterministic(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concept = ConceptStore(config).create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    crystal_ids = [
        CrystalStore(config).add_crystal(
            context,
            crystal_type="lesson",
            text=f"Treat the linked skill entry {index} as a proper noun.",
            title=f"Linked Skill {index}",
            concept_ids=(concept.id,),
            strength=0.7,
            confidence=0.8,
        )
        for index in range(60)
    ]

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert [result.id for result in results] == crystal_ids[:5]
    assert all(result.concept_ids == (concept.id,) for result in results)
    assert all(result.reason == "metadata recall match" for result in results)


def test_concept_link_candidate_limit_prefers_stronger_later_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concept = ConceptStore(config).create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    crystals = CrystalStore(config)
    low_ids = [
        crystals.add_crystal(
            context,
            crystal_type="lesson",
            text=f"Treat low confidence linked skill entry {index} as a proper noun.",
            title=f"Low Linked Skill {index}",
            concept_ids=(concept.id,),
            strength=0.1,
            confidence=0.1,
        )
        for index in range(20)
    ]
    high_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Treat high confidence linked skill entry as a proper noun.",
        title="High Linked Skill",
        concept_ids=(concept.id,),
        strength=0.95,
        confidence=0.95,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert results[0].id == high_id
    assert high_id in {result.id for result in results}
    assert results[0].score > next(result.score for result in results if result.id == low_ids[0])


def test_concept_label_candidate_limit_prefers_stronger_later_visible_concept(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concepts = ConceptStore(config)
    crystals = CrystalStore(config)
    low_ids = []
    for index in range(60):
        concept = concepts.create_concept(
            "Enchant",
            status="established",
            confidence=0.9,
            scope_type="series",
            scope_key=context.scope_key,
        )
        low_ids.append(
            crystals.add_crystal(
                context,
                crystal_type="lesson",
                text=f"Treat low confidence visible concept entry {index} as a proper noun.",
                title=f"Low Visible Concept {index}",
                concept_ids=(concept.id,),
                strength=0.1,
                confidence=0.1,
            )
        )
    high_concept = concepts.create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    high_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Treat high confidence visible concept entry as a proper noun.",
        title="High Visible Concept",
        concept_ids=(high_concept.id,),
        strength=0.95,
        confidence=0.95,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert results[0].id == high_id
    assert high_id in {result.id for result in results}
    assert results[0].score > next(result.score for result in results if result.id == low_ids[0])


def test_concept_label_candidate_limit_applies_after_visibility_scope(
    config: HieronymusConfig,
) -> None:
    context = _context(config, slug="target-series")
    other_context = _context(config, slug="other-series")
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concepts = ConceptStore(config)
    crystals = CrystalStore(config)
    for index in range(60):
        concept = concepts.create_concept(
            "Enchant",
            status="established",
            confidence=0.9,
            scope_type="series",
            scope_key=other_context.scope_key,
        )
        crystals.add_crystal(
            other_context,
            crystal_type="lesson",
            text=f"Treat out-of-scope linked skill entry {index} as a proper noun.",
            title=f"Out Of Scope Linked Skill {index}",
            concept_ids=(concept.id,),
            strength=0.7,
            confidence=0.8,
        )
    target_concept = concepts.create_concept(
        "Enchant",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
    )
    target_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Treat the in-scope linked skill entry as a proper noun.",
        title="In Scope Linked Skill",
        concept_ids=(target_concept.id,),
        strength=0.7,
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "Enchant", limit=5)

    assert [result.id for result in results] == [target_crystal_id]
    assert results[0].concept_ids == (target_concept.id,)
    assert results[0].concept_labels == ("Enchant",)


def test_concept_semantic_tag_candidate_limit_prefers_stronger_later_visible_concept(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concepts = ConceptStore(config)
    crystals = CrystalStore(config)
    low_ids = []
    for index in range(60):
        concept = concepts.create_concept(
            f"Low Menu Skill {index}",
            status="established",
            confidence=0.9,
            scope_type="series",
            scope_key=context.scope_key,
            semantic_tags=("skill:name",),
        )
        low_ids.append(
            crystals.add_crystal(
                context,
                crystal_type="lesson",
                text=f"Treat low confidence tagged concept entry {index} as a proper noun.",
                title=f"Low Tagged Concept {index}",
                concept_ids=(concept.id,),
                strength=0.1,
                confidence=0.1,
            )
        )
    high_concept = concepts.create_concept(
        "High Menu Skill",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("skill:name",),
    )
    high_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Treat high confidence tagged concept entry as a proper noun.",
        title="High Tagged Concept",
        concept_ids=(high_concept.id,),
        strength=0.95,
        confidence=0.95,
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert results[0].id == high_id
    assert high_id in {result.id for result in results}
    assert results[0].score > next(result.score for result in results if result.id == low_ids[0])


def test_concept_semantic_tags_find_linked_crystal_without_text_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    concept = ConceptStore(config).create_concept(
        "Menu Skill",
        status="established",
        confidence=0.9,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("skill:name",),
    )
    crystal_id = CrystalStore(config).add_crystal(
        context,
        crystal_type="lesson",
        text="Treat the weapon imbue skill as a proper noun.",
        title="Imbue Skill Naming",
        concept_ids=(concept.id,),
        strength=0.7,
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "skill:name", limit=5)

    assert [result.id for result in results] == [crystal_id]
    assert results[0].concept_ids == (concept.id,)
    assert results[0].concept_labels == ("Menu Skill",)
    assert results[0].reason == "metadata recall match"


def test_active_relevant_rule_crystals_boost_but_do_not_filter_other_hits(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    lesson_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Yun Enchant keeps its skill-name rendering.",
        title="Enchant Lesson",
        strength=0.6,
        confidence=0.7,
    )
    rule_id = crystals.add_crystal(
        context,
        crystal_type="rule",
        text="Yun Enchant keeps its skill-name rendering.",
        title="Enchant Rule",
        source_credibility="user_rule",
        rule_intent="terminology",
        strength=0.6,
        confidence=0.7,
    )

    results = RecallService(config).recall(session.id, context, "Yun Enchant", limit=5)

    assert [result.id for result in results] == [rule_id, lesson_id]
    assert results[0].is_rule
    assert results[0].score > results[1].score
    assert {result.id for result in results} == {rule_id, lesson_id}


def test_story_scope_boosts_without_removing_relevant_book_one_result(
    config: HieronymusConfig,
) -> None:
    context = _context(config, story_scopes=("book:5/chapter:5",))
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    book_one_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Yun Enchant should keep its title-case skill rendering.",
        title="Book One Enchant",
        strength=0.6,
        confidence=0.7,
        story_scopes=("book:1/chapter:1",),
    )
    book_five_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Yun Enchant should keep its title-case skill rendering.",
        title="Book Five Enchant",
        strength=0.6,
        confidence=0.7,
        story_scopes=("book:5/chapter:5",),
    )

    results = RecallService(config).recall(session.id, context, "Yun Enchant", limit=5)

    assert [result.id for result in results] == [book_five_id, book_one_id]
    assert results[0].score > results[1].score
    assert {result.id for result in results} == {book_five_id, book_one_id}


def test_low_confidence_thought_memory_is_recallable_but_ranks_below_evidence(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    evidence_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Yun Enchant appears as a stable menu rendering.",
        title="Established Enchant Evidence",
        strength=0.8,
        confidence=0.9,
        source_credibility="expert",
    )
    thought_id = crystals.add_crystal(
        context,
        crystal_type="thought",
        text="Yun Enchant might imply a ritual nuance.",
        title="Tentative Enchant Thought",
        strength=0.2,
        confidence=0.2,
        source_credibility="thought",
    )
    with connect(config.database_path) as conn:
        conn.execute("update crystals set is_inferred = 1 where id = ?", (thought_id,))
        conn.commit()

    results = RecallService(config).recall(session.id, context, "Yun Enchant", limit=5)

    assert [result.id for result in results] == [evidence_id, thought_id]
    thought = next(result for result in results if result.id == thought_id)
    assert thought.tier == "long_term"
    assert thought.is_thought
    assert thought.confidence == pytest.approx(0.2)
    assert thought.score < results[0].score


def test_rag_recall_result_enriched_payload_contains_citation_fields() -> None:
    chunk = RagChunkRecord(
        id=12,
        source_id=3,
        series_slug="only-sense-online",
        source_ref="glossary.csv",
        chunk_kind="glossary_entry",
        text="source: Sense\ntarget: Сенс",
        display_text="source: Sense\ntarget: Сенс",
        location="row 2",
        metadata={"source": "Sense", "target": "Сенс"},
        language_tags=("ja", "ru"),
        story_scopes=("book:5/chapter:5",),
        semantic_tags=("skill:name",),
    )

    result = RecallResult.rag(chunk, rank=1, score=0.75, reason="rag glossary match")
    payload = result.enriched_payload()

    assert result.rank == 1
    assert payload["tier"] == "rag"
    assert payload["source_ref"] == "glossary.csv"
    assert payload["chunk_kind"] == "glossary_entry"
    assert payload["location"] == "row 2"
    assert payload["metadata"] == {"source": "Sense", "target": "Сенс"}
    assert payload["language_tags"] == ("ja", "ru")
    assert payload["score"] == 0.75
    assert payload["rank_reason"] == "rag glossary match"

import pytest

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.recall import RecallService
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(
    config: HieronymusConfig,
    *,
    story_scopes: tuple[str, ...] = (),
    semantic_tags: tuple[str, ...] = (),
) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
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

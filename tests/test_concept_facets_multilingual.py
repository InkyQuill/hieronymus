from __future__ import annotations

import pytest

from hieronymus.concepts import VALID_FACET_KINDS, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.dreaming import DreamService
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
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
    )


def test_multilingual_facets_store_language_scopes_tags_and_canonical_marker(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Yun")

    ja = store.add_facet(
        concept.id,
        "ユン",
        kind="name",
        language_tags=("ja", "en"),
        is_canonical=True,
        story_scopes=("volume:5",),
        semantic_tags=("character:protagonist",),
    )
    en = store.add_facet(concept.id, "Yun", kind="name", language_tags=("en",))
    ru = store.add_facet(concept.id, "Юн", kind="rendering", language_tags=("ru",))

    facets = store.list_facets(concept.id)

    assert VALID_FACET_KINDS == {"name", "rendering", "description", "note"}
    assert facets[0].id == ja.id
    assert facets[0].is_canonical is True
    assert facets[0].kind == "name"
    assert facets[0].language == "ja"
    assert facets[0].language_tags == ("ja", "en")
    assert facets[0].story_scopes == ("volume:5",)
    assert facets[0].semantic_tags == ("character:protagonist",)
    assert en.kind == "name"
    assert ru.kind == "rendering"
    assert store.search("Юн") == [concept]
    assert store.search("Yun") == [concept]


def test_partial_concept_with_one_language_facet_remains_valid(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Crafting")

    facet = store.add_facet(concept.id, "Crafting", kind="name", language_tags=("en",))

    assert facet.language == "en"
    assert facet.language_tags == ("en",)
    assert store.list_facets(concept.id) == [facet]
    assert store.search("Crafting") == [concept]


def test_store_rejects_empty_facet_content_and_invalid_public_kind(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Sense")

    with pytest.raises(ValueError, match="concept facet value must not be empty"):
        store.add_facet(concept.id, "   ", kind="name")

    with pytest.raises(ValueError, match="unknown concept facet kind"):
        store.add_facet(concept.id, "Sense", kind="alias")

    legacy = store.add_facet(concept.id, "Sense", facet_type="alias", language="EN")

    assert legacy.kind == "name"
    assert legacy.facet_type == "alias"
    assert legacy.language == "en"
    assert legacy.language_tags == ("en",)


def test_semantic_tag_search_disambiguates_ambiguous_text_query(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    talent = store.create_concept("Sense", semantic_tags=("role:talent",))
    subskill = store.create_concept("Sense", semantic_tags=("role:subskill",))
    store.add_facet(talent.id, "Sense", kind="name", semantic_tags=("ui:aptitude",))

    assert store.search("Sense", semantic_tag="role:talent") == [talent]
    assert store.search("Sense", semantic_tag="role:subskill") == [subskill]
    assert store.search("Sense", semantic_tag="ui:aptitude") == [talent]


def test_facet_story_scope_boosts_linked_crystal_recall_without_filtering_unscoped(
    config: HieronymusConfig,
) -> None:
    context = _context(config, story_scopes=("volume:5",))
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    crystals = CrystalStore(config)
    concepts = ConceptStore(config)

    unscoped_concept = concepts.create_concept("Guarded Crafting")
    scoped_concept = concepts.create_concept("Scoped Guarded Crafting")
    unscoped_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    scoped_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    concepts.add_facet(unscoped_concept.id, "guarded crafting", kind="note")
    concepts.add_facet(
        scoped_concept.id,
        "guarded crafting",
        kind="note",
        story_scopes=("volume:5",),
    )
    concepts.link_crystal(
        unscoped_crystal_id,
        unscoped_concept.id,
        link_type="mentions",
        confidence=0.8,
    )
    concepts.link_crystal(
        scoped_crystal_id,
        scoped_concept.id,
        link_type="mentions",
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "guarded crafting", limit=10)

    assert [result.crystal.id for result in results] == [scoped_crystal_id, unscoped_crystal_id]
    assert results[0].score > results[1].score
    assert {result.crystal.id for result in results} == {scoped_crystal_id, unscoped_crystal_id}


def test_unrelated_scoped_facets_do_not_rerank_recall_results(
    config: HieronymusConfig,
) -> None:
    context = _context(config, story_scopes=("volume:5",))
    session = WorkspaceStore(config).start_session(context)
    crystals = CrystalStore(config)
    concepts = ConceptStore(config)

    unrelated_concept = concepts.create_concept("Unrelated Scope")
    matched_concept = concepts.create_concept("Matched Crafting")
    unrelated_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    matched_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    concepts.add_facet(
        unrelated_concept.id,
        "unrelated honorific detail",
        kind="note",
        story_scopes=("volume:5",),
    )
    concepts.add_facet(matched_concept.id, "guarded crafting", kind="note")
    concepts.link_crystal(
        unrelated_crystal_id,
        unrelated_concept.id,
        link_type="mentions",
        confidence=0.8,
    )
    concepts.link_crystal(
        matched_crystal_id,
        matched_concept.id,
        link_type="mentions",
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "guarded crafting", limit=10)

    assert [result.crystal.id for result in results] == [matched_crystal_id, unrelated_crystal_id]
    assert results[0].score > results[1].score


def test_unrelated_context_semantic_tags_do_not_rerank_recall_results(
    config: HieronymusConfig,
) -> None:
    context = _context(config, semantic_tags=("role:unrelated",))
    session = WorkspaceStore(config).start_session(context)
    crystals = CrystalStore(config)
    concepts = ConceptStore(config)

    unrelated_concept = concepts.create_concept(
        "Unrelated Tagged Concept",
        semantic_tags=("role:unrelated",),
    )
    matched_concept = concepts.create_concept("Matched Crafting")
    unrelated_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    matched_crystal_id = crystals.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded crafting language for failed checks.",
        strength=0.5,
        confidence=0.5,
    )
    concepts.add_facet(matched_concept.id, "guarded crafting", kind="note")
    concepts.link_crystal(
        unrelated_crystal_id,
        unrelated_concept.id,
        link_type="mentions",
        confidence=0.8,
    )
    concepts.link_crystal(
        matched_crystal_id,
        matched_concept.id,
        link_type="mentions",
        confidence=0.8,
    )

    results = RecallService(config).recall(session.id, context, "guarded crafting", limit=10)

    assert [result.crystal.id for result in results] == [matched_crystal_id, unrelated_crystal_id]
    assert results[0].score > results[1].score


def test_global_migration_backfills_legacy_facet_language_tags(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        now = "2026-06-10T00:00:00+00:00"
        concept_id = conn.execute(
            """
            insert into concepts(canonical_name, created_at, updated_at)
            values ('Legacy Sense', ?, ?)
            """,
            (now, now),
        ).lastrowid
        conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              created_at,
              updated_at
            )
            values (?, 'ru', 'rendering', 'Сенс', ?, ?)
            """,
            (concept_id, now, now),
        )
        conn.commit()

    ConceptStore(config)

    facets = ConceptStore(config).list_facets(concept_id)
    assert facets[0].language_tags == ("ru",)


def test_dream_facets_penalize_malformed_metadata(
    config: HieronymusConfig,
) -> None:
    class FacetProvider:
        name = "facet-provider"

        def crystallize(self, _context, _memories):
            return {
                "facets": [
                    {
                        "concept_name": "Yun",
                        "body": "Юн",
                        "kind": "alias",
                        "language_tags": ["ru"],
                        "confidence": 0.7,
                    },
                    {
                        "concept_name": "Yun",
                        "value": "Yun",
                        "kind": "name",
                        "language_tags": ["en", 7, ""],
                        "story_scopes": {"bad": "shape"},
                        "semantic_tags": [None, "character:protagonist"],
                        "canonical": "true",
                        "confidence": 0.9,
                    },
                ],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mentor", "note", "Yun rendering was discussed.")
    workspace.complete_session(session.id)

    run = DreamService(config, FacetProvider()).run_all()
    concept = ConceptStore(config).search("Yun")[0]
    facets = ConceptStore(config).list_facets(concept.id)

    assert run.status == "completed"
    assert [
        (facet.value, facet.kind, facet.language_tags, facet.semantic_tags) for facet in facets
    ] == [
        ("Yun", "name", ("en",), ("character:protagonist",)),
        ("Юн", "name", ("ru",), ()),
    ]
    assert facets[0].confidence == pytest.approx(0.05)
    assert facets[0].is_canonical is True
    assert facets[1].confidence == pytest.approx(0.3)


def test_dream_facet_missing_content_is_rejected(config: HieronymusConfig) -> None:
    class MissingFacetContentProvider:
        name = "missing-facet-content-provider"

        def crystallize(self, _context, _memories):
            return {
                "facets": [
                    {
                        "concept_name": "Yun",
                        "kind": "name",
                        "language_tags": ["en"],
                    }
                ],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mentor", "note", "Yun rendering was discussed.")
    workspace.complete_session(session.id)

    with pytest.raises(ValueError, match=r"facets\[0\]\.value is required"):
        DreamService(config, MissingFacetContentProvider()).run_all()

    assert ConceptStore(config).search("Yun") == []


def test_dream_facet_missing_kind_defaults_to_note_without_penalty(
    config: HieronymusConfig,
) -> None:
    class NoKindFacetProvider:
        name = "no-kind-facet-provider"

        def crystallize(self, _context, _memories):
            return {
                "facets": [
                    {
                        "concept_name": "Yun",
                        "value": "Plain note",
                        "confidence": 0.6,
                    }
                ],
            }

    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    workspace.add_short_term_memory(session.id, "mentor", "note", "Yun note was discussed.")
    workspace.complete_session(session.id)

    run = DreamService(config, NoKindFacetProvider()).run_all()
    concept = ConceptStore(config).search("Yun")[0]
    facets = ConceptStore(config).list_facets(concept.id)

    assert run.status == "completed"
    assert [(facet.value, facet.kind, facet.confidence) for facet in facets] == [
        ("Plain note", "note", pytest.approx(0.6))
    ]

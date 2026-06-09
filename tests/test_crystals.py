import sqlite3

import pytest

from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


def _context(
    config: HieronymusConfig,
    *,
    slug: str = "only-sense-online",
    tags: tuple[str, ...] = (),
) -> TranslationContext:
    series = Registry(config).create_series(
        slug=slug,
        title=slug.replace("-", " ").title(),
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
        tags=tags,
    )


def test_add_and_search_series_crystal(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)
    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        title="Inventory UI",
        text="Render inventory menu labels with concise Russian nouns.",
    )

    results = store.search(context, "inventory menu")

    assert results[0].id == crystal_id
    assert results[0].crystal_type == "lesson"


def test_scores_are_clamped_when_adding_crystal(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)
    crystal_id = store.add_crystal(
        context,
        crystal_type="concept",
        text="Sense is treated as an in-world skill category.",
        strength=2.5,
        confidence=-1.0,
    )

    crystal = store.get(crystal_id)

    assert crystal.strength == 1.0
    assert crystal.confidence == 0.0


def test_crystal_scalar_metadata_defaults_hydrate_from_store(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)
    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Render inventory menu labels with concise Russian nouns.",
    )

    crystal = store.get(crystal_id)

    assert crystal.source_credibility == "observation"
    assert crystal.rule_intent == ""
    assert crystal.malformed_penalty == 0.0
    assert crystal.supersedes_crystal_id is None


def test_crystal_scalar_metadata_round_trips_through_store(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)
    base_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use terse inventory labels.",
    )
    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Replace malformed UI label guidance.",
        source_credibility="user_rule",
        rule_intent="terminology_override",
        malformed_penalty=0.35,
        supersedes_crystal_id=base_id,
    )

    crystal = store.get(crystal_id)

    assert crystal.source_credibility == "user_rule"
    assert crystal.rule_intent == "terminology_override"
    assert crystal.malformed_penalty == 0.35
    assert crystal.supersedes_crystal_id == base_id


def test_add_crystal_accepts_new_crystal_types(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)

    crystal_ids = [
        store.add_crystal(
            context,
            crystal_type=crystal_type,
            text=f"{crystal_type} text.",
        )
        for crystal_type in ("rule", "thought", "observation", "concept_note")
    ]

    assert [store.get(crystal_id).crystal_type for crystal_id in crystal_ids] == [
        "rule",
        "thought",
        "observation",
        "concept_note",
    ]


def test_add_crystal_stores_and_hydrates_side_table_fields(
    config: HieronymusConfig,
) -> None:
    context = _context(config, tags=("legacy",))
    concept_store = ConceptStore(config)
    first_concept_id = concept_store.create_or_reinforce("Sense")
    second_concept_id = concept_store.create_or_reinforce("Crafting")
    store = CrystalStore(config)

    crystal_id = store.add_crystal(
        context,
        crystal_type="rule",
        text="Keep Sense and crafting terms stable.",
        confidence=0.65,
        story_scopes=(" volume:1 ", "", "chapter:2", "volume:1"),
        semantic_tags=(" term:system ", "term:system", "", "style:ui"),
        concept_ids=(second_concept_id, first_concept_id, second_concept_id),
    )

    crystal = store.get(crystal_id)

    assert crystal.story_scopes == ("chapter:2", "volume:1")
    assert crystal.semantic_tags == ("style:ui", "term:system")
    assert crystal.concept_ids == (first_concept_id, second_concept_id)
    with connect(config.database_path) as conn:
        row = conn.execute(
            "select tags_json from crystals where id = ?",
            (crystal_id,),
        ).fetchone()
        concept_rows = conn.execute(
            """
            select concept_id, link_type, confidence
            from crystal_concepts
            where crystal_id = ?
            order by concept_id
            """,
            (crystal_id,),
        ).fetchall()

    assert row["tags_json"] == '["style:ui", "term:system"]'
    assert [(row["concept_id"], row["link_type"], row["confidence"]) for row in concept_rows] == [
        (first_concept_id, "mentions", 0.65),
        (second_concept_id, "mentions", 0.65),
    ]


def test_add_crystal_falls_back_to_context_tags_for_legacy_tags_json(
    config: HieronymusConfig,
) -> None:
    context = _context(config, tags=("legacy", "memory"))
    store = CrystalStore(config)

    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep legacy context tags in tags_json.",
    )

    crystal = store.get(crystal_id)
    with connect(config.database_path) as conn:
        row = conn.execute(
            "select tags_json from crystals where id = ?",
            (crystal_id,),
        ).fetchone()

    assert row["tags_json"] == '["legacy", "memory"]'
    assert crystal.semantic_tags == ()


def test_add_crystal_missing_concept_id_raises_fk_error(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)

    with pytest.raises(sqlite3.IntegrityError):
        store.add_crystal(
            context,
            crystal_type="lesson",
            text="This references a missing concept.",
            concept_ids=(999,),
        )


def test_superseded_crystal_reference_nulls_when_source_is_deleted(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)
    base_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use terse inventory labels.",
    )
    replacement_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Replace malformed UI label guidance.",
        supersedes_crystal_id=base_id,
    )

    with connect(config.database_path) as conn:
        conn.execute("delete from crystals where id = ?", (base_id,))
        row = conn.execute(
            "select supersedes_crystal_id from crystals where id = ?",
            (replacement_id,),
        ).fetchone()

    assert row["supersedes_crystal_id"] is None


def test_search_prefers_higher_strength_when_text_relevance_matches(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)
    low_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.1,
        confidence=0.5,
    )
    high_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.9,
        confidence=0.5,
    )

    results = store.search(context, "guarded crafting")

    assert [result.id for result in results[:2]] == [high_id, low_id]


def test_search_scored_exposes_weighted_scores_without_changing_search_api(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)
    low_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.1,
        confidence=0.5,
    )
    high_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Use guarded phrasing for crafting failures.",
        strength=0.9,
        confidence=0.5,
    )

    plain_results = store.search(context, "guarded crafting")
    scored_results = store.search_scored(context, "guarded crafting")

    assert [result.id for result in plain_results[:2]] == [high_id, low_id]
    assert [crystal.id for crystal, _score in scored_results[:2]] == [high_id, low_id]
    assert scored_results[0][1] > scored_results[1][1]


def test_search_blends_quality_with_text_relevance(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)
    low_quality_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Guarded crafting.",
        strength=0.0,
        confidence=0.0,
    )
    high_quality_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text=(
            "Guarded crafting guidance applies when the prose needs slower, "
            "more careful framing around failed item creation and retries."
        ),
        strength=1.0,
        confidence=1.0,
    )

    results = store.search(context, "guarded crafting")

    assert [result.id for result in results[:2]] == [high_quality_id, low_quality_id]


def test_search_excludes_other_series_crystals(config: HieronymusConfig) -> None:
    context = _context(config)
    other_context = _context(config, slug="another-series")
    store = CrystalStore(config)
    store.add_crystal(
        other_context,
        crystal_type="lesson",
        text="Use brisk narration for battle scenes.",
    )

    assert store.search(context, "brisk narration") == []


def test_search_includes_global_crystals(config: HieronymusConfig) -> None:
    context = _context(config)
    store = CrystalStore(config)
    with connect(config.database_path) as conn:
        cursor = conn.execute(
            """
            insert into crystals(
              crystal_type,
              text,
              title,
              scope_type,
              strength,
              confidence,
              status,
              created_at,
              updated_at
            )
            values (
              'erudition',
              'Honorific suffixes often signal social distance.',
              '',
              'global',
              0.5,
              0.5,
              'active',
              '2026-06-06T00:00:00+00:00',
              '2026-06-06T00:00:00+00:00'
            )
            """
        )
        crystal_id = int(cursor.lastrowid)
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, '', ?)",
            (crystal_id, "Honorific suffixes often signal social distance."),
        )
        conn.commit()

    results = store.search(context, "honorific social")

    assert [result.id for result in results] == [crystal_id]


def test_add_crystal_links_source_memories(config: HieronymusConfig) -> None:
    context = _context(config)
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_id = workspace.add_short_term_memory(
        session.id,
        source_role="mentor",
        kind="style-note",
        text="Keep crafting terminology terse.",
    )
    store = CrystalStore(config)

    crystal_id = store.add_crystal(
        context,
        crystal_type="lesson",
        text="Keep crafting terminology terse.",
        source_memory_ids=[memory_id],
    )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select crystal_id, short_term_memory_id
            from crystal_sources
            """
        ).fetchall()

    assert [(row["crystal_id"], row["short_term_memory_id"]) for row in rows] == [
        (crystal_id, memory_id)
    ]


def test_invalid_type_status_and_empty_text_raise_value_error(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    store = CrystalStore(config)

    with pytest.raises(ValueError, match="crystal_type"):
        store.add_crystal(context, crystal_type="memory", text="Some text.")
    with pytest.raises(ValueError, match="status"):
        store.add_crystal(
            context,
            crystal_type="lesson",
            status="draft",
            text="Some text.",
        )
    with pytest.raises(ValueError, match="text"):
        store.add_crystal(context, crystal_type="lesson", text="   ")


def test_get_unknown_crystal_raises_key_error(config: HieronymusConfig) -> None:
    store = CrystalStore(config)

    with pytest.raises(KeyError, match="crystal"):
        store.get(999)

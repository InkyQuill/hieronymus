from __future__ import annotations

import pytest

from hieronymus.concepts import (
    CONCEPT_ARCHIVED,
    CONCEPT_CANDIDATE,
    CONCEPT_ESTABLISHED,
    CONCEPT_MERGED,
    ConceptStore,
)
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import TranslationContext


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translate",
    )


def _crystal(config: HieronymusConfig, text: str, *, crystal_type: str = "lesson") -> int:
    return CrystalStore(config).add_crystal(
        _context(),
        crystal_type=crystal_type,
        text=text,
    )


def test_legacy_vague_and_solid_statuses_read_as_public_lifecycle(
    config: HieronymusConfig,
) -> None:
    with connect(config.database_path) as conn:
        apply_migration(conn, "global.sql")
        conn.execute(
            """
            insert into concepts(canonical_name, status, created_at, updated_at)
            values ('Weak Sense', 'vague', '2026-06-10T00:00:00+00:00', '2026-06-10T00:00:00+00:00')
            """
        )
        solid_id = conn.execute(
            """
            insert into concepts(canonical_name, status, created_at, updated_at)
            values ('Stable Sense', 'solid', ?, ?)
            """,
            ("2026-06-10T00:00:00+00:00", "2026-06-10T00:00:00+00:00"),
        ).lastrowid
        conn.commit()

    store = ConceptStore(config)

    assert store.get(solid_id).status == CONCEPT_ESTABLISHED
    assert [concept.canonical_name for concept in store.list_concepts(status="candidate")] == [
        "Weak Sense"
    ]
    assert [concept.canonical_name for concept in store.list_concepts(status="solid")] == [
        "Stable Sense"
    ]


def test_candidate_promotes_to_established_after_confidence_and_linked_evidence(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Sense", confidence=0.8)

    first_crystal = _crystal(config, "Sense is a stable game-system concept.")
    second_crystal = _crystal(config, "Sense appears again with the same meaning.")

    store.link_crystal(first_crystal, concept.id, link_type="mentions", confidence=0.9)
    assert store.get(concept.id).status == CONCEPT_CANDIDATE

    store.link_crystal(second_crystal, concept.id, link_type="mentions", confidence=0.9)
    assert store.get(concept.id).status == CONCEPT_ESTABLISHED


def test_rename_keeps_old_label_searchable_as_single_facet(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Yun")
    store.add_facet(concept.id, "Yun", facet_type="alias")

    renamed = store.rename_concept(concept.id, "Yun Talent")
    facets = store.list_facets(concept.id)

    assert renamed.canonical_name == "Yun Talent"
    assert [facet.value for facet in facets].count("Yun") == 1


def test_rename_old_label_is_searchable_through_store_api(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Yun")

    renamed = store.rename_concept(concept.id, "Yun Talent")

    assert store.search("Yun") == [renamed]


def test_one_crystal_can_link_to_yun_sense_and_enchant_concepts(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    yun = store.create_concept("Yun")
    sense = store.create_concept("Sense")
    enchant = store.create_concept("Enchant")
    crystal_id = _crystal(config, "Yun compares Sense and Enchant.")

    store.link_crystal(crystal_id, yun.id, link_type="mentions", confidence=0.8)
    store.link_crystal(crystal_id, sense.id, link_type="mentions", confidence=0.8)
    store.link_crystal(crystal_id, enchant.id, link_type="mentions", confidence=0.8)

    assert store.concept_ids_for_crystal(crystal_id) == (yun.id, sense.id, enchant.id)


def test_same_visible_name_can_be_differentiated_by_semantic_tags(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    talent = store.create_concept("Sense", semantic_tags=("talent",))
    subskill = store.create_concept("Sense", semantic_tags=("subskill",))

    assert talent.id != subskill.id
    assert talent.canonical_name == subskill.canonical_name == "Sense"
    assert store.list_concepts(semantic_tag="talent") == [talent]
    assert store.list_concepts(semantic_tag="subskill") == [subskill]


def test_reinforce_disambiguates_same_visible_name_by_semantic_tags(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    talent = store.create_concept("Sense", confidence=0.2, semantic_tags=("talent",))
    subskill = store.create_concept("Sense", confidence=0.2, semantic_tags=("subskill",))

    reinforced_id = store.create_or_reinforce(
        "Sense",
        tags=("subskill",),
        confidence_delta=0.3,
    )

    assert reinforced_id == subskill.id
    assert store.get(talent.id).confidence == 0.2
    assert store.get(subskill.id).confidence == 0.5


def test_reinforce_does_not_update_archived_or_merged_same_name_concepts(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    archived = store.create_concept("Sense", confidence=0.6, semantic_tags=("talent",))
    merged_source = store.create_concept("Merged Sense", confidence=0.6, semantic_tags=("legacy",))
    merge_target = store.create_concept("Merged Target", confidence=0.6)

    store.archive_concept(archived.id, "No longer active.")
    store.merge_concepts(merged_source.id, merge_target.id, "Consolidated.")

    new_archived_name_id = store.create_or_reinforce(
        "Sense",
        tags=("talent",),
        confidence_delta=0.2,
    )
    new_merged_name_id = store.create_or_reinforce(
        "Merged Sense",
        tags=("legacy",),
        confidence_delta=0.2,
    )

    assert new_archived_name_id != archived.id
    assert new_merged_name_id != merged_source.id
    assert store.get(archived.id).status == CONCEPT_ARCHIVED
    assert store.get(archived.id).confidence == 0.6
    assert store.get(merged_source.id).status == CONCEPT_MERGED
    assert store.get(merged_source.id).confidence == 0.6


def test_reinforce_single_active_same_name_concept_absorbs_disjoint_tags(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Sense", confidence=0.2, semantic_tags=("talent",))

    reinforced_id = store.create_or_reinforce(
        "Sense",
        tags=("subskill",),
        confidence_delta=0.3,
    )

    reinforced = store.get(concept.id)
    assert reinforced_id == concept.id
    assert reinforced.confidence == 0.5
    assert reinforced.tags == ("subskill", "talent")


def test_merge_rejects_archived_target(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun")
    target = store.create_concept("Yun Talent")
    store.archive_concept(target.id, "Inactive target.")

    with pytest.raises(ValueError, match="merge target concept must be active"):
        store.merge_concepts(source.id, target.id, "Should not merge into archived target.")


def test_merge_rejects_merged_target(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun")
    merged_target = store.create_concept("Yun Talent")
    final_target = store.create_concept("Yun Talent Final")
    store.merge_concepts(merged_target.id, final_target.id, "First merge.")

    with pytest.raises(ValueError, match="merge target concept must be active"):
        store.merge_concepts(source.id, merged_target.id, "Should not merge into merged target.")


def test_merge_rejects_archived_source_without_moving_rule_link(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Archived Sense")
    target = store.create_concept("Active Sense")
    crystal_id = CrystalStore(config).add_crystal(
        _context(),
        crystal_type="rule",
        text="Archived Sense is translated as Sense.",
        strength=0.9,
        confidence=0.9,
    )

    store.link_crystal(crystal_id, source.id, link_type="rule", confidence=0.9)
    store.archive_concept(source.id, "Inactive source.")

    before = CrystalStore(config).validate_rule_crystal(crystal_id)
    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        store.merge_concepts(source.id, target.id, "Should not move inactive rule link.")
    after = CrystalStore(config).validate_rule_crystal(crystal_id)

    assert store.concept_ids_for_crystal(crystal_id) == (source.id,)
    assert before["enforceable"] is False
    assert after["enforceable"] is False
    assert after["warnings"] == ["rule crystal is not linked to an active concept"]


def test_merge_rejects_merged_source(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun")
    first_target = store.create_concept("Yun Talent")
    second_target = store.create_concept("Yun Talent Final")
    store.merge_concepts(source.id, first_target.id, "First merge.")

    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        store.merge_concepts(source.id, second_target.id, "Should not merge twice.")


def test_archive_rejects_inactive_concepts(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    archived = store.create_concept("Archived Sense")
    merged = store.create_concept("Merged Sense")
    target = store.create_concept("Active Sense")

    store.archive_concept(archived.id, "Inactive.")
    store.merge_concepts(merged.id, target.id, "Merged.")

    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        store.archive_concept(archived.id, "Already archived.")
    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        store.archive_concept(merged.id, "Must not rewrite merged lifecycle.")

    merged_after = store.get(merged.id)
    assert merged_after.status == CONCEPT_MERGED
    assert merged_after.merged_into_concept_id == target.id


def test_merge_preserves_crystal_links_rule_links_and_facet_searchability(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun", semantic_tags=("talent",))
    target = store.create_concept("Yun Talent", semantic_tags=("character",), confidence=0.8)
    store.add_facet(source.id, "Yun", facet_type="alias")
    store.add_facet(source.id, "Yun-chan", facet_type="alias")
    lesson_id = _crystal(config, "Yun is the relevant talent holder.")
    rule_id = _crystal(
        config,
        "Always render Yun consistently in rules.",
        crystal_type="rule",
    )

    store.link_crystal(lesson_id, source.id, link_type="mentions", confidence=0.6)
    store.link_crystal(rule_id, source.id, link_type="rule", confidence=0.9)

    store.merge_concepts(source.id, target.id, "Yun was consolidated into Yun Talent.")

    merged_source = store.get(source.id)
    target_after_merge = store.get(target.id)

    assert merged_source.status == CONCEPT_MERGED
    assert merged_source.merged_into_concept_id == target.id
    assert store.concept_ids_for_crystal(lesson_id) == (target.id,)
    assert store.concept_ids_for_crystal(rule_id) == (target.id,)
    assert sorted(facet.value for facet in store.list_facets(target.id)) == ["Yun", "Yun-chan"]
    assert target_after_merge.tags == ("character", "talent")


def test_merge_preserves_source_canonical_name_when_source_has_no_facets(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun")
    target = store.create_concept("Yun Talent")

    store.merge_concepts(source.id, target.id, "Yun became Yun Talent.")

    assert [facet.value for facet in store.list_facets(target.id)] == ["Yun"]
    assert store.search("Yun") == [target]


def test_merge_deduplicates_active_facets_by_identity(
    config: HieronymusConfig,
) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Yun")
    target = store.create_concept("Yun Talent")
    store.add_facet(source.id, "Yun", facet_type="alias", language="en")
    store.add_facet(target.id, "Yun", facet_type="alias", language="en")

    store.merge_concepts(source.id, target.id, "Duplicate aliases collapse.")

    facets = store.list_facets(target.id)
    matching = [
        facet
        for facet in facets
        if facet.value == "Yun" and facet.facet_type == "alias" and facet.language == "en"
    ]
    assert len(matching) == 1


def test_link_crystal_rejects_archived_concept(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    concept = store.create_concept("Sense")
    crystal_id = _crystal(config, "Sense should not link to archived concepts.")
    store.archive_concept(concept.id, "Inactive.")

    with pytest.raises(ValueError, match="cannot link crystal to inactive concept"):
        store.link_crystal(crystal_id, concept.id, link_type="mentions", confidence=0.5)


def test_link_crystal_rejects_merged_concept(config: HieronymusConfig) -> None:
    store = ConceptStore(config)
    source = store.create_concept("Sense")
    target = store.create_concept("Sense Target")
    crystal_id = _crystal(config, "Sense should not link to merged concepts.")
    store.merge_concepts(source.id, target.id, "Merged.")

    with pytest.raises(ValueError, match="cannot link crystal to inactive concept"):
        store.link_crystal(crystal_id, source.id, link_type="mentions", confidence=0.5)

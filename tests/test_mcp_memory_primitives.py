from __future__ import annotations

import asyncio

import pytest

from hieronymus.config import load_config
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext

NEW_MEMORY_PRIMITIVE_TOOLS = {
    "hieronymus_concept_list",
    "hieronymus_concept_get",
    "hieronymus_concept_create",
    "hieronymus_concept_update",
    "hieronymus_concept_archive",
    "hieronymus_concept_merge",
    "hieronymus_concept_rename",
    "hieronymus_concept_facet_add",
    "hieronymus_concept_facet_update",
    "hieronymus_concept_facet_list",
    "hieronymus_concept_facet_set_canonical",
    "hieronymus_concept_semantic_tags_set",
    "hieronymus_crystal_link_concept",
    "hieronymus_crystal_story_scopes_set",
    "hieronymus_crystal_semantic_tags_set",
    "hieronymus_rule_crystals_list",
    "hieronymus_rule_crystal_archive",
    "hieronymus_rule_crystal_validate",
}
COMPATIBILITY_DESCRIPTION = (
    "Compatibility wrapper. New workflows should use concept, facet, short-term memory, "
    "and rule-crystal primitives."
)


def test_memory_primitive_tools_are_registered() -> None:
    from hieronymus import mcp_server

    tools = {tool.name: tool for tool in asyncio.run(mcp_server.server.list_tools())}

    assert NEW_MEMORY_PRIMITIVE_TOOLS <= set(tools)
    assert tools["hieronymus_termbase_contract"].description == COMPATIBILITY_DESCRIPTION
    assert tools["hieronymus_termbase_propose"].description == COMPATIBILITY_DESCRIPTION
    assert tools["hieronymus_termbase_approve"].description == COMPATIBILITY_DESCRIPTION
    assert tools["hieronymus_concept_proposals_list"].description == COMPATIBILITY_DESCRIPTION


def test_concept_create_facet_add_crystal_link_and_recall_via_mcp(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    series = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    session = mcp_server.hieronymus_session_start(series["slug"])
    concept = mcp_server.hieronymus_concept_create(
        "センス",
        description="In-world skill category.",
        confidence=0.8,
        semantic_tags=["ability"],
        series_slug=series["slug"],
    )
    facet = mcp_server.hieronymus_concept_facet_add(
        concept["id"],
        "Sense",
        language_tags=["en"],
        kind="rendering",
        confidence=0.9,
        is_canonical=True,
        story_scopes=["volume:1"],
    )

    assert mcp_server.hieronymus_concept_get(concept["id"]) == concept
    assert mcp_server.hieronymus_concept_facet_list(concept["id"]) == [facet]

    context = TranslationContext(
        series_slug=series["slug"],
        source_language=series["source_language"],
        target_language=series["target_language"],
        task_type="translation",
    )
    crystal_id = CrystalStore(load_config()).add_crystal(
        context,
        crystal_type="lesson",
        title="Sense rendering",
        text="Translate センス as Sense in user interface labels.",
        strength=0.8,
        confidence=0.9,
    )

    linked = mcp_server.hieronymus_crystal_link_concept(
        crystal_id,
        concept["id"],
        confidence=0.9,
    )
    scoped = mcp_server.hieronymus_crystal_story_scopes_set(
        crystal_id,
        ["volume:1"],
        confidence=0.9,
    )
    tagged = mcp_server.hieronymus_crystal_semantic_tags_set(
        crystal_id,
        ["ability"],
        confidence=0.9,
    )

    assert linked["concept_ids"] == [concept["id"]]
    assert scoped["story_scopes"] == ["volume:1"]
    assert tagged["semantic_tags"] == ["ability"]

    results = mcp_server.hieronymus_recall(
        session["session_id"],
        series["slug"],
        "Sense interface labels",
        limit=5,
    )

    assert results[0]["source"] == "long_term"
    assert results[0]["crystal"]["id"] == crystal_id
    assert results[0]["crystal"]["concept_ids"] == [concept["id"]]
    assert results[0]["concept_labels"] == ["センス"]


def test_user_correction_mcp_path_stores_short_term_memory_without_rule_crystal(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    series = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    session = mcp_server.hieronymus_session_start(series["slug"])

    feedback = mcp_server.hieronymus_feedback(
        session["session_id"],
        "Use Sense, not Feeling, for センス.",
    )

    with connect(load_config().database_path) as conn:
        memories = conn.execute("select * from short_term_memories order by id").fetchall()
        rule_count = conn.execute(
            "select count(*) from crystals where crystal_type = 'rule'"
        ).fetchone()[0]

    assert feedback == {"memory_id": 1}
    assert len(memories) == 1
    assert memories[0]["source_role"] == "user"
    assert memories[0]["kind"] == "correction"
    assert memories[0]["text"] == "Use Sense, not Feeling, for センス."
    assert rule_count == 0
    assert mcp_server.hieronymus_rule_crystals_list() == []


def test_rule_crystal_review_primitives_validate_list_and_archive(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    series = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    concept = mcp_server.hieronymus_concept_create(
        "センス",
        confidence=0.9,
        series_slug=series["slug"],
    )
    context = TranslationContext(
        series_slug=series["slug"],
        source_language=series["source_language"],
        target_language=series["target_language"],
        task_type="translation",
    )
    crystal_id = CrystalStore(load_config()).add_crystal(
        context,
        crystal_type="rule",
        text="センス is translated as Sense, not Feeling.",
        strength=0.9,
        confidence=0.9,
    )
    mcp_server.hieronymus_crystal_link_concept(crystal_id, concept["id"], confidence=0.9)

    validation = mcp_server.hieronymus_rule_crystal_validate(crystal_id)
    listed = mcp_server.hieronymus_rule_crystals_list(
        status="active",
        series_slug=series["slug"],
    )
    archived = mcp_server.hieronymus_rule_crystal_archive(crystal_id)

    assert validation == {
        "crystal_id": crystal_id,
        "valid": True,
        "enforceable": True,
        "errors": [],
        "warnings": [],
        "parsed_rule": {
            "source_text": "センス",
            "canonical_translation": "Sense",
            "forbidden_variants": ["Feeling"],
        },
    }
    assert [crystal["id"] for crystal in listed] == [crystal_id]
    assert archived["status"] == "archived"


def test_lifecycle_and_facet_mutation_primitives_via_mcp(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    series = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    concept = mcp_server.hieronymus_concept_create(
        "センス",
        description="Draft.",
        confidence=0.3,
        series_slug=series["slug"],
    )
    updated = mcp_server.hieronymus_concept_update(
        concept["id"],
        description="Stable skill category.",
        status="established",
        confidence=0.9,
    )
    facet = mcp_server.hieronymus_concept_facet_add(
        concept["id"],
        "Sense",
        kind="name",
        language_tags=["en"],
        story_scopes=["volume:1"],
        semantic_tags=["old"],
    )
    renamed = mcp_server.hieronymus_concept_rename(concept["id"], "Sense Concept")
    canonical = mcp_server.hieronymus_concept_facet_set_canonical(
        concept["id"],
        facet["id"],
    )
    replacement = mcp_server.hieronymus_concept_facet_update(
        facet["id"],
        value="Sense",
        story_scopes=["volume:2"],
        semantic_tags=["new"],
        is_canonical=True,
    )
    idempotent = mcp_server.hieronymus_concept_facet_update(
        facet["id"],
        story_scopes=["volume:2"],
        semantic_tags=["new"],
    )
    tagged = mcp_server.hieronymus_concept_semantic_tags_set(
        concept["id"],
        ["ability", "ui"],
    )
    target = mcp_server.hieronymus_concept_create(
        "Ability",
        confidence=0.8,
        series_slug=series["slug"],
    )
    merged = mcp_server.hieronymus_concept_merge(
        concept["id"],
        target["id"],
        reason="Consolidate.",
    )

    assert updated["description"] == "Stable skill category."
    assert updated["status"] == "established"
    assert renamed["canonical_name"] == "Sense Concept"
    assert canonical["is_canonical"] is True
    assert replacement["story_scopes"] == ["volume:2"]
    assert replacement["semantic_tags"] == ["new"]
    assert idempotent["story_scopes"] == ["volume:2"]
    assert idempotent["semantic_tags"] == ["new"]
    assert tagged["semantic_tags"] == ["ability", "ui"]
    assert merged["source"]["status"] == "merged"
    assert merged["source"]["merged_into_concept_id"] == target["id"]


def test_generic_concept_update_rejects_inactive_lifecycle_transitions(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    archived = mcp_server.hieronymus_concept_create("Archived", confidence=0.3)
    source = mcp_server.hieronymus_concept_create("Source", confidence=0.3)
    target = mcp_server.hieronymus_concept_create("Target", confidence=0.3)

    mcp_server.hieronymus_concept_archive(archived["id"], reason="Inactive.")
    mcp_server.hieronymus_concept_merge(source["id"], target["id"], reason="Merged.")

    with pytest.raises(ValueError, match="concept_update cannot set inactive status"):
        mcp_server.hieronymus_concept_update(target["id"], status="merged")
    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        mcp_server.hieronymus_concept_update(archived["id"], description="Reactivated?")
    with pytest.raises(ValueError, match="cannot mutate inactive concept"):
        mcp_server.hieronymus_concept_update(source["id"], status="candidate")


def test_inactive_concepts_reject_non_lifecycle_mutations(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    archived = mcp_server.hieronymus_concept_create("Archived", confidence=0.3)
    archived_facet = mcp_server.hieronymus_concept_facet_add(archived["id"], "Archived")
    source = mcp_server.hieronymus_concept_create("Source", confidence=0.3)
    target = mcp_server.hieronymus_concept_create("Target", confidence=0.3)
    mcp_server.hieronymus_concept_facet_add(source["id"], "Source")

    mcp_server.hieronymus_concept_archive(archived["id"], reason="Inactive.")
    mcp_server.hieronymus_concept_merge(source["id"], target["id"], reason="Merged.")
    with connect(load_config().database_path) as conn:
        merged_facet_id = conn.execute(
            """
            insert into concept_facets(
              concept_id,
              language,
              facet_type,
              value,
              confidence,
              created_at,
              updated_at
            )
            values (?, '', 'name', 'Merged-owned facet', 0.2, ?, ?)
            """,
            (
                source["id"],
                "2026-06-10T00:00:00+00:00",
                "2026-06-10T00:00:00+00:00",
            ),
        ).lastrowid
        conn.commit()

    blocked_calls = [
        lambda: mcp_server.hieronymus_concept_facet_add(archived["id"], "New"),
        lambda: mcp_server.hieronymus_concept_facet_add(source["id"], "New"),
        lambda: mcp_server.hieronymus_concept_facet_update(
            archived_facet["id"],
            value="Renamed",
        ),
        lambda: mcp_server.hieronymus_concept_facet_update(
            merged_facet_id,
            value="Merged Source",
        ),
        lambda: mcp_server.hieronymus_concept_facet_set_canonical(
            archived["id"],
            archived_facet["id"],
        ),
        lambda: mcp_server.hieronymus_concept_rename(archived["id"], "Renamed"),
        lambda: mcp_server.hieronymus_concept_rename(source["id"], "Renamed"),
        lambda: mcp_server.hieronymus_concept_semantic_tags_set(archived["id"], ["blocked"]),
        lambda: mcp_server.hieronymus_concept_semantic_tags_set(source["id"], ["blocked"]),
    ]

    for call in blocked_calls:
        with pytest.raises(ValueError, match="cannot mutate inactive concept"):
            call()


def test_rule_validation_requires_active_linked_concept(
    monkeypatch,
    tmp_path,
) -> None:
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    series = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    concept = mcp_server.hieronymus_concept_create(
        "センス",
        confidence=0.9,
        series_slug=series["slug"],
    )
    context = TranslationContext(
        series_slug=series["slug"],
        source_language=series["source_language"],
        target_language=series["target_language"],
        task_type="translation",
    )
    crystal_id = CrystalStore(load_config()).add_crystal(
        context,
        crystal_type="rule",
        text="センス is translated as Sense.",
        strength=0.9,
        confidence=0.9,
    )
    mcp_server.hieronymus_crystal_link_concept(crystal_id, concept["id"], confidence=0.9)
    mcp_server.hieronymus_concept_archive(concept["id"], reason="Inactive.")

    validation = mcp_server.hieronymus_rule_crystal_validate(crystal_id)

    assert validation["valid"] is True
    assert validation["enforceable"] is False
    assert validation["warnings"] == ["rule crystal is not linked to an active concept"]

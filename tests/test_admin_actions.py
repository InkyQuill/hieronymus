from __future__ import annotations

import json

import pytest

from hieronymus.admin import AdminStore
from hieronymus.concepts import CONCEPT_ESTABLISHED, ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def _context(config: HieronymusConfig) -> TranslationContext:
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
        task_type="translation",
        tags=("style", "sense"),
    )


def _add_crystal(
    config: HieronymusConfig,
    context: TranslationContext,
    *,
    title: str = "センス glossary",
    text: str = "Render センス as сенс in skill-system prose.",
    crystal_type: str = "lesson",
    strength: float = 0.5,
    confidence: float = 0.5,
    status: str = "active",
) -> int:
    return CrystalStore(config).add_crystal(
        context,
        crystal_type=crystal_type,
        title=title,
        text=text,
        strength=strength,
        confidence=confidence,
        status=status,
    )


def _audit_actions(config: HieronymusConfig) -> list[str]:
    with connect(config.database_path) as conn:
        rows = conn.execute("select action from audit_log order by id").fetchall()
    return [row["action"] for row in rows]


def _create_proposal(config: HieronymusConfig, context: TranslationContext) -> int:
    return ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug=context.series_slug,
        source_language=context.source_language,
        target_language=context.target_language,
        concept_text="センス",
        source_form="センス",
        canonical_rendering="сенс",
        approved_variants=[],
        forbidden_variants=["sense"],
        rationale="Use the established Russian rendering.",
    )


def test_reinforce_and_decay_crystal_update_scores_and_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(config, context, strength=0.5, confidence=0.5)
    admin = AdminStore(config)

    reinforce_result = admin.reinforce_crystal(crystal_id, evidence="センス is stable.")
    decay_result = admin.decay_crystal(crystal_id, evidence="сенс was rejected once.")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        events = conn.execute("select * from memory_events order by id").fetchall()
        audits = conn.execute("select * from audit_log order by id").fetchall()

    assert reinforce_result.action == "reinforce"
    assert decay_result.action == "decay"
    assert round(crystal["strength"], 2) == 0.45
    assert round(crystal["confidence"], 2) == 0.45
    assert [event["event_type"] for event in events] == [
        "confirmed_by_user",
        "contradicted_by_user",
    ]
    assert [audit["action"] for audit in audits] == ["reinforce", "decay"]
    assert [audit["entity_id"] for audit in audits] == [str(crystal_id), str(crystal_id)]
    assert [event["source_role"] for event in events] == ["user", "user"]
    assert [event["applied"] for event in events] == [1, 1]


def test_edit_deprecate_and_delete_crystal_refresh_status_fts_scores_and_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    edit_id = _add_crystal(config, context, title="Old", text="Old センス text.")
    archive_id = _add_crystal(config, context, title="Archive", text="Archive this.")
    delete_id = _add_crystal(config, context, title="Delete", text="Delete this.")
    admin = AdminStore(config)

    admin.edit_crystal(edit_id, title="Updated センス", text="Use сенс for センス.")
    admin.deprecate_crystal(archive_id, evidence="Superseded by later glossary.")
    admin.delete_crystal(delete_id, evidence="Bad memory.")

    with connect(config.database_path) as conn:
        edited = conn.execute("select * from crystals where id = ?", (edit_id,)).fetchone()
        edited_fts = conn.execute(
            "select * from crystals_fts where rowid = ?",
            (edit_id,),
        ).fetchone()
        archived = conn.execute("select * from crystals where id = ?", (archive_id,)).fetchone()
        deleted = conn.execute("select * from crystals where id = ?", (delete_id,)).fetchone()
        audits = conn.execute("select * from audit_log order by id").fetchall()

    assert edited["title"] == "Updated センス"
    assert edited["text"] == "Use сенс for センス."
    assert edited_fts["title"] == "Updated センス"
    assert edited_fts["text"] == "Use сенс for センス."
    assert archived["status"] == "archived"
    assert deleted["status"] == "archived"
    assert deleted["strength"] == 0
    assert deleted["confidence"] == 0
    assert [audit["action"] for audit in audits] == ["edit", "deprecate", "delete"]
    before = json.loads(audits[0]["before_json"])
    after = json.loads(audits[0]["after_json"])
    assert before["title"] == "Old"
    assert after["text"] == "Use сенс for センス."
    assert [row.id for row in CrystalStore(config).search(context, "Updated")] == [edit_id]


def test_approve_proposal_creates_advisory_concept_and_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    proposal_id = _create_proposal(config, context)

    concept_id = AdminStore(config).approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        term_count = conn.execute("select count(*) from strict_terms").fetchone()[0]
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]
        concept = conn.execute("select * from concepts where id = ?", (concept_id,)).fetchone()
        facets = conn.execute(
            "select value from concept_facets where concept_id = ? order by id", (concept_id,)
        ).fetchall()
        proposal = conn.execute(
            "select * from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        audits = conn.execute("select * from audit_log").fetchall()

    assert term_count == 0
    assert crystal_count == 0
    assert concept["canonical_name"] == "センス"
    assert concept["description"] == "Use the established Russian rendering."
    assert [facet["value"] for facet in facets] == ["センス", "сенс", "sense"]
    assert proposal["status"] == "approved"
    assert [audit["action"] for audit in audits] == ["approve"]
    assert audits[0]["entity_type"] == "strict_concept_proposal"
    assert audits[0]["entity_id"] == str(proposal_id)
    assert Termbase(config, context).contract("センス appears.") == []


def test_approve_proposal_creates_new_concept_when_duplicate_name_has_no_tag_match(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    concepts = ConceptStore(config)
    first = concepts.create_concept(
        "センス",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:first",),
    )
    second = concepts.create_concept(
        "センス",
        status=CONCEPT_ESTABLISHED,
        confidence=0.95,
        scope_type="series",
        scope_key=context.scope_key,
        semantic_tags=("role:second",),
    )
    proposal_id = _create_proposal(config, context)

    concept_id = AdminStore(config).approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        concept_count = conn.execute(
            """
            select count(*) as concept_count
            from concepts
            where canonical_name = 'センス'
              and scope_key = ?
            """,
            (context.scope_key,),
        ).fetchone()

    assert concept_id not in {first.id, second.id}
    assert concept_count["concept_count"] == 3


def test_approve_proposal_requires_pending_and_does_not_duplicate_concepts(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    proposal_id = _create_proposal(config, context)
    admin = AdminStore(config)

    concept_id = admin.approve_proposal(proposal_id)
    with pytest.raises(ValueError, match="proposal must be pending"):
        admin.approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        concepts = conn.execute("select id from concepts order by id").fetchall()
        term_count = conn.execute("select count(*) from strict_terms").fetchone()[0]
        proposal = conn.execute(
            "select status from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        audits = conn.execute("select action from audit_log order by id").fetchall()

    assert [row["id"] for row in concepts] == [concept_id]
    assert term_count == 0
    assert proposal["status"] == "approved"
    assert [row["action"] for row in audits] == ["approve"]


def test_approve_proposal_keeps_all_variant_forms_advisory(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    proposal_id = ConceptProposalStore(config).create(
        dream_run_id=None,
        series_slug=context.series_slug,
        source_language=context.source_language,
        target_language=context.target_language,
        concept_text="センス",
        source_form="センス",
        canonical_rendering="сенс",
        approved_variants=[],
        forbidden_variants=["sense", "Сенс"],
        rationale="Use the established Russian rendering.",
    )

    concept_id = AdminStore(config).approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        proposal = conn.execute(
            "select status from strict_concept_proposals where id = ?", (proposal_id,)
        ).fetchone()
        facets = conn.execute(
            "select value from concept_facets where concept_id = ? order by id", (concept_id,)
        ).fetchall()
        crystal_count = conn.execute("select count(*) from crystals").fetchone()[0]

    assert proposal["status"] == "approved"
    assert [facet["value"] for facet in facets] == ["センス", "сенс", "sense", "Сенс"]
    assert crystal_count == 0


def test_rejecting_approved_proposal_raises_without_mutating(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    proposal_id = _create_proposal(config, context)
    admin = AdminStore(config)
    admin.approve_proposal(proposal_id)

    with pytest.raises(ValueError, match="proposal must be pending"):
        admin.reject_proposal(proposal_id, evidence="Too late.")

    with connect(config.database_path) as conn:
        proposal = conn.execute(
            "select status from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        audits = conn.execute("select action from audit_log order by id").fetchall()

    assert proposal["status"] == "approved"
    assert [row["action"] for row in audits] == ["approve"]


def test_approving_rejected_proposal_raises_without_strict_term(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    proposal_id = _create_proposal(config, context)
    admin = AdminStore(config)
    admin.reject_proposal(proposal_id, evidence="Rejected by reviewer.")

    with pytest.raises(ValueError, match="proposal must be pending"):
        admin.approve_proposal(proposal_id)

    with connect(config.database_path) as conn:
        term_count = conn.execute("select count(*) from strict_terms").fetchone()[0]
        proposal = conn.execute(
            "select status from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        audits = conn.execute("select action from audit_log order by id").fetchall()

    assert term_count == 0
    assert proposal["status"] == "rejected"
    assert [row["action"] for row in audits] == ["reject"]


def test_admin_crystal_lifecycle_actions_update_links_status_scope_and_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    first_id = _add_crystal(config, context, title="First", text="First センス rule.")
    second_id = _add_crystal(config, context, title="Second", text="Second センス rule.")
    source_id = _add_crystal(config, context, title="Split", text="Split this lesson.")
    old_id = _add_crystal(config, context, title="Old", text="Old local lesson.")
    replacement_id = _add_crystal(config, context, title="New", text="New local lesson.")
    local_lesson_id = _add_crystal(config, context, title="Local", text="Promote センス.")
    admin = AdminStore(config)

    merged_id = admin.merge_crystals(
        [first_id, second_id],
        title="Merged センス",
        text="Merged rule for сенс.",
    )
    split_ids = admin.split_crystal(
        source_id,
        parts=[
            ("Part A", "Use сенс as noun."),
            ("Part B", "Inflect сенс normally."),
        ],
    )
    admin.supersede_crystal(old_id, replacement_id=replacement_id, evidence="Newer note.")
    promoted_id = admin.promote_local_lesson(local_lesson_id, evidence="Good global guidance.")
    admin.activate_global_lesson(promoted_id, evidence="Reviewed by admin.")

    with connect(config.database_path) as conn:
        old_rows = conn.execute(
            "select id, status from crystals where id in (?, ?, ?, ?)",
            (first_id, second_id, source_id, old_id),
        ).fetchall()
        merged = conn.execute("select * from crystals where id = ?", (merged_id,)).fetchone()
        split = conn.execute(
            "select * from crystals where id in (?, ?) order by id",
            tuple(split_ids),
        ).fetchall()
        promoted = conn.execute("select * from crystals where id = ?", (promoted_id,)).fetchone()
        links = conn.execute(
            """
            select source_crystal_id, target_crystal_id, link_type
            from crystal_links
            order by source_crystal_id, target_crystal_id, link_type
            """
        ).fetchall()

    assert {row["id"]: row["status"] for row in old_rows} == {
        first_id: "archived",
        second_id: "archived",
        source_id: "archived",
        old_id: "archived",
    }
    assert merged["title"] == "Merged センス"
    assert merged["status"] == "active"
    assert [row["title"] for row in split] == ["Part A", "Part B"]
    assert [row["status"] for row in split] == ["active", "active"]
    assert promoted["scope_type"] == "global"
    assert promoted["scope_key"] == "global"
    assert promoted["status"] == "active"
    assert [row.id for row in CrystalStore(config).search(context, "Merged")] == [merged_id]
    assert [row.id for row in CrystalStore(config).search(context, "noun")] == [split_ids[0]]
    assert promoted_id in [row.id for row in CrystalStore(config).search(context, "Promote")]
    assert (
        replacement_id,
        old_id,
        "supersedes",
    ) in [(row["source_crystal_id"], row["target_crystal_id"], row["link_type"]) for row in links]
    assert {
        (row["source_crystal_id"], row["target_crystal_id"], row["link_type"])
        for row in links
        if row["link_type"] in {"merged_from", "split_from"}
    } == {
        (merged_id, first_id, "merged_from"),
        (merged_id, second_id, "merged_from"),
        (split_ids[0], source_id, "split_from"),
        (split_ids[1], source_id, "split_from"),
    }
    assert _audit_actions(config) == ["merge", "split", "supersede", "promote", "activate"]


def test_supersede_rejects_self_reference_without_mutating_or_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(config, context, title="Active", text="Active lesson.")
    admin = AdminStore(config)

    with pytest.raises(ValueError, match="supersede itself"):
        admin.supersede_crystal(
            crystal_id,
            replacement_id=crystal_id,
            evidence="Bad replacement.",
        )

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        links = conn.execute("select * from crystal_links").fetchall()
        audits = conn.execute("select * from audit_log").fetchall()

    assert crystal["status"] == "active"
    assert links == []
    assert audits == []


def test_merge_rejects_duplicate_ids_without_creating_rows_or_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    crystal_id = _add_crystal(config, context, title="Active", text="Active lesson.")
    admin = AdminStore(config)

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) as count from crystals").fetchone()["count"]

    with pytest.raises(ValueError, match="distinct crystals"):
        admin.merge_crystals([crystal_id, crystal_id], title="Bad", text="Bad merge.")

    with connect(config.database_path) as conn:
        crystal = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        new_crystal_count = conn.execute("select count(*) as count from crystals").fetchone()[
            "count"
        ]
        links = conn.execute("select * from crystal_links").fetchall()
        audits = conn.execute("select * from audit_log").fetchall()

    assert crystal["status"] == "active"
    assert new_crystal_count == crystal_count
    assert links == []
    assert audits == []


def test_merge_rejects_mixed_duplicate_ids_without_creating_rows_or_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    first_id = _add_crystal(config, context, title="First", text="First lesson.")
    second_id = _add_crystal(config, context, title="Second", text="Second lesson.")
    admin = AdminStore(config)

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) as count from crystals").fetchone()["count"]

    with pytest.raises(ValueError, match="distinct crystals"):
        admin.merge_crystals([first_id, second_id, first_id], title="Bad", text="Bad merge.")

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, status from crystals where id in (?, ?) order by id",
            (first_id, second_id),
        ).fetchall()
        new_crystal_count = conn.execute("select count(*) as count from crystals").fetchone()[
            "count"
        ]
        links = conn.execute("select * from crystal_links").fetchall()
        audits = conn.execute("select * from audit_log").fetchall()

    assert {row["id"]: row["status"] for row in rows} == {
        first_id: "active",
        second_id: "active",
    }
    assert new_crystal_count == crystal_count
    assert links == []
    assert audits == []


@pytest.mark.parametrize("blocked_status", ["archived", "rejected"])
def test_merge_rejects_inactive_input_without_mutating_or_audit(
    config: HieronymusConfig,
    blocked_status: str,
) -> None:
    context = _context(config)
    active_id = _add_crystal(config, context, title="Active", text="Active lesson.")
    blocked_id = _add_crystal(
        config,
        context,
        title="Blocked",
        text="Blocked lesson.",
        status=blocked_status,
    )
    admin = AdminStore(config)

    with pytest.raises(ValueError, match="active or candidate"):
        admin.merge_crystals([active_id, blocked_id], title="Bad", text="Bad merge.")

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, status from crystals where id in (?, ?) order by id",
            (active_id, blocked_id),
        ).fetchall()
        links = conn.execute("select * from crystal_links").fetchall()

    assert {row["id"]: row["status"] for row in rows} == {
        active_id: "active",
        blocked_id: blocked_status,
    }
    assert links == []
    assert _audit_actions(config) == []


@pytest.mark.parametrize("blocked_status", ["archived", "rejected"])
def test_split_rejects_inactive_source_without_creating_rows_or_audit(
    config: HieronymusConfig,
    blocked_status: str,
) -> None:
    context = _context(config)
    source_id = _add_crystal(
        config,
        context,
        title="Blocked",
        text="Blocked lesson.",
        status=blocked_status,
    )
    admin = AdminStore(config)

    with connect(config.database_path) as conn:
        crystal_count = conn.execute("select count(*) as count from crystals").fetchone()["count"]
        link_count = conn.execute("select count(*) as count from crystal_links").fetchone()["count"]

    with pytest.raises(ValueError, match="active or candidate"):
        admin.split_crystal(
            source_id,
            parts=[
                ("Part A", "Use сенс as noun."),
                ("Part B", "Inflect сенс normally."),
            ],
        )

    with connect(config.database_path) as conn:
        source = conn.execute("select * from crystals where id = ?", (source_id,)).fetchone()
        new_crystal_count = conn.execute("select count(*) as count from crystals").fetchone()[
            "count"
        ]
        new_link_count = conn.execute("select count(*) as count from crystal_links").fetchone()[
            "count"
        ]

    assert source["status"] == blocked_status
    assert new_crystal_count == crystal_count
    assert new_link_count == link_count
    assert _audit_actions(config) == []


@pytest.mark.parametrize("blocked_status", ["archived", "rejected"])
def test_supersede_rejects_inactive_replacement_without_mutating_or_audit(
    config: HieronymusConfig,
    blocked_status: str,
) -> None:
    context = _context(config)
    old_id = _add_crystal(config, context, title="Old", text="Old lesson.")
    replacement_id = _add_crystal(
        config,
        context,
        title="Blocked replacement",
        text="Blocked replacement lesson.",
        status=blocked_status,
    )
    admin = AdminStore(config)

    with pytest.raises(ValueError, match="active or candidate"):
        admin.supersede_crystal(
            old_id,
            replacement_id=replacement_id,
            evidence="Bad replacement.",
        )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select id, status from crystals where id in (?, ?) order by id",
            (old_id, replacement_id),
        ).fetchall()
        links = conn.execute("select * from crystal_links").fetchall()

    assert {row["id"]: row["status"] for row in rows} == {
        old_id: "active",
        replacement_id: blocked_status,
    }
    assert links == []
    assert _audit_actions(config) == []


def test_merge_and_supersede_reject_scope_key_mismatch_without_mutating(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    merge_base_id = _add_crystal(config, context, title="Merge base", text="Base lesson.")
    merge_other_id = _add_crystal(config, context, title="Merge other", text="Other lesson.")
    supersede_old_id = _add_crystal(config, context, title="Old", text="Old lesson.")
    supersede_replacement_id = _add_crystal(config, context, title="New", text="New lesson.")
    admin = AdminStore(config)

    with connect(config.database_path) as conn:
        conn.execute(
            "update crystals set scope_key = ? where id in (?, ?)",
            ("chapter:01", merge_other_id, supersede_replacement_id),
        )
        conn.commit()

    with pytest.raises(ValueError, match="scope_key"):
        admin.merge_crystals([merge_base_id, merge_other_id], title="Bad", text="Bad merge.")
    with pytest.raises(ValueError, match="scope_key"):
        admin.supersede_crystal(
            supersede_old_id,
            replacement_id=supersede_replacement_id,
            evidence="Bad scope.",
        )

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select id, status
            from crystals
            where id in (?, ?, ?, ?)
            order by id
            """,
            (merge_base_id, merge_other_id, supersede_old_id, supersede_replacement_id),
        ).fetchall()
        links = conn.execute("select * from crystal_links").fetchall()

    assert {row["id"]: row["status"] for row in rows} == {
        merge_base_id: "active",
        merge_other_id: "active",
        supersede_old_id: "active",
        supersede_replacement_id: "active",
    }
    assert links == []
    assert _audit_actions(config) == []


def test_merge_and_supersede_reject_incompatible_contexts_without_mutating(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    other_series = Registry(config).create_series(
        slug="other-series",
        title="Other Series",
        source_language="ja",
        target_language="ru",
    )
    other_context = TranslationContext(
        series_slug=other_series.slug,
        source_language=other_series.source_language,
        target_language=other_series.target_language,
        task_type="translation",
    )
    admin = AdminStore(config)
    base_id = _add_crystal(config, context, title="Base", text="Base lesson.")
    other_id = _add_crystal(config, other_context, title="Other", text="Other lesson.")
    other_language_context = TranslationContext(
        series_slug=context.series_slug,
        source_language="en",
        target_language=context.target_language,
        task_type="translation",
    )
    other_language_id = _add_crystal(
        config,
        other_language_context,
        title="Other language",
        text="Other language lesson.",
    )
    concept_id = _add_crystal(
        config,
        context,
        title="Concept",
        text="Concept crystal.",
        crystal_type="concept",
    )

    with pytest.raises(ValueError, match="series_slug"):
        admin.merge_crystals([base_id, other_id], title="Bad", text="Bad merge.")
    with pytest.raises(ValueError, match="source_language"):
        admin.merge_crystals([base_id, other_language_id], title="Bad", text="Bad merge.")
    with pytest.raises(ValueError, match="crystal_type"):
        admin.supersede_crystal(base_id, replacement_id=concept_id, evidence="Bad replacement.")

    with connect(config.database_path) as conn:
        crystals = conn.execute(
            "select id, status from crystals where id in (?, ?, ?, ?) order by id",
            (base_id, other_id, other_language_id, concept_id),
        ).fetchall()
        links = conn.execute("select * from crystal_links").fetchall()
        audits = conn.execute("select * from audit_log").fetchall()

    assert {row["id"]: row["status"] for row in crystals} == {
        base_id: "active",
        other_id: "active",
        other_language_id: "active",
        concept_id: "active",
    }
    assert links == []
    assert audits == []


def test_lesson_scope_preconditions_raise_without_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    admin = AdminStore(config)
    concept_id = _add_crystal(
        config,
        context,
        title="Concept",
        text="Concept crystal.",
        crystal_type="concept",
    )
    local_lesson_id = _add_crystal(config, context, title="Local", text="Local lesson.")
    global_lesson_id = _add_crystal(config, context, title="Global", text="Global lesson.")
    archived_lesson_id = _add_crystal(
        config,
        context,
        title="Archived",
        text="Archived lesson.",
        status="archived",
    )
    with connect(config.database_path) as conn:
        conn.execute(
            """
            update crystals
            set scope_type = 'global',
                scope_key = 'global',
                series_slug = ''
            where id = ?
            """,
            (global_lesson_id,),
        )
        conn.commit()

    with pytest.raises(ValueError, match="source crystal must be a lesson"):
        admin.promote_local_lesson(concept_id, evidence="Bad promote.")
    with pytest.raises(ValueError, match="source lesson must be series-scoped"):
        admin.promote_local_lesson(global_lesson_id, evidence="Bad promote.")
    with pytest.raises(ValueError, match="source lesson must not be archived or rejected"):
        admin.promote_local_lesson(archived_lesson_id, evidence="Bad promote.")
    with pytest.raises(ValueError, match="lesson must be global-scoped"):
        admin.activate_global_lesson(local_lesson_id, evidence="Bad activate.")

    assert _audit_actions(config) == []


def test_activate_global_lesson_rejects_already_active_without_second_audit(
    config: HieronymusConfig,
) -> None:
    context = _context(config)
    lesson_id = _add_crystal(
        config,
        context,
        title="Candidate",
        text="Candidate global lesson.",
        status="candidate",
    )
    admin = AdminStore(config)
    with connect(config.database_path) as conn:
        conn.execute(
            """
            update crystals
            set scope_type = 'global',
                scope_key = 'global',
                series_slug = ''
            where id = ?
            """,
            (lesson_id,),
        )
        conn.commit()

    admin.activate_global_lesson(lesson_id, evidence="Reviewed.")
    with pytest.raises(ValueError, match="global lesson must be candidate"):
        admin.activate_global_lesson(lesson_id, evidence="Reviewed twice.")

    with connect(config.database_path) as conn:
        lesson = conn.execute("select * from crystals where id = ?", (lesson_id,)).fetchone()

    assert lesson["status"] == "active"
    assert _audit_actions(config) == ["activate"]

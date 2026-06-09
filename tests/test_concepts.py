import pytest

from hieronymus.concepts import ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.db import connect


def _valid_proposal(**overrides):
    values = {
        "dream_run_id": None,
        "series_slug": "only-sense-online",
        "source_language": "ja",
        "target_language": "ru",
        "concept_text": "Sense",
        "source_form": "センス",
        "canonical_rendering": "Сенс",
        "approved_variants": ["Сенс"],
        "forbidden_variants": ["Чувство"],
        "rationale": "User corrected this term during translation.",
    }
    values.update(overrides)
    return values


def test_concept_proposals_are_series_and_language_pair_scoped(
    config: HieronymusConfig,
) -> None:
    store = ConceptProposalStore(config)

    first_id = store.create(**_valid_proposal())
    second_id = store.create(
        **_valid_proposal(
            series_slug="another-series",
            source_language="en",
            target_language="fr",
            concept_text="Workbench",
            source_form="workbench",
            canonical_rendering="etabli",
            approved_variants=["etabli"],
            forbidden_variants=["atelier"],
        )
    )

    first = store.get(first_id)
    second = store.get(second_id)

    assert first.series_slug == "only-sense-online"
    assert first.source_language == "ja"
    assert first.target_language == "ru"
    assert first.concept_text == "Sense"
    assert first.approved_variants == ["Сенс"]
    assert first.forbidden_variants == ["Чувство"]
    assert first.status == "pending"
    assert second.series_slug == "another-series"
    assert second.source_language == "en"
    assert second.target_language == "fr"


def test_list_pending_returns_only_pending_ordered_by_id(config: HieronymusConfig) -> None:
    store = ConceptProposalStore(config)
    first_id = store.create(**_valid_proposal(concept_text="First", source_form="one"))
    rejected_id = store.create(**_valid_proposal(concept_text="Rejected", source_form="two"))
    third_id = store.create(**_valid_proposal(concept_text="Third", source_form="three"))
    approved_id = store.create(**_valid_proposal(concept_text="Approved", source_form="four"))
    store.reject(rejected_id)
    store.approve(approved_id)

    pending = store.list_pending()

    assert [proposal.id for proposal in pending] == [first_id, third_id]
    assert [proposal.status for proposal in pending] == ["pending", "pending"]


def test_list_pending_includes_vague_concepts_with_facet_suggestions(
    config: HieronymusConfig,
) -> None:
    concept_id = ConceptStore(config).create_or_reinforce(
        "Sense",
        description="A game-like aptitude category.",
        confidence_delta=0.2,
        scope_type="project",
        scope_key="only-sense-online",
    )
    with connect(config.database_path) as conn:
        conn.execute(
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
            values (?, 'ru', 'translation', 'Сенс', 0.4, ?, ?)
            """,
            (
                concept_id,
                "2026-06-09T00:00:00+00:00",
                "2026-06-09T00:00:00+00:00",
            ),
        )
        conn.commit()

    pending = ConceptProposalStore(config).list_pending()

    assert len(pending) == 1
    assert pending[0].id == concept_id
    assert pending[0].series_slug == "only-sense-online"
    assert pending[0].concept_text == "Sense"
    assert pending[0].source_form == "Sense"
    assert pending[0].canonical_rendering == "Сенс"
    assert pending[0].approved_variants == ["Сенс"]
    assert pending[0].forbidden_variants == []
    assert pending[0].rationale == "A game-like aptitude category."
    assert pending[0].status == "vague"


def test_approve_and_reject_only_change_status(config: HieronymusConfig) -> None:
    store = ConceptProposalStore(config)
    approved_id = store.create(**_valid_proposal(concept_text="Approved", source_form="one"))
    rejected_id = store.create(**_valid_proposal(concept_text="Rejected", source_form="two"))

    with connect(config.database_path) as conn:
        before_approved = dict(
            conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (approved_id,),
            ).fetchone()
        )
        before_rejected = dict(
            conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (rejected_id,),
            ).fetchone()
        )

    store.approve(approved_id)
    store.reject(rejected_id)

    with connect(config.database_path) as conn:
        after_approved = dict(
            conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (approved_id,),
            ).fetchone()
        )
        after_rejected = dict(
            conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (rejected_id,),
            ).fetchone()
        )
        hard_term_count = conn.execute("select count(*) from strict_terms").fetchone()[0]

    for before, after, expected_status in (
        (before_approved, after_approved, "approved"),
        (before_rejected, after_rejected, "rejected"),
    ):
        changed = {key for key in before if before[key] != after[key]}
        assert changed <= {"status", "updated_at"}
        assert after["status"] == expected_status

    assert hard_term_count == 0


def test_get_unknown_proposal_raises_key_error(config: HieronymusConfig) -> None:
    store = ConceptProposalStore(config)

    with pytest.raises(KeyError, match="unknown concept proposal"):
        store.get(99)


@pytest.mark.parametrize(
    "field",
    [
        "source_language",
        "target_language",
        "concept_text",
        "source_form",
        "canonical_rendering",
    ],
)
def test_create_validates_required_fields_and_variant_types(
    config: HieronymusConfig,
    field: str,
) -> None:
    store = ConceptProposalStore(config)

    with pytest.raises(ValueError, match=field):
        store.create(**_valid_proposal(**{field: "   "}))

    with pytest.raises(ValueError, match="approved_variants"):
        store.create(**_valid_proposal(approved_variants=["Сенс", 42]))

    with pytest.raises(ValueError, match="forbidden_variants"):
        store.create(**_valid_proposal(forbidden_variants="Чувство"))

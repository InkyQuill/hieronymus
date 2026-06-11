import pytest

from hieronymus.concept_models import ConceptRecord


def test_concept_record_accepts_global_scope_without_key() -> None:
    concept = ConceptRecord(
        id=1,
        canonical_name="Sense",
        description="A game-like aptitude category.",
        status="candidate",
        confidence=0.2,
    )

    assert concept.scope_type == "global"
    assert concept.scope_key == ""


def test_concept_record_rejects_global_scope_with_key() -> None:
    with pytest.raises(ValueError, match="global concept scope requires an empty key"):
        ConceptRecord(
            id=1,
            canonical_name="Sense",
            description="A game-like aptitude category.",
            status="candidate",
            confidence=0.2,
            scope_type="global",
            scope_key="oso",
        )


def test_concept_record_rejects_non_global_scope_without_key() -> None:
    with pytest.raises(ValueError, match="non-global concept scope requires a key"):
        ConceptRecord(
            id=1,
            canonical_name="Sense",
            description="A game-like aptitude category.",
            status="candidate",
            confidence=0.2,
            scope_type="project",
            scope_key="",
        )

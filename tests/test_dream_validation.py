import pytest

from hieronymus.dream_validation import (
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    NormalizedDreamOutput,
    normalize_output,
    validate_normalized_output,
)
from hieronymus.memory_models import TranslationContext


def _context() -> TranslationContext:
    return TranslationContext(
        series_slug="story",
        source_language="en",
        target_language="ru",
        task_type="translate",
    )


def test_normalize_output_preserves_typed_provider_contract() -> None:
    output = DreamOutput(
        crystals=[
            DreamCrystalCandidate(
                crystal_type="rule",
                title="Name",
                text="Keep the approved name.",
                strength=0.8,
                confidence=0.9,
                source_memory_ids=[7],
                source_credibility="user_rule",
                rule_intent="strict terminology",
            )
        ],
        concept_proposals=[
            DreamConceptProposal(
                series_slug="story",
                source_language="en",
                target_language="ru",
                concept_text="Name",
                source_form="Name",
                canonical_rendering="Имя",
            )
        ],
    )

    normalized = normalize_output(
        output,
        _context(),
        allowed_memory_ids={7},
        valid_concept_ids=set(),
    )

    assert normalized.crystals[0].source_memory_ids == [7]
    assert normalized.crystals[0].rule_intent == "strict terminology"
    assert normalized.concept_proposals == output.concept_proposals


def test_normalize_output_recovers_malformed_candidate_and_filters_provenance() -> None:
    normalized = normalize_output(
        {
            "rule_crystals": [
                {
                    "content": "  Keep   this form.  ",
                    "source_memory_ids": [7, 99],
                    "concept_ids": [3, 404],
                    "source_credibility": "user_rule",
                }
            ]
        },
        _context(),
        allowed_memory_ids={7},
        valid_concept_ids={3},
    )

    candidate = normalized.crystals[0]
    assert candidate.text == "Keep this form."
    assert candidate.source_memory_ids == [7]
    assert candidate.concept_ids == (3,)
    assert candidate.crystal_type == "rule"
    assert normalized.warnings


def test_validate_normalized_output_keeps_rule_and_scope_invariants() -> None:
    normalized = NormalizedDreamOutput(
        concept_proposals=[
            DreamConceptProposal(
                series_slug="other",
                source_language="en",
                target_language="ru",
                concept_text="Name",
                source_form="Name",
                canonical_rendering="Имя",
            )
        ],
    )

    with pytest.raises(ValueError, match="proposal series_slug must match context"):
        validate_normalized_output(normalized, _context(), {7})

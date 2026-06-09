import pytest

from hieronymus.memory_models import (
    CrystalRecord,
    RecallResult,
    ShortTermMemoryRecord,
    TranslationContext,
)


def test_translation_context_defaults_to_series_scope() -> None:
    context = TranslationContext(
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        task_type="translate",
    )

    assert context.scope_key == "series:only-sense-online"


def test_short_term_memory_record_metadata_default_is_not_shared() -> None:
    first = ShortTermMemoryRecord(
        id=1,
        session_id=10,
        source_role="agent",
        kind="note",
        text="Keep tone dry.",
        source_ref="chapter-1",
    )
    second = ShortTermMemoryRecord(
        id=2,
        session_id=10,
        source_role="agent",
        kind="note",
        text="Preserve speaker register.",
        source_ref="chapter-2",
    )

    first.metadata["tone"] = "dry"

    assert second.metadata == {}


def test_crystal_record_defaults_multilingual_memory_fields() -> None:
    crystal = CrystalRecord(
        id=1,
        crystal_type="lesson",
        text="Keep UI labels terse.",
        title="UI labels",
        scope_type="series",
        scope_key="series:only-sense-online",
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        strength=0.5,
        confidence=0.8,
        status="active",
    )

    assert crystal.source_credibility == "observation"
    assert crystal.rule_intent == ""
    assert crystal.malformed_penalty == 0.0
    assert crystal.supersedes_crystal_id is None
    assert crystal.story_scopes == ()
    assert crystal.semantic_tags == ()
    assert crystal.concept_ids == ()


def test_crystal_record_side_table_fields_default_empty_before_store_hydration() -> None:
    crystal = CrystalRecord(
        id=1,
        crystal_type="lesson",
        text="Keep UI labels terse.",
        title="UI labels",
        scope_type="series",
        scope_key="series:only-sense-online",
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        strength=0.5,
        confidence=0.8,
        status="active",
    )

    assert crystal.story_scopes == ()
    assert crystal.semantic_tags == ()
    assert crystal.concept_ids == ()


def test_recall_result_long_term_requires_crystal_payload() -> None:
    crystal = CrystalRecord(
        id=1,
        crystal_type="lesson",
        text="Keep UI labels terse.",
        title="UI labels",
        scope_type="series",
        scope_key="series:only-sense-online",
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        strength=0.5,
        confidence=0.8,
        status="active",
    )

    result = RecallResult.long_term(
        crystal,
        rank=1,
        score=0.75,
        reason="weighted search match",
    )

    assert result.source == "long_term"
    assert result.crystal == crystal
    assert result.short_term_memory is None


def test_recall_result_short_term_requires_memory_payload() -> None:
    memory = ShortTermMemoryRecord(
        id=1,
        session_id=10,
        source_role="agent",
        kind="note",
        text="The protagonist speaks tersely.",
        source_ref="chapter-1",
    )

    result = RecallResult.short_term(
        memory,
        rank=1,
        score=0.5,
        reason="recent memory match",
    )

    assert result.source == "short_term"
    assert result.crystal is None
    assert result.short_term_memory == memory


def test_recall_result_rejects_missing_payload() -> None:
    with pytest.raises(ValueError, match="long_term recall results require a crystal"):
        RecallResult(source="long_term", rank=1, score=0.5, reason="missing payload")


def test_recall_result_rejects_both_payloads() -> None:
    crystal = CrystalRecord(
        id=1,
        crystal_type="lesson",
        text="Keep UI labels terse.",
        title="UI labels",
        scope_type="series",
        scope_key="series:only-sense-online",
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        strength=0.5,
        confidence=0.8,
        status="active",
    )
    memory = ShortTermMemoryRecord(
        id=1,
        session_id=10,
        source_role="agent",
        kind="note",
        text="The protagonist speaks tersely.",
        source_ref="chapter-1",
    )

    with pytest.raises(ValueError, match="must not include short-term memory"):
        RecallResult(
            source="long_term",
            rank=1,
            score=0.5,
            reason="ambiguous payload",
            crystal=crystal,
            short_term_memory=memory,
        )


def test_recall_result_rejects_source_payload_mismatch() -> None:
    crystal = CrystalRecord(
        id=1,
        crystal_type="lesson",
        text="Keep UI labels terse.",
        title="UI labels",
        scope_type="series",
        scope_key="series:only-sense-online",
        series_slug="only-sense-online",
        source_language="ja",
        target_language="ru",
        strength=0.5,
        confidence=0.8,
        status="active",
    )

    with pytest.raises(ValueError, match="short_term recall results must not include a crystal"):
        RecallResult(
            source="short_term",
            rank=1,
            score=0.5,
            reason="mismatched payload",
            crystal=crystal,
        )


def test_recall_result_rejects_short_term_missing_payload() -> None:
    with pytest.raises(ValueError, match="short_term recall results require short-term memory"):
        RecallResult(source="short_term", rank=1, score=0.5, reason="missing payload")


def test_recall_result_rejects_unknown_source() -> None:
    with pytest.raises(ValueError, match="unknown recall source"):
        RecallResult(source="semantic", rank=1, score=0.5, reason="unknown source")

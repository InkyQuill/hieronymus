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


def test_recall_result_can_represent_long_term_crystal() -> None:
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

    result = RecallResult(
        crystal=crystal,
        rank=1,
        score=0.75,
        reason="weighted search match",
    )

    assert result.source == "long_term"
    assert result.crystal == crystal
    assert result.short_term_memory is None


def test_recall_result_can_represent_short_term_memory() -> None:
    memory = ShortTermMemoryRecord(
        id=1,
        session_id=10,
        source_role="agent",
        kind="note",
        text="The protagonist speaks tersely.",
        source_ref="chapter-1",
    )

    result = RecallResult(
        source="short_term",
        rank=1,
        score=0.5,
        reason="recent memory match",
        short_term_memory=memory,
    )

    assert result.crystal is None
    assert result.short_term_memory == memory

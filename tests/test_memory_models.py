from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext


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

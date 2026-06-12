import pytest

from hieronymus.ingest_config import ShortMemoryLimits
from hieronymus.short_memory import validate_short_memory_text


def test_short_memory_accepts_one_to_six_sentences_without_warning() -> None:
    validation = validate_short_memory_text("One. Two! Three? Four. Five. Six.")

    assert validation.ok is True
    assert validation.warning == ""
    assert validation.sentence_count == 6


def test_short_memory_warns_above_preferred_sentence_count() -> None:
    validation = validate_short_memory_text("One. Two. Three. Four. Five. Six. Seven.")

    assert validation.ok is True
    assert validation.warning == "short-term memory is large; prefer 1-6 sentences"
    assert validation.sentence_count == 7


def test_short_memory_warns_above_configured_symbol_count() -> None:
    validation = validate_short_memory_text(
        "x" * 13,
        limits=ShortMemoryLimits(warning_symbol_count=12),
    )

    assert validation.ok is True
    assert validation.warning == "short-term memory is large; prefer <= 12 symbols"
    assert validation.symbol_count == 13


def test_short_memory_rejects_above_hard_sentence_count() -> None:
    text = " ".join(f"Sentence {index}." for index in range(31))

    with pytest.raises(ValueError, match="short-term memory is too large"):
        validate_short_memory_text(text)


def test_short_memory_rejects_above_configured_symbol_count() -> None:
    with pytest.raises(ValueError, match="short-term memory exceeds 12 symbols"):
        validate_short_memory_text(
            "x" * 13,
            limits=ShortMemoryLimits(rejection_symbol_count=12),
        )


def test_short_memory_rejects_empty_text() -> None:
    with pytest.raises(ValueError, match="short-term memory text must not be empty"):
        validate_short_memory_text("   ")


def test_short_memory_without_terminator_counts_as_one_sentence() -> None:
    validation = validate_short_memory_text("No sentence terminator here")

    assert validation.sentence_count == 1


def test_short_memory_counts_japanese_sentence_punctuation() -> None:
    validation = validate_short_memory_text("一つ。二つ！三つ？")

    assert validation.sentence_count == 3

from __future__ import annotations

import re
from dataclasses import dataclass

from hieronymus.ingest_config import ShortMemoryLimits, default_ingest_config

_SENTENCE_TERMINATOR_RE = re.compile(r"[.!?。！？]+")
_LARGE_MEMORY_WARNING = "short-term memory is large; prefer 1-6 sentences"


@dataclass(frozen=True)
class ShortMemoryValidation:
    ok: bool
    warning: str
    sentence_count: int
    symbol_count: int


def validate_short_memory_text(
    text: str,
    *,
    limits: ShortMemoryLimits | None = None,
) -> ShortMemoryValidation:
    limits = limits or default_ingest_config().short_memory
    stripped = text.strip()
    if not stripped:
        raise ValueError("short-term memory text must not be empty")

    sentence_count = _count_sentences(stripped)
    symbol_count = len(stripped)
    if sentence_count > limits.rejection_sentence_count:
        raise ValueError("short-term memory is too large")
    if limits.rejection_symbol_count and symbol_count > limits.rejection_symbol_count:
        raise ValueError(f"short-term memory exceeds {limits.rejection_symbol_count} symbols")

    warning = ""
    warnings: list[str] = []
    if sentence_count > limits.warning_sentence_count:
        warnings.append(_LARGE_MEMORY_WARNING)
    if limits.warning_symbol_count and symbol_count > limits.warning_symbol_count:
        warnings.append(
            f"short-term memory is large; prefer <= {limits.warning_symbol_count} symbols",
        )
    if warnings:
        warning = "; ".join(warnings)

    return ShortMemoryValidation(
        ok=True,
        warning=warning,
        sentence_count=sentence_count,
        symbol_count=symbol_count,
    )


def _count_sentences(text: str) -> int:
    matches = list(_SENTENCE_TERMINATOR_RE.finditer(text))
    if not matches:
        return 1

    count = len(matches)
    if text[matches[-1].end() :].strip():
        count += 1
    return count

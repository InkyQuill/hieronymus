from __future__ import annotations

import re
from dataclasses import dataclass

_SENTENCE_TERMINATOR_RE = re.compile(r"[.!?。！？]+")
_PREFERRED_MAX_SENTENCES = 6
_HARD_MAX_SENTENCES = 30
_LARGE_MEMORY_WARNING = "short-term memory is large; prefer 1-6 sentences"


@dataclass(frozen=True)
class ShortMemoryValidation:
    ok: bool
    warning: str
    sentence_count: int


def validate_short_memory_text(text: str) -> ShortMemoryValidation:
    stripped = text.strip()
    if not stripped:
        raise ValueError("short-term memory text must not be empty")

    sentence_count = _count_sentences(stripped)
    if sentence_count > _HARD_MAX_SENTENCES:
        raise ValueError("short-term memory is too large")

    warning = ""
    if sentence_count > _PREFERRED_MAX_SENTENCES:
        warning = _LARGE_MEMORY_WARNING

    return ShortMemoryValidation(ok=True, warning=warning, sentence_count=sentence_count)


def _count_sentences(text: str) -> int:
    matches = list(_SENTENCE_TERMINATOR_RE.finditer(text))
    if not matches:
        return 1

    count = len(matches)
    if text[matches[-1].end() :].strip():
        count += 1
    return count

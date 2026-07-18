from __future__ import annotations

import json
import math
from collections.abc import Iterable
from datetime import UTC, datetime


def utc_now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def clamp_score(value: float) -> float:
    if math.isnan(value):
        raise ValueError("score must not be NaN")
    return min(max(value, 0.0), 1.0)


def normalize_string_tuple(
    values: Iterable[str],
    *,
    lowercase: bool = False,
) -> tuple[str, ...]:
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        item = value.strip()
        if lowercase:
            item = item.lower()
        if not item or item in seen:
            continue
        seen.add(item)
        normalized.append(item)
    return tuple(normalized)


def json_object(raw: str) -> dict[str, object]:
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError("expected JSON object")
    return value

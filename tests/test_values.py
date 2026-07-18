from __future__ import annotations

import math
from datetime import datetime

import pytest

from hieronymus.values import clamp_score, json_object, normalize_string_tuple, utc_now


def test_utc_now_returns_a_parseable_utc_timestamp_with_z_suffix() -> None:
    value = utc_now()

    assert value.endswith("Z")
    assert datetime.fromisoformat(value).utcoffset().total_seconds() == 0


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        (-math.inf, 0.0),
        (-0.25, 0.0),
        (0.4, 0.4),
        (1.25, 1.0),
        (math.inf, 1.0),
    ],
)
def test_clamp_score_normalizes_values_to_finite_bounds(value: float, expected: float) -> None:
    assert clamp_score(value) == expected


def test_clamp_score_rejects_nan() -> None:
    with pytest.raises(ValueError, match="score must not be NaN"):
        clamp_score(math.nan)


def test_normalize_string_tuple_preserves_order_and_case_sensitive_uniqueness() -> None:
    assert normalize_string_tuple((" Alpha ", "alpha", "Alpha", "", " beta ")) == (
        "Alpha",
        "alpha",
        "beta",
    )


def test_normalize_string_tuple_can_lowercase_before_deduplicating() -> None:
    assert normalize_string_tuple((" EN ", "en", " Ru "), lowercase=True) == ("en", "ru")


def test_json_object_returns_decoded_object() -> None:
    assert json_object('{"answer": 42}') == {"answer": 42}


@pytest.mark.parametrize("raw", ['["not", "an", "object"]', '"scalar"', "42", "null"])
def test_json_object_rejects_non_object_json(raw: str) -> None:
    with pytest.raises(ValueError, match="expected JSON object"):
        json_object(raw)

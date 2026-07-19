from __future__ import annotations

from typing import Any


def parse_bool(field_name: str, raw: str) -> bool:
    value = raw.strip().lower()
    if value in {"yes", "true", "1", "on"}:
        return True
    if value in {"no", "false", "0", "off"}:
        return False
    raise ValueError(f"{field_name} must be yes or no")


def parse_int(field_name: str, raw: str) -> int:
    try:
        return int(raw.strip())
    except ValueError as error:
        raise ValueError(f"{field_name} must be an integer") from error


def parse_float(field_name: str, raw: str) -> float:
    try:
        return float(raw.strip())
    except ValueError as error:
        raise ValueError(f"{field_name} must be a number") from error


def yes_no(value: bool) -> str:
    return "yes" if value else "no"


def field_value(value: Any) -> str:
    if isinstance(value, bool):
        return yes_no(value)
    if value is None:
        return ""
    return str(value)


def parse_positive_int(field_name: str, raw: str) -> int:
    value = parse_int(field_name, raw)
    if value < 1:
        raise ValueError(f"{field_name} must be at least 1")
    return value

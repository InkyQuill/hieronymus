from __future__ import annotations

from dataclasses import fields, is_dataclass
from typing import Any


def dataclass_to_json(value: Any) -> Any:
    if is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: dataclass_to_json(getattr(value, field.name))
            for field in fields(value)
            if not field.name.startswith("_")
        }
    if isinstance(value, tuple | list):
        return [dataclass_to_json(item) for item in value]
    if isinstance(value, dict):
        return {str(key): dataclass_to_json(item) for key, item in value.items()}
    return value

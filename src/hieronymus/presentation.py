from __future__ import annotations

import json
from importlib.metadata import PackageNotFoundError, version
from typing import Any

GREETING_ICON = "🪶"
STATUS_ICON = "📜"
GUIDE_ICON = "📖"
TAGLINE = "Remembers things for you."


def package_version() -> str:
    try:
        return version("hieronymus")
    except PackageNotFoundError:
        return "0.1.0"


def render_greeting(app_version: str | None = None) -> str:
    resolved_version = app_version if app_version is not None else package_version()
    return f"{GREETING_ICON} Hieronymus v{resolved_version}\n{TAGLINE}"


def render_json(payload: dict[str, Any] | list[Any]) -> str:
    return json.dumps(payload, ensure_ascii=False, sort_keys=True)


def render_pretty_json(payload: dict[str, Any] | list[Any]) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True)
